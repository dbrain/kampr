use serde::Deserialize;
use serde_json::Value;

use crate::attach::{Att, IMAGE};
use crate::summary::one_line;

/// One line of an `omp` session file. Every variant here is a record kind measured on omp 18.1.10
/// and documented in the harness's own `docs/session.md`; everything else is bookkeeping the
/// conversation does not contain — `model_change`, `thinking_level_change`, `label`,
/// `credential_pin`, `ttsr_injection`, `service_tier_change`, `branch_summary`, `reset_boundary`.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Record {
    Title(Slot),
    /// The audit entry a rename appends beside the in-place rewrite of the slot. Same two fields,
    /// and it is the only one a fold reading appended bytes ever sees.
    TitleChange(Slot),
    Session(Header),
    Message(Entry),
    CustomMessage(Notice),
    Compaction(Compacted),
    ModeChange(ModeChange),
    #[serde(other)]
    Other,
}

impl Record {
    /// The entry's own place in the session tree: its id and its parent's.
    ///
    /// **Every entry with an id is a node, bookkeeping included** — the chain runs through a
    /// `title_change` and a `model_change` as much as through a message, and a walk that stopped at
    /// one would drop the branch above it. The kinds this parser types carry the pair already; the
    /// rest are read by [`Node::of`] off the same line, which is a small parse of a short record
    /// rather than a second parse of every message in the file.
    pub fn walked(&self) -> Option<(String, Option<String>)> {
        let (id, parent) = match self {
            Self::Message(entry) => (entry.id.clone()?, entry.parent_id.clone()),
            Self::CustomMessage(notice) => (notice.id.clone()?, notice.parent_id.clone()),
            Self::Compaction(compacted) => (compacted.id.clone()?, compacted.parent_id.clone()),
            _ => return None,
        };
        Some((id, parent))
    }
}

/// The two fields every entry but the header and the title slot carries.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
}

impl Node {
    pub fn of(line: &str) -> Option<(String, Option<String>)> {
        let node: Self = serde_json::from_str(line).ok()?;
        Some((node.id?, node.parent_id))
    }
}

