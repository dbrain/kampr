use kampr_herdr::Herdr;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use tokio::process::Command;

/// `caps` is answered from here rather than from the host, because a client may send it as often
/// as it likes and one of the two answers costs a process. Long enough that a socket cannot turn
/// itself into a fork bomb; short enough that a session created at the keyboard shows up.
const CAPS_TTL: Duration = Duration::from_secs(10);

#[derive(Debug, Default)]
pub struct Caps {
    cached: Mutex<Option<(Instant, Value)>>,
    /// Test-visible: what "cached" has to mean is "did not spawn", not "returned quickly".
    spawns: AtomicU64,
}

impl Caps {
    pub fn spawns(&self) -> u64 {
        self.spawns.load(Ordering::Relaxed)
    }

    /// `served` is the set of session names this node actually reaches. Listing a session the
    /// node cannot serve is what made this capability a promise nothing kept.
    pub async fn get(&self, node_id: &str, herdr: &Herdr, binary: &str, served: &[String]) -> Value {
        if let Some((at, value)) = self.cached.lock().unwrap().as_ref()
            && at.elapsed() < CAPS_TTL
        {
            return value.clone();
        }
        self.spawns.fetch_add(1, Ordering::Relaxed);
        let sessions: Vec<Value> = sessions(binary)
            .await
            .into_iter()
            .map(|s| {
                serde_json::json!({
                    "name": s.name,
                    "running": s.running,
                    "served": served.iter().any(|n| n == &s.name),
                })
            })
            .collect();
        let value = serde_json::json!({
            "t": "caps",
            "node": node_id,
            "agent_kinds": agent_kinds(herdr).await,
            "sessions": sessions,
        });
        *self.cached.lock().unwrap() = Some((Instant::now(), value.clone()));
        value
    }
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SessionEntry {
    pub name: String,
    pub running: bool,
    #[serde(skip)]
    pub socket_path: Option<PathBuf>,
}

/// Agent kinds come from the host at runtime and are never a list baked into the node or the
/// client — there were 20 on the machine this was probed on, and the set is per-host and
/// self-updating (probe #48).
pub async fn agent_kinds(herdr: &Herdr) -> Vec<String> {
    let Ok(reply) = herdr
        .call::<Value>("server.agent_manifests", serde_json::json!({}))
        .await
    else {
        return Vec::new();
    };
    let mut kinds: Vec<String> = reply["manifests"]
        .as_array()
        .map(|m| {
            m.iter()
                .filter_map(|entry| entry["agent"].as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    kinds.sort();
    kinds.dedup();
    kinds
}

/// A named session is a whole separate Herdr server with its own socket, created by the CLI and
/// absent from the socket API (probe #49) — so this is the one discovery that shells out.
pub async fn sessions(binary: &str) -> Vec<SessionEntry> {
    let Ok(output) = Command::new(kampr_herdr::locate::program(binary))
        .args(["session", "list", "--json"])
        .output()
        .await
    else {
        return Vec::new();
    };
    if !output.status.success() {
        return Vec::new();
    }
    parse_sessions(&String::from_utf8_lossy(&output.stdout))
}

pub fn parse_sessions(json: &str) -> Vec<SessionEntry> {
    let Ok(value) = serde_json::from_str::<Value>(json) else {
        return Vec::new();
    };
    value["sessions"]
        .as_array()
        .map(|entries| {
            entries
                .iter()
                .filter_map(|e| {
                    Some(SessionEntry {
                        name: e["name"].as_str()?.to_string(),
                        running: e["running"].as_bool().unwrap_or(false),
                        socket_path: e["socket_path"].as_str().map(PathBuf::from),
                    })
                })
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sessions_are_parsed_from_the_cli_shape() {
        let json = r#"{"sessions":[
          {"default":true,"name":"default","running":true,"session_dir":"/c/herdr","socket_path":"/c/herdr/herdr.sock"},
          {"default":false,"name":"agents","running":false,"session_dir":"/c/a","socket_path":"/c/a/herdr.sock"}]}"#;
        let sessions = parse_sessions(json);
        assert_eq!(sessions.len(), 2);
        assert_eq!(sessions[0].name, "default");
        assert!(sessions[0].running);
        assert!(!sessions[1].running);
        assert_eq!(
            sessions[1].socket_path.as_deref(),
            Some(std::path::Path::new("/c/a/herdr.sock"))
        );
    }

    #[test]
    fn junk_from_the_cli_is_no_sessions_rather_than_a_panic() {
        assert!(parse_sessions("not json").is_empty());
        assert!(parse_sessions("{}").is_empty());
        assert!(parse_sessions(r#"{"sessions":[{"running":true}]}"#).is_empty());
    }
}
