use std::path::Path;

use serde::Deserialize;
use serde_json::Value;

use super::running::Watch;
use crate::facet::{Compaction, FacetFold, Facets, Mode, Queued, Timing, Titles};
use crate::marker::SessionMarker;
use crate::process::Started;
use crate::scan::{Appended, Cursor};
use crate::sub;

/// A title a person typed, filed beside the session's own directory rather than in the transcript.
const MANUAL: &str = "custom-title.json";

/// Every record kind below is measured, with counts, in probe #312. Nothing here is inferred from
/// a name.
pub fn collect(transcript: &Path, marker: Option<&SessionMarker>) -> Facets {
    Fold::default().facets(transcript, marker)
}

/// The same fold, kept between reads: the accumulated state and the byte it has reached, so a
/// second look costs the records appended since the first rather than the whole transcript.
#[derive(Default)]
pub struct Fold {
    cursor: Cursor,
    accumulated: Facets,
    titles: Titles,
    queue: Vec<Queued>,
    mode: Mode,
    watch: Watch,
}

impl FacetFold for Fold {
    fn facets(&mut self, transcript: &Path, marker: Option<&SessionMarker>) -> Facets {
        self.advance(transcript);
        Facets {
            title: self.levels(transcript, marker).resolve(),
            queued: self.queue.clone(),
            running: self.watch.running(marker.map_or(Started::Unknown, |m| m.started)),
            mode: (self.mode != Mode::default()).then(|| self.mode.clone()),
            ..self.accumulated.clone()
        }
    }

    fn titles(&mut self, transcript: &Path, marker: Option<&SessionMarker>) -> Titles {
        self.advance(transcript);
        self.levels(transcript, marker)
    }
}

impl Fold {
    fn advance(&mut self, transcript: &Path) {
        let mut appended = Appended::open(transcript, self.cursor);
        if appended.restarted() {
            *self = Self::default();
        }
        for line in appended.by_ref() {
            self.push(&line);
        }
        self.cursor = appended.cursor();
    }

    fn levels(&self, transcript: &Path, marker: Option<&SessionMarker>) -> Titles {
        let mut titles = self.titles.clone();
        // The file beside the session is what the operator typed most recently, and the marker is
        // the live copy of a name the transcript only records as of its last write.
        titles.manual = manual_title(transcript).or(titles.manual);
        titles.named = marker.and_then(|m| m.name.clone()).or(titles.named);
        titles
    }

    // Every one of these is rewritten as the session moves — 1165 `ai-title` records over the
    // twelve transcripts of one project here — so the last of each is the one it has now.
    fn push(&mut self, line: &str) {
        let Ok(record) = serde_json::from_str::<FacetRecord>(line) else {
            return;
        };
        match record.kind.as_str() {
            "ai-title" => self.titles.generated = record.ai_title.or(self.titles.generated.take()),
            "agent-name" => self.titles.named = record.agent_name.or(self.titles.named.take()),
            "custom-title" => self.titles.manual = record.custom_title.or(self.titles.manual.take()),
            "queue-operation" => {
                queued(&mut self.queue, &record);
                self.watch.notified(record.content.as_ref());
            }
            "user" => {
                delivered(&mut self.queue, &record);
                self.watch.record(
                    record.message.as_ref(),
                    record.tool_use_result.as_ref(),
                    record.timestamp.as_deref(),
                );
            }
            "assistant" => self.watch.record(
                record.message.as_ref(),
                record.tool_use_result.as_ref(),
                record.timestamp.as_deref(),
            ),
            "permission-mode" => {
                self.mode.permission = record.permission_mode.or(self.mode.permission.take())
            }
            "mode" => self.mode.mode = record.mode.or(self.mode.mode.take()),
            "system" => match record.subtype.as_deref() {
                Some("turn_duration") => self.accumulated.timings.extend(timing(&record)),
                Some("compact_boundary") => self.accumulated.compactions.extend(compaction(&record)),
                _ => {}
            },
            _ => {}
        }
    }
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
/// The harness handing itself the result of its own background work. **909 of the 992 `enqueue`
/// records on this machine are these**, and none of them is a thing a person typed — so a queue
/// that keeps them attributes the harness's plumbing to the operator, and buries the conversation
/// under it on a view pinned to its end (#363).
fn plumbing(text: &str) -> bool {
    text.trim_start().starts_with("<task-notification>")
}

/// A prompt the harness has taken off the queue and answered, proved by the record it wrote for
/// it rather than by the operation log.
///
/// **The operations alone cannot settle it.** A `dequeue` carries no `content` (#363), so the fold
/// can only pop the head by position, and one unmatched `enqueue` at position 0 makes every later
/// delivery pop the wrong entry — after which the queue drains one short for ever. A record of the
/// prompt *being delivered* is the harness's own answer to the same question.
fn delivered(queue: &mut Vec<Queued>, record: &FacetRecord) {
    let Some(spoken) = spoken(record) else {
        return;
    };
    queue.retain(|waiting| waiting.text != spoken);
}

fn spoken(record: &FacetRecord) -> Option<String> {
    match record.message.as_ref()?.get("content")? {
        Value::String(text) => Some(text.clone()),
        Value::Array(blocks) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .find_map(|block| block.get("text").and_then(Value::as_str))
            .map(str::to_string),
        _ => None,
    }
}

fn queued(queue: &mut Vec<Queued>, record: &FacetRecord) {
    let text = record.content.as_ref().and_then(Value::as_str);
    match (record.operation.as_deref(), text) {
        (Some("enqueue"), Some(text)) if !plumbing(text) => queue.push(Queued {
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
    message: Option<Value>,
    #[serde(rename = "toolUseResult")]
    tool_use_result: Option<Value>,
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
