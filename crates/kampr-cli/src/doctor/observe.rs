//! The binary half of herdr, which the socket half says nothing about.
//!
//! A node reaches herdr two ways: the socket, for the herd and for input, and a spawned
//! `herdr terminal session observe`, which is the entire grid stream. The unit pins
//! `HERDR_SOCKET_PATH`, so the socket half works under a service manager whatever the
//! environment; the binary half was resolved through a `PATH` the service does not have. The
//! result passed every check this command made and showed a blank grid in every client.

use super::{Check, Status};
use crate::service;
use kampr_herdr::locate::{Found, NotFound, Origin, Search, locate};
use kampr_herdr::{Herdr, Observer, StreamEvent};
use kampr_node::Config;
use std::path::Path;
use std::time::Duration;
use tokio::process::Command;

/// Long enough for a cold page-in of a 20 MB binary, short enough that `kampr doctor` still
/// answers on a wedged host.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// The same budget as `--version`, and for the same reason. A stream opens with a full frame of
/// its own (#53), so nothing has to happen inside the pane for one to arrive — a wait this long
/// with nothing on it is a stream that is not coming.
const FRAME_TIMEOUT: Duration = Duration::from_secs(5);

const SNAPSHOT_TIMEOUT: Duration = Duration::from_secs(2);

/// What `terminal session observe` did when it was actually run.
///
/// `--version` proves the file is there and executes. It does not prove the subcommand exists,
/// that this build speaks the record format, or that the child can open the socket — and a herdr
/// that resolves, runs and cannot observe is exactly the node of #233: a correct herd, input
/// accepted, and every pane blank for ever.
#[derive(Debug)]
enum Stream {
    Framed(String),
    Silent(String),
    Unchecked(String),
}

pub async fn check(config: &Config, socket: &Path, installed: bool) -> Check {
    let binary = config.herdr.binary.as_str();
    let here = locate(binary, &Search::from_env());
    // `Some(None)`: a unit is installed and its manager could not be asked what PATH it will hand
    // the node — which is not the same as there being no unit.
    let manager = installed.then(service::manager_path);
    let under = manager
        .as_ref()
        .map(|path| locate(binary, &service_search(path.clone())));

    // The service's answer wins where there is one: it is the environment the node actually runs
    // in, and this command's own PATH is the thing that made the old check a lie.
    let resolved = under
        .as_ref()
        .and_then(|r| r.as_ref().ok())
        .or(here.as_ref().ok())
        .cloned();
    let Some(found) = resolved else {
        return missing(here.expect_err("neither search found a binary"));
    };
    let version = match version(&found.path).await {
        Ok(version) => version,
        Err(why) => {
            return Check::fail(
                "observe",
                format!("{} is there and did not run: {why}", found.path.display()),
            )
            .fix("install a herdr that runs on this host, or set [herdr] binary in config.toml");
        }
    };
    let stream = observed(&found.path, socket).await;
    verdict(&found, &version, under, manager.flatten(), stream)
}

async fn observed(binary: &Path, socket: &Path) -> Stream {
    let Some((pane, cols, rows)) = a_pane(socket).await else {
        return Stream::Unchecked(format!("no pane on {} to try it against", socket.display()));
    };
    stream(binary, socket, &pane, cols, rows, FRAME_TIMEOUT).await
}

/// The focused pane where there is one — it is the pane an operator is looking at, and the one
/// they will say is blank.
async fn a_pane(socket: &Path) -> Option<(String, u32, u32)> {
    let snapshot = tokio::time::timeout(SNAPSHOT_TIMEOUT, Herdr::new(socket).snapshot())
        .await
        .ok()?
        .ok()?;
    let pane = snapshot
        .focused_pane_id
        .iter()
        .chain(snapshot.panes.iter().map(|p| &p.pane_id))
        .find(|id| snapshot.geometry(id).is_some())?
        .clone();
    let (cols, rows) = snapshot.geometry(&pane)?;
    Some((pane, cols, rows))
}

