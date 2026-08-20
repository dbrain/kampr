pub mod model;
pub mod observe;
pub mod rpc;

pub use model::{AgentStatus, Pane, Snapshot, SnapshotReply};
pub use observe::{Observer, StreamEvent};
pub use rpc::{Herdr, Sub};

use anyhow::Result;

impl Herdr {
    pub async fn snapshot(&self) -> Result<Snapshot> {
        let r: SnapshotReply = self.call("session.snapshot", serde_json::json!({})).await?;
        Ok(r.snapshot)
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
    /// the socket API offers. `lines` must not exceed the viewport: past it, `recent` harvests a
    /// recognised agent pane through its own mouse-scroll interface (probe #27's interlock).
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
