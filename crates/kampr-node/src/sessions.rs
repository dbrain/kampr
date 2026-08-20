use crate::caps::{SessionEntry, sessions as list_sessions};
use crate::config::Config;
use kampr_core::registry::RegistryConfig;
use kampr_core::{HerdrConfig, HerdrProvider, PaneRegistry};
use kampr_herdr::Herdr;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::Notify;
use tracing::info;

/// A named herdr session is a whole separate server with its own socket (probe #49), so it is a
/// separate node in the herd model rather than more panes on this one. Polled rather than
/// subscribed because sessions are created by the CLI and are absent from the socket API.
const DISCOVERY_POLL: Duration = Duration::from_secs(15);

/// One herdr session: its own socket, its own provider, its own emulators.
///
/// The node id is derived from the configured node id and the session name, so a client can tell
/// `default` from `agents` and a global pane id stays unique across both. The configured session
/// keeps the bare node id — it is the node, and the others hang off it.
pub struct SessionNode {
    pub name: String,
    pub node_id: String,
    pub node_name: String,
    pub herdr: Herdr,
    pub provider: Arc<HerdrProvider>,
    pub registry: Arc<PaneRegistry>,
}

impl SessionNode {
    fn open(config: &Config, name: &str, socket: PathBuf, primary: bool) -> Arc<Self> {
        let herdr = Herdr::new(&socket);
        let provider = Arc::new(HerdrProvider::spawn(
            herdr.clone(),
            HerdrConfig {
                binary: config.herdr.binary.clone(),
                ..HerdrConfig::default()
            },
        ));
        let registry = PaneRegistry::with_config(
            provider.clone(),
            RegistryConfig {
                scrollback_max_rows: config.limits.scrollback_max_rows,
                ..RegistryConfig::default()
            },
        );
        Arc::new(Self {
            node_id: if primary {
                config.node_id.clone()
            } else {
                format!("{}.{name}", config.node_id)
            },
            node_name: if primary {
                config.node_name.clone()
            } else {
                format!("{}/{name}", config.node_name)
            },
            name: name.to_string(),
            herdr,
            provider,
            registry,
        })
    }

    pub fn online(&self) -> bool {
        self.provider.health().online
    }

    /// Strips this node's `<node_id>/` prefix. Exact rather than a bare prefix match: `01J` and
    /// `01J.agents` are two nodes, and only the separator tells them apart.
    pub fn local_pane(&self, global: &str) -> Option<String> {
        global
            .strip_prefix(&self.node_id)
            .and_then(|rest| rest.strip_prefix('/'))
            .filter(|rest| !rest.is_empty())
            .map(str::to_string)
    }

    pub fn global_pane(&self, local: &str) -> String {
        format!("{}/{local}", self.node_id)
    }

    pub fn owns(&self, id: &str) -> bool {
        id == self.node_id || self.local_pane(id).is_some()
    }
}

/// Every herdr session this node serves, with the configured one first.
pub struct Sessions {
    config: Config,
    primary: Arc<SessionNode>,
    all: RwLock<Vec<Arc<SessionNode>>>,
    changed: Notify,
}

impl Sessions {
    /// **Never touches the network.** Every provider connects on its own supervised loop, so a
    /// node with no herdr at all still binds its port and serves an empty herd.
    pub fn open(config: &Config) -> Arc<Self> {
        let socket = match config.herdr.socket.as_str() {
            "" => Herdr::discover().map_or_else(|_| default_socket(), |h| h.socket().to_path_buf()),
            path => PathBuf::from(path),
        };
        let primary = SessionNode::open(config, &session_name_of(&socket), socket, true);
        Arc::new(Self {
            config: config.clone(),
            all: RwLock::new(vec![primary.clone()]),
            primary,
            changed: Notify::new(),
        })
    }

    pub fn primary(&self) -> Arc<SessionNode> {
        self.primary.clone()
    }

    pub fn all(&self) -> Vec<Arc<SessionNode>> {
        self.all.read().unwrap().clone()
    }

    /// Resolves a node id or a global pane id to the session that serves it.
    pub fn route(&self, id: &str) -> Option<Arc<SessionNode>> {
        self.all.read().unwrap().iter().find(|s| s.owns(id)).cloned()
    }

