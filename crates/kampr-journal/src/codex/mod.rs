mod facet;
mod record;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::adapter::{JournalAdapter, SessionKind, SessionRef};
use crate::attach::{self, Fetched, Origin};
use crate::composer::{Caret, Composed, ComposerReader};
use crate::discover;
use crate::envelope::push_text;
use crate::error::JournalError;
use crate::facet::{FacetFold, Facets};
use crate::live::{Layout, LiveBlock, ScreenReader};
use crate::marker::SessionMarker;
use crate::model::{Attachment, Block, Role, ToolState, Turn};
use crate::root::TranscriptRoot;
use crate::store::TurnStore;
use crate::summary::{count_lines, image_marker, marker_of, one_line, summarise};
use crate::tail::TranscriptParser;

use record::{
    PATCH_PREFIX, Payload, Record, data_url_att, data_url_subtype, envelope_output, output_failed,
    output_text, patch_target,
};

pub const AGENT: &str = "codex";

pub struct CodexAdapter {
    root: TranscriptRoot,
}

impl CodexAdapter {
    pub fn new(root: TranscriptRoot) -> Self {
        Self { root }
    }

    /// `~/.codex/sessions/YYYY/MM/DD/rollout-<stamp>-<uuid>.jsonl`. The date is not part of the
    /// session id, so an id is found by walking the three date levels under the root.
    fn find_by_id(&self, id: &str) -> Result<PathBuf, JournalError> {
        self.root.check_id(id)?;
        let suffix = format!("-{id}.jsonl");
        let sessions = self.root.path().join("sessions");
        let mut level = vec![sessions];
        for _ in 0..3 {
            let mut next = Vec::new();
            for dir in level {
                let Ok(entries) = std::fs::read_dir(&dir) else {
                    continue;
                };
                next.extend(entries.flatten().map(|e| e.path()).filter(|p| p.is_dir()));
            }
            level = next;
        }
        for dir in level {
            let Ok(entries) = std::fs::read_dir(&dir) else {
                continue;
            };
            for entry in entries.flatten() {
                let path = entry.path();
                let matches = path
                    .file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-") && n.ends_with(&suffix));
                if matches {
                    return Ok(path);
                }
            }
        }
        Err(JournalError::NotFound(id.to_string()))
    }
}

impl JournalAdapter for CodexAdapter {
    fn agent(&self) -> &str {
        AGENT
    }

    fn root(&self) -> &TranscriptRoot {
        &self.root
    }

    fn locate(&self, session: &SessionRef) -> Result<PathBuf, JournalError> {
        match session.kind {
            SessionKind::Id => self.find_by_id(&session.value),
            SessionKind::Path => self.root.contain(&session.value),
        }
    }

    /// A rollout carries its working directory in the `session_meta` record it opens with
    /// (probe #45). The directory, bounded by when the pane's `codex` started, is the whole
    /// handle *this* function has — but it is no longer the only one available.
    ///
    /// **Codex does publish a process-to-thread map, and this crate does not read it yet.**
    /// `~/.codex/thread-writer-locks/<thread-id>.lock` is empty, which is what made it look
    /// like nothing, but it is held with `flock` from before the first prompt until the process
    /// dies — the same kernel-backed handle [`crate::agy`] already calls the strongest one
    /// available. Two cautions for whoever wires it: the holder is the **native** `codex`
    /// binary, not the `bin/codex.js` wrapper that spawns it, so the pipeline walk has to reach
    /// the child; and `/new` **takes a second lock without releasing the first**, so agy's
    /// "exactly one held lock or nothing" rule would refuse a session that has used it.
    fn locate_by_cwd(&self, cwd: &Path, since: Option<SystemTime>) -> Result<PathBuf, JournalError> {
        let rollouts = discover::descend(&self.root.path().join("sessions"), 3)
            .iter()
            .flat_map(|d| discover::jsonl_files(d))
            .filter(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with("rollout-"))
            })
            .collect();
        discover::newest_declaring(rollouts, cwd, since, discover::Silent::Refuse, |record| {
            if record.get("type").and_then(Value::as_str) != Some("session_meta") {
                return None;
            }
            record
                .pointer("/payload/cwd")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .ok_or_else(|| discover::not_found(cwd))
    }

    fn parser(&self) -> Box<dyn TranscriptParser> {
        Box::new(CodexParser::default())
    }

    fn facets(&self, transcript: &Path, _marker: Option<&SessionMarker>) -> Facets {
        facet::collect(transcript)
    }

    fn fold(&self) -> Option<Box<dyn FacetFold>> {
        Some(Box::new(facet::Fold::default()))
    }

    fn screen(&self) -> Option<ScreenReader> {
        Some(live)
    }

    fn composer(&self) -> Option<ComposerReader> {
        Some(composer)
    }

    fn attachment(&self, record: &str, index: u32) -> Result<Fetched, JournalError> {
        let refuse = || JournalError::NotFound(index.to_string());
        let record: Record = serde_json::from_str(record).map_err(|_| refuse())?;
        let payload: Payload = serde_json::from_value(record.payload).map_err(|_| refuse())?;
        attach::nth(record::attachments(&payload), index)
    }
}

