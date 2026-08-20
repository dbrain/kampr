use kampr_herdr::Herdr;
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use tokio::process::Command;

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
    let Ok(output) = Command::new(binary)
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
