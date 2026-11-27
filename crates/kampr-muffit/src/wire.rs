use serde::{Deserialize, Deserializer, Serialize};
use serde_json::Value;

/// What this node speaks and the floor it assumes of a daemon whose greeting names no version.
pub const WIRE_VERSION: u32 = 309;
pub const BASELINE: u32 = 303;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionState {
    Created,
    Running,
    WaitingTool,
    WaitingUser,
    WaitingProvider,
    Completed,
    Failed,
    Paused,
    Stopped,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PermissionPrompt {
    pub id: u64,
    pub tool: String,
    pub action: String,
}

/// The model's `ask` question (wire 3.4): an option's index or, with `free_text`, words answer it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Question {
    pub id: u64,
    pub text: String,
    #[serde(default)]
    pub options: Vec<String>,
    #[serde(default)]
    pub free_text: bool,
}

/// A background `bash` job (wire 3.6). Its output is the model's to read and is not on the wire.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ShellTask {
    pub id: String,
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub state: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub struct RateLimit {
    #[serde(default)]
    pub limited: bool,
    #[serde(default)]
    pub retry_after_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct SessionInfo {
    pub id: String,
    pub state: SessionState,
    #[serde(default)]
    pub turns: u64,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub branch: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub permission: Option<PermissionPrompt>,
    #[serde(default)]
    pub question: Option<Question>,
    #[serde(default)]
    pub clients: Vec<AttachedClient>,
}

/// Who is attached to a session, as each client named itself in its hello.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AttachedClient {
    pub kind: String,
    #[serde(default)]
    pub pid: Option<u32>,
    #[serde(default)]
    pub host: Option<String>,
    #[serde(default)]
    pub observing: bool,
}

/// A picture sent with a message: muffit shrinks it to what the model takes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Image {
    pub mime: String,
    /// Base64.
    pub data: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ToolCallStatus {
    Started,
    Running,
    Finished,
    Failed,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct DiffHunk {
    pub old_start: u64,
    pub old_lines: u64,
    pub new_start: u64,
    pub new_lines: u64,
    #[serde(default)]
    pub old_text: String,
    #[serde(default)]
    pub new_text: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    Thinking {},
    Image {
        mime: String,
        #[serde(default)]
        bytes: u64,
    },
    ToolCall {
        id: String,
        tool: String,
        #[serde(default)]
        args: String,
        status: ToolCallStatus,
    },
    /// A slice of a call's arguments as the model writes them (wire 3.7).
    ToolCallDelta {
        call_id: String,
        #[serde(default)]
        tool: String,
        #[serde(default)]
        delta: String,
    },
    ToolOutputChunk {
        call_id: String,
        #[serde(default)]
        text: String,
        /// The chunk's own sequence, so a followed task can tell what it has already read.
        #[serde(default)]
        seq: u64,
    },
    ToolResult {
        call_id: String,
        #[serde(default)]
        summary: String,
        #[serde(default)]
        exit_status: Option<i32>,
        #[serde(default)]
        total_lines: Option<u64>,
    },
    Diff {
        #[serde(default)]
        call_id: Option<String>,
        path: String,
        #[serde(default)]
        hunks: Vec<DiffHunk>,
    },
    Error {
        #[serde(default)]
        message: String,
    },
    UserMessage {
        text: String,
    },
    Recorded {
        seq: u64,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct TranscriptEntry {
    pub seq: u64,
    pub role: String,
    #[serde(default)]
    pub blocks: Vec<ContentBlock>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
pub struct TranscriptPage {
    #[serde(default)]
    pub transcript: Vec<TranscriptEntry>,
    #[serde(default)]
    pub watermark: Option<u64>,
    #[serde(default)]
    pub has_older: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Liveness {
    Alive,
    Stalled,
    Dead,
    #[serde(other)]
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Subagent {
    pub id: String,
    #[serde(default)]
    pub call_id: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    pub liveness: Liveness,
    /// `Orchestrator`, `TaskChimp`, `Review`, `Interactive`, `Debugging`, `Planner`, or a custom
    /// role's key, which muffit's `AgentRole::Other` sends as `{"Other": "<key>"}`.
    #[serde(default, deserialize_with = "lenient_role")]
    pub role: Option<String>,
    /// The worker's own state: `running`, `waiting_user`, `paused`, `completed` or `failed`.
    #[serde(default)]
    pub state: Option<String>,
    #[serde(default)]
    pub task_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct ContextStatus {
    /// The window's prompt size, once a turn has measured one.
    #[serde(default)]
    pub measured_prompt_tokens: Option<u32>,
    #[serde(default)]
    pub pressure: Option<Pressure>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Pressure {
    /// The strategy's budget: the size the window is cut back from.
    #[serde(default)]
    pub budget: Option<u32>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct StatusSnapshot {
    #[serde(default)]
    pub session_id: String,
    #[serde(default)]
    pub title: Option<String>,
    pub state: SessionState,
    #[serde(default)]
    pub permission: Option<PermissionPrompt>,
    #[serde(default)]
    pub question: Option<Question>,
    #[serde(default)]
    pub subagents: Vec<Subagent>,
    #[serde(default)]
    pub shell_tasks: Vec<ShellTask>,
    /// `<provider catalog name>: <model>` (wire 3.9).
    #[serde(default)]
    pub billed_to: Option<String>,
    #[serde(default)]
    pub rate_limit: Option<RateLimit>,
    #[serde(default)]
    pub model: Option<String>,
    #[serde(default)]
    pub thinking: Option<String>,
    #[serde(default)]
    pub plan: bool,
    /// When tools ask first: `yolo`, `auto`, `ask-outside-project` or `ask-all`.
    #[serde(default)]
    pub permissions: Option<String>,
    #[serde(default)]
    pub cwd: Option<String>,
    #[serde(default)]
    pub context: Option<ContextStatus>,
    #[serde(default)]
    pub prompt_tokens: u32,
    #[serde(default)]
    pub completion_tokens: u32,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerFrame {
    Hello {
        #[serde(default)]
        version: Option<u32>,
        #[serde(default)]
        cursor: i64,
        session: SessionInfo,
    },
    Sessions {
        sessions: Vec<SessionInfo>,
    },
    Session {
        session: SessionInfo,
    },
    Content {
        blocks: Vec<ContentBlock>,
    },
    Turn {},
    Status {
        snapshot: StatusSnapshot,
    },
    Lagged {},
    /// A durable event, with the `events` capability. `event` is read leniently: the daemon's
    /// enum has no catch-all (kampr.md §3).
    Event {
        #[serde(default)]
        event: Value,
    },
    Error {
        message: String,
    },
    /// What a `control` verb did, answered on the socket that asked.
    CommandResult {
        verb: String,
        ok: bool,
        #[serde(default)]
        payload: Value,
    },
    #[serde(other)]
    Other,
}

pub fn hello(capabilities: &[&str]) -> serde_json::Value {
    serde_json::json!({
        "type": "hello", "version": WIRE_VERSION, "capabilities": capabilities,
        "client": { "kind": "kampr" },
    })
}

/// A role this node cannot name is no role, never a status frame it drops whole.
fn lenient_role<'de, D: Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    Ok(match Value::deserialize(d)? {
        Value::String(role) => Some(role),
        Value::Object(one) if one.len() == 1 => one
            .into_iter()
            .next()
            .and_then(|(_, key)| key.as_str().map(str::to_string)),
        _ => None,
    })
}
