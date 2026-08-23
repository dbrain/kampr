pub mod model;
pub mod observe;
pub mod rpc;

pub use model::{AgentStatus, Pane, ProcessInfo, Snapshot, SnapshotReply};
pub use observe::{Observer, StreamEvent};
pub use rpc::{Herdr, Sub};

use anyhow::Result;

impl Herdr {
    pub async fn snapshot(&self) -> Result<Snapshot> {
        let r: SnapshotReply = self.call("session.snapshot", serde_json::json!({})).await?;
        Ok(r.snapshot)
    }

    /// The processes inside a pane.
    ///
    /// **The pane record carries no pid**, so this call is the whole of what a node can learn
    /// about which process a pane is running — and the working directory it does carry names a
    /// project, not a session.
    pub async fn process_info(&self, pane_id: &str) -> Result<model::ProcessInfo> {
        let r: model::ProcessInfoReply = self
            .call("pane.process_info", serde_json::json!({ "pane_id": pane_id }))
            .await?;
        Ok(r.process_info)
    }

    pub async fn send_text(&self, pane_id: &str, text: &str) -> Result<()> {
        let _: serde_json::Value = self
            .call(
                "pane.send_text",
                serde_json::json!({ "pane_id": pane_id, "text": text }),
            )
            .await?;
        Ok(())
    }

    pub async fn send_keys<S: AsRef<str> + Sync>(&self, pane_id: &str, keys: &[S]) -> Result<()> {
        let keys: Vec<&str> = keys.iter().map(AsRef::as_ref).collect();
        let _: serde_json::Value = self
            .call(
                "pane.send_keys",
                serde_json::json!({ "pane_id": pane_id, "keys": keys }),
            )
            .await?;
        Ok(())
    }

    /// Reads the last `lines` rows twice: once wrapped at the pane's real width and once as
    /// logical lines.
    ///
    /// Probe #84: both render at the **true PTY width**, which in a headless session is not the
    /// layout rect, and the difference between them is the only exact measurement of that width
    /// the socket API offers. `lines` is the viewport and not more: past it, a pane whose harness
    /// is live and whose ring is empty answers in ~375 ms rather than under one (probe #231), and
    /// this runs on a poll.
    pub async fn read_wrapped_and_logical(
        &self,
        pane_id: &str,
        lines: u64,
    ) -> Result<(model::Read, model::Read)> {
        let read = async |source: &str| -> Result<model::Read> {
            let r: model::ReadReply = self
                .call(
                    "pane.read",
                    serde_json::json!({
                        "pane_id": pane_id, "source": source, "lines": lines, "format": "text"
                    }),
                )
                .await?;
            Ok(r.read)
        };
        Ok((read("recent").await?, read("recent_unwrapped").await?))
    }

    /// Reads herdr's scrollback ring. Callers must gate this on
    /// [`Pane::scrollback_is_safe_to_read`] before asking for more than the viewport.
    pub async fn read_scrollback(&self, pane_id: &str, lines: u64) -> Result<model::Read> {
        let r: model::ReadReply = self
            .call(
                "pane.read",
                serde_json::json!({
                    "pane_id": pane_id, "source": "recent", "lines": lines, "format": "ansi"
                }),
            )
            .await?;
        Ok(r.read)
    }
}
