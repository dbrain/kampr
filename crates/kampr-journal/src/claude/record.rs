use serde::Deserialize;
use serde_json::Value;

use crate::attach::{Att, IMAGE};

#[derive(Debug, Deserialize)]
pub struct Record {
    #[serde(rename = "type")]
    pub kind: String,
    pub uuid: Option<String>,
    pub timestamp: Option<String>,
    #[serde(rename = "isSidechain")]
    pub is_sidechain: Option<bool>,
    #[serde(rename = "isMeta")]
    pub is_meta: Option<bool>,
    pub message: Option<Message>,
    #[serde(rename = "toolUseResult")]
    pub tool_use_result: Option<Value>,
}

#[derive(Debug, Deserialize)]
pub struct Message {
    pub content: Option<Content>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
pub enum Content {
    Text(String),
    Blocks(Vec<ContentBlock>),
}

#[derive(Debug, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ContentBlock {
    Text {
        text: String,
    },
    ToolUse {
        id: String,
        name: String,
        #[serde(default)]
        input: Value,
    },
    Image {
        #[serde(default)]
        source: Value,
    },
    ToolResult {
        tool_use_id: String,
        #[serde(default)]
        content: Value,
        #[serde(default)]
        is_error: bool,
    },
    #[serde(other)]
    Other,
}

/// Every attachment in one record, in the order [`super::ClaudeParser`] meets them. **The two
/// walks have to agree**: an id names an attachment by its ordinal within the record, so a block
/// this yields and the parser skips would hand the next marker somebody else's bytes.
pub fn attachments(record: &Record) -> Vec<Att<'_>> {
    let Some(Content::Blocks(blocks)) = record.message.as_ref().and_then(|m| m.content.as_ref()) else {
        return Vec::new();
    };
    let mut found = Vec::new();
    for block in blocks {
        match block {
            ContentBlock::Image { source } => found.extend(image(source)),
            ContentBlock::ToolResult { content, .. } => found.extend(result_atts(content)),
            _ => {}
        }
    }
    found
}

/// A `Read` of a picture arrives as a `tool_result` whose content array holds an `image` and no
/// text at all, so there is nothing in the result for a reader to go on unless this is named. It
/// is also **512 of the 513 images** on this machine, against a single paste (#248).
pub fn result_atts(content: &Value) -> Vec<Att<'_>> {
    content
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|item| item.get("type").and_then(Value::as_str) == Some("image"))
        .filter_map(|item| image(item.get("source")?))
        .collect()
}

fn image(source: &Value) -> Option<Att<'_>> {
    Some(Att {
        kind: IMAGE,
        mime: source.get("media_type").and_then(Value::as_str),
        name: None,
        data: source.get("data").and_then(Value::as_str)?,
    })
}

pub fn image_subtype(source: &Value) -> Option<&str> {
    source
        .get("media_type")
        .and_then(Value::as_str)
        .and_then(|m| m.strip_prefix("image/"))
}

pub fn result_text(content: &Value) -> String {
    match content {
        Value::String(s) => s.clone(),
        Value::Array(items) => items
            .iter()
            .filter_map(|i| i.get("text").and_then(Value::as_str))
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

/// Rebuilds a unified diff from the `structuredPatch` hunks Edit/Write leave on the tool result.
pub fn unified_patch(tool_use_result: &Value) -> Option<(Option<String>, String)> {
    let hunks = tool_use_result.get("structuredPatch")?.as_array()?;
    if hunks.is_empty() {
        return None;
    }
    let path = tool_use_result
        .get("filePath")
        .and_then(Value::as_str)
        .map(str::to_string);
    let mut out = String::new();
    for hunk in hunks {
        let old_start = hunk.get("oldStart").and_then(Value::as_u64).unwrap_or(0);
        let old_lines = hunk.get("oldLines").and_then(Value::as_u64).unwrap_or(0);
        let new_start = hunk.get("newStart").and_then(Value::as_u64).unwrap_or(0);
        let new_lines = hunk.get("newLines").and_then(Value::as_u64).unwrap_or(0);
        out.push_str(&format!(
            "@@ -{old_start},{old_lines} +{new_start},{new_lines} @@\n"
        ));
        let lines = hunk
            .get("lines")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        for line in lines {
            if let Some(line) = line.as_str() {
                out.push_str(line);
                out.push('\n');
            }
        }
    }
    Some((path, out))
}
