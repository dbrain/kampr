use crate::config::Config;
use crate::herd::HerdModel;
use crate::sessions::{SessionNode, Sessions};
use anyhow::{Context, Result};
use kampr_auth::{AuditLog, Auth, NodeIdentity, Store, Tier};
use kampr_core::provider::{AgentStatus, PaneInfo};
use kampr_core::wire::{NodeEntry, PaneEntry};
use kampr_herdr::Herdr;
use kampr_journal::{FacetFold, Harness, Registry as Journals, SessionMarker, Titles};
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

/// How long the herd model may go un-rebuilt when nothing has told it to.
///
/// **The model is event-driven; this is the sweep behind it.** Every structural change reaches it
/// through the provider's own subscription, a herdr going away reaches it through the health
/// watch, and a viewer arriving reaches it through the registry — so what is left for a timer is
/// only what none of those carry: each node's round trip, and a pane whose transcript appeared
/// after it was last looked for. Neither is worth a rebuild every three seconds on a box nobody is
/// connected to.
const HERD_RECONCILE: Duration = Duration::from_secs(30);

pub struct Node {
    pub config: Config,
    /// Where this node keeps what it owns on disk. A paste writes here, so it is the one
    /// directory a client can cause bytes to land in and the node picks every part of the path.
    pub state_dir: PathBuf,
    pub origin: String,
    /// Resolved once: the wildcard case asks the routing table which address a phone would find
    /// this machine on, and that is not a thing to do per request.
    pub allowed_origins: Vec<String>,
    pub sessions: Arc<Sessions>,
    /// Every node reached over a mesh link, and what this node remembers of the ones that
    /// dropped. Empty until somebody joins, which is most nodes.
    pub peers: Arc<Peers>,
    /// Controllers held open on the operator's behalf by `pane.size`. Empty on every node that has
    /// never been asked to reshape a pane, which is nearly all of them.
    pub holds: Arc<crate::holds::PaneHolds>,
    pub auth: Arc<Auth>,
    pub push: Arc<crate::push::Push>,
    /// One per node, because the rate limit it exists for is per *desktop*: a Toaster made at the
    /// call site has never seen the toast before it and refuses nothing.
    pub toaster: crate::toast::Toaster,
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
        // A hub is told its own node id so a peer cannot advertise the machine the operator is
        // standing on.
        let peers = Peers::new(PeersConfig {
            own_node_id: Some(config.node_id.clone()),
            ..PeersConfig::default()
        });

        let (herd, _) = watch::channel(Arc::new(HerdModel::default()));
        let home = config.journal_home();
        let (journals, _) = watch::channel(Arc::new(kampr_journal::registry_from_home(&home)));
        let (available, mut tasks) = crate::update::start(&config, state_dir);
        tasks.extend([
            tokio::spawn(refresh_herd(
                sessions.clone(),
                peers.clone(),
                herd.clone(),
                journals.clone(),
                home,
                available,
            )),
            tokio::spawn(crate::sessions::discover(sessions.clone())),
        ]);

        let (push, mut push_tasks) = crate::push::start(
            load_vapid(&config, state_dir, auth.tier()),
            auth.store().clone(),
            sessions.clone(),
            herd.subscribe(),
        );
        tasks.append(&mut push_tasks);

