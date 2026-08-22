use serde::Deserialize;
use serde_json::Value;

use crate::summary::one_line;

#[derive(Debug, Deserialize)]
pub struct Record {
    #[serde(rename = "type")]
    pub kind: Option<String>,
    pub source: Option<String>,
    pub created_at: Option<String>,
    pub content: Option<String>,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
}

#[derive(Debug, Deserialize)]
pub struct ToolCall {
    pub name: String,
    #[serde(default)]
    pub args: Value,
}

/// What `agy` writes around the operator's words before handing them to its model: the request
/// itself, then the local time and any setting the operator changed on the way in.
const OPEN: &str = "<USER_REQUEST>";
const CLOSE: &str = "</USER_REQUEST>";

pub fn request(content: &str) -> String {
    match content.find(OPEN).map(|at| at + OPEN.len()) {
        Some(from) => match content[from..].find(CLOSE) {
            Some(to) => content[from..from + to].trim().to_string(),
            None => content[from..].trim().to_string(),
        },
        None => content.trim().to_string(),
    }
}

/// Every result opens with the same two stamps. They are the harness's bookkeeping, identical on
/// every result, and counting them would add two lines to every tool card on the screen.
pub fn result_body(content: &str) -> &str {
    let mut rest = content;
    for prefix in ["Created At: ", "Completed At: "] {
        let Some(after) = rest.strip_prefix(prefix) else {
            break;
        };
        rest = match after.find('\n') {
            Some(at) => &after[at + 1..],
            None => "",
        };
    }
    rest.trim()
}

/// `agy` 1.1.18 records a shell failure only in the prose of the result — `status` stays `DONE`
/// on the call whether it worked or not.
pub fn exit_failed(body: &str) -> bool {
    body.lines().any(|line| {
        line.strip_prefix("The command exited with code ")
            .and_then(|rest| rest.trim_end_matches('.').parse::<i64>().ok())
            .is_some_and(|code| code != 0)
    })
}

const DIFF_OPEN: &str = "[diff_block_start]";
const DIFF_CLOSE: &str = "[diff_block_end]";

/// The edit tool puts a real unified diff in its *result*, fenced by markers of its own.
pub fn diff(body: &str) -> Option<String> {
    let from = body.find(DIFF_OPEN)? + DIFF_OPEN.len();
    let to = body[from..].find(DIFF_CLOSE)? + from;
    Some(body[from..to].trim().to_string())
}

/// The arguments worth putting on a tool card, best first. `toolSummary` is the harness's own
/// one-line description of the call, written by the model that made it, so nothing below it is
/// reached for a call that has one.
const SUMMARY_KEYS: &[&str] = &[
    "toolSummary",
    "CommandLine",
    "TargetFile",
    "AbsolutePath",
    "DirectoryPath",
    "SearchDirectory",
    "Query",
    "Pattern",
];

pub fn summarise(args: &Value) -> Option<String> {
    let object = args.as_object()?;
    SUMMARY_KEYS
        .iter()
        .find_map(|k| object.get(*k).and_then(Value::as_str))
        .map(one_line)
}

pub fn arg<'a>(args: &'a Value, key: &str) -> Option<&'a str> {
    args.get(key).and_then(Value::as_str)
}
