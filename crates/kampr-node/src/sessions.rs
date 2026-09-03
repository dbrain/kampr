use crate::caps::{SessionEntry, SessionListError, sessions as list_sessions};
use crate::config::Config;
use kampr_core::registry::RegistryConfig;
use kampr_core::{Composite, HerdrConfig, HerdrProvider, PaneRegistry};
use kampr_fleet::FleetProvider;
use kampr_herdr::Herdr;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::sync::watch;
use tracing::{info, warn};

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
    /// Runs this node forked itself. Kept beside the herdr provider rather than inside it: a fleet
    /// run is not a herdr pane and must not appear in the operator's own workspaces (probe #331).
    pub fleet: Arc<FleetProvider>,
    pub registry: Arc<PaneRegistry>,
}

impl SessionNode {
    fn open(config: &Config, name: &str, socket: PathBuf, primary: bool) -> Arc<Self> {
        let herdr = Herdr::new(&socket);
        let provider = Arc::new(HerdrProvider::spawn(
            herdr.clone(),
            HerdrConfig {
                binary: config.herdr.binary.clone(),
                send_argv: config.naming.send_argv,
                report_names: config.naming.reporting(),
                desk_agents: config.naming.desk_agents(),
                primary,
                ..HerdrConfig::default()
            },
        ));
        let fleet = Arc::new(FleetProvider::with_path(
            Some(config.fleet.path.clone()).filter(|p| !p.is_empty()),
        ));
        // Fleet first: it answers `owns` only for runs it is actually running, so herdr stays the
        // catch-all for everything else.
        let registry = PaneRegistry::with_config(
            Composite::new(vec![fleet.clone(), provider.clone()]),
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
            fleet,
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
    changed: watch::Sender<u64>,
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
            changed: watch::Sender::new(0),
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
    ///
    /// **A subscription rather than a notification, because the reader looks and then waits.** The
    /// herd loop reads the session set to build its model and only afterwards waits to be told it
    /// moved, and those are two separate moments; a `Notify` wakes whoever is already waiting and
    /// keeps nothing for whoever is about to. A session created by a manage op in that gap was
    /// therefore announced to nobody, and since `reconcile` reports no further change the next
    /// discovery poll finds nothing to say either — leaving the herd a whole `HERD_RECONCILE`
    /// behind the ack that had already promised the session was in it. A receiver taken before
    /// the read remembers the edge.
    pub fn changes(&self) -> watch::Receiver<u64> {
        self.changed.subscribe()
    }

    /// Adds and drops providers to match what is running on the host, leaving every other
    /// session's watchers untouched — a dead session is one node going offline, not this one.
    ///
    /// **`false` means the host was never read**, not that nothing changed. A caller whose promise
    /// depends on the herd being current has to know the difference: the session list is a spawned
    /// process and it can time out under load, and a reconcile that could not read one leaves the
    /// herd exactly as stale as it found it.
    pub async fn reconcile(&self) -> bool {
        self.apply(list_sessions(&self.config.herdr.binary).await)
    }

    /// **`Err` is no information, and no information may not evict.** A named session *is* a node
    /// (`ARCHITECTURE.md` §2), so treating one failed `herdr session list` as "there are none"
    /// dropped every extra server on the host out of every client's herd — tearing down its
    /// watchers — until the next sweep put them back. Adding on a partial read is still safe;
    /// only removal has to stand on an answer.
    ///
    /// Crate-visible for [`crate::state`]'s own test: the session list is a spawned process, and a
    /// test that needs a session to appear at one particular *moment* cannot make one appear by
    /// starting a herdr.
    pub(crate) fn apply(&self, found: Result<Vec<SessionEntry>, SessionListError>) -> bool {
        let found = match found {
            Ok(found) => found,
            Err(e) => {
                warn!(error = %e, "could not list herdr sessions; keeping the ones already served");
                return false;
            }
        };
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
            all.retain(|s| {
                let keep = Arc::ptr_eq(s, &self.primary) || live.contains(s.name.as_str());
                if !keep {
                    info!(session = %s.name, node = %s.node_id, "a herdr session ended");
                }
                keep
            });
            changed |= all.len() != before;
        }
        if changed {
            self.changed.send_modify(|seen| *seen += 1);
        }
        true
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
        // The sweep has nothing to promise anybody: the next tick is the retry.
        let _ = sessions.reconcile().await;
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

    /// **A list that could not be read is not a list of nothing**, and the difference is the whole
    /// contract of a session op's ack: it promises the herd already knows, and a reconcile that
    /// never reached the host has not made that true. Swallowing this left a stopped session in
    /// the herd until the next 15 s sweep, and surfaced only as an intermittent live failure that
    /// named nothing.
    #[tokio::test]
    async fn a_reconcile_that_never_reached_the_host_says_so_rather_than_looking_done() {
        let sessions = sessions();
        assert!(
            !sessions.apply(Err(SessionListError::Timeout {
                program: "herdr".into()
            })),
            "a failed read must report that it read nothing"
        );
        assert!(
            sessions.apply(Ok(Vec::new())),
            "a read that found nothing is still a read"
        );
    }

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

    fn entry(name: &str, socket: &str) -> SessionEntry {
        SessionEntry {
            name: name.into(),
            running: true,
            socket_path: Some(socket.into()),
        }
    }

    fn sessions() -> Arc<Sessions> {
        let mut config = Config::bootstrap("host");
        config.node_id = "01J".into();
        config.herdr.socket = "/nowhere/default/herdr.sock".into();
        Sessions::open(&config)
    }

    /// One `herdr session list` that could not be read used to read as "there are no named
    /// sessions", and a named session *is* a node: every extra herdr server on the host — with
    /// every pane in it — left every client's herd and its watchers came down, until the next
    /// sweep put them back. This is #233's shape applied to the multi-server feature.
    #[tokio::test]
    async fn a_session_list_that_could_not_be_read_does_not_empty_the_herd() {
        let sessions = sessions();
        sessions.apply(Ok(vec![entry("agents", "/nowhere/sessions/agents/herdr.sock")]));
        assert_eq!(sessions.all().len(), 2, "the named session joined the herd");

        sessions.apply(Err(SessionListError::Unreadable("not json".into())));
        assert_eq!(
            sessions.all().len(),
            2,
            "a list nobody could read is not a host with no named sessions"
        );

        // And an answer that really does say so still evicts, or a stopped session would never go.
        sessions.apply(Ok(Vec::new()));
        assert_eq!(
            sessions.all().len(),
            1,
            "a read answer still removes what it omits"
        );
    }
}
