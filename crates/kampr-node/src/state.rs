use crate::config::Config;
use crate::herd::HerdModel;
use crate::sessions::{SessionNode, Sessions};
use anyhow::{Context, Result};
use kampr_auth::{AuditLog, Auth, NodeIdentity, Store, Tier};
use kampr_core::provider::PaneInfo;
use kampr_core::wire::{NodeEntry, PaneEntry};
use kampr_herdr::Herdr;
use kampr_journal::Registry as Journals;
use kampr_mesh::{Peers, PeersConfig};
use kampr_push::Vapid;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::{Semaphore, watch};
use tokio::task::JoinHandle;

pub const BUILD: &str = match option_env!("KAMPR_BUILD") {
    Some(b) => b,
    None => env!("CARGO_PKG_VERSION"),
};

/// Herdr fires no event when the attached client resizes (probe #52), so the herd model is
/// re-derived on a timer as well as on every structural event. This poll is the only thing that
/// notices a desk resize.
const HERD_POLL: Duration = Duration::from_secs(3);

pub struct Node {
    pub config: Config,
    pub origin: String,
    /// Resolved once: the wildcard case asks the routing table which address a phone would find
    /// this machine on, and that is not a thing to do per request.
    pub allowed_origins: Vec<String>,
    pub sessions: Arc<Sessions>,
    /// Every node reached over a mesh link, and what this node remembers of the ones that
    /// dropped. Empty until somebody joins, which is most nodes.
    pub peers: Arc<Peers>,
    pub auth: Arc<Auth>,
    pub push: Arc<crate::push::Push>,
    /// Client sessions in flight. A permit is held for the life of a socket, so this is a bound
    /// on live sessions rather than on the rate they are opened at.
    pub sockets: Arc<Semaphore>,
    /// Mesh handshakes in flight. Held only across the handshake — an accepted link releases its
    /// permit before it starts serving, or a handful of peers would close the door behind them.
    pub handshakes: Arc<Semaphore>,
    journals: watch::Sender<Arc<Journals>>,
    caps: crate::caps::Caps,
    herd: watch::Sender<Arc<HerdModel>>,
    /// Loaded on first use rather than at startup: a node that never meshes never needs a key,
    /// and writing one it will not use is a file written for nothing.
    identity: OnceLock<NodeIdentity>,
    tasks: Mutex<Vec<JoinHandle<()>>>,
}

impl Drop for Node {
    fn drop(&mut self) {
        self.shutdown();
    }
}

impl Node {
    /// **Nothing here waits on herdr.** Every session's connection is a supervised loop that
    /// retries for as long as the process lives, so `kampr serve` binds its port whether or not
    /// a herd is running and can serve its own "herdr is not running" state.
    pub async fn start(config: Config, state_dir: &Path) -> Result<Arc<Self>> {
        let sessions = Sessions::open(&config);
        let auth = Arc::new(build_auth(&config, state_dir).await?);
        let peers = Peers::new(PeersConfig::default());

        let (herd, _) = watch::channel(Arc::new(HerdModel::default()));
        let home = config.journal_home();
        let (journals, _) = watch::channel(Arc::new(kampr_journal::registry_from_home(&home)));
        let tasks = vec![
            tokio::spawn(refresh_herd(
                sessions.clone(),
                peers.clone(),
                herd.clone(),
                journals.clone(),
                home,
            )),
            tokio::spawn(crate::sessions::discover(sessions.clone())),
        ];

        let (push, mut push_tasks) = crate::push::start(
            load_vapid(&config, state_dir, auth.tier()),
            auth.store().clone(),
            sessions.clone(),
            herd.subscribe(),
        );
        let mut tasks = tasks;
        tasks.append(&mut push_tasks);

        let node = Arc::new(Self {
            origin: config.origin(),
            allowed_origins: config.allowed_origins(),
            // A configured zero would be a node that answers nothing, which is never what an
            // operator editing a limit meant.
            sockets: Arc::new(Semaphore::new(config.limits.sockets.max(1))),
            handshakes: Arc::new(Semaphore::new(config.limits.mesh_handshakes.max(1))),
            config,
            sessions,
            peers,
            auth,
            push,
            journals,
            caps: crate::caps::Caps::default(),
            herd,
            identity: OnceLock::new(),
            tasks: Mutex::new(tasks),
        });
        // Outbound links are what make a NAT'd host joinable, so they are supervised for the life
        // of the process rather than dialled once at startup.
        node.tasks
            .lock()
            .unwrap()
            .push(tokio::spawn(crate::mesh::dial_hubs(Arc::downgrade(&node))));
        Ok(node)
    }

    pub async fn caps(&self) -> serde_json::Value {
        let primary = self.sessions.primary();
        let served: Vec<String> = self.sessions.all().iter().map(|s| s.name.clone()).collect();
        self.caps
            .get(
                &self.config.node_id,
                &primary.herdr,
                &self.config.herdr.binary,
                &served,
            )
            .await
    }