async fn stream(binary: &Path, socket: &Path, pane: &str, cols: u32, rows: u32, wait: Duration) -> Stream {
    let ran = format!("`{} terminal session observe {pane}`", binary.display());
    let mut observer = match Observer::spawn(&binary.display().to_string(), socket, pane, cols, rows) {
        Ok(observer) => observer,
        Err(e) => return Stream::Silent(format!("{ran} would not start: {e}")),
    };
    let answer = match tokio::time::timeout(wait, observer.events.recv()).await {
        Ok(Some(StreamEvent::Frame { cols, rows, .. })) => {
            Stream::Framed(format!("{pane} streamed a {cols}×{rows} frame"))
        }
        Ok(Some(StreamEvent::Closed { reason })) => {
            Stream::Silent(format!("{ran} closed before a frame: {reason}"))
        }
        Ok(None) => Stream::Silent(format!("{ran} exited without a frame")),
        Err(_) => Stream::Silent(format!("{ran} sent no frame in {}s", wait.as_secs())),
    };
    observer.shutdown().await;
    answer
}

fn verdict(
    found: &Found,
    version: &str,
    under: Option<Result<Found, NotFound>>,
    manager_path: Option<String>,
    stream: Stream,
) -> Check {
    let at = format!(
        "{version} at {} ({}) — every pane's grid is streamed by this binary, not by the socket",
        found.path.display(),
        how(found.origin)
    );
    // A binary that cannot stream is the finding whatever the manager's PATH says about it: the
    // node in #233 resolved herdr perfectly on the machine it was reported from.
    if let Stream::Silent(why) = &stream {
        return Check::fail("observe", format!("{at}. It does not stream: {why}"))
            .fix("install a herdr that answers `terminal session observe`, or set [herdr] binary in config.toml to one that does");
    }
    match (under, manager_path) {
        (Some(Err(_)), Some(path)) => Check::fail(
            "observe",
            format!(
                "{at}. The service will not find it: the kampr.service manager's PATH is {path}, \
                 and a bare `{}` is resolved through that and not through your shell's",
                found_name(found)
            ),
        )
        .fix(PIN),
        (Some(Err(_)), None) => Check::warn(
            "observe",
            format!(
                "{at}. Could not ask the service manager which PATH the node will have, and a \
                 bare name is resolved through it rather than through your shell's"
            ),
        )
        .fix(PIN),
        _ => match stream {
            Stream::Framed(how) => Check::new("observe", Status::Ok, format!("{at}, and {how}")),
            // Not green: the whole point of this check is the run, and it did not happen.
            Stream::Unchecked(why) => Check::warn(
                "observe",
                format!("{at}. Whether it can actually stream one was not established: {why}"),
            )
            .fix("start herdr and re-run `kampr doctor` — a resolved binary is only half of it"),
            Stream::Silent(_) => unreachable!("answered above"),
        },
    }
}

const PIN: &str = "kampr service install — it records the resolved path in config.toml";

fn found_name(found: &Found) -> String {
    found.path.file_name().map_or_else(
        || found.path.display().to_string(),
        |n| n.to_string_lossy().into_owned(),
    )
}

fn missing(error: NotFound) -> Check {
    let fix = match error.explicit {
        true => "install herdr there, or point [herdr] binary in config.toml at the one you have",
        false => "install herdr, or set [herdr] binary in config.toml to its absolute path",
    };
    // Not a warning: `pane.read` still answers, so the herd, the pane list and input all keep
    // working, and every grid stays blank for ever.
    Check::fail("observe", format!("{error}; no pane can stream its grid")).fix(fix)
}

fn how(origin: Origin) -> &'static str {
    match origin {
        Origin::Configured => "pinned in config.toml",
        Origin::Injected => "named by HERDR_BIN_PATH",
        Origin::Path => "on PATH",
        Origin::BesideKampr => "beside the kampr binary",
        Origin::Prefix => "in the usual install prefix",
    }
}

