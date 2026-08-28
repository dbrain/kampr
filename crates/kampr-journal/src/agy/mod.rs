mod facet;
mod presence;
mod record;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use crate::adapter::{JournalAdapter, SessionKind, SessionRef};
use crate::composer::{Caret, Composed, ComposerReader};
use crate::error::JournalError;
use crate::facet::{FacetFold, Facets};
use crate::live::{Layout, LiveBlock, ScreenReader};
use crate::marker::SessionMarker;
use crate::model::{Block, Role, ToolState, Turn};
use crate::process::PaneProcess;
use crate::root::TranscriptRoot;
use crate::store::TurnStore;
use crate::summary::count_lines;
use crate::tail::TranscriptParser;

use record::{Record, ToolCall, arg, diff, exit_failed, request, result_body, summarise};

pub use presence::{flocks, holder_from};

pub const AGENT: &str = "agy";

/// `agy` writes under the directory its `gemini-cli` ancestor used, in a home of its own.
pub const HOME: &str = ".gemini/antigravity-cli";

/// The **complete** transcript. The `transcript.jsonl` beside it is what the harness feeds its
/// own model: large tool results are cut and marked with `truncated_fields`, and every
/// `tool_calls[].args` value is re-encoded as a JSON string, so a summary read out of it carries
/// the quotes. Kampr's conversation view exists to show what the agent actually said, and both
/// files are appended to a line at a time, so the bigger tail costs nothing but bytes.
const TRANSCRIPT: &str = ".system_generated/logs/transcript_full.jsonl";

pub struct AgyAdapter {
    root: TranscriptRoot,
}

impl AgyAdapter {
    pub fn new(root: TranscriptRoot) -> Self {
        Self { root }
    }

    fn find_by_id(&self, id: &str) -> Result<PathBuf, JournalError> {
        self.root.check_id(id)?;
        self.root.contain(&format!("brain/{id}/{TRANSCRIPT}"))
    }
}

impl JournalAdapter for AgyAdapter {
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

    /// `presence/<conversation-id>.lock`, held open with `flock` for exactly as long as the
    /// conversation is the one the process is on — including across `/new`, which moves the lock
    /// rather than taking a second one.
    ///
    /// The kernel is the whole guard here, and it is a stronger one than a recorded start time:
    /// a lock is released when the process holding it dies, so a pid that holds one is a pid that
    /// is alive, and [`PaneProcess::owns`] has nothing left to disprove. The files themselves are
    /// never unlinked, so a directory of them is a list of conversations that have *ended*.
    fn locate_by_process(&self, process: &PaneProcess) -> Result<PathBuf, JournalError> {
        let presence = self.root.path().join("presence");
        let id = presence::holder(&presence, process.pid)
            .ok_or_else(|| JournalError::NotFound(process.pid.to_string()))?;
        self.find_by_id(&id)
    }

    /// Nothing. **No file `agy` writes before it exits binds a conversation to a directory**: the
    /// transcript declares no working directory of its own, and the two caches that do —
    /// `cache/last_conversations.json` and `conversation_summaries.db` — are written on the way
    /// out, so while a conversation is live they name the one before it. Answering from either
    /// would be answering with the previous conversation, which is the exact failure the process
    /// handle exists to stop.
    fn locate_by_cwd(&self, cwd: &Path, _since: Option<SystemTime>) -> Result<PathBuf, JournalError> {
        Err(crate::discover::not_found(cwd))
    }