        let node = Arc::new(Self {
            state_dir: state_dir.to_path_buf(),
            origin: config.origin(),
            allowed_origins: config.allowed_origins(),
            // A configured zero would be a node that answers nothing, which is never what an
            // operator editing a limit meant.
            sockets: Arc::new(Semaphore::new(config.limits.sockets.max(1))),
            handshakes: Arc::new(Semaphore::new(config.limits.mesh_handshakes.max(1))),
            config,
            sessions,
            peers,
            holds: Arc::new(crate::holds::PaneHolds::default()),
            auth,
            push,
            toaster: crate::toast::Toaster::default(),
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

    /// Test-visible: the model is built by [`refresh_herd`] from what a herdr reports, and a test
    /// that needs a *particular* herd — or one that moves at a particular moment — cannot make a
    /// herdr produce it.
    pub fn publish_herd(&self, model: HerdModel) {
        self.herd.send_replace(Arc::new(model));
    }

    /// Puts every desk's own agent order back, for a node that is on its way out.
    ///
    /// **`Drop` cannot do this** — the clear is a socket round trip per session — and nothing else
    /// will: herdr will not say what view it is holding, so a sort this node set and did not clear
    /// is a sidebar left wrong until somebody clears it by hand.
    pub async fn restore_desks(&self) {
        for session in self.sessions.all() {
            session.provider.restore_desk().await;
        }
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
    mut update: watch::Receiver<Option<String>>,
) {
    let mut previous = Arc::new(HerdModel::default());
    let mut mesh = peers.subscribe();
    let conversations = Conversations::default();
    let names = Names::default();
    loop {
        // Subscribed *before* the model is built. A viewer joining while `build_model` is still
        // running would otherwise be seen by neither this round nor the wait that follows it, and
        // what is behind that wait is a sweep measured in tens of seconds.
        let mut changes = session_changes(&sessions);
        let journal = journals.borrow().clone();
        let available = update.borrow_and_update().clone();
        let mut model = build_model(&sessions, &journal, &conversations, &names, available.as_deref()).await;
        // One herd, whatever host a pane is on. A peer's own nodes arrive already marked `peer`
        // and stamped with the link's measured round trip, so a pane two hops away *looks* two
        // hops away rather than quietly lagging.
        let remote = mesh.borrow_and_update().clone();
        // A node this process answers for is never a peer's to describe: `HerdModel::diff` would
        // emit both entries and a client keyed on node id keeps whichever came last, which is the
        // remote one. `Peers::keep_own` drops the claim as it arrives and warns, and it is told one
        // id — the configured one. This is the wider net: a node serving several herdr sessions
        // answers for a node id per session, and that set is known here and changes while the
        // process runs.
        let local: HashSet<String> = model.nodes.iter().map(|node| node.id.clone()).collect();
        let ours = |id: &str| local.contains(id.split('/').next().unwrap_or(id));
        model
            .nodes
            .extend(remote.nodes.iter().filter(|n| !ours(&n.id)).cloned());
        model.panes.extend(
            remote
                .panes
                .iter()
                .filter(|p| !ours(&p.node_id) && !ours(&p.id))
                .cloned(),
        );
        model.stamp(&previous);
        let model = Arc::new(model);
        previous = model.clone();
        herd.send_replace(model);

        // A transcript appearing on disk is the one change nothing signals — no herdr event, no
        // provider revision, no watcher — so the sweep shortens to the retry floor for exactly as
        // long as some pane is still waiting for one, and goes back to slow when none is.
        let sweep = match conversations.pending() {
            true => CONVERSATION_RETRY,
            false => HERD_RECONCILE,
        };
        tokio::select! {
            _ = wait_for_change(&mut changes) => {}
            _ = sessions.notified() => {}
            _ = moved(&mut mesh) => {}
            _ = moved(&mut update) => {}
            _ = tokio::time::sleep(sweep) => {}
        }
        // A harness installed after the node started should not need a restart to be seen.
        journals.send_replace(Arc::new(kampr_journal::registry_from_home(&home)));
    }
}

/// Waits for the next value, and for ever once there can be no next value.
///
/// A `watch` whose sender is gone answers `changed()` immediately and keeps answering, so a
/// closed channel in a `select!` is a spin and not a wait. Release discovery is off on most
/// nodes and its sender is dropped the moment it is declined, which made that spin the default:
/// the herd was rebuilt — and every node pinged — as fast as the loop could go round.
async fn moved<T>(rx: &mut watch::Receiver<T>) {
    if rx.changed().await.is_err() {
        std::future::pending::<()>().await;
    }
}

/// The three things a session can report between two rebuilds: a structural change, a herdr going
/// away, and a viewer joining or leaving a pane.
type SessionChanges = (
    watch::Receiver<u64>,
    watch::Receiver<kampr_core::herdr_provider::Health>,
    watch::Receiver<u64>,
);

fn session_changes(sessions: &Sessions) -> Vec<SessionChanges> {
    sessions
        .all()
        .iter()
        .map(|s| {
            (
                s.registry.topology(),
                s.provider.watch_health(),
                s.registry.watchers_changed(),
            )
        })
        .collect()
}

/// Wakes on the first session to report any of them, so all three land on the wire at once rather
/// than at the next sweep.
async fn wait_for_change(watches: &mut [SessionChanges]) {
    if watches.is_empty() {
        std::future::pending::<()>().await;
    }
    let waits = watches.iter_mut().map(|(topology, health, watchers)| {
        Box::pin(async move {
            tokio::select! {
                _ = moved(topology) => {}
                _ = moved(health) => {}
                _ = moved(watchers) => {}
            }
        })
    });
    futures_util::future::select_all(waits).await;
}

/// How long a pane that resolved to no transcript is left alone before the directories are
/// searched again. Deriving a transcript from a working directory is a `read_dir` and up to 64
/// file heads, and an event-driven rebuild can fire far faster than that — so the miss is what
/// needs a floor. A hit needs none: the answer stays true for as long as the file is there, which
/// is a `stat`.
const CONVERSATION_RETRY: Duration = Duration::from_secs(5);

/// Whether a pane has a conversation, which is whether a transcript resolves — not whether the
/// harness is one Kampr knows. The distinction is the whole defect: a `claude` started a minute
/// ago has no transcript, and a pane that claimed one opened on a blank Conversation view whose
/// `convo.load` answered `not_found`.
#[derive(Default)]
struct Conversations {
    seen: Mutex<HashMap<ConversationKey, Resolved>>,
    /// Whether the last round left an agent pane whose transcript has not appeared yet.
    waiting: std::sync::atomic::AtomicBool,
}

/// What a resolution can be cached against: a pane whose harness, directory *and* session are
/// all the same is the same conversation. The session is the last field, and it is the one that
/// moves when an agent is quit and a fresh one started in the same directory — without it a
/// restarted pane kept advertising the transcript of the run before it for as long as the node
/// lived.
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
    ) -> Option<PathBuf> {
        if !journals.serves(info.agent.as_deref()) {
            return None;
        }
        let announced = crate::convo::identity(journals, &session.provider, &info.pane_id).announced;
        let key = (
            info.agent.clone().unwrap_or_default(),
            info.cwd.clone().unwrap_or_default(),
            match announced.as_ref() {
                Some(a) => a.value.clone(),
                // A harness that is absent and one nothing could look for resolve differently, so
                // they may not share a cache entry.
                None => match &info.agent_harness {
                    Harness::Running(p) => format!("{}:{}", p.pid, p.start.as_deref().unwrap_or_default()),
                    Harness::Absent => "absent".into(),
                    Harness::Unknown => String::new(),
                },
            },
        );
        live.insert(key.clone());
        match self.seen.lock().unwrap().get(&key) {
            Some(Resolved { path: Some(path), .. }) if path.is_file() => return Some(path.clone()),
            Some(Resolved { path: None, at }) if at.elapsed() < CONVERSATION_RETRY => return None,
            _ => {}
        }
        let path = journals
            .locate(
                info.agent.as_deref(),
                announced.as_ref(),
                info.cwd.as_deref().map(Path::new),
                &info.agent_harness,
            )
            .unwrap_or_default();
        self.seen.lock().unwrap().insert(
            key,
            Resolved {
                path: path.clone(),
                at: Instant::now(),
            },
        );
        path
    }

    /// Working directories churn and a node runs for weeks.
    fn keep(&self, live: &HashSet<ConversationKey>) {
        let mut seen = self.seen.lock().unwrap();
        seen.retain(|key, _| live.contains(key));
        let waiting = seen.values().any(|r| r.path.is_none());
        self.waiting.store(waiting, std::sync::atomic::Ordering::Relaxed);
    }

    fn pending(&self) -> bool {
        self.waiting.load(std::sync::atomic::Ordering::Relaxed)
    }
}

/// A title fold per transcript, kept between rebuilds.
///
/// **The whole-transcript read is what this exists to avoid.** The good names a session has —
/// `ai-title`, `agent-name`, and a `custom-title.json` the operator typed — are in the transcript
/// and nowhere else, and reading one to the end costs 1.0 ms at this machine's median transcript
/// (958 KB of 262) and 26 ms at its largest (29 MB). Paying that per pane per rebuild is what kept
/// the herd off them, and a herd is rebuilt on every structural event. A fold holds its byte
/// cursor, so every look after the first costs the records the file has grown by: **1.9 us**,
/// flat, whatever the transcript's size — beside the 34 us the marker beside it already costs.
///
/// Keyed on the transcript path, and pruned the way [`Conversations`] is: a `/clear` opens a new
/// file under a new session id (#259), so the entry for the old one is dropped on the first round
/// that stops naming it rather than kept for the weeks a node runs.
#[derive(Default)]
struct Names {
    folds: Mutex<HashMap<PathBuf, Box<dyn FacetFold>>>,
}

impl Names {
    fn title(
        &self,
        journals: &Journals,
        agent: Option<&str>,
        transcript: &Path,
        marker: Option<&SessionMarker>,
        live: &mut HashSet<PathBuf>,
    ) -> Option<String> {
        live.insert(transcript.to_path_buf());
        let mut folds = self.folds.lock().unwrap();
        let fold = folds
            .entry(transcript.to_path_buf())
            .or_insert_with(|| journals.folder(agent));
        best_name(fold.titles(transcript, None), marker)
    }

