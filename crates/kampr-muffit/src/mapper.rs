use crate::wire::{ContentBlock, DiffHunk, Liveness, Subagent, ToolCallStatus, TranscriptEntry};
use kampr_journal::{Block, CodeRole, Role, ToolState, Turn};
use std::collections::HashMap;

const SUMMARY_CHARS: usize = 120;
const DELEGATE: &str = "delegate";

pub fn turn_id(seq: u64) -> String {
    format!("m{seq}")
}

pub fn seq_of(turn: &str) -> Option<u64> {
    turn.strip_prefix('m')?.parse().ok()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worker {
    pub id: String,
    pub title: Option<String>,
    pub running: bool,
}

/// The workers a session's delegate calls started, by the call that started each.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Workers(pub HashMap<String, Worker>);

impl Workers {
    pub fn of(subagents: &[Subagent]) -> Self {
        Self(
            subagents
                .iter()
                .filter_map(|s| {
                    let call = s.call_id.clone()?;
                    Some((
                        call,
                        Worker {
                            id: s.id.clone(),
                            title: s.title.clone(),
                            running: s.liveness != Liveness::Dead,
                        },
                    ))
                })
                .collect(),
        )
    }
}

#[derive(Debug, Clone)]
struct Call {
    name: String,
    summary: Option<String>,
    title: Option<String>,
    failed: bool,
    finished: bool,
    lines: Option<u32>,
    output: Option<String>,
    after: Vec<Block>,
}

#[derive(Debug, Clone)]
enum Segment {
    Plain(Block),
    Call(String),
}

/// One turn's blocks as they arrive, with each tool call kept in one place so that its output,
/// its result and its diff land beside it however the calls of a turn interleave.
#[derive(Debug, Clone, Default)]
pub struct TurnBuilder {
    segments: Vec<Segment>,
    calls: HashMap<String, Call>,
}

impl TurnBuilder {
    pub fn is_empty(&self) -> bool {
        self.segments.is_empty()
    }

    pub fn push(&mut self, block: &ContentBlock) {
        match block {
            ContentBlock::Text { text } => self.text(text),
            ContentBlock::Image { mime, .. } => {
                self.segments
                    .push(Segment::Plain(Block::md(format!("[image · {mime}]"))));
            }
            ContentBlock::ToolCall {
                id,
                tool,
                args,
                status,
            } => self.call(id, tool, args, *status),
            ContentBlock::ToolOutputChunk { call_id, text } => {
                if let Some(call) = self.calls.get_mut(call_id) {
                    call.output.get_or_insert_with(String::new).push_str(text);
                }
            }
            ContentBlock::ToolResult {
                call_id,
                summary,
                total_lines,
                ..
            } => self.result(call_id, summary, *total_lines),
            ContentBlock::Diff { call_id, path, hunks } => {
                let diff = Block::Diff {
                    path: Some(path.clone()),
                    text: unified(hunks),
                };
                match call_id.as_ref().and_then(|c| self.calls.get_mut(c)) {
                    Some(call) => call.after.push(diff),
                    None => self.segments.push(Segment::Plain(diff)),
                }
            }
            _ => {}
        }
    }

    fn text(&mut self, text: &str) {
        if let Some(Segment::Plain(Block::Md {
            text: held,
            att: None,
        })) = self.segments.last_mut()
        {
            held.push_str(text);
            return;
        }
        self.segments.push(Segment::Plain(Block::md(text)));
    }

    fn call(&mut self, id: &str, tool: &str, args: &str, status: ToolCallStatus) {
        let failed = status == ToolCallStatus::Failed;
        let finished = status == ToolCallStatus::Finished;
        match self.calls.get_mut(id) {
            Some(call) => {
                call.failed |= failed;
                call.finished |= finished;
                if !args.is_empty() {
                    call.summary = one_line(args);
                    call.title = arg_title(args);
                }
            }
            None => {
                self.calls.insert(
                    id.to_string(),
                    Call {
                        name: tool.to_string(),
                        summary: one_line(args),
                        title: arg_title(args),
                        failed,
                        finished,
                        lines: None,
                        output: None,
                        after: Vec::new(),
                    },
                );
                self.segments.push(Segment::Call(id.to_string()));
            }
        }
    }

    pub fn result(&mut self, call_id: &str, summary: &str, total_lines: Option<u64>) {
        if let Some(call) = self.calls.get_mut(call_id) {
            call.finished = true;
            call.output = Some(summary.to_string());
            let lines = total_lines.unwrap_or_else(|| summary.lines().count() as u64);
            call.lines = Some(u32::try_from(lines).unwrap_or(u32::MAX));
        }
    }

    /// `results` is whether a call's result lands in this turn: a transcript holds it in a later
    /// `tool` entry, so a recorded call with none yet is still running however its status reads.
    pub fn blocks(&self, workers: &Workers, results: bool) -> Vec<Block> {
        let mut out = Vec::new();
        for segment in &self.segments {
            match segment {
                Segment::Plain(Block::Md { text, att }) => {
                    let text = text.trim();
                    if !text.is_empty() {
                        out.push(Block::Md {
                            text: text.to_string(),
                            att: att.clone(),
                        });
                    }
                }
                Segment::Plain(block) => out.push(block.clone()),
                Segment::Call(id) => {
                    let call = &self.calls[id];
                    let done = match results {
                        true => call.lines.is_some(),
                        false => call.finished || call.lines.is_some(),
                    };
                    out.push(Block::Tool {
                        name: call.name.clone(),
                        summary: call.summary.clone(),
                        lines: call.lines,
                        state: match (call.failed, done) {
                            (true, _) => ToolState::Error,
                            (false, true) => ToolState::Done,
                            (false, false) => ToolState::Running,
                        },
                    });
                    if call.name == DELEGATE
                        && let Some(worker) = workers.0.get(id)
                    {
                        out.push(Block::Sub {
                            id: worker.id.clone(),
                            kind: Some("worker".into()),
                            title: worker.title.clone().or_else(|| call.title.clone()),
                            depth: None,
                        });
                    }
                    if let Some(output) = call.output.as_deref().filter(|o| !o.trim().is_empty()) {
                        out.push(Block::Code {
                            lang: None,
                            text: output.trim_end().to_string(),
                            role: Some(CodeRole::Output),
                        });
                    }
                    out.extend(call.after.iter().cloned());
                }
            }
        }
        out
    }
}

/// One `WireTranscriptEntry` is one turn, `m<seq>`; a `tool` entry folds into the turns whose calls
/// it answers, and `system` entries are not the conversation (docs/18).
pub fn turns<'a>(entries: impl IntoIterator<Item = &'a TranscriptEntry>, workers: &Workers) -> Vec<Turn> {
    let mut built: Vec<(u64, Role, TurnBuilder)> = Vec::new();
    let mut owner: HashMap<String, usize> = HashMap::new();
    for entry in entries {
        let role = match entry.role.as_str() {
            "user" => Some(Role::User),
            "assistant" => Some(Role::Assistant),
            _ => None,
        };
        let mut own = role.map(|_| TurnBuilder::default());
        for block in &entry.blocks {
            let call = match block {
                ContentBlock::ToolResult { call_id, .. } => Some(call_id.as_str()),
                ContentBlock::Diff { call_id, .. } => call_id.as_deref(),
                _ => None,
            };
            if let Some(at) = call.and_then(|c| owner.get(c)) {
                built[*at].2.push(block);
                continue;
            }
            if let Some(own) = own.as_mut() {
                own.push(block);
                if let ContentBlock::ToolCall { id, .. } = block {
                    owner.insert(id.clone(), built.len());
                }
            }
        }
        if let (Some(role), Some(own)) = (role, own) {
            built.push((entry.seq, role, own));
        }
    }
    built
        .into_iter()
        .filter_map(|(seq, role, builder)| {
            let blocks = builder.blocks(workers, true);
            (!blocks.is_empty()).then(|| Turn {
                blocks,
                ..Turn::new(turn_id(seq), role, None)
            })
        })
        .collect()
}