/// What the node will resolve from under its unit: the manager's `PATH`, and the directory
/// holding the kampr that unit runs — which need not be the one running this command.
fn service_search(path: Option<String>) -> Search {
    Search {
        injected: None,
        path: path.map(Into::into),
        beside: service::unit_binary()
            .or_else(|| std::env::current_exe().ok())
            .and_then(|exe| exe.parent().map(Path::to_path_buf)),
        ..Search::from_env()
    }
}

async fn version(path: &Path) -> Result<String, String> {
    let output = tokio::time::timeout(VERSION_TIMEOUT, Command::new(path).arg("--version").output())
        .await
        .map_err(|_| format!("no answer in {} seconds", VERSION_TIMEOUT.as_secs()))?
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!("`--version` exited {}", output.status));
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .unwrap_or_default()
        .trim()
        .to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn found(origin: Origin) -> Found {
        Found {
            path: PathBuf::from("/home/x/.local/bin/herdr"),
            origin,
        }
    }

    fn framed() -> Stream {
        Stream::Framed("n/w1:p1 streamed a 80×40 frame".into())
    }

    fn not_found() -> NotFound {
        NotFound {
            binary: "herdr".into(),
            tried: vec![PathBuf::from("/usr/bin/herdr")],
            explicit: false,
        }
    }

    #[test]
    fn a_binary_this_shell_can_run_and_the_service_cannot_is_a_failure_and_says_whose_path() {
        let check = verdict(
            &found(Origin::Path),
            "herdr 0.8.2",
            Some(Err(not_found())),
            Some("/usr/local/bin:/usr/bin:/bin".into()),
            framed(),
        );
        assert_eq!(check.status, Status::Fail);
        assert!(check.detail.contains("/usr/local/bin:/usr/bin:/bin"), "{check:?}");
        assert!(check.detail.contains("herdr 0.8.2"), "{check:?}");
        assert!(check.fix.unwrap().contains("kampr service install"));
    }

    #[test]
    fn a_manager_that_cannot_be_asked_is_a_warning_rather_than_a_verdict() {
        let check = verdict(
            &found(Origin::Path),
            "herdr 0.8.2",
            Some(Err(not_found())),
            None,
            framed(),
        );
        assert_eq!(check.status, Status::Warn);
        assert!(check.detail.contains("Could not ask"), "{check:?}");
    }

    #[test]
    fn a_binary_both_can_run_is_ok_and_names_where_it_came_from() {
        for origin in [
            Origin::Configured,
            Origin::Injected,
            Origin::Path,
            Origin::BesideKampr,
            Origin::Prefix,
        ] {
            let check = verdict(
                &found(origin),
                "herdr 0.8.2",
                Some(Ok(found(origin))),
                Some("/usr/bin".into()),
                framed(),
            );
            assert_eq!(check.status, Status::Ok, "{check:?}");
            assert!(check.detail.contains(how(origin)), "{check:?}");
            assert!(check.fix.is_none(), "nothing to fix: {check:?}");
        }
    }

    #[test]
    fn a_node_with_no_service_is_judged_on_its_own_shell_alone() {
        let check = verdict(&found(Origin::Path), "herdr 0.8.2", None, None, framed());
        assert_eq!(check.status, Status::Ok, "{check:?}");
    }

    /// The check exists for the machine in #233, where every resolution answered correctly. A
    /// stream that never arrives has to outrank a green PATH, or the check is back to reporting
    /// the half that was never broken.
    #[test]
    fn a_herdr_that_resolves_and_runs_and_cannot_stream_is_a_failure() {
        let check = verdict(
            &found(Origin::Path),
            "herdr 0.8.2",
            Some(Ok(found(Origin::Path))),
            Some("/usr/bin".into()),
            Stream::Silent("`herdr terminal session observe n/w1:p1` exited without a frame".into()),
        );
        assert_eq!(check.status, Status::Fail, "{check:?}");
        assert!(check.detail.contains("does not stream"), "{check:?}");
        assert!(check.detail.contains("observe n/w1:p1"), "{check:?}");
    }

    #[test]
    fn a_herd_with_no_pane_to_try_leaves_the_stream_unproved_rather_than_green() {
        let check = verdict(
            &found(Origin::Path),
            "herdr 0.8.2",
            Some(Ok(found(Origin::Path))),
            Some("/usr/bin".into()),
            Stream::Unchecked("no pane on /run/herdr.sock to try it against".into()),
        );
        assert_eq!(check.status, Status::Warn, "{check:?}");
        assert!(check.detail.contains("not established"), "{check:?}");
        assert!(check.fix.is_some(), "{check:?}");
    }

    fn shim(dir: &Path, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let path = dir.join("herdr");
        std::fs::write(&path, body).expect("a shim");
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        path
    }

    const FRAME: &str = r#"#!/bin/sh
