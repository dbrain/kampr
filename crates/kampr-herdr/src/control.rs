use crate::locate::{self, Search};
use anyhow::{Context, Result, anyhow};
use std::path::Path;
use std::time::Duration;
use tokio::io::AsyncWriteExt;
use tokio::process::{Child, ChildStdin, Command};

/// How long a release is given to land before the controller is killed instead.
///
/// [`Controller::release`] asks politely first because a clean `terminal.release` is what restores
/// an attached desk's geometry inside a second (#19). But a controller that has *stopped* holds the
/// PTY for ever — herdr never reclaims it (#20) — so the polite path can never be the only one.
/// This is the timeout that `docs/01-implementation-findings.md` calls the one thing Kampr has to
/// build itself.
const RELEASE_GRACE: Duration = Duration::from_secs(3);

/// The ceiling on a held controller, counted from the claim.
///
/// A hold is released when the operator closes the panel, and a client that dies with it open never
/// sends that. Without a ceiling the pane keeps the controller's geometry and the desk is ignored
/// (#18) until the node restarts. Generous enough to adjust a pane unhurriedly, short enough that a
/// forgotten hold is a nuisance rather than a wedge.
pub const HOLD_LIMIT: Duration = Duration::from_secs(600);

/// A `herdr terminal session control` child, which is the only instrument that can change a pane's
/// PTY size — `stty` inside the pane moves the kernel winsize only and herdr goes on wrapping at
/// its own grid width (#221), and nothing on the socket API sets a column count at all.
///
/// Claiming one is not free and the cost is the reason this crate went without it for so long:
/// `control` **always** takes the PTY, with no flag to decline (#17), it overrides the desk while
/// held (#18), and against an attached desk TUI it neither refuses nor evicts — the desk simply
/// renders wrong until it is let go (#298). So it is only ever claimed for an operator who asked
/// for it by name, and it is always released.
pub struct Controller {
    child: Child,
    stdin: ChildStdin,
}

impl Controller {
    /// Claims the PTY at `cols`x`rows`.
    ///
    /// The size goes on the command line rather than through a `terminal.resize` afterwards
    /// because a controller with no size flags forces the pane to 120x40 on the way in (#17) —
    /// asking for the size we want is what stops the claim itself being a resize to the wrong one.
    pub async fn claim(herdr_bin: &str, socket: &Path, pane_id: &str, cols: u32, rows: u32) -> Result<Self> {
        let herdr = locate::locate(herdr_bin, &Search::from_env())?;
        let mut child = Command::new(&herdr.path)
            .args(["terminal", "session", "control", pane_id])
            .args(["--cols", &cols.to_string(), "--rows", &rows.to_string()])
            .env("HERDR_SOCKET_PATH", socket)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            // The whole message, the way `Observer::spawn` learned to: anyhow's `Display` is the
            // outermost context alone, so a context line here throws away the io error that is the
            // actual diagnosis.
            .map_err(|e| {
                anyhow!(
                    "spawning `{} terminal session control`: {e}",
                    herdr.path.display()
                )
            })?;

        let stdin = child.stdin.take().context("control child had no stdin")?;
        Ok(Self { child, stdin })
    }

    /// Hands the PTY back, and does not trust the asking.
    ///
    /// On a pane with a desk client attached this is what puts the operator's geometry back (#19);
    /// on a headless one there is no desk to restore from and the size simply stays where it was
    /// put (#219). Either way the child must actually go, so a release that does not land inside
    /// [`RELEASE_GRACE`] becomes a kill — that is the whole answer to #20.
    pub async fn release(mut self) -> Result<()> {
        let line = serde_json::json!({ "type": "terminal.release" });
        let asked = self
            .stdin
            .write_all(format!("{line}\n").as_bytes())
            .await
            .and(self.stdin.flush().await);
        // Dropping stdin closes the pipe, which is the other half of asking: a controller reading
        // its stdin sees EOF even if the write above never landed.
        drop(self.stdin);

        match tokio::time::timeout(RELEASE_GRACE, self.child.wait()).await {
            Ok(Ok(_)) => Ok(()),
            Ok(Err(e)) => Err(anyhow!("waiting for the control child: {e}")),
            Err(_) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                match asked {
                    Ok(()) => Err(anyhow!(
                        "the control child ignored terminal.release for {}s and was killed",
                        RELEASE_GRACE.as_secs()
                    )),
                    Err(e) => Err(anyhow!("could not ask the control child to release: {e}")),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The same diagnosis `Observer::spawn` owes: a control child that cannot start has to say
    /// which binary and what stopped it, or a resize that never happened looks like a resize that
    /// was refused.
    #[tokio::test]
    async fn a_claim_that_fails_says_which_binary_and_what_stopped_it() {
        let dir = tempfile::tempdir().expect("a dir");
        let missing = dir.path().join("no-such-herdr");
        let claimed = Controller::claim(
            &missing.display().to_string(),
            &missing.with_file_name("herdr.sock"),
            "w1:p1",
            200,
            50,
        )
        .await;
        let Err(error) = claimed else {
            panic!("a missing binary claimed something");
        };
        let said = error.to_string();
        assert!(said.contains(&missing.display().to_string()), "{said}");
        assert!(said.contains("not an executable file"), "{said}");
    }

    /// A controller that will not go is the one failure mode herdr cannot recover from on its own
    /// (#20), so the release path is timed and ends in a kill. `sleep` stands in for a control
    /// child that reads nothing: it holds its stdin open and ignores everything written to it.
    #[tokio::test]
    async fn a_controller_that_ignores_release_is_killed_rather_than_left_holding_the_pty() {
        let mut child = Command::new("sleep")
            .arg("600")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("sleep");
        let stdin = child.stdin.take().expect("stdin");
        let controller = Controller { child, stdin };

        let started = std::time::Instant::now();
        let outcome = controller.release().await;
        let took = started.elapsed();

        assert!(
            outcome.is_err(),
            "a controller that never exits is not a clean release"
        );
        assert!(
            took >= RELEASE_GRACE && took < RELEASE_GRACE + Duration::from_secs(5),
            "release waited {took:?}, which is not the {RELEASE_GRACE:?} grace",
        );
        assert!(
            outcome.unwrap_err().to_string().contains("killed"),
            "the operator has to be told the pane was taken back by force",
        );
    }

    /// The ordinary path: a child that exits when its stdin closes is released, not killed, and
    /// says so.
    #[tokio::test]
    async fn a_controller_that_lets_go_is_a_clean_release() {
        let mut child = Command::new("cat")
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .expect("cat");
        let stdin = child.stdin.take().expect("stdin");
        let controller = Controller { child, stdin };
        assert!(controller.release().await.is_ok(), "cat exits on EOF");
    }
}
