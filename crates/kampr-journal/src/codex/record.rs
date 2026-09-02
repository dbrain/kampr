use serde::Deserialize;
use serde_json::Value;

use crate::attach::{Att, IMAGE};

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
    pub image_url: Option<String>,
}

/// Every attachment in one payload, in the order [`super::CodexParser`] meets them.
///
/// **Every `input_image` measured on a real machine — all 67 of them — is in a
/// `function_call_output`, not in a user message** (probe #247): it is what `view_image` answers
/// with. The message arm is the paste, which this machine has never recorded but the format
/// allows.
pub fn attachments(payload: &Payload) -> Vec<Att<'_>> {
    match payload {
        Payload::Message { content, .. } => content
            .iter()
            .filter(|item| item.kind == "input_image")
            .filter_map(|item| data_url_att(item.image_url.as_deref()?))
            .collect(),
        Payload::FunctionCallOutput { output, .. } | Payload::CustomToolCallOutput { output, .. } => {
            output_atts(output)
        }
        _ => Vec::new(),
    }
}

pub fn output_atts(output: &Value) -> Vec<Att<'_>> {
    output
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("input_image"))
        .filter_map(|item| data_url_att(item.get("image_url").and_then(Value::as_str)?))
        .collect()
}

pub fn data_url_att(url: &str) -> Option<Att<'_>> {
    let (meta, data) = url.strip_prefix("data:")?.split_once(',')?;
    if !meta.split(';').any(|p| p == "base64") {
        return None;
    }
    let mime = meta.split(';').next().filter(|m| !m.is_empty());
    Some(Att {
        kind: IMAGE,
        mime,
        name: None,
        data,
    })
}

pub fn data_url_subtype(url: &str) -> Option<&str> {
    url.strip_prefix("data:")?
        .split([';', ','])
        .next()?
        .strip_prefix("image/")
}

/// Codex 0.147 moved shell output to an array of content items; older rollouts use a bare string.
///
/// The items are **chunks of one stream**, not lines: code mode files the `Script completed` /
/// `Output:` header as one item and the bytes the script printed as the next, so joining them on
/// a newline puts a blank line at the top of every result that was never in it.
pub fn output_text(output: &Value) -> String {
    match output {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .filter(|t| !t.is_empty())
            .fold(String::new(), |mut whole, chunk| {
                if !whole.is_empty() && !whole.ends_with('\n') {
                    whole.push('\n');
                }
                whole.push_str(chunk);
                whole
            }),
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

/// The lines Codex 0.147 writes above a command's own output, measured over 1706 `exec_command`,
/// 443 `write_stdin` and 33 `exec` results on this machine: `Chunk ID`, `Wall time`, the exit
/// status or the session id of a process still running, `Original token count`, and `Output:`
/// under them. The `exec` shape opens `Script completed` instead. A `write_stdin` that failed
/// outright writes none of it, and is handed back whole.
const COMMAND_HEADER: &[&str] = &[
    "Chunk ID: ",
    "Wall time",
    "Process exited with code ",
    "Process running with session ID ",
    "Original token count: ",
    "Script completed",
];

pub fn command_output(text: &str) -> &str {
    crate::envelope::after_header(text, COMMAND_HEADER)
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