    fn keep(&self, live: &HashSet<PathBuf>) {
        self.folds.lock().unwrap().retain(|path, _| live.contains(path));
    }
}

/// The conversation surface's precedence with the herd's weakest level in place of its own.
///
/// Manual beats generated beats named, exactly as `convo.facets` publishes it — the marker is
/// handed to the fold as `None` and its name substituted here, because the name a harness derived
/// for itself is a real title on the conversation and is refused on the herd ([`chosen_name`],
/// #311). Substituting rather than overriding is the point: a derived name that reached `named`
/// would be resolved and shown wherever a session has no title yet, which is the machine name
/// this path already decided not to say.
fn best_name(mut titles: Titles, marker: Option<&SessionMarker>) -> Option<String> {
    titles.named = marker.and_then(chosen_name).or(titles.named);
    titles.resolve().map(|title| title.text)
}

async fn build_model(
    sessions: &Sessions,
    journals: &Journals,
    conversations: &Conversations,
    names: &Names,
    update: Option<&str>,
) -> HerdModel {
    let mut nodes = Vec::new();
    let mut panes = Vec::new();
    let mut live = HashSet::new();
    let mut titled = HashSet::new();
    for session in sessions.all() {
        let health = session.provider.health();
        nodes.push(NodeEntry {
            id: session.node_id.clone(),
            name: session.node_name.clone(),
            kind: "local".into(),
            online: health.online,
            // This process is answering by definition — it is the one assembling the message. A
            // herdr that is down takes the panes and leaves the node able to run a fleet command.
            reachable: Some(true),
            rtt_ms: match health.online {
                true => ping(&session.herdr).await,
                false => None,
            },
            herdr_version: session.provider.herdr_version(),
            build: Some(BUILD.to_string()),
            // Reported by the node it describes, never judged by a hub: only this process knows
            // what it is running and whether its operator allowed it to ask.
            update: update.map(str::to_string),
            detail: health.detail.clone(),
        });
        // A herdr restart keeps its workspaces and panes (probe #70), so an outage marks the node
        // offline and leaves the last-known panes standing rather than emptying the herd under a
        // client that is about to get them all back.
        for info in session.registry.list_panes().await.unwrap_or_default() {
            let transcript = conversations.resolves(journals, &session, &info, &mut live);
            let has_conversation = transcript.is_some();
            let watchers = session.registry.watcher_count(&info.pane_id);
            // The harness's own name for the session, off the marker it writes by pid (#311) —
            // 34 us on a hit and 1.7 us on a pane that is not an agent, which is what makes it
            // affordable once per pane per rebuild.
            let marker = journals.marker(&session.provider.pane_processes(&info.pane_id));
            // And the name the session actually goes by, off the transcript the pane is already
            // known to be on — folded from the byte the last rebuild reached rather than read
            // whole. A pane with no transcript yet has only what the marker says.
            let title = transcript
                .as_deref()
                .and_then(|t| names.title(journals, info.agent.as_deref(), t, marker.as_ref(), &mut titled))
                .or_else(|| marker.as_ref().and_then(chosen_name));
            let mut entry = PaneEntry::new(&session.node_id, &info, has_conversation)
                .with_watchers(watchers)
                .with_title(title);
            // The harness's own answer beats the screen. Herdr's status comes from regexes over
            // a pane's rendered output (#355), and its evidence buffer only records titles
            // written *after* it attached the label — never backfilled — so a harness that
            // titles itself too early leaves herdr publishing `idle` indefinitely at a pane
            // whose title says working (#360). `idle` from a scrape is never evidence, only the
            // absence of a match. This costs nothing: the marker is already in hand for the
            // title.
            if let Some(status) = marker.as_ref().and_then(harness_status) {
                entry.agent_status = status;
            }
            panes.push(entry);
        }
    }
    conversations.keep(&live);
    names.keep(&titled);
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

/// What the harness says it is doing, mapped onto the herd's five.
///
/// `waiting` is the one worth having. Herdr can reach `blocked` — its manifests carry rules for
/// it (#355) — but only by matching a regex against the screen, and its answer when nothing
/// matches is `idle`, published with as much confidence as a match (#360). A harness saying so
/// itself needs no prompt to be on screen and no rule to have been written for it. `shell` is
/// idle with a background shell task, which is idle to anyone deciding where to look. A word
/// this does not know leaves the pane's existing status alone rather than flattening it.
fn harness_status(marker: &SessionMarker) -> Option<AgentStatus> {
    match marker.status.as_deref()? {
        "busy" => Some(AgentStatus::Working),
        "waiting" => Some(AgentStatus::Blocked),
        "idle" | "shell" => Some(AgentStatus::Idle),
        _ => None,
    }
}

/// The harness's own name for a session, when it is a name somebody chose.
///
/// Claude derives one the moment a session opens — the working directory's basename
/// and two hex characters, `kampr-44` — and it is what a pane is called before a word
/// has been said in it. It identifies nothing a person would recognise, and it sits in
/// the naming template *above* the workspace label the operator did choose. A name a
/// person set is a different thing and still wins; `nameSource` is measured as `auto`,
/// `derived` and absent (#311), and none of those is a person.
fn chosen_name(marker: &SessionMarker) -> Option<String> {
    match marker.name_source.as_deref() {
        Some("auto" | "derived") | None => None,
        Some(_) => marker.name.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn named(name: &str, source: Option<&str>) -> SessionMarker {
        SessionMarker {
            agent: "claude".into(),
            pid: 1,
            session: "s".into(),
            cwd: None,
            name: Some(name.into()),
            name_source: source.map(str::to_string),
            status: None,
            transcript: None,
        }
    }

    #[test]
    fn a_name_the_harness_derived_for_itself_is_not_what_the_pane_is_called() {
        assert_eq!(chosen_name(&named("kampr-44", Some("derived"))), None);
        assert_eq!(chosen_name(&named("kampr-1f", Some("auto"))), None);
        assert_eq!(chosen_name(&named("kampr-44", None)), None);
    }

    fn saying(status: Option<&str>) -> SessionMarker {
        SessionMarker {
            status: status.map(str::to_string),
            ..named("n", Some("derived"))
        }
    }

    /// The state herdr cannot see: a pane blocked on a prompt and a pane that has finished look
    /// identical on screen, so a scrape calls both idle.
    #[test]
    fn a_harness_that_says_it_is_waiting_is_blocked_rather_than_idle() {
        assert_eq!(
            harness_status(&saying(Some("waiting"))),
            Some(AgentStatus::Blocked)
        );
        assert_eq!(harness_status(&saying(Some("busy"))), Some(AgentStatus::Working));
        assert_eq!(harness_status(&saying(Some("idle"))), Some(AgentStatus::Idle));
        assert_eq!(harness_status(&saying(Some("shell"))), Some(AgentStatus::Idle));
    }

    /// A word from a newer harness than this one leaves the pane as it was. Flattening an
    /// unrecognised state to `Unknown` would throw away herdr's answer to keep our own silence.
    #[test]
    fn a_status_this_node_does_not_know_leaves_the_pane_alone() {
        assert_eq!(harness_status(&saying(Some("compacting"))), None);
        assert_eq!(harness_status(&saying(None)), None);
    }

    fn levels(manual: Option<&str>, generated: Option<&str>, named: Option<&str>) -> Titles {
        Titles {
            manual: manual.map(str::to_string),
            generated: generated.map(str::to_string),
            named: named.map(str::to_string),
        }
    }

    /// The cost the herd used to accept for refusing `kampr-44`: two Claude panes in one workspace
    /// fell through to the workspace label and rendered identically. The real name was in the
    /// transcript all along.
    #[test]
    fn a_name_the_harness_derived_never_displaces_the_one_the_transcript_carries() {
        let derived = named("kampr-44", Some("derived"));

        assert_eq!(
            best_name(
                levels(None, Some("Inferring a pane's width"), None),
                Some(&derived)
            )
            .as_deref(),
            Some("Inferring a pane's width")
        );
        assert_eq!(
            best_name(levels(None, None, Some("kampr-queue")), Some(&derived)).as_deref(),
            Some("kampr-queue"),
            "an `agent-name` off the transcript is a name too, and it outranks nothing at all"
        );
        assert_eq!(
            best_name(Titles::default(), Some(&derived)),
            None,
            "and with nothing in the transcript the pane is still not called what it called itself"
        );
    }

    /// The same order `convo.facets` publishes, so the herd and the conversation cannot disagree
    /// about what a session is called.
    #[test]
    fn a_title_the_operator_typed_outranks_every_title_a_machine_made() {
        let chosen = named("the release", Some("user"));

        assert_eq!(
            best_name(
                levels(
                    Some("the width inference rewrite"),
                    Some("Inferring a pane's width"),
                    Some("kampr-fb")
                ),
                Some(&chosen),
            )
            .as_deref(),
            Some("the width inference rewrite")
        );
        assert_eq!(
            best_name(
                levels(None, Some("Inferring a pane's width"), None),
                Some(&chosen)
            )
            .as_deref(),
            Some("Inferring a pane's width"),
            "a name is the weakest level even when a person set it, exactly as the conversation has it"
        );
        assert_eq!(
            best_name(Titles::default(), Some(&chosen)).as_deref(),
            Some("the release")
        );
        assert_eq!(best_name(Titles::default(), None), None);
    }

    /// The fold is handed no marker on purpose: its own weakest level is the marker's name
    /// whatever that name is, and a `derived` one reaching it would be resolved and shown — the
    /// machine name this path already refuses.
    #[test]
    fn a_name_the_harness_derived_does_not_reach_the_fold_as_the_session_title() {
        let home = tempfile::tempdir().expect("a home");
        let project = home.path().join(".claude/projects/-home-u-demo");
        std::fs::create_dir_all(&project).expect("a project");
        let transcript = project.join("3c9e7a10-0000-4000-8000-0000000000f3.jsonl");
        std::fs::write(&transcript, "{\"type\":\"user\",\"uuid\":\"u1\"}\n").expect("a transcript");
        let journals = kampr_journal::registry_from_home(home.path());

        assert_eq!(
            Names::default().title(
                &journals,
                Some("claude"),
                &transcript,
                Some(&named("kampr-44", Some("derived"))),
                &mut HashSet::new(),
            ),
            None,
            "an untitled session is not called what the harness called itself"
        );
        assert_eq!(
            Names::default()
                .title(
                    &journals,
                    Some("claude"),
                    &transcript,
                    Some(&named("the release", Some("user"))),
                    &mut HashSet::new(),
                )
                .as_deref(),
            Some("the release"),
            "and a name a person set still lands, at the level the conversation puts it"
        );
    }

    /// A node runs for weeks and a session ends. Every `/clear` opens a transcript under a new
    /// session id (#259), so a cache keyed on the path collects one entry per session the machine
    /// has ever had unless the round that stops naming one drops it.
    #[test]
    fn a_title_fold_for_a_transcript_no_pane_is_on_any_more_is_dropped() {
        let home = tempfile::tempdir().expect("a home");
        let project = home.path().join(".claude/projects/-home-u-demo");
        std::fs::create_dir_all(&project).expect("a project");
        let transcript = project.join("3c9e7a10-0000-4000-8000-0000000000f1.jsonl");
        std::fs::write(
            &transcript,
            "{\"type\":\"ai-title\",\"aiTitle\":\"the width inference rewrite\"}\n",
        )
        .expect("a transcript");
        let journals = kampr_journal::registry_from_home(home.path());

        let names = Names::default();
        let mut live = HashSet::new();
        for _ in 0..2 {
            assert_eq!(
                names
                    .title(&journals, Some("claude"), &transcript, None, &mut live)
                    .as_deref(),
                Some("the width inference rewrite")
            );
        }
        assert_eq!(
            names.folds.lock().unwrap().len(),
            1,
            "two looks at one transcript are one fold, or the cursor would restart every rebuild"
        );

        names.keep(&live);
        assert_eq!(names.folds.lock().unwrap().len(), 1);

        names.keep(&HashSet::new());
        assert_eq!(
            names.folds.lock().unwrap().len(),
            0,
            "a round that named no transcript at all leaves nothing behind"
        );
    }

    #[test]
    fn a_name_somebody_chose_is_still_what_the_pane_is_called() {
        assert_eq!(
            chosen_name(&named("the release", Some("user"))).as_deref(),
            Some("the release")
        );
    }
}