fn arg_title(args: &str) -> Option<String> {
    let value: serde_json::Value = serde_json::from_str(args).ok()?;
    value["title"].as_str().map(str::to_string)
}

/// The arguments a tool was called with, as one line a card can carry.
pub fn one_line(args: &str) -> Option<String> {
    let value: Option<serde_json::Value> = serde_json::from_str(args).ok();
    let picked = value.as_ref().and_then(|v| {
        [
            "command",
            "title",
            "path",
            "file_path",
            "pattern",
            "query",
            "url",
            "brief",
        ]
        .iter()
        .find_map(|key| v[*key].as_str().map(str::to_string))
    });
    let text = match (picked, &value) {
        (Some(picked), _) => picked,
        (None, Some(serde_json::Value::Object(o))) if o.is_empty() => return None,
        (None, Some(value)) => value.to_string(),
        (None, None) => args.to_string(),
    };
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.is_empty() {
        return None;
    }
    Some(match flat.chars().count() > SUMMARY_CHARS {
        true => format!("{}…", flat.chars().take(SUMMARY_CHARS).collect::<String>()),
        false => flat,
    })
}

pub fn unified(hunks: &[DiffHunk]) -> String {
    let mut out = String::new();
    for hunk in hunks {
        out.push_str(&format!(
            "@@ -{},{} +{},{} @@\n",
            hunk.old_start, hunk.old_lines, hunk.new_start, hunk.new_lines
        ));
        for line in hunk.old_text.lines() {
            out.push('-');
            out.push_str(line);
            out.push('\n');
        }
        for line in hunk.new_text.lines() {
            out.push('+');
            out.push_str(line);
            out.push('\n');
        }
    }
    out.trim_end_matches('\n').to_string()
}

#[cfg(test)]
mod tests;
