//! The shapes the binary sends. A shape that has only been read off the binary's schema, and not
//! captured on the wire, says so.

use serde_json::{Value, json};

pub(super) const PLAN_ACCOUNT: &str =
    r#"{"email":"o@x.dev","subscriptionType":"Claude Max","apiProvider":"firstParty"}"#;
/// #661: a key beside the login.
pub(super) const KEY_ACCOUNT: &str =
    r#"{"tokenSource":"claude.ai","apiKeySource":"ANTHROPIC_API_KEY","apiProvider":"firstParty"}"#;
/// #661: an empty config directory, or `--bare`.
pub(super) const LOGGED_OUT: &str = r#"{"tokenSource":"none","apiProvider":"firstParty"}"#;

/// `initialize`'s answer: #672 names its `commands`, `models`, `account` and `fast_mode_state`;
/// a model's fields are the binary's schema (`value`, `displayName`, `resolvedModel`).
pub(super) fn initialize_response(id: &str, account: &str) -> String {
    let account: Value = serde_json::from_str(account).unwrap();
    json!({
        "type": "control_response",
        "response": {
            "subtype": "success",
            "request_id": id,
            "response": {
                "account": account,
                "commands": [
                    {"name": "compact", "description": "shrink the context"},
                    {"name": "model", "description": "pick a model"},
                ],
                "models": [
                    {"value": "default", "displayName": "Default", "description": "the usual", "resolvedModel": "claude-opus-5-5"},
                    {"value": "haiku", "displayName": "Haiku", "description": "fast"},
                ],
                "fast_mode_state": "off",
                "current_permission_mode": "default",
            },
        },
    })
    .to_string()
}

/// The same, as an SDK session on 2.1.326 really answers: fast mode is off and cannot be turned on
/// (#696).
pub(super) fn initialize_response_sdk(id: &str) -> String {
    let mut line: Value = serde_json::from_str(&initialize_response(id, PLAN_ACCOUNT)).unwrap();
    line["response"]["response"]["fast_mode_disabled_reason"] = json!("sdk_opt_in_required");
    line.to_string()
}

pub(super) fn response(id: &str, body: Value) -> String {
    json!({"type":"control_response","response":{"subtype":"success","request_id":id,"response":body}})
        .to_string()
}

pub(super) fn error_response(id: &str, error: &str) -> String {
    json!({"type":"control_response","response":{"subtype":"error","request_id":id,"error":error}})
        .to_string()
}

/// #644's `system/init` of a turn on a plan login.
pub(super) const TURN_INIT: &str = r#"{"type":"system","subtype":"init","session_id":"s1","apiKeySource":"none","model":"claude-opus-5-5","permissionMode":"default","cwd":"/w"}"#;
pub(super) const TURN_INIT_KEY: &str = r#"{"type":"system","subtype":"init","session_id":"s1","apiKeySource":"ANTHROPIC_API_KEY","model":"claude-opus-5-5"}"#;

/// The binary's schema for the event (2.1.326): `rate_limit_info` with `isUsingOverage` and a
/// window per `unifiedWindows` key. Read off the binary, not captured on the wire.
pub(super) const RATE_OVERAGE: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","resetsAt":1795800000,"rateLimitType":"five_hour","utilization":1.02,"unifiedWindows":{"five_hour":{"utilization":1.02,"resetsAt":1795800000},"seven_day":{"utilization":0.5,"resetsAt":1796000000}},"overageStatus":"allowed","isUsingOverage":true},"uuid":"u1","session_id":"s1"}"#;
pub(super) const RATE_PLAIN: &str = r#"{"type":"rate_limit_event","rate_limit_info":{"status":"allowed","utilization":0.4,"unifiedWindows":{"five_hour":{"utilization":0.4,"resetsAt":1795800000}},"isUsingOverage":false},"uuid":"u2","session_id":"s1"}"#;

/// #682: a self-started turn's result carries an object `origin`.
pub(super) const RESULT_TASK: &str = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.01,"origin":{"kind":"task-notification","producer":"session-task","runId":"0mwyfgylr"},"num_turns":1}"#;
/// #682: a person's turn has no `origin` at all.
pub(super) const RESULT_USER: &str = r#"{"type":"result","subtype":"success","is_error":false,"total_cost_usd":0.01,"result":"hi","usage":{"input_tokens":10,"output_tokens":5}}"#;

/// #645's `can_use_tool` for a Bash command, with a suggestion that makes it "always".
pub(super) fn can_use_bash(id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": id,
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "Bash",
            "display_name": "Bash",
            "input": {"command": "touch probe-file"},
            "permission_suggestions": [{"type": "setMode", "mode": "acceptEdits", "destination": "session"}],
            "tool_use_id": "tu1",
        },
    })
    .to_string()
}

/// #645: `AskUserQuestion` is a `can_use_tool` with `questions[]`.
pub(super) fn can_use_question(id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": id,
        "request": {
            "subtype": "can_use_tool",
            "tool_name": "AskUserQuestion",
            "requires_user_interaction": true,
            "input": {"questions": [
                {"question": "Which colour?", "header": "Colour", "multiSelect": false,
                 "options": [{"label": "Red", "description": "warm"}, {"label": "Blue", "description": "cool"}]},
                {"question": "Which fruits?", "header": "Fruit", "multiSelect": true,
                 "options": [{"label": "Apple"}, {"label": "Cherry"}]},
            ]},
        },
    })
    .to_string()
}

/// #674: plan approval is a `can_use_tool` for `ExitPlanMode` with the plan in `input.plan`.
pub(super) fn can_use_plan(id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": id,
        "request": {"subtype": "can_use_tool", "tool_name": "ExitPlanMode", "input": {"plan": "1. echo planned"}},
    })
    .to_string()
}

/// #684: an MCP server's form.
pub(super) fn elicitation(id: &str) -> String {
    json!({
        "type": "control_request",
        "request_id": id,
        "request": {
            "subtype": "elicitation",
            "mcp_server_name": "elicit-probe",
            "message": "What is the answer?",
            "mode": "form",
            "requested_schema": {"type": "object", "properties": {"answer": {"type": "string"}}},
            "property_order": ["answer"],
        },
    })
    .to_string()
}
