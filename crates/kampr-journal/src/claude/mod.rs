mod facet;
mod record;
mod running;
mod subagent;

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

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
use crate::model::{Attachment, Block, Role, ToolState, Turn, TurnKind};
use crate::output;
use crate::process::{PaneProcess, Started};
use crate::root::TranscriptRoot;
use crate::store::TurnStore;
use crate::sub::SubRef;
use crate::summary::{clip, count_lines, image_marker, marker_of, summarise};
use crate::tail::{FileJournal, Journal, TranscriptParser};

use record::{Content, ContentBlock, Record, image, image_subtype, result_atts, result_text, unified_patch};

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

    /// `~/.claude/sessions/<pid>.json`, which Claude 2.1.236 and later writes when a session
    /// opens and **removes when it exits** — so its presence is already the claim that this pid
    /// is on this session right now.
    ///
    /// `procStart` beside it is field 22 of `/proc/<pid>/stat` verbatim, and checking it is what
    /// stops a file the kernel has since handed the pid to somebody else from being believed.
    fn read_marker(&self, process: &PaneProcess) -> Result<SessionMarker, JournalError> {
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
            .ok_or_else(|| JournalError::NotFound(named.clone()))?;
        let field = |key: &str| session.get(key).and_then(Value::as_str).map(str::to_string);
        Ok(SessionMarker {
            agent: AGENT.to_string(),
            pid: process.pid,
            session: id.to_string(),
            cwd: field("cwd").map(PathBuf::from),
            name: field("name"),
            name_source: field("nameSource"),
            status: field("status"),
            transcript: self.find_by_id(id).ok(),
            started: started(&session),
        })
    }
}

