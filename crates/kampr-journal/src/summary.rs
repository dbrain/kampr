use serde_json::Value;

use crate::model::Attachment;

const SUMMARY_KEYS: &[&str] = &[
    "description",
    "file_path",
    "path",
    "pattern",
    "query",
    "url",
    "cmd",
    "command",
    "prompt",
];

pub fn summarise(input: &Value) -> Option<String> {
    let object = input.as_object()?;
    SUMMARY_KEYS
        .iter()
        .find_map(|k| object.get(*k).and_then(Value::as_str))
        .map(one_line)
}

pub fn one_line(text: &str) -> String {
    let first = text.lines().next().unwrap_or("").trim();
    if first.chars().count() <= 120 {
        return first.to_string();
    }
    let cut: String = first.chars().take(119).collect();
    format!("{cut}…")
}

pub fn count_lines(text: &str) -> Option<u32> {
    if text.is_empty() {
        return None;
    }
    Some(text.lines().count() as u32)
}

/// An image the wire has no way to carry. It is an `md` block rather than a block of its own
/// because a client that does not know a `b` value drops the block silently, and the phones
/// already installed would go on showing a turn with its screenshot missing and nothing said.
pub fn image_marker(subtype: Option<&str>) -> String {
    match subtype {
        Some(kind) => format!("[image · {kind}]"),
        None => "[image]".to_string(),
    }
}

pub fn marker_of(att: &Attachment) -> String {
    image_marker(att.mime.as_deref().and_then(|m| m.strip_prefix("image/")))
}
