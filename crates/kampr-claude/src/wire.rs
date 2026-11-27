use serde_json::{Value, json};

/// One line of Claude's stream-json, read off stdout. Parsing is lenient by rule: an unknown
/// `type` is `Other`, a missing field is `None`, and nothing here is allowed to fail a turn on a
/// line it does not recognise.
#[derive(Debug)]
pub enum Line {
    Init(Init),
    Status(Status),
    ThinkingTokens { tokens: u64 },
    TaskStarted(TaskInfo),
    TaskUpdated(TaskInfo),
    TaskProgress(TaskProgress),
    TaskNotification(TaskNotification),
    BackgroundTasks(Vec<TaskInfo>),
    ApiRetry(ApiRetry),
    StreamEvent(StreamEvent),
    SubagentLive(SubagentLive),
    Assistant(Assistant),
    User(UserLine),
    Result(ResultLine),
    RateLimit(RateLimit),
    ControlRequest(ControlRequest),
    ControlResponse(ControlResponse),
    Other,
}

impl Line {
    /// The first field named `key` on the line's JSON, whatever `type` it carries.
    pub fn str_of(v: &Value, key: &str) -> Option<String> {
        v.get(key).and_then(Value::as_str).map(str::to_string)
    }

    pub fn parse(line: &str) -> Option<Line> {
        let v: Value = serde_json::from_str(line).ok()?;
        let ty = v.get("type")?.as_str()?;
        Some(match ty {
            "system" => system(&v),
            "stream_event" => Line::StreamEvent(stream_event(&v)),
            "subagent_live" => Line::SubagentLive(subagent_live(&v)),
            "assistant" => Line::Assistant(assistant(&v)),
            "user" => Line::User(user(&v)),
            "result" => Line::Result(result(&v)),
            "rate_limit_event" => Line::RateLimit(rate_limit(&v)),
            "control_request" => Line::ControlRequest(parse_control_request(&v)),
            "control_response" => Line::ControlResponse(parse_control_response(&v)),
            _ => Line::Other,
        })
    }
}

fn system(v: &Value) -> Line {
    match v.get("subtype").and_then(Value::as_str) {
        Some("init") => Line::Init(Init::from(v)),
        Some("status") => Line::Status(Status::from(v)),
        Some("thinking_tokens") => Line::ThinkingTokens {
            tokens: v.get("tokens").and_then(Value::as_u64).unwrap_or(0),
        },
        Some("task_started") => Line::TaskStarted(task_info(v)),
        Some("task_updated") => Line::TaskUpdated(task_info(v)),
        Some("task_progress") => Line::TaskProgress(TaskProgress::from(v)),
        Some("task_notification") => Line::TaskNotification(TaskNotification::from(v)),
        Some("background_tasks_changed") => Line::BackgroundTasks(background_tasks(v)),
        Some("api_retry") => Line::ApiRetry(ApiRetry::from(v)),
        _ => Line::Other,
    }
}

/// `system/init` — what the session is and, before a word is sent, how it would be paid for.
#[derive(Debug, Clone, Default)]
pub struct Init {
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub model: Option<String>,
    pub permission_mode: Option<String>,
    /// Guard 4: `"none"` on a login that is paid by the plan; a key name is a key in the env.
    pub api_key_source: Option<String>,
    pub capabilities: Vec<String>,
    pub slash_commands: Vec<SlashCommand>,
    pub agents: Vec<String>,
    pub version: Option<String>,
}

