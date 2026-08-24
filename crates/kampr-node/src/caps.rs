use kampr_herdr::Herdr;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};
use tokio::process::Command;

/// `caps` is answered from here rather than from the host, because a client may send it as often
/// as it likes and one of the two answers costs a process. Long enough that a socket cannot turn
/// itself into a fork bomb; short enough that a session created at the keyboard shows up.
const CAPS_TTL: Duration = Duration::from_secs(10);

/// Bumped by a manage op that changes the set of named sessions, and compared here so the very
/// next `caps` is re-read rather than served from a cache that predates the op. A client
/// refreshing on its own ack was otherwise told for up to ten seconds that the session it had
/// just made did not exist, or that the one it had just stopped was still running (#241).
///
/// Keyed by node rather than kept as one counter, because the counter is the only shared state
/// between an op and the cache and a bare one reaches every node in the process. That is right
/// for the one node a released binary runs and wrong for a test binary, where it turned every
/// other harness's `caps` into a re-read and cost the anti-amplification bound in
/// `prefs_and_caps_are_bounded_rather_than_an_amplifier` its meaning.
static SESSION_SET: LazyLock<Mutex<HashMap<String, u64>>> = LazyLock::new(Default::default);

/// A named session joins the herd as its own node, `<primary>.<name>` (`sessions.rs`), and a
/// session op may be addressed at either — but `caps` is only ever answered for the primary. So
/// both spell the same key.
fn host_of(node_id: &str) -> &str {
    node_id.split('.').next().unwrap_or(node_id)
}

pub fn sessions_changed(node_id: &str) {
    *SESSION_SET
        .lock()
        .unwrap()
        .entry(host_of(node_id).to_string())
        .or_default() += 1;
}

fn generation(node_id: &str) -> u64 {
    SESSION_SET
        .lock()
        .unwrap()
        .get(host_of(node_id))
        .copied()
        .unwrap_or(0)
}

#[derive(Debug, Default)]
pub struct Caps {
    cached: Mutex<Option<(Instant, u64, Value)>>,
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
        let generation = generation(node_id);
        if let Some((at, seen, value)) = self.cached.lock().unwrap().as_ref()
            && *seen == generation
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
        *self.cached.lock().unwrap() = Some((Instant::now(), generation, value.clone()));
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

    /// The ten-second cache is what a client's own refresh runs into: it acks a session op, asks
    /// `caps` straight back, and is handed the answer from before the op (#241).
    #[tokio::test]
    async fn a_session_op_makes_the_next_caps_answer_fresh_rather_than_cached() {
        let caps = Caps::default();
        let herdr = Herdr::new("/nowhere/kampr-caps-test.sock");
        let ask = async || caps.get("01J", &herdr, "/nonexistent/herdr", &[]).await;

        ask().await;
        assert_eq!(caps.spawns(), 1);
        ask().await;
        assert_eq!(caps.spawns(), 1, "inside the TTL nothing is re-read");

        // Another node in the same process is another host's answer, and re-reading for it is
        // the amplifier the TTL exists to prevent.
        sessions_changed("01K");
        ask().await;
        assert_eq!(
            caps.spawns(),
            1,
            "a session op somewhere else is not this node's business"
        );

        sessions_changed("01J.agents");
        ask().await;
        assert_eq!(
            caps.spawns(),
            2,
            "a session's own node is the same host, and its op has to outrank the TTL"
        );
        ask().await;
        assert_eq!(caps.spawns(), 2, "and the fresh answer is cached again");
    }

    #[test]
    fn junk_from_the_cli_is_no_sessions_rather_than_a_panic() {
        assert!(parse_sessions("not json").is_empty());
        assert!(parse_sessions("{}").is_empty());
        assert!(parse_sessions(r#"{"sessions":[{"running":true}]}"#).is_empty());
    }
}