/// `startedAt`, which the harness writes as wall-clock milliseconds when the session opens and
/// never rewrites — `nameSince` beside it carries the same value, and `updatedAt` is the field that
/// moves. Measured against the same marker's `procStart`: **0.7 s and 2.4 s after** the process
/// itself, being the harness's own boot, so it is a hair *later* than the fork rather than earlier.
///
/// A marker without it is a harness older than the one this was measured on, and that is
/// [`Started::Unknown`] rather than a guess ([#233](#)).
fn started(session: &Value) -> Started {
    match session.get("startedAt").and_then(Value::as_u64) {
        Some(millis) => Started::At(SystemTime::UNIX_EPOCH + Duration::from_millis(millis)),
        None => Started::Unknown,
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

    fn locate_by_process(&self, process: &PaneProcess) -> Result<PathBuf, JournalError> {
        let marker = self.read_marker(process)?;
        marker.transcript.ok_or(JournalError::Unwritten(marker.session))
    }

    /// Reading `sessions/<pid>.json` per candidate *is* the intersection with the marker
    /// directory, and it is cheaper than listing one.
    fn marker(&self, pipeline: &[PaneProcess]) -> Option<SessionMarker> {
        pipeline.iter().find_map(|p| self.read_marker(p).ok())
    }

    fn locate_by_cwd(&self, cwd: &Path, since: Option<SystemTime>) -> Result<PathBuf, JournalError> {
        let projects = self.root.path().join("projects");
        let named = projects.join(slug(cwd));
        let declared = |record: &Value| record.get("cwd").and_then(Value::as_str).map(str::to_string);
        if named.is_dir()
            && let Some(found) = discover::newest_declaring(
                discover::jsonl_files(&named),
                cwd,
                since,
                discover::Silent::Belongs,
                declared,
            )
        {
            return Ok(found);
        }
        let everything = discover::subdirectories(&projects)
            .iter()
            .flat_map(|d| discover::jsonl_files(d))
            .collect();
        discover::newest_declaring(everything, cwd, since, discover::Silent::Refuse, declared)
            .ok_or_else(|| discover::not_found(cwd))
    }

    fn parser(&self) -> Box<dyn TranscriptParser> {
        Box::new(ClaudeParser::default())
    }

    /// A launched conversation read as its own, which is the one place a sidechain record is this
    /// transcript's own words rather than somebody else's. `filed` is kept so an agent that
    /// launched its own is still reachable one level further down.
    fn open_sub(&self, sub: &SubRef) -> Result<Box<dyn Journal>, JournalError> {
        let path = self.root.contain(&sub.path)?;
        let mut parser = ClaudeParser {
            launched: true,
            filed: Some(Filed {
                root: self.root.clone(),
                transcript: path.clone(),
            }),
            ..ClaudeParser::default()
        };
        parser.set_origin(Origin::new(AGENT, &self.root, &path));
        Ok(Box::new(FileJournal::new(path, Box::new(parser), self.screen())))
    }

    /// A launched agent's transcript is filed relative to the launching one, so the parser is told
    /// where on disk it is reading rather than only which agent and which relative path — which is
    /// all an [`Origin`] carries.
    fn open_path(&self, path: PathBuf) -> Box<dyn Journal> {
        let mut parser = ClaudeParser {
            filed: Some(Filed {
                root: self.root.clone(),
                transcript: path.clone(),
            }),
            ..ClaudeParser::default()
        };
        parser.set_origin(Origin::new(AGENT, &self.root, &path));
        Box::new(FileJournal::new(path, Box::new(parser), self.screen()))
    }

    fn screen(&self) -> Option<ScreenReader> {
        Some(live)
    }

    fn composer(&self) -> Option<ComposerReader> {
        Some(composer)
    }

    fn facets(&self, transcript: &Path, marker: Option<&SessionMarker>) -> Facets {
        facet::collect(transcript, marker)
    }

    fn fold(&self) -> Option<Box<dyn FacetFold>> {
        Some(Box::new(facet::Fold::default()))
    }

    fn attachment(&self, record: &str, index: u32) -> Result<Fetched, JournalError> {
        let record: Record =
            serde_json::from_str(record).map_err(|_| JournalError::NotFound(index.to_string()))?;
        attach::nth(record::attachments(&record), index)
    }
}

struct Filed {
    root: TranscriptRoot,
    transcript: PathBuf,
}

#[derive(Default)]
pub struct ClaudeParser {
    /// Whether a sidechain record is this conversation's own words rather than another's.
    ///
    /// **The same flag means opposite things either side of the boundary.** In a pane's own
    /// transcript `isSidechain` marks a record that belongs to something the agent *launched*, and
    /// inlining it would put a subagent's words in the parent's voice — so it is dropped. In the
    /// launched conversation's own file every record carries it (134 of 134, measured), because
    /// the whole file is the sidechain: dropping them there leaves a reader an empty panel, which
    /// is what shipped until a real transcript was put through this rather than a fixture built to
    /// pass.
    launched: bool,
    store: TurnStore,
    tool_turns: HashMap<String, (String, usize)>,
    metas: subagent::Metas,
    /// The calls a card has already been minted for. One `tool_use_id` is one launch and one
    /// handle, and both ends now mint: the meta at the call, the `agentId` at the result.
    minted: HashSet<String>,
    seq: u64,
    origin: Option<Origin>,
    filed: Option<Filed>,
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
            filed: self.filed.take(),
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
        if (record.is_sidechain == Some(true) && !self.launched) || record.is_meta == Some(true) {
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
        // `/compact` files the harness's summary as a user record and leaves this flag as the only
        // thing separating it from a prompt (#259). Dropping it would take the operator's own
        // history away; leaving it unmarked is what put it in their voice.
        turn.kind = record
            .is_compact_summary
            .unwrap_or(false)
            .then_some(TurnKind::Compact);
        let mut atts = atts.into_iter();

        match content {
            Content::Text(text) => push_text(&mut turn, text),
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
            ContentBlock::Text { text } => push_text(turn, text),
            ContentBlock::Image { source } => turn.blocks.push(Block::Md {
                text: image_marker(image_subtype(&source)),
                att: image(&source).and_then(|_| atts.next()),
            }),
            ContentBlock::ToolUse { id, name, input } => {
                let at = turn.blocks.len();
                let calling = self.calling(&id, &input);
                turn.blocks.push(Block::Tool {
                    summary: calling
                        .as_ref()
                        .and_then(|(_, found)| found.label())
                        .or_else(|| summarise(&input)),
                    lines: None,
                    state: ToolState::Running,
                    name: name.clone(),
                });
                if let Some((handle, found)) = calling {
                    self.minted.insert(id.clone());
                    turn.blocks.push(Block::Sub {
                        id: handle,
                        kind: found.kind,
                        title: found.title,
                        depth: found.depth,
                    });
                }
                if let Some(command) = input.get("command").and_then(Value::as_str) {
                    turn.blocks.push(Block::Code {
                        lang: Some("bash".into()),
                        text: command.to_string(),
                        role: None,
                    });
                }
                self.tool_turns.insert(id, (turn_id.to_string(), at));
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

    /// The handle a launched conversation is opened by — an `Agent` call is named by the `agentId`
    /// on its result rather than by the tool's own name, which is a label.
    fn launched(&self, result: &Value) -> Option<(String, subagent::Launched)> {
        let filed = self.filed.as_ref()?;
        let found = subagent::launched(&filed.transcript, result)?;
        let id = SubRef::new(AGENT, &filed.root, &found.transcript).encode();
        Some((id, found))
    }

    /// The same handle a launch's own `toolUseId` names, at the moment of the call. A launch with
    /// `run_in_background: false` writes no result for 65–146 s, so this is the only way to watch
    /// one while it is working.
    ///
    /// `subagent_type` is what makes a call a launch, and it is checked to keep the directory
    /// listing off every `Bash` and `Read` rather than to decide anything: a call that is not a
    /// launch matches no `toolUseId` and would yield nothing anyway.
    fn calling(&mut self, tool_use_id: &str, input: &Value) -> Option<(String, subagent::Launched)> {
        input.get("subagent_type")?;
        let filed = self.filed.as_ref()?;
        let (transcript, root) = (filed.transcript.clone(), filed.root.clone());
        let found = subagent::calling(&transcript, &mut self.metas, tool_use_id, input)?;
        let id = SubRef::new(AGENT, &root, &found.transcript).encode();
        Some((id, found))
    }

    fn settle(
        &mut self,
        tool_use_id: &str,
        text: &str,
        is_error: bool,
        tool_use_result: Option<&Value>,
        images: Vec<Attachment>,
    ) {
        let Some((target, at)) = self.tool_turns.get(tool_use_id).cloned() else {
            return;
        };
        let patch = tool_use_result.and_then(unified_patch);
        let launched = (!self.minted.contains(tool_use_id))
            .then(|| tool_use_result.and_then(|result| self.launched(result)))
            .flatten();
        let inserted = {
            let Some(turn) = self.store.revise(&target) else {
                return;
            };
            let mut carry = false;
            if let Some(Block::Tool {
                state,
                lines,
                summary,
                name,
            }) = turn.tool_block_mut(at)
            {
                *state = if is_error {
                    ToolState::Error
                } else {
                    ToolState::Done
                };
                *lines = count_lines(text);
                carry = lines.is_some() && (is_error || RESULT_IS_THE_POINT.contains(&name.as_str()));
                if let Some(label) = launched.as_ref().and_then(|(_, found)| found.label()) {
                    *summary = Some(label);
                }
            }
            if let Some((id, found)) = launched {
                turn.blocks.push(Block::Sub {
                    id,
                    kind: found.kind,
                    title: found.title,
                    depth: found.depth,
                });
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
            carry.then(|| output::place(turn, at, clip(text))).flatten()
        };
        // A card records where it sits, and one parallel call's result landing before another's
        // would leave the second pointing at the block that was pushed in front of it.
        if let Some(from) = inserted {
            for (turn_id, other) in self.tool_turns.values_mut() {
                if turn_id == &target && *other >= from {
                    *other += 1;
                }
            }
        }
    }
}

/// The calls whose result *is* the point, and so the only ones worth the bytes above.
///
/// `Read` is absent because the client has a better surface for one already — it fetches the real
/// file from the path on the card — and `Edit`/`Write` because their result is the `diff` block
/// beside them. Repeating either costs a page's budget and tells a reader nothing new. An error
/// is carried whatever the call was, because then the text is the whole message.
const RESULT_IS_THE_POINT: &[&str] = &["Bash", "Glob", "Grep"];

/// Claude 2.1.239 opens every assistant message with `●` in column zero and indents the wrapped
/// remainder by two, and opens a tool card exactly the same way — `● Write(notes.md)` — so the
/// call shape is what separates a card from prose. Captured live: `tests/fixtures/screens`.
const LAYOUT: Layout = Layout {
    message: '●',
    prompt: '❯',
    result: '⎿',
    indent: 2,
    input: 2,
    reject: is_tool_card,
};

/// **`ctrl+u` is the wrong key here and looks like the right one.** Measured against a 200-column
/// entry wrapped over three rows, one `ctrl+u` took a single *visual row* and left the rest, and
/// so did `ctrl+a ctrl+k`; eight of them cleared it, which is a number that only holds for that
/// length. `ctrl+c` took the whole buffer in one send and left the harness running. On agy the
/// same key arms an exit instead, which is why this is per-harness and not a constant.
const CLEAR: &str = "\u{3}";

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

pub fn composer(screen: &[&str], caret: Caret) -> Option<Composed> {
    crate::composer::read(screen, caret, &LAYOUT, Some(CLEAR))
}
