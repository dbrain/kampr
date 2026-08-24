mod record;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::adapter::{JournalAdapter, SessionKind, SessionRef};
use crate::attach::{self, Fetched, Origin};
use crate::discover;
use crate::error::JournalError;
use crate::live::{Layout, LiveBlock, ScreenReader};
use crate::model::{Attachment, Block, Role, ToolState, Turn};
use crate::root::TranscriptRoot;
use crate::store::TurnStore;
use crate::summary::{count_lines, image_marker, marker_of, one_line, summarise};
use crate::tail::TranscriptParser;

use record::{
    PATCH_PREFIX, Payload, Record, data_url_subtype, envelope_output, output_failed, output_text,
    patch_target,
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
    /// (probe #45). Codex publishes no map from a process to the thread it is on — its
    /// `~/.codex/thread-writer-locks` entries are empty files named after the thread — so the
    /// directory, bounded by when the pane's `codex` started, is the whole handle here.
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
        discover::newest_declaring(rollouts, cwd, since, |record| {
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

    fn screen(&self) -> Option<ScreenReader> {
        Some(live)
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
    tool_turns: HashMap<String, String>,
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
                // `developer` carries the harness's own instruction blocks, not the conversation.
                let role = match role.as_deref() {
                    Some("assistant") => Role::Assistant,
                    Some("user") => Role::User,
                    _ => return,
                };
                let mut turn = Turn::new(id, role, at);
                for item in content {
                    match item.kind.as_str() {
                        "input_text" | "output_text" => {
                            if let Some(text) = item.text.filter(|t| !t.is_empty()) {
                                turn.blocks.push(Block::md(text));
                            }
                        }
                        "input_image" => turn.blocks.push(Block::Md {
                            text: image_marker(item.image_url.as_deref().and_then(data_url_subtype)),
                            att: atts.next(),
                        }),
                        _ => {}
                    }
                }
                if !turn.blocks.is_empty() {
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
                self.tool_turns.insert(call_id, id);
                self.store.push(turn);
            }
            Payload::CustomToolCall { name, input, call_id } => {
                let mut turn = Turn::new(id.clone(), Role::Assistant, at);
                let patch = input.starts_with(PATCH_PREFIX);
                let target = patch.then(|| patch_target(&input)).flatten();
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
                self.tool_turns.insert(call_id, id);
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
        let Some(target) = self.tool_turns.get(call_id).cloned() else {
            return;
        };
        let failed = output_failed(raw);
        let text = envelope_output(raw);
        let Some(turn) = self.store.revise(&target) else {
            return;
        };
        if let Some(Block::Tool { state, lines, .. }) = turn.tool_block_mut() {
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

/// Codex 0.149 opens both its assistant messages and its own status line with `•` in column zero.
/// The status line is the only head worth rejecting: while a turn runs, the block at the foot of
/// the screen is either the spinner or the message being written — a tool card always has the
/// spinner painted underneath it. Captured live: `tests/fixtures/screens`.
const LAYOUT: Layout = Layout {
    message: '•',
    prompt: '›',
    result: '└',
    indent: 2,
    reject: is_status,
};

fn is_status(head: &str) -> bool {
    head.starts_with("Working (")
}

pub fn live(screen: &[&str]) -> Option<LiveBlock> {
    crate::live::read(screen, &LAYOUT)
}