    /// The adapters this node has, which is what `caps.conversation` and every pane's
    /// `has_conversation` are both answered from — so the two cannot disagree.
    pub fn journals(&self) -> Arc<Journals> {
        self.journals.borrow().clone()
    }

    pub fn caps_spawns(&self) -> u64 {
        self.caps.spawns()
    }

    pub fn herd(&self) -> Arc<HerdModel> {
        self.herd.borrow().clone()
    }

    pub fn subscribe_herd(&self) -> watch::Receiver<Arc<HerdModel>> {
        self.herd.subscribe()
    }

    /// Stops everything this node is running: the herd poller, session discovery and every
    /// outbound mesh link. Drop does the same; this is for a caller that holds the last handle
    /// and wants the links gone *now*.
    pub fn shutdown(&self) {
        for task in self.tasks.lock().unwrap().iter() {
            task.abort();
        }
    }

    pub fn node_id(&self) -> &str {
        &self.config.node_id
    }

    /// This node's long-lived mesh identity — a different credential from any device token, so a
    /// compromised viewer session cannot present itself as a node.
    pub fn identity(&self) -> Result<NodeIdentity> {
        if let Some(identity) = self.identity.get() {
            return Ok(identity.clone());
        }
        let path = self.config.key_path();
        let identity =
            NodeIdentity::load_or_create(&path).with_context(|| format!("node key at {}", path.display()))?;
        let _ = self.identity.set(identity.clone());
        Ok(identity)
    }

    pub fn primary(&self) -> Arc<SessionNode> {
        self.sessions.primary()
    }

    /// The session serving this node id or global pane id. A pane addressed on a node this
    /// process does not serve is not ours to act on, which is what keeps ids unambiguous once
    /// the herd is meshed.
    pub fn route(&self, id: &str) -> Option<Arc<SessionNode>> {
        self.sessions.route(id)
    }

    /// The session and the herdr-local pane id behind a global one.
    pub fn resolve(&self, global: &str) -> Option<(Arc<SessionNode>, String)> {
        let session = self.route(global)?;
        let local = session.local_pane(global)?;
        Some((session, local))
    }

    pub fn global_pane(&self, local: &str) -> String {
        self.sessions.primary().global_pane(local)
    }
}

/// The VAPID key, or nothing.
///
/// **Nothing is the honest answer on Tier 0.** Push needs a secure context, and a LAN IP over
/// plain HTTP is not one — a browser there cannot register a service worker at all, so a node
/// that generated a key and advertised `caps.push` would be offering a control that fails at the
/// last step rather than one the client can hide (findings §3.7).
fn load_vapid(config: &Config, state_dir: &Path, tier: &Tier) -> Option<Arc<Vapid>> {
    if !config.push.enabled {
        return None;
    }
    if !tier.push {
        tracing::info!(
            origin = %tier.origin,
            "web push is unavailable on this origin: it is not a secure context"
        );
        return None;
    }
    match Vapid::load_or_create(&Config::vapid_path(state_dir), &config.push_subject()) {
        Ok(vapid) => Some(Arc::new(vapid)),
        Err(e) => {
            tracing::warn!(error = %e, "web push is unavailable: no VAPID key");
            None
        }
    }
}

async fn build_auth(config: &Config, state_dir: &Path) -> Result<Auth> {
    let store = Store::open(&Config::state_db(state_dir))
        .await
        .context("opening the device store")?;
    let mut tier = Tier::detect(&config.origin()).with_context(|| format!("origin {:?}", config.origin()))?;
    if !config.auth.rp_id.is_empty() {
        tier = tier.with_rp_id(&config.auth.rp_id);
    }
    let audit = if config.auth.audit {
        AuditLog::open(&Config::audit_path(state_dir)).context("opening the audit log")?
    } else {
        AuditLog::disabled()
    };
    let policy = kampr_auth::Policy {
        pairing_ttl: Duration::from_secs(config.auth.pairing_ttl_secs),
        tier0_token_ttl: (config.auth.token_days > 0)
            .then(|| Duration::from_secs(config.auth.token_days * 86_400)),
        ..kampr_auth::Policy::default()
    };
    Ok(Auth::new(
        store,
        tier,
        audit,
        policy,
        &config.android.fingerprints,
    )?)
}

async fn refresh_herd(
    sessions: Arc<Sessions>,
    peers: Arc<Peers>,
    herd: watch::Sender<Arc<HerdModel>>,
    journals: watch::Sender<Arc<Journals>>,
    home: PathBuf,
) {
    let mut previous = Arc::new(HerdModel::default());
    let mut mesh = peers.subscribe();
    let conversations = Conversations::default();
    loop {
        let journal = journals.borrow().clone();
        let mut model = build_model(&sessions, &journal, &conversations).await;
        // One herd, whatever host a pane is on. A peer's own nodes arrive already marked `peer`
        // and stamped with the link's measured round trip, so a pane two hops away *looks* two
        // hops away rather than quietly lagging.
        let remote = mesh.borrow_and_update().clone();
        model.nodes.extend(remote.nodes.iter().cloned());
        model.panes.extend(remote.panes.iter().cloned());
        model.stamp(&previous);
        let model = Arc::new(model);
        previous = model.clone();
        herd.send_replace(model);

        tokio::select! {
            _ = wait_for_change(&sessions) => {}
            _ = sessions.notified() => {}
            _ = mesh.changed() => {}
            _ = tokio::time::sleep(HERD_POLL) => {}
        }
        // A harness installed after the node started should not need a restart to be seen.
        journals.send_replace(Arc::new(kampr_journal::registry_from_home(&home)));
    }
}

