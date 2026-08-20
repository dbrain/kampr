use super::{Check, Status};
use kampr_herdr::Herdr;
use kampr_node::Config;
use kampr_node::caps::{SessionEntry, sessions};
use kampr_node::sessions::session_name_of;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// The floor the plugin manifest declares. Declared there, enforced here, and kept in step by
/// [`tests::the_declared_floor_is_the_enforced_one`] — a manifest promising a version nothing
/// checks is how a node ends up talking to a herdr that does not answer `observe`.
const MIN_HERDR_VERSION: &str = "0.8.2";

/// A herdr that is up answers this in microseconds; one that is wedged never answers at all.
const PING_TIMEOUT: Duration = Duration::from_secs(2);

pub async fn checks(config: &Config) -> Vec<Check> {
    let socket = socket_of(config);
    let (herdr_check, reachable) = herdr(&socket).await;
    vec![herdr_check, sessions_check(config, &socket, reachable).await]
}

fn socket_of(config: &Config) -> PathBuf {
    match config.herdr.socket.as_str() {
        "" => Herdr::discover().map_or_else(|_| PathBuf::from("herdr.sock"), |h| h.socket().to_path_buf()),
        path => PathBuf::from(path),
    }
}

async fn herdr(socket: &Path) -> (Check, bool) {
    let at = socket.display();
    let Some(pong) = ping(socket).await else {
        let detail = if socket.exists() {
            format!("the socket at {at} exists but nothing answered on it")
        } else {
            format!("no herdr socket at {at}")
        };
        return (
            Check::fail("herdr", detail).fix("start herdr, or set herdr.socket in config.toml"),
            false,
        );
    };
    let version = pong["version"].as_str().unwrap_or("unknown").to_string();
    let protocol = pong["protocol"].as_u64().unwrap_or_default();
    let detail = format!("{version} (protocol {protocol}) at {at}");
    if below_floor(&version) {
        return (
            Check::fail(
                "herdr",
                format!("{detail} — this build needs {MIN_HERDR_VERSION} or newer"),
            )
            .fix("herdr update"),
            true,
        );
    }
    (
        Check::ok("herdr", format!("{detail}, floor {MIN_HERDR_VERSION}")),
        true,
    )
}

async fn ping(socket: &Path) -> Option<Value> {
    tokio::time::timeout(
        PING_TIMEOUT,
        Herdr::new(socket).call::<Value>("ping", serde_json::json!({})),
    )
    .await
    .ok()?
    .ok()
}

/// `None` for anything unparseable, which is treated as "not below the floor": refusing to run
/// against a version string we failed to read would be worse than trusting it.
fn parts(version: &str) -> Option<(u64, u64, u64)> {
    let mut fields = version
        .trim()
        .trim_start_matches('v')
        .split(['.', '-', '+'])
        .map(str::parse::<u64>);
    Some((
        fields.next()?.ok()?,
        fields.next().transpose().ok()?.unwrap_or(0),
        fields.next().transpose().ok()?.unwrap_or(0),
    ))
}

fn below_floor(version: &str) -> bool {
    match (parts(version), parts(MIN_HERDR_VERSION)) {
        (Some(found), Some(floor)) => found < floor,
        _ => false,
    }
}

async fn sessions_check(config: &Config, socket: &Path, reachable: bool) -> Check {
    let primary = session_name_of(socket);
    let found = sessions(&config.herdr.binary).await;
    if found.is_empty() {
        let detail = format!("`{} session list` named none", config.herdr.binary);
        return match reachable {
            // A session with no entry in the session list is the default session, which is
            // normal; a herdr that is not running at all is already reported above.
            true => Check::ok("sessions", format!("{detail}; serving {primary}")),
            false => Check::warn("sessions", detail).fix("herdr"),
        };
    }
    let served = served(config, &primary, &found);
    let running: Vec<&SessionEntry> = found.iter().filter(|s| s.running).collect();
    let listed = |names: Vec<String>| match names.is_empty() {
        true => "none".to_string(),
        false => names.join(", "),
    };
    let detail = format!(
        "{} running ({}); serving {}",
        running.len(),
        listed(running.iter().map(|s| s.name.clone()).collect()),
        listed(served.clone()),
    );

    let missing: Vec<&String> = config
        .herdr
        .sessions
        .iter()
        .filter(|name| !found.iter().any(|s| &&s.name == name && s.running))
        .collect();
    if missing.is_empty() {
        return Check::new("sessions", Status::Ok, detail);
    }
    Check::warn(
        "sessions",
        format!(
            "{detail}; configured but not running: {}",
            missing.iter().map(|n| n.as_str()).collect::<Vec<_>>().join(", ")
        ),
    )
    .fix(format!(
        "herdr server --session {}",
        missing.first().map_or("<name>", |n| n.as_str())
    ))
}

/// Mirrors what the node itself serves: the session its socket points at, plus every running
/// session the config allows — an empty allow-list meaning all of them.
fn served(config: &Config, primary: &str, found: &[SessionEntry]) -> Vec<String> {
    let allowed = &config.herdr.sessions;
    let up = found.iter().any(|s| s.running && s.name == primary);
    let mut names = vec![match up {
        true => primary.to_string(),
        false => format!("{primary} (not running)"),
    }];
    names.extend(
        found
            .iter()
            .filter(|s| s.running && s.name != primary)
            .filter(|s| allowed.is_empty() || allowed.iter().any(|a| a == &s.name))
            .map(|s| s.name.clone()),
    );
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The manifest declares the floor and this module enforces it. If they drift, the manifest
    /// is a promise nothing keeps.
    #[test]
    fn the_declared_floor_is_the_enforced_one() {
        let manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../herdr-plugin.toml");
        let Ok(text) = std::fs::read_to_string(manifest) else {
            return;
        };
        let declared = text
            .lines()
            .find_map(|l| l.strip_prefix("min_herdr_version"))
            .map(|l| {
                l.trim_start_matches([' ', '='])
                    .trim()
                    .trim_matches('"')
                    .to_string()
            })
            .expect("herdr-plugin.toml declares min_herdr_version");
        assert_eq!(declared, MIN_HERDR_VERSION);
    }

    #[test]
    fn a_version_below_the_floor_is_caught_and_a_newer_one_is_not() {
        assert!(below_floor("0.8.1"));
        assert!(below_floor("0.7.9"));
        assert!(!below_floor("0.8.2"));
        assert!(!below_floor("0.8.3"));
        assert!(!below_floor("v0.9.0-preview.2"));
        assert!(!below_floor("1.0.0"));
        assert!(!below_floor("nonsense"), "an unreadable version is not a refusal");
    }

    #[test]
    fn the_served_set_matches_what_the_node_would_serve() {
        let entry = |name: &str, running: bool| SessionEntry {
            name: name.into(),
            running,
            socket_path: None,
        };
        let stopped = [entry("agents", true)];
        assert_eq!(
            served(&Config::bootstrap("x"), "default", &stopped),
            ["default (not running)", "agents"],
            "a primary whose session is gone must not be reported as served"
        );
        let found = [
            entry("default", true),
            entry("agents", true),
            entry("stopped", false),
        ];
        let mut config = Config::bootstrap("x");
        assert_eq!(served(&config, "default", &found), ["default", "agents"]);

        config.herdr.sessions = vec!["agents".into()];
        assert_eq!(served(&config, "default", &found), ["default", "agents"]);

        config.herdr.sessions = vec!["nothing".into()];
        assert_eq!(served(&config, "default", &found), ["default"]);
    }
}
