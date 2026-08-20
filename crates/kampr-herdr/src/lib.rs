pub mod model;
pub mod observe;
pub mod rpc;

pub use model::{AgentStatus, Pane, Snapshot, SnapshotReply};
pub use observe::{Observer, StreamEvent};
pub use rpc::Herdr;

use anyhow::Result;

impl Herdr {
    pub async fn snapshot(&self) -> Result<Snapshot> {
        let r: SnapshotReply = self.call("session.snapshot", serde_json::json!({})).await?;
        Ok(r.snapshot)
    }

    pub async fn send_text(&self, pane_id: &str, text: &str) -> Result<()> {
        let _: serde_json::Value = self
            .call("pane.send_text", serde_json::json!({ "pane_id": pane_id, "text": text }))
            .await?;
        Ok(())
    }

    pub async fn send_keys(&self, pane_id: &str, keys: &[&str]) -> Result<()> {
        let _: serde_json::Value = self
            .call("pane.send_keys", serde_json::json!({ "pane_id": pane_id, "keys": keys }))
            .await?;
        Ok(())
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
