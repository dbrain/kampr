use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::PathBuf;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SessionFacts {
    pub id: String,
    pub cwd: Option<String>,
    pub title: Option<String>,
    pub model: Option<String>,
    pub mode: Option<String>,
    pub version: Option<String>,
    pub account: Option<String>,
    /// Always present: where this session's usage is billed is what the safeguards guard.
    pub billing: Billing,
    pub started: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Billing {
    pub provider: Option<String>,
    pub key: Option<String>,
    pub rate_limited: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Limits {
    pub plan: Option<String>,
    pub usage: Option<Value>,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct ContextUse {
    pub used: u64,
    pub total: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Cost {
    pub usd: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Trouble {
    pub kind: String,
    pub text: String,
    pub status: String,
    pub retry: Option<Retry>,
    pub fix: Option<Fix>,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Retry {
    pub attempt: u32,
    pub of: u32,
    pub in_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fix {
    Login,
    Wait { until: String },
    Billing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RunState {
    Idle,
    Running,
    Waiting,
    Paused,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    /// Shell, subagent or monitor, as the source says it.
    pub kind: String,
    pub call: Option<String>,
    /// The task a subagent's own shell runs under.
    pub parent: Option<String>,
    pub title: Option<String>,
    pub since: Option<String>,
    pub backgrounded: bool,
    pub state: TaskState,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TaskState {
    Running,
    Ended,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct TaskEnd {
    pub ok: bool,
    pub summary: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Suggestion {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
}

/// A picture sent with a message; the source shrinks it to what the model takes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub mime: String,
    /// Base64.
    pub data: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub path: PathBuf,
}
