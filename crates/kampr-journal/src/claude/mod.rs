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
use crate::process::PaneProcess;
use crate::root::TranscriptRoot;
use crate::store::TurnStore;
use crate::summary::{count_lines, image_marker, marker_of, summarise};
use crate::tail::TranscriptParser;

use record::{Content, ContentBlock, Record, image_subtype, result_atts, result_text, unified_patch};

pub const AGENT: &str = "claude";

pub struct ClaudeAdapter {
    root: TranscriptRoot,
}

impl ClaudeAdapter {
    pub fn new(root: TranscriptRoot) -> Self {
        Self { root }
    }

    /// `~/.claude/projects/<slug>/<uuid>.jsonl`. The slug is derived from the pane's cwd, which
    /// the pane does not tell us, so an id is found by scanning the project directories.
    fn find_by_id(&self, id: &str) -> Result<PathBuf, JournalError> {
        self.root.check_id(id)?;
        let name = format!("{id}.jsonl");
        let projects = self.root.path().join("projects");
        let Ok(entries) = std::fs::read_dir(&projects) else {
            return Err(JournalError::NotFound(id.to_string()));
        };
        for entry in entries.flatten() {
            let candidate = entry.path().join(&name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
        Err(JournalError::NotFound(id.to_string()))
    }
}

/// Claude names a project directory after its working directory with every `/` replaced by `-`.
/// It is only a hint here — the transcript's own `cwd` is what decides — so a future change to
/// the rule costs a slower search rather than a wrong conversation.
fn slug(cwd: &Path) -> String {
    cwd.to_string_lossy().trim_end_matches('/').replace('/', "-")
}

impl JournalAdapter for ClaudeAdapter {
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

    /// `~/.claude/sessions/<pid>.json`, which Claude 2.1.236 and later writes when a session
    /// opens and **removes when it exits** — so its presence is already the claim that this pid
    /// is on this session right now.
    ///
    /// `procStart` beside it is field 22 of `/proc/<pid>/stat` verbatim, and checking it is what
    /// stops a file the kernel has since handed the pid to somebody else from being believed.
    fn locate_by_process(&self, process: &PaneProcess) -> Result<PathBuf, JournalError> {
        let named = format!("{}.json", process.pid);
        let record = self.root.contain(&format!("sessions/{named}"))?;
        let text = std::fs::read_to_string(&record).map_err(|_| JournalError::NotFound(named.clone()))?;
        let session: Value =
            serde_json::from_str(&text).map_err(|_| JournalError::NotFound(named.clone()))?;
        if !process.owns(session.get("procStart").and_then(Value::as_str)) {
            return Err(JournalError::NotFound(named));
        }
        let id = session
            .get("sessionId")
            .and_then(Value::as_str)
            .ok_or(JournalError::NotFound(named))?;
        self.find_by_id(id)
    }

    fn locate_by_cwd(&self, cwd: &Path, since: Option<SystemTime>) -> Result<PathBuf, JournalError> {
        let projects = self.root.path().join("projects");
        let named = projects.join(slug(cwd));
        let declared = |record: &Value| record.get("cwd").and_then(Value::as_str).map(str::to_string);
        if named.is_dir()
            && let Some(found) =
                discover::newest_declaring(discover::jsonl_files(&named), cwd, since, declared)
        {
            return Ok(found);
        }
        let everything = discover::subdirectories(&projects)
            .iter()
            .flat_map(|d| discover::jsonl_files(d))
            .collect();
        discover::newest_declaring(everything, cwd, since, declared).ok_or_else(|| discover::not_found(cwd))
    }

    fn parser(&self) -> Box<dyn TranscriptParser> {
        Box::new(ClaudeParser::default())
    }

    fn screen(&self) -> Option<ScreenReader> {
        Some(live)
    }

    fn attachment(&self, record: &str, index: u32) -> Result<Fetched, JournalError> {
        let record: Record =
            serde_json::from_str(record).map_err(|_| JournalError::NotFound(index.to_string()))?;
        attach::nth(record::attachments(&record), index)
    }
}

#[derive(Default)]
pub struct ClaudeParser {
    store: TurnStore,
    tool_turns: HashMap<String, String>,
    seq: u64,
    origin: Option<Origin>,
}

impl TranscriptParser for ClaudeParser {
    fn push_line(&mut self, line: &str, at: u64) {
        let seq = self.seq;
        self.seq += 1;
        let Ok(record) = serde_json::from_str::<Record>(line) else {
            return;
        };
        self.ingest(record, seq, at);
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

impl ClaudeParser {
    fn ingest(&mut self, record: Record, seq: u64, at: u64) {
        let role = match record.kind.as_str() {
            "assistant" => Role::Assistant,
            "user" => Role::User,
            _ => return,
        };
        if record.is_sidechain == Some(true) || record.is_meta == Some(true) {
            return;
        }
        let atts = match &self.origin {
            Some(origin) => attach::headers(origin, at, &record::attachments(&record)),
            None => Vec::new(),
        };
        let Some(content) = record.message.and_then(|m| m.content) else {
            return;
        };

        let id = record.uuid.unwrap_or_else(|| format!("c{seq}"));
        let mut turn = Turn::new(id.clone(), role, record.timestamp);
        let mut atts = atts.into_iter();

        match content {
            Content::Text(text) => turn.blocks.push(Block::md(text)),
            Content::Blocks(blocks) => {
                for block in blocks {
                    self.ingest_block(block, &id, &mut turn, record.tool_use_result.as_ref(), &mut atts);
                }
            }
        }

        if !turn.blocks.is_empty() {
            self.store.push(turn);
        }
    }

    fn ingest_block(
        &mut self,
        block: ContentBlock,
        turn_id: &str,
        turn: &mut Turn,
        tool_use_result: Option<&Value>,
        atts: &mut impl Iterator<Item = Attachment>,
    ) {
        match block {
            ContentBlock::Text { text } => turn.blocks.push(Block::md(text)),
            ContentBlock::Image { source } => turn.blocks.push(Block::Md {
                text: image_marker(image_subtype(&source)),
                att: atts.next(),
            }),
            ContentBlock::ToolUse { id, name, input } => {
                turn.blocks.push(Block::Tool {
                    summary: summarise(&input),
                    lines: None,
                    state: ToolState::Running,
                    name: name.clone(),
                });
                if let Some(command) = input.get("command").and_then(Value::as_str) {
                    turn.blocks.push(Block::Code {
                        lang: Some("bash".into()),
                        text: command.to_string(),
                    });
                }
                self.tool_turns.insert(id, turn_id.to_string());
            }
            ContentBlock::ToolResult {
                tool_use_id,
                content,
                is_error,
            } => {
                let images: Vec<Attachment> = atts.by_ref().take(result_atts(&content).len()).collect();
                self.settle(
                    &tool_use_id,
                    &result_text(&content),
                    is_error,
                    tool_use_result,
                    images,
                );
            }
            ContentBlock::Other => {}
        }
    }

    fn settle(
        &mut self,
        tool_use_id: &str,
        text: &str,
        is_error: bool,
        tool_use_result: Option<&Value>,
        images: Vec<Attachment>,
    ) {
        let Some(target) = self.tool_turns.get(tool_use_id).cloned() else {
            return;
        };
        let patch = tool_use_result.and_then(unified_patch);
        let Some(turn) = self.store.revise(&target) else {
            return;
        };
        if let Some(Block::Tool { state, lines, .. }) = turn.tool_block_mut() {
            *state = if is_error {
                ToolState::Error
            } else {
                ToolState::Done
            };
            *lines = count_lines(text);
        }
        if let Some((path, text)) = patch {
            turn.blocks.push(Block::Diff { path, text });
        }
        for att in images {
            turn.blocks.push(Block::Md {
                text: marker_of(&att),
                att: Some(att),
            });
        }
    }
}

/// Claude 2.1.239 opens every assistant message with `●` in column zero and indents the wrapped
/// remainder by two, and opens a tool card exactly the same way — `● Write(notes.md)` — so the
/// call shape is what separates a card from prose. Captured live: `tests/fixtures/screens`.
const LAYOUT: Layout = Layout {
    message: '●',
    prompt: '❯',
    result: '⎿',
    indent: 2,
    reject: is_tool_card,
};

/// `Write(notes.md)`, `Bash(herdr pane list)`, `Read(…)`. Prose does not open with a bare
/// identifier and an opening bracket, and a card is already in the transcript under its own turn.
fn is_tool_card(head: &str) -> bool {
    let name: String = head
        .chars()
        .take_while(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect();
    !name.is_empty()
        && name.starts_with(|c: char| c.is_ascii_alphabetic())
        && head[name.len()..].starts_with('(')
}

pub fn live(screen: &[&str]) -> Option<LiveBlock> {
    crate::live::read(screen, &LAYOUT)
}
