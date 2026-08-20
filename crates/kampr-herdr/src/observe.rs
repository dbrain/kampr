use anyhow::{Context, Result};
use base64::Engine;
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub enum StreamEvent {
    /// `full` marks a complete repaint: safe to reset the emulator before applying.
    Frame { seq: u64, full: bool, cols: u32, rows: u32, bytes: Vec<u8> },
    Closed { reason: String },
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
    /// It never touches the PTY — unlike `terminal session control`, which always claims it.
    pub fn spawn(herdr_bin: &str, socket: &std::path::Path, pane_id: &str, cols: u32, rows: u32) -> Result<Self> {
        let mut child = Command::new(herdr_bin)
            .args(["terminal", "session", "observe", pane_id])
            .args(["--cols", &cols.to_string(), "--rows", &rows.to_string()])
            .env("HERDR_SOCKET_PATH", socket)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .context("spawning herdr terminal session observe")?;

        let stdout = child.stdout.take().context("observe child had no stdout")?;
        let (tx, rx) = mpsc::channel(64);
        tokio::spawn(async move {
            let mut lines = BufReader::new(stdout).lines();
            while let Ok(Some(line)) = lines.next_line().await {
                let Ok(rec) = serde_json::from_str::<Record>(&line) else { continue };
                let ev = match rec {
                    Record::Frame { seq, full, width, height, bytes } => {
                        match base64::engine::general_purpose::STANDARD.decode(bytes) {
                            Ok(bytes) => StreamEvent::Frame { seq, full, cols: width, rows: height, bytes },
                            Err(_) => continue,
                        }
                    }
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
