use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::sub;

/// One conversation an `Agent` call launched: where its transcript is, and what the harness wrote
/// beside it about the agent it started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Launched {
    pub transcript: PathBuf,
    pub kind: Option<String>,
    pub title: Option<String>,
    pub depth: Option<u32>,
}

impl Launched {
    /// `Explore — Map the manage op end-to-end path`. The description is on the launching call's
    /// own input too; the *kind* exists nowhere but the meta file, and it is the half that tells
    /// an operator which of five agents is running.
    pub fn label(&self) -> Option<String> {
        match (&self.kind, &self.title) {
            (Some(kind), Some(title)) => Some(format!("{kind} — {title}")),
            (Some(kind), None) => Some(kind.clone()),
            (None, title) => title.clone(),
        }
    }
}

/// The transcript an `Agent` call launched, from the `agentId` its `toolUseResult` carries.
///
/// Two directories are tried because only depth 1 has been measured — `<session>/subagents/`,
/// where a top-level launch writes — and a subagent that launches its own could file it either
/// beside itself or under a directory of its own. A launch whose file is not there yields nothing
/// at all rather than a handle that opens nothing.
pub fn launched(parent: &Path, result: &Value) -> Option<Launched> {
    let named = filename(result.get("agentId").and_then(Value::as_str)?)?;
    let transcript = directories(parent)
        .into_iter()
        .map(|dir| dir.join(format!("{named}.jsonl")))
        .find(|path| path.is_file())?;

    let meta: Value = std::fs::read_to_string(transcript.with_file_name(format!("{named}.meta.json")))
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or(Value::Null);
    let text = |key: &str| meta.get(key).and_then(Value::as_str).map(str::to_string);
    let described = || {
        result
            .get("description")
            .and_then(Value::as_str)
            .map(str::to_string)
    };

    Some(Launched {
        kind: text("agentType"),
        title: text("description").or_else(described),
        depth: meta.get("spawnDepth").and_then(Value::as_u64).map(|d| d as u32),
        transcript,
    })
}

fn directories(parent: &Path) -> [PathBuf; 2] {
    [
        parent.with_extension("").join(sub::LAUNCHED),
        sub::tree(parent).join(sub::LAUNCHED),
    ]
}

/// An `agentId` comes out of a transcript and is pasted into a filename, so it may only be one
/// path-safe segment.
fn filename(id: &str) -> Option<String> {
    let ok = !id.is_empty()
        && id.len() <= 128
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    ok.then(|| format!("agent-{id}"))
}
