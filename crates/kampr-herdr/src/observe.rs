use crate::locate::{self, Search};
use anyhow::{Context, Result, anyhow};
use base64::Engine;
use serde::Deserialize;
use std::path::Path;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// `full` marks a complete repaint: safe to reset the emulator before applying.
    Frame {
        seq: u64,
        full: bool,
        cols: u32,
        rows: u32,
        bytes: Vec<u8>,
    },
    Closed {
        reason: String,
    },
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type")]
enum Record {
    #[serde(rename = "terminal.frame")]
    Frame {
        seq: u64,
        #[serde(default)]
        full: bool,
        width: u32,
        height: u32,
        bytes: String,
    },
    #[serde(rename = "terminal.closed")]
    Closed { reason: String },
}

pub struct Observer {
    child: Child,
    pub events: mpsc::Receiver<StreamEvent>,
}

impl Observer {
    /// Always pass the pane's *native* geometry. `observe` defaults to 120x40 when the flags are
    /// omitted, which crops or pads a pane of any other size, and it crops rather than reflows.
    /// It never touches the PTY (#14) — unlike `terminal session control`, which always claims it
    /// and lives next door in `control.rs` for the one op allowed to use it.
    pub fn spawn(herdr_bin: &str, socket: &Path, pane_id: &str, cols: u32, rows: u32) -> Result<Self> {
        let herdr = locate::locate(herdr_bin, &Search::from_env())?;
        let mut child = Command::new(&herdr.path)
            .args(["terminal", "session", "observe", pane_id])
            .args(["--cols", &cols.to_string(), "--rows", &rows.to_string()])
            .env("HERDR_SOCKET_PATH", socket)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            // The whole message, not a context line: the call site logs this error's `Display`,
            // and anyhow's `Display` is the outermost context alone — which is how the io error
            // that *is* the diagnosis got thrown away.
            .map_err(|e| {
                anyhow!(
                    "spawning `{} terminal session observe`: {e}",
                    herdr.path.display()
                )
            })?;

        let stdout = child.stdout.take().context("observe child had no stdout")?;
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(rec) = serde_json::from_str::<Record>(&line) else {
                    continue;
                };
                let ev = match rec {
                    Record::Frame {
                        seq,
                        full,
                        width,
                        height,
                        bytes,
                    } => match base64::engine::general_purpose::STANDARD.decode(bytes) {
                        Ok(bytes) => StreamEvent::Frame {
                            seq,
                            full,
                            cols: width,
                            rows: height,
                            bytes,
                        },
                        Err(_) => continue,
                    },
                    Record::Closed { reason } => StreamEvent::Closed { reason },
                };
                if tx.send(ev).await.is_err() {
                    break;
                }
            }
        });

        Ok(Self { child, events: rx })
    }

    pub async fn shutdown(mut self) {
        let _ = self.child.kill().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spawn_error(binary: &Path) -> String {
        let spawned = Observer::spawn(
            &binary.display().to_string(),
            &binary.with_file_name("herdr.sock"),
            "wA:p1",
            80,
            24,
        );
        let Err(error) = spawned else {
            panic!("{} spawned something", binary.display());
        };
        error.to_string()
    }

    /// The whole diagnosis of a blank grid is in this line, and naming the command it tried to run
    /// instead of naming the failure is what let a node stream nothing for months in silence.
    #[tokio::test]
    async fn a_spawn_that_fails_says_which_binary_and_what_stopped_it() {
        let dir = tempfile::tempdir().expect("a dir");

        let missing = dir.path().join("no-such-herdr");
        let said = spawn_error(&missing);
        assert!(said.contains(&missing.display().to_string()), "{said}");
        assert!(said.contains("not an executable file"), "{said}");

        // An exec bit on something that is not a program — a truncated download, a binary for
        // another architecture. It gets as far as execve, so the os error is the diagnosis.
        let junk = dir.path().join("herdr");
        std::fs::write(&junk, [0u8, 1, 2, 3]).expect("a file");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&junk, std::fs::Permissions::from_mode(0o755)).expect("chmod");
        }
        let said = spawn_error(&junk);
        assert!(said.contains(&junk.display().to_string()), "{said}");
        assert!(said.contains("terminal session observe"), "{said}");
        assert!(said.contains("os error"), "the io error is the diagnosis: {said}");
    }
}
