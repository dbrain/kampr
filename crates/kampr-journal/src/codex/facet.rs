use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::facet::{Compaction, Facets, Mode, Timing};
use crate::scan::records;

use super::message_turn;
use super::record::{Payload, Record};

/// Every record kind below is measured, with counts, in probe #322. Codex fills three of the five
/// facets and none of the other two: there is no session title — the 2700 `payload.name` hits
/// across this machine's rollouts are tool-call names — and no queue of any kind.
pub fn collect(transcript: &Path) -> Facets {
    let mut facets = Facets::default();
    let mut mode = Mode::default();
    let mut turn: Option<String> = None;

    for (seq, line) in records(transcript).enumerate() {
        let Ok(record) = serde_json::from_str::<Record>(&line) else {
            continue;
        };
        match record.kind.as_str() {
            "response_item" => {
                if produces_turn(record.payload) {
                    turn = Some(format!("x{seq}"));
                }
            }
            "turn_context" => {
                if let Ok(context) = serde_json::from_value::<TurnContext>(record.payload) {
                    mode.mode = context.collaboration_mode.and_then(|c| c.mode).or(mode.mode);
                    mode.permission = context.approval_policy.or(mode.permission);
                }
            }
            "event_msg" => {
                let Ok(event) = serde_json::from_value::<Event>(record.payload) else {
                    continue;
                };
                match event.kind.as_str() {
                    "task_complete" => facets.timings.extend(timing(&turn, &event)),
                    // The whole payload is `{"type":"context_compacted"}`, and the `compacted`
                    // record beside it carries `replacement_history` and no counts — so a Codex
                    // compaction can say *here* and nothing else. Counting the history's entries
                    // would put a number on the wire the harness never wrote, and the client
                    // cannot tell one of those from a measurement.
                    "context_compacted" => facets.compactions.push(Compaction {
                        at: record.timestamp,
                        ..Compaction::default()
                    }),
                    _ => {}
                }
            }
            _ => {}
        }
    }

    facets.mode = (mode != Mode::default()).then_some(mode);
    facets
}

/// **The turn a timing closes is the last one the parser produced before it, not the `turn_id`
/// the payload carries.** That id is the harness's own handle — it appears on `turn_context` and
/// `task_started` and nowhere an id a client holds is minted from — so serving it would be a
/// duration hanging off nothing.
fn timing(turn: &Option<String>, event: &Event) -> Option<Timing> {
    Some(Timing {
        turn: turn.clone()?,
        duration_ms: event.duration_ms?,
        messages: None,
    })
}

fn produces_turn(payload: Value) -> bool {
    match serde_json::from_value::<Payload>(payload) {
        Ok(Payload::Message { role, content }) => message_turn(
            String::new(),
            role.as_deref(),
            content,
            None,
            &mut std::iter::empty(),
        )
        .is_some(),
        Ok(Payload::FunctionCall { .. } | Payload::CustomToolCall { .. }) => true,
        _ => false,
    }
}

#[derive(Debug, Deserialize)]
struct Event {
    #[serde(rename = "type")]
    kind: String,
    duration_ms: Option<u64>,
}

/// **Codex's axes are not Claude's, and #322 is explicit that they must not be assumed to mean
/// the same thing.** `collaboration_mode.mode` is the one that names how the model has been asked
/// to work — `default` against `plan` — which is the axis Claude's `mode` carries, and
/// `approval_policy` (`never`, `on-request`, `untrusted`) is the one that decides what runs
/// without being asked, which is what `permission` means. `sandbox_policy` and
/// `permission_profile` are a third axis about the filesystem and the network rather than about
/// the operator, they are structured objects rather than words, and there is no field here that
/// would carry either without misrepresenting it.
#[derive(Debug, Deserialize)]
struct TurnContext {
    approval_policy: Option<String>,
    collaboration_mode: Option<Collaboration>,
}

#[derive(Debug, Deserialize)]
struct Collaboration {
    mode: Option<String>,
}