#[derive(Default)]
pub struct CodexParser {
    store: TurnStore,
    tool_turns: HashMap<String, (String, usize)>,
    seq: u64,
    origin: Option<Origin>,
}

impl TranscriptParser for CodexParser {
    fn push_line(&mut self, line: &str, at: u64) {
        let seq = self.seq;
        self.seq += 1;
        let Ok(record) = serde_json::from_str::<Record>(line) else {
            return;
        };
        if record.kind != "response_item" {
            return;
        }
        let Ok(payload) = serde_json::from_value::<Payload>(record.payload) else {
            return;
        };
        self.ingest(payload, record.timestamp, format!("x{seq}"), at);
    }

    fn set_origin(&mut self, origin: Origin) {
        self.origin = Some(origin);
    }

    fn reset(&mut self) {
        *self = Self {
            origin: self.origin.take(),
            ..Self::default()
        };
    }

    fn store(&self) -> &TurnStore {
        &self.store
    }

    fn store_mut(&mut self) -> &mut TurnStore {
        &mut self.store
    }
}

impl CodexParser {
    fn ingest(&mut self, payload: Payload, at: Option<String>, id: String, offset: u64) {
        let atts = match &self.origin {
            Some(origin) => attach::headers(origin, offset, &record::attachments(&payload)),
            None => Vec::new(),
        };
        let mut atts = atts.into_iter();
        match payload {
            Payload::Message { role, content } => {
                if let Some(turn) = message_turn(id, role.as_deref(), content, at, &mut atts) {
                    self.store.push(turn);
                }
            }
            Payload::FunctionCall {
                name,
                arguments,
                call_id,
            } => {
                let args: Value = serde_json::from_str(&arguments).unwrap_or(Value::Null);
                let mut turn = Turn::new(id.clone(), Role::Assistant, at);
                let card = turn.blocks.len();
                turn.blocks.push(Block::Tool {
                    summary: summarise(&args),
                    lines: None,
                    state: ToolState::Running,
                    name,
                });
                if let Some(cmd) = args.get("cmd").and_then(Value::as_str) {
                    turn.blocks.push(Block::Code {
                        lang: Some("bash".into()),
                        text: cmd.to_string(),
                    });
                }
                self.tool_turns.insert(call_id, (id, card));
                self.store.push(turn);
            }
            Payload::CustomToolCall { name, input, call_id } => {
                let mut turn = Turn::new(id.clone(), Role::Assistant, at);
                let patch = input.starts_with(PATCH_PREFIX);
                let target = patch.then(|| patch_target(&input)).flatten();
                let card = turn.blocks.len();
                turn.blocks.push(Block::Tool {
                    summary: Some(one_line(target.as_deref().unwrap_or(&input))),
                    lines: None,
                    state: ToolState::Running,
                    name,
                });
                // Codex 0.147 sends shell work through the same `custom_tool_call` shape as
                // `apply_patch`, carrying JavaScript rather than a patch.
                turn.blocks.push(if patch {
                    Block::Diff {
                        path: target,
                        text: input,
                    }
                } else {
                    Block::Code {
                        lang: None,
                        text: input,
                    }
                });
                self.tool_turns.insert(call_id, (id, card));
                self.store.push(turn);
            }
            Payload::FunctionCallOutput { call_id, output }
            | Payload::CustomToolCallOutput { call_id, output } => {
                let raw = output_text(&output);
                self.settle(&call_id, &raw, atts.collect());
            }
            Payload::Other => {}
        }
    }

