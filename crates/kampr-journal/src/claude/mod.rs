mod record;

use std::collections::HashMap;
use std::path::PathBuf;

use serde_json::Value;

use crate::adapter::{JournalAdapter, SessionKind, SessionRef};
use crate::error::JournalError;
use crate::model::{Block, Role, ToolState, Turn};
use crate::root::TranscriptRoot;
use crate::store::TurnStore;
use crate::summary::{count_lines, summarise};
use crate::tail::TranscriptParser;

use record::{Content, ContentBlock, Record, result_text, unified_patch};

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

impl JournalAdapter for ClaudeAdapter {
    fn agent(&self) -> &str {
        AGENT
    }

    fn locate(&self, session: &SessionRef) -> Result<PathBuf, JournalError> {
        match session.kind {
            SessionKind::Id => self.find_by_id(&session.value),
            SessionKind::Path => self.root.contain(&session.value),
        }
    }

    fn parser(&self) -> Box<dyn TranscriptParser> {
        Box::new(ClaudeParser::default())
    }
}

#[derive(Default)]
pub struct ClaudeParser {
    store: TurnStore,
    tool_turns: HashMap<String, String>,
    seq: u64,
}

impl TranscriptParser for ClaudeParser {
    fn push_line(&mut self, line: &str) {
        let seq = self.seq;
        self.seq += 1;
        let Ok(record) = serde_json::from_str::<Record>(line) else {
            return;
        };
        self.ingest(record, seq);
    }

    fn reset(&mut self) {
        *self = Self::default();
    }

    fn store(&self) -> &TurnStore {
        &self.store
    }

    fn store_mut(&mut self) -> &mut TurnStore {
        &mut self.store
    }
}

impl ClaudeParser {
    fn ingest(&mut self, record: Record, seq: u64) {
        let role = match record.kind.as_str() {
            "assistant" => Role::Assistant,
            "user" => Role::User,
            _ => return,
        };
        if record.is_sidechain == Some(true) || record.is_meta == Some(true) {
            return;
        }
        let Some(content) = record.message.and_then(|m| m.content) else {
            return;
        };

        let id = record.uuid.unwrap_or_else(|| format!("c{seq}"));
        let mut turn = Turn::new(id.clone(), role, record.timestamp);

        match content {
            Content::Text(text) => turn.blocks.push(Block::Md { text }),
            Content::Blocks(blocks) => {
                for block in blocks {
                    self.ingest_block(block, &id, &mut turn, record.tool_use_result.as_ref());
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
    ) {
        match block {
            ContentBlock::Text { text } => turn.blocks.push(Block::Md { text }),
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
                self.settle(&tool_use_id, &result_text(&content), is_error, tool_use_result);
            }
            ContentBlock::Other => {}
        }
    }

    fn settle(&mut self, tool_use_id: &str, text: &str, is_error: bool, tool_use_result: Option<&Value>) {
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
    }
}