impl Init {
    fn from(v: &Value) -> Self {
        Self {
            session_id: Line::str_of(v, "session_id"),
            cwd: Line::str_of(v, "cwd"),
            model: Line::str_of(v, "model"),
            permission_mode: Line::str_of(v, "permissionMode").or_else(|| Line::str_of(v, "permission_mode")),
            api_key_source: Line::str_of(v, "apiKeySource").or_else(|| Line::str_of(v, "api_key_source")),
            capabilities: v
                .get("capabilities")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|c| c.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            slash_commands: v
                .get("slash_commands")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| {
                            Some(SlashCommand {
                                name: c.get("name")?.as_str()?.to_string(),
                                description: c.get("description").and_then(Value::as_str).map(str::to_string),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default(),
            agents: v
                .get("agents")
                .and_then(Value::as_array)
                .map(|a| {
                    a.iter()
                        .filter_map(|c| c.get("name").and_then(Value::as_str).map(str::to_string))
                        .collect()
                })
                .unwrap_or_default(),
            version: Line::str_of(v, "version"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Default)]
pub struct Status {
    pub status: Option<String>,
    pub permission_mode: Option<String>,
}

impl Status {
    fn from(v: &Value) -> Self {
        Self {
            status: Line::str_of(v, "status"),
            permission_mode: Line::str_of(v, "permissionMode").or_else(|| Line::str_of(v, "permission_mode")),
        }
    }
}

/// A task the session is running: a shell, a subagent, a workflow. `task_started`, `task_updated`
/// and each row of `background_tasks_changed` carry some of these; a field a line does not name
/// stays `None`.
#[derive(Debug, Clone, Default)]
pub struct TaskInfo {
    pub task_id: Option<String>,
    pub tool_use_id: Option<String>,
    pub task_type: Option<String>,
    pub is_backgrounded: bool,
    pub parent_task_id: Option<String>,
    pub subagent_type: Option<String>,
    pub prompt: Option<String>,
    pub status: Option<String>,
}

fn task_info(v: &Value) -> TaskInfo {
    TaskInfo {
        task_id: Line::str_of(v, "task_id"),
        tool_use_id: Line::str_of(v, "tool_use_id"),
        task_type: Line::str_of(v, "task_type"),
        is_backgrounded: v.get("is_backgrounded").and_then(Value::as_bool).unwrap_or(false),
        parent_task_id: Line::str_of(v, "parent_task_id"),
        subagent_type: Line::str_of(v, "subagent_type"),
        prompt: Line::str_of(v, "prompt"),
        status: Line::str_of(v, "status"),
    }
}

fn background_tasks(v: &Value) -> Vec<TaskInfo> {
    v.get("tasks")
        .and_then(Value::as_array)
        .map(|a| a.iter().map(task_info).filter(|t| t.task_id.is_some()).collect())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Default)]
pub struct TaskProgress {
    pub task_id: Option<String>,
    pub description: Option<String>,
    pub last_tool_name: Option<String>,
}

impl TaskProgress {
    fn from(v: &Value) -> Self {
        Self {
            task_id: Line::str_of(v, "task_id"),
            description: Line::str_of(v, "description"),
            last_tool_name: Line::str_of(v, "last_tool_name"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct TaskNotification {
    pub task_id: Option<String>,
    pub output_file: Option<String>,
}

impl TaskNotification {
    fn from(v: &Value) -> Self {
        Self {
            task_id: Line::str_of(v, "task_id"),
            output_file: Line::str_of(v, "output_file"),
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct ApiRetry {
    pub attempt: Option<u64>,
    pub max_retries: Option<u64>,
    pub retry_delay_ms: Option<u64>,
    pub error_status: Option<u64>,
    pub error: Option<String>,
}

impl ApiRetry {
    fn from(v: &Value) -> Self {
        Self {
            attempt: v.get("attempt").and_then(Value::as_u64),
            max_retries: v.get("max_retries").and_then(Value::as_u64),
            retry_delay_ms: v.get("retry_delay_ms").and_then(Value::as_u64),
            error_status: v.get("error_status").and_then(Value::as_u64),
            error: Line::str_of(v, "error"),
        }
    }
}

/// A partial message from the Messages API, the live edge of a turn.
#[derive(Debug, Clone)]
pub enum StreamEvent {
    BlockStart { index: u64, block: BlockStart },
    TextDelta { text: String },
    ThinkingDelta { thinking: String },
    InputJsonDelta { partial_json: String },
    MessageDelta { stop_reason: Option<String> },
    Other,
}

#[derive(Debug, Clone)]
pub enum BlockStart {
    Text,
    ToolUse { id: String, name: String },
    Thinking,
    Other,
}

fn stream_event(v: &Value) -> StreamEvent {
    let e = match v.get("event") {
        Some(e) => e,
        None => return StreamEvent::Other,
    };
    match e.get("type").and_then(Value::as_str) {
        Some("content_block_start") => StreamEvent::BlockStart {
            index: e.get("index").and_then(Value::as_u64).unwrap_or(0),
            block: block_start(e.get("content_block").unwrap_or(e)),
        },
        Some("content_block_delta") => {
            let delta = e.get("delta").unwrap_or(e);
            match delta.get("type").and_then(Value::as_str) {
                Some("text_delta") => StreamEvent::TextDelta {
                    text: delta
                        .get("text")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                },
                Some("thinking_delta") => StreamEvent::ThinkingDelta {
                    thinking: delta
                        .get("thinking")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                },
                Some("input_json_delta") => StreamEvent::InputJsonDelta {
                    partial_json: delta
                        .get("partial_json")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_string(),
                },
                _ => StreamEvent::Other,
            }
        }
        Some("message_delta") => StreamEvent::MessageDelta {
            stop_reason: e
                .get("delta")
                .and_then(|d| d.get("stop_reason"))
                .and_then(Value::as_str)
                .map(str::to_string),
        },
        _ => StreamEvent::Other,
    }
}

fn block_start(v: &Value) -> BlockStart {
    match v.get("type").and_then(Value::as_str) {
        Some("text") => BlockStart::Text,
        Some("tool_use") => BlockStart::ToolUse {
            id: v.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
            name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
        },
        Some("thinking") => BlockStart::Thinking,
        _ => BlockStart::Other,
    }
}

/// A subagent's live edge: a stream event filed under the subagent that is speaking, not the
/// parent turn. `parent_tool_use_id` is the call that launched it — the name its own
/// `assistant` lines carry — and `task_id` is the binary's task ledger, when the line names one.
#[derive(Debug, Clone)]
pub struct SubagentLive {
    pub parent_tool_use_id: Option<String>,
    pub task_id: Option<String>,
    pub event: StreamEvent,
}

fn subagent_live(v: &Value) -> SubagentLive {
    SubagentLive {
        parent_tool_use_id: Line::str_of(v, "parent_tool_use_id"),
        task_id: Line::str_of(v, "task_id"),
        event: stream_event(v),
    }
}

/// A finished assistant message: its blocks, and whether it is the API's error, not an answer.
#[derive(Debug, Clone, Default)]
pub struct Assistant {
    pub model: Option<String>,
    pub content: Vec<ContentBlock>,
    /// A failing turn's `result` says `subtype: success`; this is the turn's own confession.
    pub error: Option<String>,
    pub is_api_error_message: bool,
    pub parent_tool_use_id: Option<String>,
    pub session_id: Option<String>,
}

fn assistant(v: &Value) -> Assistant {
    let message = v.get("message").unwrap_or(v);
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(content_block).collect())
        .unwrap_or_default();
    Assistant {
        model: Line::str_of(message, "model"),
        content,
        error: v.get("error").and_then(Value::as_str).map(str::to_string),
        is_api_error_message: v
            .get("is_api_error_message")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        parent_tool_use_id: Line::str_of(v, "parent_tool_use_id"),
        session_id: Line::str_of(v, "session_id"),
    }
}

#[derive(Debug, Clone)]
pub enum ContentBlock {
    Text { text: String },
    ToolUse { id: String, name: String, input: Value },
    Thinking { thinking: String },
    Other,
}

fn content_block(v: &Value) -> Option<ContentBlock> {
    match v.get("type").and_then(Value::as_str)? {
        "text" => Some(ContentBlock::Text {
            text: v.get("text").and_then(Value::as_str).unwrap_or("").to_string(),
        }),
        "tool_use" => Some(ContentBlock::ToolUse {
            id: v.get("id").and_then(Value::as_str).unwrap_or("").to_string(),
            name: v.get("name").and_then(Value::as_str).unwrap_or("").to_string(),
            input: v.get("input").cloned().unwrap_or(Value::Null),
        }),
        "thinking" => Some(ContentBlock::Thinking {
            thinking: v
                .get("thinking")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string(),
        }),
        _ => None,
    }
}

/// A `user` line: the echo of a tool result, or a subagent's record.
#[derive(Debug, Clone, Default)]
pub struct UserLine {
    pub content: Vec<ContentBlock>,
    pub parent_tool_use_id: Option<String>,
}

fn user(v: &Value) -> UserLine {
    let message = v.get("message").unwrap_or(v);
    let content = message
        .get("content")
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(content_block).collect())
        .unwrap_or_default();
    UserLine {
        content,
        parent_tool_use_id: Line::str_of(v, "parent_tool_use_id"),
    }
}

/// A turn's end. `result` is the answer's text; `origin` names a turn the model started for
/// itself (guard 7); `total_cost_usd` is what the turn cost.
#[derive(Debug, Clone, Default)]
pub struct ResultLine {
    pub subtype: Option<String>,
    pub is_error: bool,
    pub terminal_reason: Option<String>,
    pub stop_reason: Option<String>,
    pub result: Option<String>,
    pub total_cost_usd: f64,
    pub usage: Usage,
    pub duration_ms: u64,
    pub num_turns: u64,
    pub origin: Option<String>,
    pub error: Option<String>,
    pub session_id: Option<String>,
}

fn result(v: &Value) -> ResultLine {
    ResultLine {
        subtype: Line::str_of(v, "subtype"),
        is_error: v.get("is_error").and_then(Value::as_bool).unwrap_or(false),
        terminal_reason: Line::str_of(v, "terminal_reason"),
        stop_reason: Line::str_of(v, "stop_reason"),
        result: Line::str_of(v, "result"),
        total_cost_usd: v.get("total_cost_usd").and_then(Value::as_f64).unwrap_or(0.0),
        usage: usage(v.get("usage").unwrap_or(v)),
        duration_ms: v.get("duration_ms").and_then(Value::as_u64).unwrap_or(0),
        num_turns: v.get("num_turns").and_then(Value::as_u64).unwrap_or(0),
        origin: Line::str_of(v, "origin"),
        error: Line::str_of(v, "error"),
        session_id: Line::str_of(v, "session_id"),
    }
}

#[derive(Debug, Clone, Default)]
pub struct Usage {
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_creation_tokens: u64,
    pub cache_read_tokens: u64,
}

fn usage(v: &Value) -> Usage {
    Usage {
        input_tokens: v.get("input_tokens").and_then(Value::as_u64).unwrap_or(0),
        output_tokens: v.get("output_tokens").and_then(Value::as_u64).unwrap_or(0),
        cache_creation_tokens: v
            .get("cache_creation_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
        cache_read_tokens: v
            .get("cache_read_input_tokens")
            .and_then(Value::as_u64)
            .unwrap_or(0),
    }
}

/// Guard 6 and the limits on screen: a plan running on its usage credits.
#[derive(Debug, Clone, Default)]
pub struct RateLimit {
    pub is_using_overage: bool,
    pub overage_status: Option<String>,
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
}

#[derive(Debug, Clone)]
pub struct Window {
    pub utilization: f64,
    pub resets_at: Option<String>,
}

fn rate_limit(v: &Value) -> RateLimit {
    let v = v.get("rate_limit_event").unwrap_or(v);
    let window = |v: &Value, key: &str| {
        v.get(key).map(|w| Window {
            utilization: w.get("utilization").and_then(Value::as_f64).unwrap_or(0.0),
            resets_at: w.get("resets_at").and_then(Value::as_str).map(str::to_string),
        })
    };
    RateLimit {
        is_using_overage: v
            .get("is_using_overage")
            .and_then(Value::as_bool)
            .unwrap_or(false),
        overage_status: Line::str_of(v, "overage_status"),
        five_hour: window(v, "five_hour"),
        seven_day: window(v, "seven_day"),
    }
}

/// A request the binary asks of us: a `can_use_tool` standing in the middle of a turn.
#[derive(Debug, Clone)]
pub struct ControlRequest {
    pub request_id: String,
    pub tool_name: Option<String>,
    pub input: Value,
    pub permission_suggestions: Vec<Value>,
    pub raw: Value,
}

fn parse_control_request(v: &Value) -> ControlRequest {
    let request = v.get("request").unwrap_or(v);
    ControlRequest {
        request_id: Line::str_of(v, "request_id").unwrap_or_default(),
        tool_name: request
            .get("tool_name")
            .and_then(Value::as_str)
            .map(str::to_string),
        input: request.get("input").cloned().unwrap_or(Value::Null),
        permission_suggestions: request
            .get("permission_suggestions")
            .and_then(Value::as_array)
            .cloned()
            .unwrap_or_default(),
        raw: v.clone(),
    }
}

/// The binary's answer to one of our requests, matched by the `request_id` it carries back.
#[derive(Debug, Clone)]
pub struct ControlResponse {
    pub request_id: String,
    pub ok: bool,
    pub body: Value,
    pub error: Option<String>,
}

fn parse_control_response(v: &Value) -> ControlResponse {
    let r = v.get("response").unwrap_or(v);
    let request_id = r
        .get("request_id")
        .and_then(Value::as_str)
        .or_else(|| v.get("request_id").and_then(Value::as_str))
        .unwrap_or("")
        .to_string();
    ControlResponse {
        ok: r.get("subtype").and_then(Value::as_str) == Some("success"),
        body: r.get("response").cloned().unwrap_or(Value::Null),
        error: r.get("error").and_then(Value::as_str).map(str::to_string),
        request_id,
    }
}

// ── the writer: our lines to the binary's stdin ──────────────────────────────

/// A `user` message: the prompt, with optional images. This is a word sent, and it is what the
/// billing guards stand in front of. `uuid` is the word's id — the handle a
/// `cancel_async_message` names — and `priority` is the binary's queue priority: `now` preempts
/// the turn in flight, `next` waits behind it.
pub fn user_message(text: &str, images: &[Image], uuid: Option<&str>, priority: Option<&str>) -> String {
    let content = if images.is_empty() {
        Value::String(text.to_string())
    } else {
        let mut blocks = vec![Value::Object(
            [("type".to_string(), json_text(text))].into_iter().collect(),
        )];
        for image in images {
            blocks.push(json!({
                "type": "image",
                "source": { "type": "base64", "media_type": image.mime, "data": image.data },
            }));
        }
        Value::Array(blocks)
    };
    let mut line = json!({
        "type": "user",
        "message": { "role": "user", "content": content },
        "parent_tool_use_id": null,
        "session_id": "",
    });
    if let Some(uuid) = uuid {
        line["uuid"] = Value::String(uuid.to_string());
    }
    if let Some(priority) = priority {
        line["priority"] = Value::String(priority.to_string());
    }
    line.to_string()
}

fn json_text(text: &str) -> Value {
    json!({ "type": "text", "text": text })
}

pub struct Image {
    pub mime: String,
    pub data: String,
}

/// A control request, our side: `initialize`, `list_models`, `get_usage`, `set_model`, ...
/// The reply is matched to the caller's `request_id`.
pub fn control_request(request_id: &str, subtype: &str, body: Value) -> String {
    let mut request = serde_json::to_value(body).unwrap_or(Value::Null);
    if let Some(obj) = request.as_object_mut() {
        obj.insert("subtype".to_string(), Value::String(subtype.to_string()));
    }
    json!({
        "type": "control_request",
        "request_id": request_id,
        "request": request,
    })
    .to_string()
}

/// The answer to a `can_use_tool`: `behavior` is `allow` or `deny`.
pub fn control_response(request_id: &str, behavior: &str, updated_input: Value) -> String {
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": request_id,
            "response": {
                "behavior": behavior,
                "updatedInput": updated_input,
            },
        },
    })
    .to_string()
}

/// The answer to our own `initialize`/`set_model`/... request: a success the binary accepts.
pub fn control_response_success(request_id: &str) -> String {
    json!({
        "type": "control_response",
        "response": { "subtype": "success", "request_id": request_id, "response": {} },
    })
    .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_init_line_reads_its_billing_fields() {
        let line = Line::parse(
            r#"{"type":"system","subtype":"init","session_id":"s1","cwd":"/w","model":"sonnet","permissionMode":"plan","apiKeySource":"none","capabilities":["fs"],"slash_commands":[{"name":"/model","description":"pick"}],"version":"2.1.326"}"#,
        )
        .unwrap();
        let Line::Init(init) = line else {
            panic!("wanted init, got {line:?}")
        };
        assert_eq!(init.session_id.as_deref(), Some("s1"));
        assert_eq!(init.api_key_source.as_deref(), Some("none"));
        assert_eq!(init.capabilities, vec!["fs"]);
        assert_eq!(init.slash_commands[0].name, "/model");
        assert_eq!(init.version.as_deref(), Some("2.1.326"));
    }

    #[test]
    fn a_result_line_reads_its_origin_and_cost() {
        let line = Line::parse(
            r#"{"type":"result","subtype":"success","is_error":false,"result":"hi","total_cost_usd":0.012,"origin":"task_notification","usage":{"input_tokens":10,"output_tokens":5},"duration_ms":1200,"num_turns":1}"#,
        )
        .unwrap();
        let Line::Result(result) = line else {
            panic!("wanted result, got {line:?}")
        };
        assert_eq!(result.origin.as_deref(), Some("task_notification"));
        assert!((result.total_cost_usd - 0.012).abs() < 1e-9);
        assert_eq!(result.usage.input_tokens, 10);
        assert_eq!(result.result.as_deref(), Some("hi"));
    }

    #[test]
    fn a_rate_limit_event_reads_its_overage_flag() {
        let line = Line::parse(
            r#"{"type":"rate_limit_event","rate_limit_event":{"is_using_overage":true,"overage_status":"active","five_hour":{"utilization":1.0,"resets_at":"2026-01-01T00:00:00Z"}}}"#,
        )
        .unwrap();
        let Line::RateLimit(limit) = line else {
            panic!("wanted rate limit, got {line:?}")
        };
        assert!(limit.is_using_overage);
        assert_eq!(limit.five_hour.unwrap().utilization, 1.0);
    }

    #[test]
    fn an_assistant_line_reads_its_blocks_and_error() {
        let line = Line::parse(
            r#"{"type":"assistant","message":{"model":"sonnet","content":[{"type":"text","text":"hi"},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"ls"}}]},"session_id":"s1"}"#,
        )
        .unwrap();
        let Line::Assistant(a) = line else {
            panic!("wanted assistant, got {line:?}")
        };
        assert_eq!(a.content.len(), 2);
        assert!(!a.is_api_error_message);
    }

    #[test]
    fn a_stream_event_reads_its_deltas() {
        let text = Line::parse(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"he"}}}"#,
        )
        .unwrap();
        assert!(matches!(text, Line::StreamEvent(StreamEvent::TextDelta { .. })));
        let thinking = Line::parse(
            r#"{"type":"stream_event","event":{"type":"content_block_delta","index":0,"delta":{"type":"thinking_delta","thinking":"hm"}}}"#,
        )
        .unwrap();
        assert!(matches!(
            thinking,
            Line::StreamEvent(StreamEvent::ThinkingDelta { .. })
        ));
        let block = Line::parse(
            r#"{"type":"stream_event","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t1","name":"Bash"}}}"#,
        )
        .unwrap();
        assert!(matches!(block, Line::StreamEvent(StreamEvent::BlockStart { .. })));
    }

    #[test]
    fn a_subagent_live_line_reads_its_name_and_its_event() {
        let line = Line::parse(
            r#"{"type":"subagent_live","parent_tool_use_id":"tu1","task_id":"t1","event":{"type":"content_block_delta","index":0,"delta":{"type":"text_delta","text":"digging"}}}"#,
        )
        .unwrap();
        let Line::SubagentLive(live) = line else {
            panic!("wanted subagent live, got {line:?}")
        };
        assert_eq!(live.parent_tool_use_id.as_deref(), Some("tu1"));
        assert_eq!(live.task_id.as_deref(), Some("t1"));
        assert!(matches!(
            live.event,
            StreamEvent::TextDelta { ref text } if text == "digging"
        ));

        // A line that names neither carries the event alone; the seam decides what to do with it.
        let unnamed = Line::parse(
            r#"{"type":"subagent_live","event":{"type":"content_block_start","index":0,"content_block":{"type":"tool_use","id":"t9","name":"Read"}}}"#,
        )
        .unwrap();
        let Line::SubagentLive(live) = unnamed else {
            panic!("wanted subagent live, got {live:?}")
        };
        assert!(live.parent_tool_use_id.is_none());
        assert!(live.task_id.is_none());
        assert!(matches!(live.event, StreamEvent::BlockStart { .. }));
    }

    #[test]
    fn a_control_request_and_response_round_trip_their_ids() {
        let req = Line::parse(
            r#"{"type":"control_request","request_id":"r1","request":{"subtype":"can_use_tool","tool_name":"Bash","input":{"command":"ls"},"permission_suggestions":[]}}"#,
        )
        .unwrap();
        let Line::ControlRequest(req) = req else {
            panic!("wanted control request, got {req:?}")
        };
        assert_eq!(req.request_id, "r1");
        assert_eq!(req.tool_name.as_deref(), Some("Bash"));

        let reply = control_response("r1", "allow", json!({"command": "ls"}));
        let parsed: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(parsed["response"]["request_id"], "r1");
        assert_eq!(parsed["response"]["response"]["behavior"], "allow");

        let resp = Line::parse(
            r#"{"type":"control_response","response":{"subtype":"success","request_id":"r1","response":{"subscriptionType":"Claude Max"}}}"#,
        )
        .unwrap();
        let Line::ControlResponse(resp) = resp else {
            panic!("wanted control response, got {resp:?}")
        };
        assert_eq!(resp.request_id, "r1");
        assert!(resp.ok);
        assert_eq!(resp.body["subscriptionType"], "Claude Max");
    }

    #[test]
    fn an_unknown_line_is_other_not_an_error() {
        assert!(matches!(
            Line::parse(r#"{"type":"something_new"}"#),
            Some(Line::Other)
        ));
        assert!(Line::parse("not json at all").is_none());
        assert!(matches!(
            Line::parse(r#"{"type":"system","subtype":"a_new_subtype"}"#),
            Some(Line::Other)
        ));
    }

    #[test]
    fn a_task_line_reads_its_identity() {
        let line = Line::parse(
            r#"{"type":"system","subtype":"task_started","task_id":"t1","tool_use_id":"tu1","task_type":"shell","is_backgrounded":false,"parent_task_id":null,"prompt":"ls"}"#,
        )
        .unwrap();
        let Line::TaskStarted(task) = line else {
            panic!("wanted task started, got {line:?}")
        };
        assert_eq!(task.task_id.as_deref(), Some("t1"));
        assert_eq!(task.task_type.as_deref(), Some("shell"));
        assert!(!task.is_backgrounded);
    }
}
