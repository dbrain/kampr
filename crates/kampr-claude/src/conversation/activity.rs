use std::path::Path;

use kampr_keep::{PipeDir, PipeRead, SeqReader};
use serde_json::Value;

use super::asks::Standing;
use crate::wire::Line;

/// What the keeper's log says the child is in the middle of, read the way a conversation's
/// replay reads it, for a pane nobody may have open.
#[derive(Default)]
pub struct Activity {
    pub turn: bool,
    asks: Vec<Standing>,
}

impl Activity {
    pub fn read(session: &Path) -> Self {
        let mut activity = Self::default();
        let Ok(mut reader) = SeqReader::at(session, 0) else {
            return activity;
        };
        loop {
            match reader.read() {
                Ok(PipeRead::Entry(entry)) => match entry.dir {
                    PipeDir::Out => activity.out(&entry.line),
                    PipeDir::In => activity.sent(&entry.line),
                },
                Ok(PipeRead::Truncated { .. }) => {}
                _ => return activity,
            }
        }
    }

    /// What a hand-over now would stop, in words; empty when the child is at rest.
    pub fn stopping(&self) -> Vec<String> {
        let turn = self.turn.then(|| "the turn in progress".to_string());
        turn.into_iter()
            .chain(self.asks.iter().filter_map(Standing::stops))
            .collect()
    }

    fn out(&mut self, raw: &str) {
        match Line::parse(raw) {
            Some(Line::Init(_)) => self.turn = true,
            Some(Line::Result(_)) => self.turn = false,
            Some(Line::ControlRequest(request)) => self.asks.extend(Standing::of(request)),
            _ => {}
        }
    }

    fn sent(&mut self, raw: &str) {
        let Ok(line) = serde_json::from_str::<Value>(raw) else {
            return;
        };
        match line.get("type").and_then(Value::as_str) {
            Some("user") => self.turn = true,
            Some("control_response") => {
                if let Some(id) = line.pointer("/response/request_id").and_then(Value::as_str) {
                    self.asks.retain(|s| !s.owns_request(id));
                }
            }
            _ => {}
        }
    }
}
