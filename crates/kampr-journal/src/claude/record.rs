use serde::Deserialize;
use serde_json::Value;

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
