use serde::Deserialize;
use serde_json::Value;

#[derive(Debug, Deserialize)]
pub struct Record {
    #[serde(rename = "type")]
    pub kind: String,
    pub timestamp: Option<String>,
    #[serde(default)]
    pub payload: Value,
}

/// Only `response_item` records carry the conversation. `event_msg` duplicates the assistant
/// messages for the TUI, and `session_meta` / `turn_context` / `world_state` / `compacted` are
/// context bookkeeping, so parsing response items alone avoids double-rendering every turn.
#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Payload {
    Message {
        role: Option<String>,
        #[serde(default)]
        content: Vec<ContentItem>,
    },
    FunctionCall {
        name: String,
        #[serde(default)]
        arguments: String,
        call_id: String,
    },
    FunctionCallOutput {
        call_id: String,
        #[serde(default)]
        output: Value,
    },
    CustomToolCall {
        name: String,
        #[serde(default)]
        input: String,
        call_id: String,
    },
    CustomToolCallOutput {
        call_id: String,
        #[serde(default)]
        output: Value,
    },
    #[serde(other)]
    Other,
}

#[derive(Debug, Deserialize)]
pub struct ContentItem {
    #[serde(rename = "type")]
    pub kind: String,
    pub text: Option<String>,
}

/// Codex 0.147 moved shell output to an array of content items; older rollouts use a bare string.
pub fn output_text(output: &Value) -> String {
    match output {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

pub const PATCH_PREFIX: &str = "*** Begin Patch";

/// `exec_command` reports its exit status inside the output text; `apply_patch` wraps a JSON
/// envelope around it and falls back to a bare error string when the patch does not apply.
pub fn output_failed(text: &str) -> bool {
    let exit_code = serde_json::from_str::<Value>(text)
        .ok()
        .and_then(|v| v.pointer("/metadata/exit_code").and_then(Value::as_i64));
    if let Some(code) = exit_code {
        return code != 0;
    }
    text.lines().any(|l| {
        (l.starts_with("Process exited with code") && !l.ends_with(" 0"))
            || l.starts_with("apply_patch verification failed")
    })
}

pub fn envelope_output(text: &str) -> String {
    match serde_json::from_str::<Value>(text) {
        Ok(v) => v
            .get("output")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| text.to_string()),
        Err(_) => text.to_string(),
    }
}

pub fn patch_target(patch: &str) -> Option<String> {
    patch.lines().find_map(|l| {
        for marker in ["*** Update File: ", "*** Add File: ", "*** Delete File: "] {
            if let Some(rest) = l.strip_prefix(marker) {
                return Some(rest.trim().to_string());
            }
        }
        None
    })
}
