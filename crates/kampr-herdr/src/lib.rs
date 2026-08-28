pub mod control;
pub mod locate;
pub mod model;
pub mod observe;
pub mod rpc;

pub use control::{Controller, HOLD_LIMIT};
pub use locate::{Found, Origin, Search};
pub use model::{AgentStatus, Command, Pane, ProcessInfo, Snapshot, SnapshotReply};
pub use observe::{Observer, StreamEvent};
pub use rpc::{Herdr, Sub};

use anyhow::Result;
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    fn as_str(self) -> &'static str {
        match self {
            Self::Asc => "asc",
            Self::Desc => "desc",
        }
    }
}

/// What herdr says it is holding after an `agent.view.*` call: whether a view is active, and the
/// `source` and `label` it was set with. The sort is not in it.
#[derive(Debug, Clone, Default, Deserialize, PartialEq, Eq)]
pub struct AgentView {
    pub active: bool,
    #[serde(default)]
    pub source: Option<String>,
    #[serde(default)]
    pub label: Option<String>,
}

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

    /// Reports a name Kampr computed for a pane into herdr's own metadata table.
    ///
    /// **`ok` here means well-formed, never applied** (probe #295). A `seq` older than the one
    /// this source last sent is dropped silently and still answered `ok`, and the record is
    /// per-source under last-writer-wins, so the only honest confirmation is [`Self::pane_title`]
    /// read back afterwards.
    pub async fn report_metadata(
        &self,
        pane_id: &str,
        source: &str,
        title: &str,
        tokens: &BTreeMap<String, String>,
        seq: u64,
    ) -> Result<()> {
        let _: serde_json::Value = self
            .call(
                "pane.report_metadata",
                serde_json::json!({
                    "pane_id": pane_id,
                    "source": source,
                    "title": title,
                    "tokens": tokens,
                    "seq": seq,
                }),
            )
            .await?;
        Ok(())
    }

    /// Shapes herdr's **own** agents sidebar, at whoever's desk this session belongs to.
    ///
    /// Sortable fields are the tokens a source reported plus exactly two builtins, `agent` and
    /// `status` — nothing else is accepted, and there is no builtin for `title`. So a sort on a
    /// name Kampr computed only means anything once that name has been reported as a *token*,
    /// which is [`Self::report_metadata`]'s job and is itself behind a setting.
    ///
    /// `label` replaces the sort-mode word in the sidebar's section header and herdr refuses one
    /// that is empty or past 32 characters. The reply echoes `active`, `source` and `label` and
    /// says **nothing about the sort**, and there is no `agent.view.get`: what was sorted on is
    /// unreadable once sent.
    pub async fn set_agent_view(
        &self,
        source: &str,
        token: &str,
        order: SortOrder,
        label: &str,
    ) -> Result<AgentView> {
        self.call(
            "agent.view.set",
            serde_json::json!({
                "source": source,
                "sort": [{ "field": { "token": token }, "order": order.as_str() }],
                "label": label,
            }),
        )
        .await
    }

    /// Puts the desk's own agent order back.
    ///
    /// **This takes no source and is not scoped to one.** It clears whatever view is active,
    /// whoever set it, so a caller that never set one must not call it.
    pub async fn clear_agent_view(&self) -> Result<AgentView> {
        self.call("agent.view.clear", serde_json::json!({})).await
    }

    /// The title herdr is *showing* for a pane — whoever's report is winning the field.
    pub async fn pane_title(&self, pane_id: &str) -> Result<Option<String>> {
        let r: model::PaneReply = self
            .call("pane.get", serde_json::json!({ "pane_id": pane_id }))
            .await?;
        Ok(r.pane.title)
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
