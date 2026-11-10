use kampr_client::Pending;
use serde_json::json;

use super::{Convo, Reading, Transcript, decode};

/// Never a handle a node hands out, so no `convo.sub` page is ever filed under it.
const CONTEXT: &str = "pending.context";

impl Convo {
    /// The pending question's whole context, read where the transcript is drawn and left with esc
    /// exactly as a launched conversation is. **Opened at its top**: a plan is read from its title.
    pub fn open_context(&mut self, pane: &str) -> bool {
        let Some(read) = self.pending(pane).and_then(reading) else {
            return false;
        };
        self.launched.insert(pane.to_string(), read);
        true
    }

    pub(super) fn follow_context(&mut self, pane: &str) {
        if self.launched.get(pane).is_none_or(|r| r.id != CONTEXT) {
            return;
        }
        match self.pending(pane).and_then(reading) {
            Some(read) => {
                let open = self.launched.get_mut(pane).expect("checked above");
                let scroll = open.held.scroll;
                *open = read;
                open.held.scroll = scroll;
            }
            None => {
                self.launched.remove(pane);
            }
        }
    }
}

fn reading(pending: &Pending) -> Option<Reading> {
    let context = pending.context.as_deref().filter(|c| !c.trim().is_empty())?;
    let head = [pending.header.as_deref(), pending.question.as_deref()]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let turn = json!({ "id": CONTEXT, "role": "assistant", "blocks": [{ "b": "md", "text": context }] });
    Some(Reading {
        id: CONTEXT.to_string(),
        head,
        held: Transcript {
            turns: decode(&[turn]),
            scroll: usize::MAX,
            ..Transcript::default()
        },
    })
}
