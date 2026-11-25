use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const WIRE_VERSION: u32 = 303;

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
    /// `Orchestrator`, `TaskChimp`, `Review`, `Interactive`, `Debugging`, `Planner` or a key.
    #[serde(default)]
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
    pub subagents: Vec<Subagent>,
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