/// Wakes on the first session to report anything new — a structural change or a herdr going
/// away — so an outage lands on the wire at once rather than at the next poll.
async fn wait_for_change(sessions: &Sessions) {
    let mut watches: Vec<_> = sessions
        .all()
        .iter()
        .map(|s| (s.registry.topology(), s.provider.watch_health()))
        .collect();
    if watches.is_empty() {
        std::future::pending::<()>().await;
    }
    let waits = watches.iter_mut().map(|(topology, health)| {
        Box::pin(async move {
            tokio::select! {
                _ = topology.changed() => {}
                _ = health.changed() => {}
            }
        })
    });
    futures_util::future::select_all(waits).await;
}

/// How long a pane that resolved to no transcript is left alone before the directories are
/// searched again. Deriving a transcript from a working directory is a `read_dir` and up to 64
/// file heads, and the herd is rebuilt every three seconds — so the miss is what needs a floor.
/// A hit needs none: the answer stays true for as long as the file is there, which is a `stat`.
const CONVERSATION_RETRY: Duration = Duration::from_secs(5);

/// Whether a pane has a conversation, which is whether a transcript resolves — not whether the
/// harness is one Kampr knows. The distinction is the whole defect: a `claude` started a minute
/// ago has no transcript, and a pane that claimed one opened on a blank Conversation view whose
/// `convo.load` answered `not_found`.
#[derive(Default)]
struct Conversations {
    seen: Mutex<HashMap<ConversationKey, Resolved>>,
}

type ConversationKey = (String, String, String);

struct Resolved {
    path: Option<PathBuf>,
    at: Instant,
}

impl Conversations {
    /// `live` collects every key this round touched; [`Self::keep`] then drops the rest, so the
    /// cache cannot outlive the herd it describes.
    fn resolves(
        &self,
        journals: &Journals,
        session: &SessionNode,
        info: &PaneInfo,
        live: &mut HashSet<ConversationKey>,
    ) -> bool {
        if !journals.serves(info.agent.as_deref()) {
            return false;
        }
        let announced = crate::convo::announced(&session.provider, &info.pane_id);
        let key = (
            info.agent.clone().unwrap_or_default(),
            info.cwd.clone().unwrap_or_default(),
            announced.as_ref().map(|a| a.value.clone()).unwrap_or_default(),
        );
        live.insert(key.clone());
        match self.seen.lock().unwrap().get(&key) {
            Some(Resolved { path: Some(path), .. }) if path.is_file() => return true,
            Some(Resolved { path: None, at }) if at.elapsed() < CONVERSATION_RETRY => return false,
            _ => {}
        }
        let path = journals
            .locate(
                info.agent.as_deref(),
                announced.as_ref(),
                info.cwd.as_deref().map(Path::new),
            )
            .unwrap_or_default();
        let found = path.is_some();
        self.seen.lock().unwrap().insert(
            key,
            Resolved {
                path,
                at: Instant::now(),
            },
        );
        found
    }

    /// Working directories churn and a node runs for weeks.
    fn keep(&self, live: &HashSet<ConversationKey>) {
        self.seen.lock().unwrap().retain(|key, _| live.contains(key));
    }
}

async fn build_model(sessions: &Sessions, journals: &Journals, conversations: &Conversations) -> HerdModel {
    let mut nodes = Vec::new();
    let mut panes = Vec::new();
    let mut live = HashSet::new();
    for session in sessions.all() {
        let health = session.provider.health();
        nodes.push(NodeEntry {
            id: session.node_id.clone(),
            name: session.node_name.clone(),
            kind: "local".into(),
            online: health.online,
            rtt_ms: match health.online {
                true => ping(&session.herdr).await,
                false => None,
            },
            herdr_version: session.provider.herdr_version(),
            build: Some(BUILD.to_string()),
            detail: health.detail.clone(),
        });
        // A herdr restart keeps its workspaces and panes (probe #70), so an outage marks the node
        // offline and leaves the last-known panes standing rather than emptying the herd under a
        // client that is about to get them all back.
        for info in session.registry.list_panes().await.unwrap_or_default() {
            let has_conversation = conversations.resolves(journals, &session, &info, &mut live);
            panes.push(PaneEntry::new(&session.node_id, &info, has_conversation));
        }
    }
    conversations.keep(&live);
    HerdModel { nodes, panes }
}

async fn ping(herdr: &Herdr) -> Option<f64> {
    let at = Instant::now();
    herdr
        .call::<serde_json::Value>("ping", serde_json::json!({}))
        .await
        .ok()
        .map(|_| at.elapsed().as_secs_f64() * 1000.0)
}