/// The fixed-width first line. It carries the title the session has *now* — rewritten in place
/// rather than appended — and 256 bytes of padding so rewriting it never moves the header.
#[derive(Debug, Deserialize)]
pub struct Slot {
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub source: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Header {
    pub cwd: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub title_source: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub timestamp: Option<String>,
    pub message: Message,
}

/// When a message was written, in epoch milliseconds — the stamp inside the message rather than
/// the entry's own RFC 3339 one, because it is the same clock `completedAt` is on.
pub fn wrote_at(message: &Message) -> Option<f64> {
    match message {
        Message::User { timestamp, .. } | Message::Assistant { timestamp, .. } => *timestamp,
        _ => None,
    }
}

/// The harness's own notice, injected into the conversation rather than spoken by anyone.
/// `async-result` is the one that matters: it is how a detached subagent's yield comes back, and
/// it names the job in `details.jobs[].jobId`.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub custom_type: Option<String>,
    #[serde(default)]
    pub details: Value,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Compacted {
    pub id: Option<String>,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub timestamp: Option<String>,
    #[serde(default)]
    pub summary: Option<String>,
    #[serde(default)]
    pub tokens_before: Option<u64>,
}

#[derive(Debug, Deserialize)]
pub struct ModeChange {
    pub mode: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(tag = "role", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Message {
    User {
        #[serde(default)]
        content: Content,
        #[serde(default)]
        timestamp: Option<f64>,
    },
    Assistant {
        #[serde(default)]
        content: Content,
        #[serde(default)]
        timestamp: Option<f64>,
        /// When the model finished writing this message, in epoch milliseconds. `duration` and
        /// `ttft` sit beside it and are the model call's own time — a finer measurement than a
        /// turn's, and deliberately not published as one.
        #[serde(default)]
        completed_at: Option<f64>,
        /// `toolUse` while the turn goes on, `stop` when it is over, `error` on a call the harness
        /// then retried.
        #[serde(default)]
        stop_reason: Option<String>,
    },
    ToolResult {
        tool_call_id: String,
        #[serde(default)]
        content: Content,
        #[serde(default)]
        is_error: bool,
        /// What the tool filed beside its own words. `edit` puts the whole change in here — the
        /// path, the op, and a line-numbered diff.
        #[serde(default)]
        details: Value,
    },
    /// The operator's own `!` shell escape: a command they ran at the desk, filed in the
    /// conversation with its output rather than as a tool the model called.
    BashExecution {
        command: String,
        #[serde(default)]
        output: Option<String>,
        #[serde(default)]
        exit_code: Option<i64>,
    },
    /// `developer` carries the harness's system reminders, `custom` an extension's injection.
    /// Neither is a thing a person or the model said.
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<Block>),
    /// A shape nobody has measured. It is the last arm on purpose: an untagged enum that refuses
    /// takes the whole record down with it, and one unreadable message must not empty a pane.
    Anything(serde::de::IgnoredAny),
}

impl Default for Content {
    fn default() -> Self {
        Self::Anything(serde::de::IgnoredAny)
    }
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum Block {
    Text {
        text: String,
    },
    /// A picture, base64 in the record.
    ///
    /// **Measured inline** ([#493](#)): omp re-encoded a 7.8 KB PNG down to 678 bytes, so it never
    /// reached the 1 024-character threshold its own `session.md` says a payload is
    /// content-addressed into `blobs/<sha256>` at and replaced with `blob:sha256:<hash>`. That
    /// shape is documented and unobserved, and a record carrying one instead of `data` yields no
    /// attachment here rather than a header pointing at bytes the record does not hold.
    Image {
        #[serde(default)]
        data: Option<String>,
        #[serde(default)]
        mime_type: Option<String>,
    },
    ToolCall {
        id: String,
        name: String,
        #[serde(default)]
        arguments: Value,
    },
    /// `thinking` is the model reasoning about its answer rather than the answer, and no client
    /// this crate serves renders one for any other harness either.
    #[serde(other)]
    Other,
}

/// Every attachment in one entry, in the order [`super::OmpParser`] meets them.
pub fn attachments(entry: &Entry) -> Vec<Att<'_>> {
    let content = match &entry.message {
        Message::User { content, .. }
        | Message::Assistant { content, .. }
        | Message::ToolResult { content, .. } => content,
        _ => return Vec::new(),
    };
    let Content::Blocks(blocks) = content else {
        return Vec::new();
    };
    blocks.iter().filter_map(picture).collect()
}

pub fn picture(block: &Block) -> Option<Att<'_>> {
    let Block::Image { data, mime_type } = block else {
        return None;
    };
    Some(Att {
        kind: IMAGE,
        mime: mime_type.as_deref(),
        name: None,
        data: data.as_deref()?,
    })
}

pub fn subtype(block: &Block) -> Option<&str> {
    match block {
        Block::Image { mime_type, .. } => mime_type.as_deref()?.strip_prefix("image/"),
        _ => None,
    }
}

/// The tool result a *detached* spawn writes at the call: an acknowledgement, not an ending.
///
/// `task/index.ts` writes `Spawned agent \`<id>\` (job \`<id>\`). Its result auto-delivers on
/// yield…`, and the yield arrives minutes later as an `async-result` notice. A spawn the parent
/// blocked on writes its report here instead, so the prefix is what separates a launch that is
/// still running from one that is over.
const SPAWNED: &str = "Spawned agent `";

pub fn acknowledgement(text: &str) -> bool {
    text.trim_start().starts_with(SPAWNED)
}

/// Every agent named by a `Spawned agent \`x\`` line in a result — the only place an omp spawn
/// that was given no `name` publishes the one it generated for itself.
pub fn spawned(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    for tail in text.split(SPAWNED).skip(1) {
        if let Some(name) = tail.split('`').next().filter(|n| !n.is_empty()) {
            found.push(name.to_string());
        }
    }
    found
}

/// The jobs an `async-result` notice reports finished, from the `details` the harness files
/// beside its own prose.
pub fn finished(details: &Value) -> Vec<String> {
    details
        .get("jobs")
        .and_then(Value::as_array)
        .map(|jobs| {
            jobs.iter()
                .filter_map(|job| job.get("jobId").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

/// The spawns one `task` call asks for: the flat form is one agent, the batch form a list of
/// them under `tasks`. A spawn with no `name` is not named here at all — omp generates an
/// `AdjectiveNoun` for it and the call never learns it, which is what [`spawned`] is for.
pub struct Spawn {
    pub name: Option<String>,
    pub kind: Option<String>,
    pub task: Option<String>,
}

pub fn spawns(arguments: &Value) -> Vec<Spawn> {
    let one = |item: &Value| Spawn {
        name: item.get("name").and_then(Value::as_str).map(str::to_string),
        kind: item.get("agent").and_then(Value::as_str).map(str::to_string),
        task: item.get("task").and_then(Value::as_str).map(one_line),
    };
    match arguments.get("tasks").and_then(Value::as_array) {
        Some(items) => items.iter().map(one).collect(),
        None => vec![one(arguments)],
    }
}

/// What a tool result put on the screen, out of the block list omp files it as.
pub fn result_text(content: &Content) -> String {
    match content {
        Content::Text(text) => text.clone(),
        Content::Blocks(blocks) => blocks
            .iter()
            .filter_map(|block| match block {
                Block::Text { text } => Some(text.as_str()),
                _ => None,
            })
            .collect::<Vec<_>>()
            .join("\n"),
        Content::Anything(_) => String::new(),
    }
}

/// omp's `edit` result, as the unified hunks the wire already carries.
///
/// **Its own diff is line-numbered rather than unified**: `details.diff` is one row per line,
/// `-2|line two` / `+2|second line, changed` / ` 3|line three`, with the numbers referring to the
/// file rather than to the patch and a **gap in them** where a hunk ends. Measured on omp 18.1.10
/// against a two-place edit, which is what says the gap is the boundary ([#491](#)).
///
/// So the header is arithmetic over the harness's own numbers and not an invention: a hunk runs
/// while the old side stays contiguous, its start is the first number on that side, and its length
/// is the rows that side has. A row that does not parse yields **nothing at all** — a block that
/// merely looks like a diff is worse on the wire than no block, because a reader cannot tell.
pub fn unified_patch(details: &Value) -> Option<(Option<String>, String)> {
    let rows: Vec<Row> = details
        .get("diff")?
        .as_str()?
        .lines()
        .map(row)
        .collect::<Option<_>>()?;
    if rows.is_empty() {
        return None;
    }
    let path = details.get("path").and_then(Value::as_str).map(str::to_string);
    let mut out = String::new();
    let mut hunk: Vec<&Row> = Vec::new();
    let mut last: Option<u64> = None;
    let mut shift: i64 = 0;
    for entry in &rows {
        let broken = matches!((entry.old(), last), (Some(at), Some(prev)) if at != prev + 1);
        if broken && !hunk.is_empty() {
            out.push_str(&emit(&hunk, shift));
            shift += drift(&hunk);
            hunk.clear();
        }
        last = entry.old().or(last);
        hunk.push(entry);
    }
    out.push_str(&emit(&hunk, shift));
    (!out.is_empty()).then_some((path, out))
}

struct Row {
    marker: char,
    at: u64,
    text: String,
}

impl Row {
    /// The line number on the *old* side, which is the side a hunk's contiguity is counted on. An
    /// inserted row has none, and so cannot break a hunk it sits inside.
    fn old(&self) -> Option<u64> {
        (self.marker != '+').then_some(self.at)
    }

    fn fresh(&self) -> bool {
        self.marker != '-'
    }
}

fn row(line: &str) -> Option<Row> {
    let mut chars = line.chars();
    let marker = chars.next().filter(|c| matches!(c, '+' | '-' | ' '))?;
    let rest = chars.as_str();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let text = rest[digits.len()..].strip_prefix('|')?;
    Some(Row {
        marker,
        at: digits.parse().ok()?,
        text: text.to_string(),
    })
}

/// What one hunk does to every line number below it: what it added, less what it took away.
fn drift(hunk: &[&Row]) -> i64 {
    hunk.iter().filter(|r| r.marker == '+').count() as i64
        - hunk.iter().filter(|r| r.marker == '-').count() as i64
}

/// One hunk, with `shift` being what the hunks above it have already done to the new side's
/// numbering.
///
/// **The new side's start is arithmetic, not a number read off the rows.** omp numbers a context
/// row on the *old* side and an added row on the new one — measured against an insertion, where
/// ` 3|line three` follows `+3|`/`+4|` and is old line 3 at new line 5 ([#497](#)) — so the first
/// number in a hunk is an old one whatever marker carries it. Taking it as the new start is right
/// only until something above has changed the file's length, which is exactly when a two-hunk
/// patch stops agreeing with itself.
fn emit(hunk: &[&Row], shift: i64) -> String {
    if hunk.is_empty() {
        return String::new();
    }
    let old: Vec<u64> = hunk.iter().filter_map(|r| r.old()).collect();
    let old_start = old.first().copied().unwrap_or(0);
    let old_lines = old.len();
    let new_lines = hunk.iter().filter(|r| r.fresh()).count();
    let new_start = (old_start as i64 + shift).max(0) as u64;
    let mut out = format!("@@ -{old_start},{old_lines} +{new_start},{new_lines} @@\n");
    for entry in hunk {
        out.push(entry.marker);
        out.push_str(&entry.text);
        out.push('\n');
    }
    out
}