    fn parser(&self) -> Box<dyn TranscriptParser> {
        Box::new(AgyParser::default())
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
}

/// A tool call waiting for the record that answers it.
#[derive(Debug)]
struct Pending {
    turn: String,
    card: usize,
    target: Option<String>,
}

#[derive(Default)]
pub struct AgyParser {
    store: TurnStore,
    /// **There is no call id anywhere in this format.** A result is the record immediately after
    /// the call, so anything that is not a result ends the run — without which a call that failed
    /// hard, and wrote no result at all, would take the *next* call's result and mark the wrong
    /// tool done.
    pending: VecDeque<Pending>,
    seq: u64,
}

impl TranscriptParser for AgyParser {
    fn push_line(&mut self, line: &str, _at: u64) {
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

impl AgyParser {
    fn ingest(&mut self, record: Record, seq: u64) {
        let source = record.source.as_deref().unwrap_or_default();
        let kind = record.kind.as_deref().unwrap_or_default();
        if (source, kind) == ("MODEL", "GENERIC") {
            if let Some(content) = record.content.as_deref() {
                self.settle(content);
            }
            return;
        }
        self.pending.clear();
        match (source, kind) {
            // `SYSTEM` / `CHECKPOINT` is the harness telling its model what it dropped from the
            // context, and it is not something anybody said.
            ("USER_EXPLICIT", "USER_INPUT") => {
                let Some(content) = record.content.as_deref() else {
                    return;
                };
                let text = request(content);
                if text.is_empty() {
                    return;
                }
                let mut turn = Turn::new(format!("a{seq}"), Role::User, record.created_at);
                turn.blocks.push(Block::md(text));
                self.store.push(turn);
            }
            ("MODEL", "PLANNER_RESPONSE") => {
                // `thinking` rides on the same record as the answer and is the harness reasoning
                // about the answer, not the answer.
                if let Some(text) = record.content.filter(|c| !c.trim().is_empty()) {
                    let mut turn = Turn::new(format!("a{seq}"), Role::Assistant, record.created_at.clone());
                    turn.blocks.push(Block::md(text));
                    self.store.push(turn);
                }
                for (at, call) in record.tool_calls.into_iter().enumerate() {
                    self.call(call, format!("a{seq}.{at}"), record.created_at.clone());
                }
            }
            _ => {}
        }
    }

    fn call(&mut self, call: ToolCall, id: String, at: Option<String>) {
        let mut turn = Turn::new(id.clone(), Role::Assistant, at);
        let card = turn.blocks.len();
        turn.blocks.push(Block::Tool {
            summary: summarise(&call.args),
            lines: None,
            state: ToolState::Running,
            name: call.name,
        });
        if let Some(command) = arg(&call.args, "CommandLine") {
            turn.blocks.push(Block::Code {
                lang: Some("bash".into()),
                text: command.to_string(),
            });
        }
        self.pending.push_back(Pending {
            turn: id,
            card,
            target: arg(&call.args, "TargetFile").map(str::to_string),
        });
        self.store.push(turn);
    }

    fn settle(&mut self, content: &str) {
        let Some(pending) = self.pending.pop_front() else {
            return;
        };
        let body = result_body(content);
        let failed = exit_failed(body);
        let patch = diff(body);
        let Some(turn) = self.store.revise(&pending.turn) else {
            return;
        };
        if let Some(Block::Tool { state, lines, .. }) = turn.tool_block_mut(pending.card) {
            *state = if failed { ToolState::Error } else { ToolState::Done };
            *lines = count_lines(body);
        }
        if let Some(text) = patch {
            turn.blocks.push(Block::Diff {
                path: pending.target,
                text,
            });
        }
    }
}

/// `agy` 1.1.18 paints an answer *inside* the block it opened for its own reasoning: `▸ Thought
/// for 4s` in column zero, the reasoning's one-line title under it, and then the message, all
/// indented by two and all in the same block. A tool card opens with `●` and is a boundary rather
/// than a message — the answer never carries that glyph. Captured live:
/// `tests/fixtures/screens`.
const LAYOUT: Layout = Layout {
    message: '▸',
    prompt: '>',
    result: '└',
    indent: 2,
    input: 2,
    reject: is_not_a_thought,
};

/// One `ctrl+u` takes the whole buffer, wrapped or not. **`ctrl+c` must not be sent to this
/// harness**: measured against the same wrapped entry it cleared nothing and painted `press
/// ctrl+c again to exit`, so a second one anywhere near it ends the session.
const CLEAR: &str = "\u{15}";

/// **Not exercised by any capture, and deliberately kept.** Every `▸` line in every frame
/// captured off `agy` 1.1.18 reads `Thought for …` — asserted over the whole corpus in
/// `tests/live.rs` — so nothing here can produce a head this refuses. It is the guard the
/// stripping below depends on: two lines are taken off the front because the harness put two
/// there, and a block opened any other way is not a block whose front is known.
fn is_not_a_thought(head: &str) -> bool {
    !head.starts_with("Thought for ")
}

/// The two lines the harness put above its own answer, taken back off.
///
/// A clipped block has already lost them off the top of the screen, and what is left is the
/// message; one with its header still on screen has them both, in that order, and they belong to
/// no turn — the record carries the reasoning in a field of its own.
pub fn live(screen: &[&str]) -> Option<LiveBlock> {
    let block = crate::live::read(screen, &LAYOUT)?;
    if block.clipped {
        return Some(block);
    }
    let text = block.text.splitn(3, '\n').nth(2)?.trim().to_string();
    (!text.is_empty()).then_some(LiveBlock { text, clipped: false })
}

pub fn composer(screen: &[&str], caret: Caret) -> Option<Composed> {
    crate::composer::read(screen, caret, &LAYOUT, Some(CLEAR))
}
