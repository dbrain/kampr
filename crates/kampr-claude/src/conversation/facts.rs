use kampr_core::conversation::facts::SessionFacts;
use kampr_core::conversation::{ContextUse, Suggestion, Update};
use serde_json::Value;
use time::{OffsetDateTime, format_description::well_known::Rfc3339};

use super::machine::Machine;
use super::state::State;
use crate::billing;

pub(super) fn session_facts(st: &State) -> SessionFacts {
    let init = st.init.as_ref();
    SessionFacts {
        id: st.session_id.clone(),
        cwd: init.and_then(|i| i.cwd.clone()),
        title: None,
        model: st.model.clone(),
        mode: st.mode.clone(),
        version: init.and_then(|i| i.version.clone()),
        account: st.account.as_ref().and_then(|a| a.email.clone()),
        billing: billing::billing_of(&st.account.clone().unwrap_or_default(), st.overage),
        started: None,
    }
}

/// Epoch seconds as the ISO time `get_usage` already speaks.
pub(super) fn iso(secs: u64) -> String {
    i64::try_from(secs)
        .ok()
        .and_then(|s| OffsetDateTime::from_unix_timestamp(s).ok())
        .and_then(|t| t.format(&Rfc3339).ok())
        .unwrap_or_default()
}

/// The `get_context_usage` answer, in the shape the binary sends it: `totalTokens` against
/// `maxTokens`, or the older binary's `context_breakdown` (probe #668).
pub(super) fn context_use(body: &Value) -> Option<ContextUse> {
    let breakdown = body.get("context_breakdown");
    let total = body.get("maxTokens").and_then(Value::as_u64).or_else(|| {
        breakdown
            .and_then(|b| b.get("max_tokens"))
            .and_then(Value::as_u64)
    })?;
    let used = body.get("totalTokens").and_then(Value::as_u64).or_else(|| {
        breakdown.and_then(|b| {
            b.as_object().map(|fields| {
                fields
                    .iter()
                    .filter(|(key, _)| key.ends_with("tokens") && key.as_str() != "max_tokens")
                    .filter_map(|(_, value)| value.as_u64())
                    .sum()
            })
        })
    })?;
    Some(ContextUse { used, total })
}

/// The `file_suggestions` answer: the binary's own file list for the pane's @-completion.
pub(super) fn file_suggestions(body: &Value) -> Vec<Suggestion> {
    let items = body
        .get("suggestions")
        .or_else(|| body.get("results"))
        .and_then(Value::as_array);
    items
        .into_iter()
        .flatten()
        .filter_map(|item| {
            let label = item
                .as_str()
                .or_else(|| item.get("path").and_then(Value::as_str))
                .or_else(|| item.get("name").and_then(Value::as_str))?
                .to_string();
            Some(Suggestion {
                id: label.clone(),
                label,
                detail: item
                    .get("description")
                    .and_then(Value::as_str)
                    .map(str::to_string),
            })
        })
        .collect()
}

impl Machine {
    pub(super) fn publish_session(&mut self) {
        let facts = session_facts(&self.st);
        self.say(Update::Session(facts));
    }

    /// A context read in flight: the pane's bar is what it answers with.
    pub(super) fn fetch_context(&mut self) {
        if !self.replaying {
            self.request(
                super::state::Request::Context,
                "get_context_usage",
                serde_json::json!({}),
            );
        }
    }
}