    /// Bumps whenever a session appears or vanishes, so the herd model is rebuilt without
    /// waiting for the next poll.
    pub fn notified(&self) -> impl std::future::Future<Output = ()> + '_ {
        self.changed.notified()
    }

    /// Adds and drops providers to match what is running on the host, leaving every other
    /// session's watchers untouched — a dead session is one node going offline, not this one.
    pub async fn reconcile(&self) {
        let found = list_sessions(&self.config.herdr.binary).await;
        let wanted = self.wanted(&found);
        let mut changed = false;

        let known: HashSet<String> = self.all().iter().map(|s| s.name.clone()).collect();
        for entry in &wanted {
            if known.contains(&entry.name) {
                continue;
            }
            let Some(socket) = entry.socket_path.clone() else {
                continue;
            };
            if socket == self.primary.herdr.socket() {
                continue;
            }
            let session = SessionNode::open(&self.config, &entry.name, socket, false);
            info!(session = %entry.name, node = %session.node_id, "serving a herdr session");
            self.all.write().unwrap().push(session);
            changed = true;
        }

        let live: HashSet<&str> = wanted.iter().map(|s| s.name.as_str()).collect();
        {
            let mut all = self.all.write().unwrap();
            let before = all.len();
            all.retain(|s| Arc::ptr_eq(s, &self.primary) || live.contains(s.name.as_str()));
            changed |= all.len() != before;
        }
        if changed {
            self.changed.notify_waiters();
        }
    }

    fn wanted(&self, found: &[SessionEntry]) -> Vec<SessionEntry> {
        let allowed = self.config.herdr.sessions.as_deref();
        found
            .iter()
            .filter(|s| s.running && s.name != self.primary.name)
            .filter(|s| allowed.is_none_or(|names| names.iter().any(|a| a == &s.name)))
            .cloned()
            .collect()
    }
}

pub async fn discover(sessions: Arc<Sessions>) {
    let mut poll = tokio::time::interval(DISCOVERY_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        poll.tick().await;
        sessions.reconcile().await;
    }
}

/// Herdr keeps a named session at `<config>/herdr/sessions/<name>/herdr.sock`; anything else is
/// the default session's socket.
pub fn session_name_of(socket: &Path) -> String {
    let named = socket
        .parent()
        .filter(|dir| dir.parent().is_some_and(|p| p.ends_with("sessions")))
        .and_then(|dir| dir.file_name())
        .and_then(|n| n.to_str());
    named.unwrap_or("default").to_string()
}

fn default_socket() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config/herdr/herdr.sock")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_named_sessions_socket_carries_its_name() {
        assert_eq!(
            session_name_of(Path::new("/home/x/.config/herdr/sessions/agents/herdr.sock")),
            "agents"
        );
        assert_eq!(
            session_name_of(Path::new("/home/x/.config/herdr/herdr.sock")),
            "default"
        );
        assert_eq!(session_name_of(Path::new("herdr.sock")), "default");
    }

    fn node(id: &str) -> Arc<SessionNode> {
        let mut config = Config::bootstrap("host");
        config.node_id = "01J".into();
        config.herdr.socket = "/nowhere/herdr.sock".into();
        let primary = id == "01J";
        SessionNode::open(
            &config,
            if primary { "default" } else { "agents" },
            "/nowhere/herdr.sock".into(),
            primary,
        )
    }

    #[tokio::test]
    async fn a_second_session_is_a_distinct_node_with_unambiguous_pane_ids() {
        let default = node("01J");
        let agents = node("01J.agents");
        assert_eq!(default.node_id, "01J");
        assert_eq!(agents.node_id, "01J.agents");

        assert_eq!(default.local_pane("01J/w1:p1").as_deref(), Some("w1:p1"));
        assert_eq!(agents.local_pane("01J.agents/w1:p1").as_deref(), Some("w1:p1"));
        // The one that would break under a bare prefix match.
        assert_eq!(default.local_pane("01J.agents/w1:p1"), None);
        assert_eq!(agents.local_pane("01J/w1:p1"), None);
        assert!(default.owns("01J") && !default.owns("01J.agents"));
    }
}