    fn settle(&mut self, call_id: &str, raw: &str, images: Vec<Attachment>) {
        let Some((target, card)) = self.tool_turns.get(call_id).cloned() else {
            return;
        };
        let failed = output_failed(raw);
        let text = envelope_output(raw);
        let Some(turn) = self.store.revise(&target) else {
            return;
        };
        if let Some(Block::Tool { state, lines, .. }) = turn.tool_block_mut(card) {
            *state = if failed { ToolState::Error } else { ToolState::Done };
            *lines = count_lines(&text);
        }
        for att in images {
            turn.blocks.push(Block::Md {
                text: marker_of(&att),
                att: Some(att),
            });
        }
    }
}

/// The turn one `message` payload becomes, or nothing where it becomes no turn at all.
///
/// **[`facet`] asks this the same question the parse does, and must get the same answer.** A
/// timing is named by the id the parser minted from a record's position, so a record this drops
/// and the facet scan keeps — a `developer` instruction block, a user record that is nothing but
/// a harness envelope — would leave the timing hanging off an id no turn ever carried.
fn message_turn(
    id: String,
    role: Option<&str>,
    content: Vec<record::ContentItem>,
    at: Option<String>,
    atts: &mut impl Iterator<Item = Attachment>,
) -> Option<Turn> {
    // `developer` carries the harness's own instruction blocks, not the conversation.
    let role = match role {
        Some("assistant") => Role::Assistant,
        Some("user") => Role::User,
        _ => return None,
    };
    let mut turn = Turn::new(id, role, at);
    for item in content {
        match item.kind.as_str() {
            "input_text" | "output_text" => {
                if let Some(text) = item.text.filter(|t| !t.is_empty()) {
                    push_text(&mut turn, text);
                }
            }
            "input_image" => turn.blocks.push(Block::Md {
                text: image_marker(item.image_url.as_deref().and_then(data_url_subtype)),
                att: item
                    .image_url
                    .as_deref()
                    .and_then(data_url_att)
                    .and_then(|_| atts.next()),
            }),
            _ => {}
        }
    }
    (!turn.blocks.is_empty()).then_some(turn)
}

/// Codex 0.149 opens both its assistant messages and its own status line with `•` in column zero.
/// The status line is the only head worth rejecting: while a turn runs, the block at the foot of
/// the screen is either the spinner or the message being written — a tool card always has the
/// spinner painted underneath it. Captured live: `tests/fixtures/screens`.
const LAYOUT: Layout = Layout {
    message: '•',
    prompt: '›',
    result: '└',
    indent: 2,
    input: 2,
    reject: is_status,
};

/// One `ctrl+u` takes the whole buffer, wrapped or not — measured against a 200-column entry over
/// three rows. `ctrl+c` clears it too, and is not used: it is the key that quits two of the three
/// harnesses this crate serves, and there is nothing to win by spending it here.
const CLEAR: &str = "\u{15}";

fn is_status(head: &str) -> bool {
    head.starts_with("Working (")
}

pub fn live(screen: &[&str]) -> Option<LiveBlock> {
    crate::live::read(screen, &LAYOUT)
}

pub fn composer(screen: &[&str], caret: Caret) -> Option<Composed> {
    crate::composer::read(screen, caret, &LAYOUT, Some(CLEAR))
}
