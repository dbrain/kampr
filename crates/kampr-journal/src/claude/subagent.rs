use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::sub;

const META: &str = ".meta.json";

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

/// The `agent-<id>.meta.json` files a session has written, indexed by the call each one names.
///
/// **This is the only link from a `tool_use` to the agent it started.** A synchronous launch
/// writes no `toolUseResult` for 65–146 s and its input carries no `agentId` at all, so without
/// this there is no card until the agent has finished — which is the whole of the time somebody
/// would want to watch it. The meta lands 12–20 ms after the call, and carries `toolUseId`.
///
/// **Matching is on `toolUseId` and never on creation order.** Three launches in flight at once
/// each wrote their own meta, and ordering would be a guess dressed as a rule.
///
/// A file that failed to parse is deliberately *not* recorded as read: the meta is written
/// milliseconds after the call, so a look that catches one half-written must take it again.
#[derive(Default)]
pub struct Metas {
    read: HashSet<String>,
    by_call: HashMap<String, (String, Value)>,
}

impl Metas {
    fn refresh(&mut self, dir: &Path) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(named) = name.strip_suffix(META) else {
                continue;
            };
            if self.read.contains(&name) || !is_named(named) {
                continue;
            }
            let Some(meta) = read_json(&entry.path()) else {
                continue;
            };
            let named = named.to_string();
            self.read.insert(name);
            if let Some(call) = meta.get("toolUseId").and_then(Value::as_str) {
                self.by_call.insert(call.to_string(), (named, meta));
            }
        }
    }
}

/// The transcript an `Agent` call launched, from the `agentId` its `toolUseResult` carries.
pub fn launched(parent: &Path, result: &Value) -> Option<Launched> {
    let named = filename(result.get("agentId").and_then(Value::as_str)?)?;
    let transcript = transcript(parent, &named);
    let meta = read_json(&transcript.with_file_name(format!("{named}{META}"))).unwrap_or(Value::Null);
    Some(describe(transcript, &meta, described(result)))
}

/// The transcript an `Agent` call launched, from the meta the harness wrote at the call — before
/// there is any result to read an `agentId` out of.
///
/// Nothing here is load-bearing for a launch that *does* report an `agentId` later: a meta that
/// has not appeared yet yields no card and [`launched`] mints one when the result lands. 15% of
/// asynchronous launches write their meta between 229 s and 11 030 s after the call, and they must
/// go on working exactly as they did.
pub fn calling(parent: &Path, metas: &mut Metas, tool_use_id: &str, input: &Value) -> Option<Launched> {
    metas.refresh(&filed(parent));
    let (named, meta) = metas.by_call.get(tool_use_id)?;
    Some(describe(transcript(parent, named), meta, described(input)))
}

fn describe(transcript: PathBuf, meta: &Value, described: Option<String>) -> Launched {
    let text = |key: &str| meta.get(key).and_then(Value::as_str).map(str::to_string);
    Launched {
        kind: text("agentType"),
        title: text("description").or(described),
        depth: meta.get("spawnDepth").and_then(Value::as_u64).map(|d| d as u32),
        transcript,
    }
}

fn described(source: &Value) -> Option<String> {
    source
        .get("description")
        .and_then(Value::as_str)
        .map(str::to_string)
}

/// **The file is not proof and must not be asked for.** The result record beats its own transcript
/// to disk by up to 0.777 s, `settle` runs once per `tool_use_id` and nothing retries, so a poll
/// landing in that window used to lose the card for the life of the session. A handle is the
/// file's *name*: `sub.rs` resolves one through [`crate::root::TranscriptRoot::contain`] at open
/// time, which canonicalises and so refuses until the file exists — which is the honest place to
/// answer, because a transcript 100 ms late and one that is never coming are the same thing here.
///
/// Two directories only differ below the top level, where nothing has been measured — the session
/// tree's is where every launch measured so far wrote — so the deeper one is taken only when a
/// file is actually sitting in it.
fn transcript(parent: &Path, named: &str) -> PathBuf {
    let file = format!("{named}.jsonl");
    let beside = parent.with_extension("").join(sub::LAUNCHED).join(&file);
    let filed = filed(parent).join(&file);
    if beside.is_file() && !filed.is_file() {
        beside
    } else {
        filed
    }
}

fn filed(parent: &Path) -> PathBuf {
    sub::tree(parent).join(sub::LAUNCHED)
}

fn read_json(path: &Path) -> Option<Value> {
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
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

fn is_named(stem: &str) -> bool {
    stem.strip_prefix("agent-")
        .and_then(filename)
        .is_some_and(|named| named == stem)
}