echo '{"type":"terminal.frame","seq":1,"full":true,"width":80,"height":24,"bytes":"aGk="}'
"#;

    /// The whole of what `--version` could not tell anyone.
    #[tokio::test]
    async fn a_frame_off_the_spawned_child_is_what_makes_this_check_green() {
        let dir = tempfile::tempdir().unwrap();
        let binary = shim(dir.path(), FRAME);
        let streamed = stream(
            &binary,
            &dir.path().join("herdr.sock"),
            "n/w1:p1",
            80,
            24,
            Duration::from_secs(5),
        )
        .await;
        assert!(
            matches!(&streamed, Stream::Framed(how) if how.contains("80×24")),
            "{streamed:?}"
        );
    }

    #[tokio::test]
    async fn a_child_that_exits_without_a_frame_is_not_a_stream() {
        let dir = tempfile::tempdir().unwrap();
        let binary = shim(dir.path(), "#!/bin/sh\nexit 1\n");
        let streamed = stream(
            &binary,
            &dir.path().join("herdr.sock"),
            "n/w1:p1",
            80,
            24,
            Duration::from_secs(5),
        )
        .await;
        assert!(matches!(streamed, Stream::Silent(_)), "{streamed:?}");
    }

    /// A herdr that accepts the subcommand and then says nothing is the same blank grid, and
    /// `kampr doctor` has to answer rather than hang on it.
    #[tokio::test]
    async fn a_child_that_never_sends_a_frame_gives_up_and_says_so() {
        let dir = tempfile::tempdir().unwrap();
        let binary = shim(dir.path(), "#!/bin/sh\nsleep 30\n");
        let streamed = stream(
            &binary,
            &dir.path().join("herdr.sock"),
            "n/w1:p1",
            80,
            24,
            Duration::from_millis(300),
        )
        .await;
        assert!(
            matches!(&streamed, Stream::Silent(why) if why.contains("no frame in")),
            "{streamed:?}"
        );
    }

    /// A blank grid for ever is not a warning, and the remedy differs: a path the operator wrote
    /// is theirs to correct, a name nobody can find is a missing install.
    #[test]
    fn nothing_to_run_is_a_failure_whose_remedy_matches_what_was_asked_for() {
        let searched = missing(not_found());
        assert_eq!(searched.status, Status::Fail);
        assert!(searched.detail.contains("/usr/bin/herdr"), "{searched:?}");
        assert!(
            searched.detail.contains("no pane can stream its grid"),
            "{searched:?}"
        );
        assert!(searched.fix.unwrap().contains("install herdr"));

        let explicit = missing(NotFound {
            binary: "/opt/herdr".into(),
            tried: vec![PathBuf::from("/opt/herdr")],
            explicit: true,
        });
        assert!(explicit.detail.contains("/opt/herdr"), "{explicit:?}");
        assert!(explicit.fix.unwrap().contains("config.toml"));
    }
}
