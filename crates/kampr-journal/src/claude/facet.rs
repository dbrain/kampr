use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use crate::facet::{Compaction, Facets, Mode, Queued, Timing, Titles};
use crate::marker::SessionMarker;
use crate::sub;

/// A title a person typed, filed beside the session's own directory rather than in the transcript.
const MANUAL: &str = "custom-title.json";

/// Every record kind below is measured, with counts, in probe #312. Nothing here is inferred from
/// a name.
pub fn collect(transcript: &Path, marker: Option<&SessionMarker>) -> Facets {
    let mut facets = Facets::default();
    let mut titles = Titles::default();
    let mut queue: Vec<Queued> = Vec::new();
    let mut mode = Mode::default();

    // Every one of these is rewritten as the session moves — 1165 `ai-title` records over the
    // twelve transcripts of one project here — so the last of each is the one it has now.
    for line in crate::scan::records(transcript) {
        let Ok(record) = serde_json::from_str::<FacetRecord>(&line) else {
            continue;
        };
        match record.kind.as_str() {
            "ai-title" => titles.generated = record.ai_title.or(titles.generated),
            "agent-name" => titles.named = record.agent_name.or(titles.named),
            "custom-title" => titles.manual = record.custom_title.or(titles.manual),
            "queue-operation" => queued(&mut queue, &record),
            "permission-mode" => mode.permission = record.permission_mode.or(mode.permission),
            "mode" => mode.mode = record.mode.or(mode.mode),
            "system" => match record.subtype.as_deref() {
                Some("turn_duration") => facets.timings.extend(timing(&record)),
                Some("compact_boundary") => facets.compactions.extend(compaction(&record)),
                _ => {}
            },
            _ => {}
        }
    }

    // The file beside the session is what the operator typed most recently, and the marker is the
    // live copy of a name the transcript only records as of its last write.
    titles.manual = manual_title(transcript).or(titles.manual);
    titles.named = marker.and_then(|m| m.name.clone()).or(titles.named);
    facets.title = titles.resolve();
    facets.queued = queue;
    facets.mode = (mode != Mode::default()).then_some(mode);
    facets
}

fn manual_title(transcript: &Path) -> Option<String> {
    let text = std::fs::read_to_string(sub::tree(transcript).join(MANUAL)).ok()?;
    serde_json::from_str::<Value>(&text)
        .ok()?
        .get("customTitle")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// A harness records every operation on the queue and never the queue itself, so what is still
/// waiting is the fold of all four — and it is four, not the two an `enqueue`/`remove` pair
/// suggests. Counted over this machine's transcripts: 826 `enqueue`, 551 `remove`, 257 `dequeue`
/// and 4 `popAll`. Folding the first two alone left **141** prompts standing on a session that had
/// finished them all, because a prompt delivered in the ordinary way leaves a `dequeue` rather
/// than a `remove` — every one of which carries a null `content`, which is why the head has to be
/// taken by position.
fn queued(queue: &mut Vec<Queued>, record: &FacetRecord) {
    let text = record.content.as_ref().and_then(Value::as_str);
    match (record.operation.as_deref(), text) {
        (Some("enqueue"), Some(text)) => queue.push(Queued {
            text: text.to_string(),
            at: record.timestamp.clone(),
        }),
        (Some("remove"), Some(text)) => {
            if let Some(at) = queue.iter().position(|q| q.text == text) {
                queue.remove(at);
            }
        }
        (Some("dequeue"), _) => {
            if !queue.is_empty() {
                queue.remove(0);
            }
        }
        (Some("popAll"), _) => queue.clear(),
        _ => {}
    }
}

fn timing(record: &FacetRecord) -> Option<Timing> {
    Some(Timing {
        turn: record.parent_uuid.clone()?,
        duration_ms: record.duration_ms?,
        messages: record.message_count,
    })
}

fn compaction(record: &FacetRecord) -> Option<Compaction> {
    let meta = record.compact.as_ref()?;
    Some(Compaction {
        at: record.timestamp.clone(),
        trigger: meta.trigger.clone(),
        pre_tokens: meta.pre_tokens,
        post_tokens: meta.post_tokens,
        dropped_tokens: meta.cumulative_dropped_tokens,
    })
}

/// Every field one of the record kinds above carries, and no other: the parse has to survive a
/// transcript full of records this knows nothing about, which is all but a few hundred of them.
#[derive(Debug, Deserialize)]
struct FacetRecord {
    #[serde(rename = "type")]
    kind: String,
    subtype: Option<String>,
    #[serde(rename = "parentUuid")]
    parent_uuid: Option<String>,
    timestamp: Option<String>,
    #[serde(rename = "aiTitle")]
    ai_title: Option<String>,
    #[serde(rename = "agentName")]
    agent_name: Option<String>,
    #[serde(rename = "customTitle")]
    custom_title: Option<String>,
    operation: Option<String>,
    content: Option<Value>,
    #[serde(rename = "durationMs")]
    duration_ms: Option<u64>,
    #[serde(rename = "messageCount")]
    message_count: Option<u32>,
    #[serde(rename = "compactMetadata")]
    compact: Option<CompactMetadata>,
    #[serde(rename = "permissionMode")]
    permission_mode: Option<String>,
    mode: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CompactMetadata {
    trigger: Option<String>,
    #[serde(rename = "preTokens")]
    pre_tokens: Option<u64>,
    #[serde(rename = "postTokens")]
    post_tokens: Option<u64>,
    #[serde(rename = "cumulativeDroppedTokens")]
    cumulative_dropped_tokens: Option<u64>,
}
