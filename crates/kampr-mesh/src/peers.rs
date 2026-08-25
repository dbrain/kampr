use crate::handshake::Accepted;
use crate::shadow::{History, Shadow, StyleTable};
use crate::transport::{Incoming, Outgoing};
use base64::Engine;
use kampr_auth::Store;
use kampr_core::registry::PaneUpdate;
use kampr_core::scrollback::ScrollbackDoc;
use kampr_core::wire::{Cursor, ErrorCode, HerdDelta, NodeEntry, PaneEntry, RowRuns, Styles};
use kampr_term::RowDiff;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot, watch};
use tracing::{debug, info, warn};

/// Per-pane fan-out depth on the hub. Overflowing it costs a client one `grid.reset` out of the
/// hub's own shadow — never a stall, and never a round trip to the peer.
const PANE_FANOUT: usize = 64;

/// Requests the hub may have outstanding towards one peer. These are `watch`, `input` and
/// `manage` — keystrokes and structure, never frames — so the bound is generous and reaching it
/// means the link is wedged rather than busy.
const REQUEST_QUEUE: usize = 256;

const PING_INTERVAL: Duration = Duration::from_secs(5);
const MANAGE_TIMEOUT: Duration = Duration::from_secs(10);

/// How much of an attachment one `att.chunk` carries, decoded.
///
/// This is the unit of fairness at the WAN hop: a terminal frame queued while a transfer is in
/// progress waits for one chunk to be written, never for the record. The largest attachment ever
/// measured is 2.22 MB (#247), so that record is 36 chunks rather than one message that stops
/// every pane on the link for as long as the link takes to drain it.
pub const ATT_CHUNK_BYTES: usize = 64 * 1024;

/// Chunks a peer may have sent that the hub has not yet asked to replace.
///
/// The hub grants one back for each chunk it has handed to the client, so this is at once the
/// most the hub will ever hold for a transfer — 256 KiB, whatever the attachment's size — and the
/// in-flight window that keeps a WAN round trip from costing a chunk of throughput.
pub const ATT_WINDOW: u32 = 4;

/// Keepalive rounds a peer may leave unanswered before the link is dropped. At the default five
/// second interval that is fifteen seconds of silence: long enough to sit out a stalled writer or
/// a paused process, short enough that a client is told a node is gone rather than left reading a
/// frozen grid and an `rtt_ms` that stopped moving.
const MISSED_PONGS: u64 = 3;

#[derive(Debug, thiserror::Error)]
pub enum RelayError {
    #[error("no node in this herd owns {0}")]
    Unknown(String),
    #[error("{0}")]
    Offline(String),
    #[error("the link to that node is not accepting requests")]
    Wedged,
    #[error("that node did not answer")]
    NoAnswer,
}

impl RelayError {
    /// The wire's own error codes, so a relayed failure reads to a client exactly like a local one.
    pub fn code(&self) -> ErrorCode {
        match self {
            Self::Unknown(_) => ErrorCode::UnknownPane,
            Self::Offline(_) | Self::Wedged | Self::NoAnswer => ErrorCode::NodeOffline,
        }
    }
}

/// Why a relayed attachment did not arrive.
///
/// Nothing here is shown to a client as itself. The route that serves an attachment answers every
/// refusal the same way on purpose — an escape, a stale id and an id for somebody else's
/// transcript must not be distinguishable from outside — and a hop across the mesh must not be
/// the one that says which. This exists so the *log* can, and so the one refusal that is already
/// its own status locally (too large) still is.
#[derive(Debug, thiserror::Error)]
pub enum FetchError {
    #[error(transparent)]
    Link(#[from] RelayError),
    #[error("this attachment is larger than the node will serve")]
    TooLarge(u64),
    #[error("that node refused the attachment: {0}")]
    Refused(String),
    #[error("that node stopped sending the attachment")]
    Truncated,
}

/// What the peer says its attachment is. Every field is the peer's own claim and none of it
/// decides what this hub serves: the media type is run through the same allowlist a local one is,
/// and `bytes` is checked against the ceiling before a byte is pulled and again as they arrive.
#[derive(Debug, Clone, Default)]
pub struct AttHeader {
    pub kind: String,
    pub mime: Option<String>,
    pub name: Option<String>,
    pub bytes: u64,
}

#[derive(Debug)]
enum AttEvent {
    Open(AttHeader),
    Chunk(Vec<u8>),
    End,
    Refused(String),
}

#[derive(Debug, Clone)]
pub enum RemoteEvent {
    Update(PaneUpdate),
    Scrollback(ScrollbackDoc),
    /// Forwarded verbatim: a pending prompt is already addressed by a global pane id, so the hub
    /// has nothing to rewrite.
    Passthrough(Value),
    Error {
        code: String,
        message: String,
    },
}

/// Every node this hub reaches over a mesh link, live or remembered.
#[derive(Debug, Clone, Default)]
pub struct PeerHerd {
    pub nodes: Vec<NodeEntry>,
    pub panes: Vec<PaneEntry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PeerState {
    Live,
    Offline,
    Unknown,
}

#[derive(Debug, Clone)]
pub struct PeersConfig {
    pub ping_interval: Duration,
    pub pane_fanout: usize,
}

impl Default for PeersConfig {
    fn default() -> Self {
        Self {
            ping_interval: PING_INTERVAL,
            pane_fanout: PANE_FANOUT,
        }
    }
}

#[derive(Debug, Default)]
struct Remembered {
    nodes: Vec<NodeEntry>,
    panes: Vec<PaneEntry>,
    detail: String,
}

/// The hub's side of the mesh: the live links, and what it remembers of the ones that dropped.
///
/// A peer going away is one node in the herd going offline. It costs its own panes and nothing
/// else — every other link, and the hub's own sessions, are untouched by construction, because a
/// link owns nothing but its own tasks.
pub struct Peers {
    config: PeersConfig,
    links: Mutex<Vec<Arc<PeerLink>>>,
    remembered: Mutex<HashMap<String, Remembered>>,
    herd: watch::Sender<Arc<PeerHerd>>,
}

impl Peers {
    pub fn new(config: PeersConfig) -> Arc<Self> {
        let (herd, _) = watch::channel(Arc::new(PeerHerd::default()));
        Arc::new(Self {
            config,
            links: Mutex::new(Vec::new()),
            remembered: Mutex::new(HashMap::new()),
            herd,
        })
    }

    pub fn herd(&self) -> Arc<PeerHerd> {
        self.herd.borrow().clone()
    }

    pub fn subscribe(&self) -> watch::Receiver<Arc<PeerHerd>> {
        self.herd.subscribe()
    }

    pub fn links(&self) -> Vec<Arc<PeerLink>> {
        self.links.lock().unwrap().clone()
    }

    /// The node id behind a node id or a global pane id. A pane id is `<node_id>/<pane_id>` and a
    /// herdr pane id never contains a slash, so the first one separates them.
    fn node_of(id: &str) -> &str {
        id.split_once('/').map_or(id, |(node, _)| node)
    }

    /// The handshake is the only thing that says which link answers for a node id, so it is asked
    /// first and the peer's own advertisement is only the fallback.
    pub fn link_for(&self, id: &str) -> Option<Arc<PeerLink>> {
        let node = Self::node_of(id);
        let links = self.links.lock().unwrap();
        links
            .iter()
            .find(|link| link.node_id == node)
            .or_else(|| links.iter().find(|link| link.advertises(node)))
            .cloned()
    }

    pub fn state(&self, id: &str) -> PeerState {
        let node = Self::node_of(id);
        if self.link_for(node).is_some() {
            return PeerState::Live;
        }
        match self
            .remembered
            .lock()
            .unwrap()
            .values()
            .any(|r| r.nodes.iter().any(|n| n.id == node))
        {
            true => PeerState::Offline,
            false => PeerState::Unknown,
        }
    }

    /// Why a node is unreachable, in the words the log would use.
    pub fn detail(&self, id: &str) -> Option<String> {
        let node = Self::node_of(id);
        self.remembered
            .lock()
            .unwrap()
            .values()
            .find(|r| r.nodes.iter().any(|n| n.id == node))
            .map(|r| r.detail.clone())
    }

    pub fn watch(&self, pane: &str, conversation: bool) -> Result<RemoteWatcher, RelayError> {
        self.resolve(pane)?
            .watch(pane, conversation, self.config.pane_fanout)
    }

    /// Re-watching a relayed pane must not close it in between — the mesh twin of #252.
    ///
    /// A [`RemotePane`] is kept alive by its watchers and by nothing else, so a caller that stops
    /// its old watch before starting the new one *is* the last watcher: the hub's shadow of the
    /// pane, the history it has stitched and the upstream `watch` all go with it, and what the
    /// replacement gets is a freshly minted pane — a blank grid over content a viewer was already
    /// looking at, an empty history, and a second crossing of the WAN. Taken across the swap, a
    /// re-watch is a re-attach: the peer is asked nothing at all.
    ///
    /// The hold is a strong reference and nothing more. It sends no request, so it cannot become a
    /// second upstream watch, and it releases the pane exactly as a watcher does — a relayed pane
    /// nobody is watching and nobody is holding is still unwatched upstream.
    pub fn hold_while(&self, pane: &str, stop: impl FnOnce()) -> Option<PeerHold> {
        let hold = self.link_for(pane).and_then(|link| link.hold(pane));
        stop();
        hold
    }

    /// Keystrokes and answers: fire and forget, exactly as they are locally. The peer's own
    /// session decides whether they are allowed, and says so on its own error channel.
    pub fn relay(&self, addressed_to: &str, message: Value) -> Result<(), RelayError> {
        self.resolve(addressed_to)?.request(message)
    }

    pub async fn manage(&self, addressed_to: &str, message: Value) -> Result<Value, RelayError> {
        self.resolve(addressed_to)?.manage(message).await
    }

    /// Whether a pane's node can hand this hub the bytes behind an attachment id.
    ///
    /// False for a node that is offline, for one this hub has never met, and for one whose build
    /// predates `att.fetch` — three different reasons the button must be absent rather than
    /// present and answering 404. It is the peer's own `hello` that says so, which is the only
    /// claim about a build that arrives after the handshake and before the first pane.
    pub fn can_serve_attachments(&self, id: &str) -> bool {
        self.link_for(id).is_some_and(|link| link.serves_attachments())
    }

    /// Pulls one attachment off a peer, a chunk at a time.
    ///
    /// `ceiling` is the caller's own decoded-bytes limit, enforced *before* anything is pulled —
    /// the peer's header is a claim, so it is refused on the claim and then again on the arithmetic
    /// as chunks arrive, because a peer that lied about the size would otherwise stream for ever.
    pub async fn fetch_attachment(&self, pane: &str, id: &str, ceiling: u64) -> Result<Transfer, FetchError> {
        let link = self.resolve(pane)?;
        // The same fact the promise is keyed on, asked again at the moment it is relied on: a
        // build with no `att.fetch` would leave this request waiting out its deadline for an
        // answer that is never coming.
        if !link.serves_attachments() {
            return Err(FetchError::Refused("that node has no attachment route".into()));
        }
        link.fetch_attachment(pane, id, ceiling).await
    }

    fn resolve(&self, id: &str) -> Result<Arc<PeerLink>, RelayError> {
        if let Some(link) = self.link_for(id) {
            return Ok(link);
        }
        match self.state(id) {
            PeerState::Offline => {
                Err(RelayError::Offline(self.detail(id).unwrap_or_else(|| {
                    "that node is not connected to this hub".into()
                })))
            }
            _ => Err(RelayError::Unknown(id.to_string())),
        }
    }

    /// Ends every live link to a node, by key, id, name or fingerprint. Revoking a peer has to
    /// bite on the connection that is already open, not at the next handshake.
    pub fn disconnect(&self, needle: &str) -> bool {
        let mut ended = false;
        for link in self.links() {
            if link.pubkey == needle || link.node_id == needle || link.name == needle {
                link.close("revoked");
                ended = true;
            }
        }
        ended
    }

    /// Serves one authenticated mesh link until it ends. Every failure inside is this link's
    /// alone: it detaches, its nodes go offline, and the rest of the herd never notices.
    pub async fn serve<O: Outgoing, I: Incoming>(
        self: &Arc<Self>,
        accepted: Accepted,
        mut out: O,
        mut incoming: I,
    ) {
        let (requests, mut outbound) = mpsc::channel(REQUEST_QUEUE);
        let link = Arc::new(PeerLink {
            node_id: accepted.node.node_id.clone(),
            name: accepted.node.name.clone(),
            pubkey: accepted.node.pubkey.clone(),
            build: accepted.build.clone(),
            requests,
            state: Mutex::new(LinkState::default()),
            panes: Mutex::new(HashMap::new()),
            manages: Mutex::new(HashMap::new()),
            transfers: Mutex::new(HashMap::new()),
            next_request: AtomicU64::new(1),
            closed: Arc::new(tokio::sync::Notify::new()),
            closed_reason: Mutex::new(None),
            superseded: AtomicBool::new(false),
        });
        // A key is a node's identity, so a second link holding the same one is that node dialling
        // again before this hub noticed the first socket die. Two rows for one peer publish it
        // twice and route to whichever the scan reaches first, which may be the dead one.
        for stale in self.take_links(|other| other.pubkey == link.pubkey) {
            stale.superseded.store(true, Ordering::Relaxed);
            stale.close("the same node dialled again");
        }
        // Nothing after the handshake constrains what a peer says about itself, so the node id a
        // link answers for is the authenticated one and no two links may hold it. An enrolled but
        // hostile machine claiming another's id would otherwise be handed its watch, input and
        // manage traffic — one compromised host reading another's terminals through the hub that
        // exists to join them.
        if let Some(holder) = self.holder_of(&link.node_id) {
            warn!(
                node = %link.node_id, name = %link.name,
                fingerprint = %accepted.node.fingerprint(),
                held_by = %holder.name, "refused a mesh link claiming a node id another link holds",
            );
            out.close().await;
            return;
        }
        self.links.lock().unwrap().push(link.clone());
        self.evict_claims(&link);
        // Forget the old record by key *and* by node id: a node that regenerated its identity is
        // a new row here, and leaving the previous one would list it twice, once offline forever.
        self.remembered.lock().unwrap().retain(|key, remembered| {
            *key != link.pubkey && !remembered.nodes.iter().any(|n| n.id == link.node_id)
        });
        info!(node = %link.node_id, name = %link.name, "peer joined the herd");
        self.publish();

        let writer = tokio::spawn(async move {
            while let Some(text) = outbound.recv().await {
                if !out.send(text).await {
                    break;
                }
            }
            out.close().await;
        });
        let pinger = tokio::spawn(keepalive(
            link.clone(),
            self.config.ping_interval,
            accepted.store.clone(),
        ));

        let closed = link.closed.clone();
        let reason = loop {
            tokio::select! {
                text = incoming.recv() => {
                    let Some(text) = text else { break "the peer closed the link".to_string() };
                    self.receive(&link, &text);
                }
                () = closed.notified() => break link
                    .closed_reason
                    .lock()
                    .unwrap()
                    .clone()
                    .unwrap_or_else(|| "the link was closed by this hub".into()),
            }
        };

        pinger.abort();
        writer.abort();
        self.detach(&link, &reason);
    }

    fn detach(&self, link: &Arc<PeerLink>, reason: &str) {
        self.links
            .lock()
            .unwrap()
            .retain(|other| !Arc::ptr_eq(other, link));
        let state = link.state.lock().unwrap();
        let detail = format!("{} is not connected: {reason}", link.name);
        for pane in link.panes.lock().unwrap().values().filter_map(Weak::upgrade) {
            pane.fail("node_offline", &detail);
        }
        // A half-served attachment ends with the link rather than waiting out its own deadline:
        // the request behind it is a client holding a socket open for bytes that cannot arrive.
        link.transfers.lock().unwrap().clear();
        // A superseded link is the same node's previous socket, and its successor is already
        // serving: remembering it would list that node twice, once offline for ever.
        if !link.superseded.load(Ordering::Relaxed) {
            self.remembered.lock().unwrap().insert(
                link.pubkey.clone(),
                Remembered {
                    nodes: state.nodes.clone(),
                    panes: state.panes.clone(),
                    detail: detail.clone(),
                },
            );
        }
        drop(state);
        warn!(node = %link.node_id, name = %link.name, %reason, "peer left the herd");
        self.publish();
    }

    /// Everything a peer says that changes the herd, applied and republished; everything it says
    /// about a pane, handed to that pane's watchers.
    fn receive(&self, link: &Arc<PeerLink>, text: &str) {
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            debug!(node = %link.node_id, "a peer sent something that is not JSON");
            return;
        };
        match message["t"].as_str().unwrap_or_default() {
            "herd" => {
                let mut nodes: Vec<NodeEntry> = from_value(message.get("nodes"));
                let mut panes: Vec<PaneEntry> = from_value(message.get("panes"));
                self.keep_own(link, &mut nodes, &mut panes);
                let mut state = link.state.lock().unwrap();
                state.nodes = nodes;
                state.panes = panes;
                drop(state);
                self.publish();
            }
            "herd.patch" => {
                let mut added: HerdDelta = from_one(message.get("added"));
                let mut changed: HerdDelta = from_one(message.get("changed"));
                let removed: Vec<String> = from_value(message.get("removed_ids"));
                self.keep_own(link, &mut added.nodes, &mut added.panes);
                self.keep_own(link, &mut changed.nodes, &mut changed.panes);
                let mut state = link.state.lock().unwrap();
                state.apply(added, changed, &removed);
                drop(state);
                self.publish();
            }
            "styles" => {
                if let Ok(styles) = serde_json::from_value::<Styles>(message) {
                    let absorbed = link.state.lock().unwrap().styles.absorb(&styles);
                    if !absorbed {
                        warn!(
                            node = %link.node_id, from = styles.from,
                            "a peer's styles did not continue the table it was appending to",
                        );
                        link.close("a styles message skipped past the table it appends to");
                    }
                }
            }
            "grid.reset" => link.grid_reset(&message),
            "grid.patch" => link.grid_patch(&message),
            "scrollback" => link.scrollback(&message),
            "pending" | "convo" | "convo.turn" => link.passthrough(&message),
            "error" => link.error(&message),
            "managed" => link.managed(message),
            "att.open" | "att.chunk" | "att.end" | "att.error" => link.attachment(&message),
            // The one thing in `hello` the handshake did not already establish: whether this
            // peer's build answers `att.fetch`. A hub that guessed would advertise an attachment
            // button an older peer cannot serve, which is the bug this whole path exists to fix.
            "hello" => {
                let serves = message["caps"]["attachments"].as_bool().unwrap_or(false);
                link.state.lock().unwrap().attachments = serves;
            }
            // A fresh round trip is a change to the herd like any other: it is what a client
            // renders to say how far away a node is.
            "pong" => {
                let measured = link.pong(&message);
                if measured {
                    self.publish();
                }
            }
            // An unknown `t` is ignored rather than refused — the same forward-compatibility rule
            // as everywhere.
            _ => {}
        }
    }

    fn take_links(&self, doomed: impl Fn(&Arc<PeerLink>) -> bool) -> Vec<Arc<PeerLink>> {
        let mut links = self.links.lock().unwrap();
        let (taken, kept) = links.iter().cloned().partition(doomed);
        *links = kept;
        taken
    }

    fn holder_of(&self, node_id: &str) -> Option<Arc<PeerLink>> {
        self.links().into_iter().find(|link| link.node_id == node_id)
    }

    /// Drops everything a peer advertised that belongs to some *other* link's authenticated node.
    /// A `herd` message is the peer's own words about itself and nothing verifies them, so it may
    /// name any node it likes and only its own id is evidence.
    fn keep_own(&self, link: &Arc<PeerLink>, nodes: &mut Vec<NodeEntry>, panes: &mut Vec<PaneEntry>) {
        let claimed = |id: &str| {
            self.links()
                .iter()
                .any(|other| !Arc::ptr_eq(other, link) && other.node_id == id)
        };
        nodes.retain(|node| {
            let mine = !claimed(&node.id);
            if !mine {
                warn!(
                    node = %link.node_id, name = %link.name, claimed = %node.id,
                    "a peer advertised a node another link authenticated as; dropping it",
                );
            }
            mine
        });
        panes.retain(|pane| !claimed(&pane.node_id) && !claimed(Self::node_of(&pane.id)));
    }

    /// A link that authenticates as a node id takes it back from anything that had merely claimed
    /// it, so the order two peers connected in cannot decide which one a client reaches.
    fn evict_claims(&self, link: &Arc<PeerLink>) {
        for other in self.links() {
            if Arc::ptr_eq(&other, link) {
                continue;
            }
            let mut state = other.state.lock().unwrap();
            let before = state.nodes.len() + state.panes.len();
            state.nodes.retain(|node| node.id != link.node_id);
            state
                .panes
                .retain(|pane| pane.node_id != link.node_id && Self::node_of(&pane.id) != link.node_id);
            let dropped = before - (state.nodes.len() + state.panes.len());
            drop(state);
            if dropped > 0 {
                warn!(
                    node = %other.node_id, name = %other.name, claimed = %link.node_id,
                    dropped, "a peer had advertised a node that has now authenticated elsewhere",
                );
            }
        }
    }

    /// The merged view: every live link's nodes marked `peer` and stamped with the link's own
    /// round trip, plus the nodes of every link that dropped, marked offline with a reason.
    ///
    /// A dropped peer's panes stay listed on purpose. Dropping them empties a node out of the UI
    /// at the moment the user most needs to see that it exists and is unreachable.
    fn publish(&self) {
        let mut herd = PeerHerd::default();
        for link in self.links() {
            let state = link.state.lock().unwrap();
            for node in &state.nodes {
                herd.nodes.push(NodeEntry {
                    kind: "peer".into(),
                    rtt_ms: state.rtt_ms.map(|link_rtt| link_rtt + node.rtt_ms.unwrap_or(0.0)),
                    build: node.build.clone().or_else(|| Some(link.build.clone())),
                    ..node.clone()
                });
            }
            herd.panes.extend(state.panes.iter().cloned());
        }
        for remembered in self.remembered.lock().unwrap().values() {
            for node in &remembered.nodes {
                herd.nodes.push(NodeEntry {
                    kind: "peer".into(),
                    online: false,
                    rtt_ms: None,
                    detail: Some(remembered.detail.clone()),
                    ..node.clone()
                });
            }
            herd.panes.extend(remembered.panes.iter().cloned());
        }
        self.herd.send_replace(Arc::new(herd));
    }
}

fn from_value<T: serde::de::DeserializeOwned + Default>(value: Option<&Value>) -> T {
    value
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn from_one(value: Option<&Value>) -> HerdDelta {
    from_value(value)
}

#[derive(Debug, Default)]
struct LinkState {
    styles: StyleTable,
    nodes: Vec<NodeEntry>,
    panes: Vec<PaneEntry>,
    rtt_ms: Option<f64>,
    pings: HashMap<u64, Instant>,
    attachments: bool,
}

impl LinkState {
    fn apply(&mut self, added: HerdDelta, changed: HerdDelta, removed: &[String]) {
        for node in added.nodes.into_iter().chain(changed.nodes) {
            match self.nodes.iter_mut().find(|n| n.id == node.id) {
                Some(existing) => *existing = node,
                None => self.nodes.push(node),
            }
        }
        for pane in added.panes.into_iter().chain(changed.panes) {
            match self.panes.iter_mut().find(|p| p.id == pane.id) {
                Some(existing) => *existing = pane,
                None => self.panes.push(pane),
            }
        }
        // A removal names a pane or a node, and a client drops it from whichever list holds it.
        self.panes.retain(|p| !removed.contains(&p.id));
        self.nodes.retain(|n| !removed.contains(&n.id));
    }
}

/// One authenticated peer, and the panes the hub is currently relaying from it.
pub struct PeerLink {
    pub node_id: String,
    pub name: String,
    pub pubkey: String,
    pub build: String,
    requests: mpsc::Sender<String>,
    state: Mutex<LinkState>,
    panes: Mutex<HashMap<String, Weak<RemotePane>>>,
    manages: Mutex<HashMap<u64, oneshot::Sender<Value>>>,
    transfers: Mutex<HashMap<u64, mpsc::Sender<AttEvent>>>,
    next_request: AtomicU64,
    closed: Arc<tokio::sync::Notify>,
    closed_reason: Mutex<Option<String>>,
    superseded: AtomicBool,
}

impl PeerLink {
    pub fn nodes(&self) -> Vec<NodeEntry> {
        self.state.lock().unwrap().nodes.clone()
    }

    pub fn rtt_ms(&self) -> Option<f64> {
        self.state.lock().unwrap().rtt_ms
    }

    /// What the peer says it holds, which is evidence about nothing until it has been checked
    /// against what the links around it authenticated as. Its own [`Self::node_id`] is the part
    /// the handshake proved, and a client may address that before the first `herd` message.
    fn advertises(&self, node_id: &str) -> bool {
        self.state.lock().unwrap().nodes.iter().any(|n| n.id == node_id)
    }

    /// The reason reaches a client: it is what the herd shows beside a node that has gone, and
    /// "revoked" or "it stopped answering keepalives" is the whole of what an operator can act on.
    pub fn close(&self, reason: &str) {
        debug!(node = %self.node_id, %reason, "closing a mesh link");
        self.closed_reason
            .lock()
            .unwrap()
            .get_or_insert_with(|| reason.to_string());
        // `notify_one` leaves a permit behind, and that is the whole point: the serving loop only
        // waits on this between messages, so a `notify_waiters` fired while it was busy would be
        // dropped on the floor — and a revocation that is dropped on the floor is a peer that
        // keeps typing into terminals it has been cut off from.
        self.closed.notify_one();
    }

    fn request(&self, message: Value) -> Result<(), RelayError> {
        self.requests
            .try_send(message.to_string())
            .map_err(|_| RelayError::Wedged)
    }

    async fn manage(&self, mut message: Value) -> Result<Value, RelayError> {
        let id = self.next_request.fetch_add(1, Ordering::Relaxed);
        message["rid"] = json!(id);
        let (tx, rx) = oneshot::channel();
        self.manages.lock().unwrap().insert(id, tx);
        if let Err(e) = self.request(message) {
            self.manages.lock().unwrap().remove(&id);
            return Err(e);
        }
        match tokio::time::timeout(MANAGE_TIMEOUT, rx).await {
            Ok(Ok(reply)) => Ok(reply),
            _ => {
                self.manages.lock().unwrap().remove(&id);
                Err(RelayError::NoAnswer)
            }
        }
    }

    fn managed(&self, message: Value) {
        let Some(id) = message["rid"].as_u64() else {
            debug!(node = %self.node_id, "a peer answered a manage op without echoing its rid");
            return;
        };
        if let Some(waiter) = self.manages.lock().unwrap().remove(&id) {
            let _ = waiter.send(message);
        }
    }

    fn serves_attachments(&self) -> bool {
        self.state.lock().unwrap().attachments
    }

    async fn fetch_attachment(
        self: &Arc<Self>,
        pane: &str,
        id: &str,
        ceiling: u64,
    ) -> Result<Transfer, FetchError> {
        let rid = self.next_request.fetch_add(1, Ordering::Relaxed);
        // One slot per chunk the peer is allowed to have in flight, plus the two events that are
        // not chunks. A peer that fills it has sent more than it was granted, which is the one
        // way this queue can grow and the reason the transfer ends rather than buffering.
        let (tx, rx) = mpsc::channel(ATT_WINDOW as usize + 2);
        self.transfers.lock().unwrap().insert(rid, tx);
        let asked = self.request(json!({
            "t": "att.fetch", "rid": rid, "pane": pane, "id": id, "window": ATT_WINDOW
        }));
        if let Err(e) = asked {
            self.transfers.lock().unwrap().remove(&rid);
            return Err(e.into());
        }
        let mut transfer = Transfer {
            link: self.clone(),
            rid,
            header: AttHeader::default(),
            rx,
            remaining: 0,
            done: false,
        };
        match tokio::time::timeout(MANAGE_TIMEOUT, transfer.rx.recv()).await {
            Ok(Some(AttEvent::Open(header))) => {
                // The ceiling is checked on the peer's claim, before a chunk is asked for, so a
                // record naming a gigabyte costs a comparison here exactly as it does locally.
                if header.bytes > ceiling {
                    return Err(FetchError::TooLarge(header.bytes));
                }
                transfer.remaining = header.bytes;
                transfer.header = header;
                Ok(transfer)
            }
            Ok(Some(AttEvent::Refused(code))) => Err(FetchError::Refused(code)),
            Ok(Some(_)) => Err(FetchError::Refused("out of order".into())),
            Ok(None) => Err(RelayError::Offline("that node left the herd".into()).into()),
            Err(_) => Err(RelayError::NoAnswer.into()),
        }
    }

    fn attachment(&self, message: &Value) {
        let Some(rid) = message["rid"].as_u64() else {
            debug!(node = %self.node_id, "a peer sent an attachment frame without an rid");
            return;
        };
        let event = match message["t"].as_str().unwrap_or_default() {
            "att.open" => AttEvent::Open(AttHeader {
                kind: message["kind"].as_str().unwrap_or_default().to_string(),
                mime: message["mime"].as_str().map(str::to_string),
                name: message["name"].as_str().map(str::to_string),
                bytes: message["bytes"].as_u64().unwrap_or_default(),
            }),
            "att.chunk" => match message["b64"]
                .as_str()
                .and_then(|b64| base64::engine::general_purpose::STANDARD.decode(b64).ok())
            {
                Some(data) => AttEvent::Chunk(data),
                None => AttEvent::Refused("unreadable chunk".into()),
            },
            "att.end" => AttEvent::End,
            _ => AttEvent::Refused(message["code"].as_str().unwrap_or("refused").to_string()),
        };
        let sink = self.transfers.lock().unwrap().get(&rid).cloned();
        // A transfer nobody is waiting for is one the client walked away from, and the `att.stop`
        // that said so is still crossing the link — dropping what is already in flight is what
        // that costs, and it is not an error.
        let Some(sink) = sink else { return };
        if sink.try_send(event).is_err() {
            debug!(node = %self.node_id, rid, "a peer overran its attachment window");
            self.transfers.lock().unwrap().remove(&rid);
        }
    }

    /// Stops caring about a transfer, and tells the peer so unless it has already finished.
    fn end_transfer(&self, rid: u64, tell_the_peer: bool) {
        self.transfers.lock().unwrap().remove(&rid);
        if tell_the_peer {
            let _ = self.request(json!({ "t": "att.stop", "rid": rid }));
        }
    }

    fn watch(
        self: &Arc<Self>,
        pane: &str,
        conversation: bool,
        fanout: usize,
    ) -> Result<RemoteWatcher, RelayError> {
        let mut panes = self.panes.lock().unwrap();
        if let Some(existing) = panes.get(pane).and_then(Weak::upgrade) {
            let upgrade = conversation && !existing.conversation.load(Ordering::Relaxed);
            drop(panes);
            // One `watch` per pane per link is what keeps the WAN hop carrying one copy, so the
            // first viewer decides what the peer sends. A later one asking for the transcript has
            // to say so, or it lands on the agent pane's default surface with nothing in it.
            // Recorded only once the ask is away, because a request that was never sent has bought
            // this pane nothing.
            if upgrade {
                self.request(json!({
                    "t": "watch", "pane": pane, "scrollback": true, "conversation": true
                }))?;
                existing.conversation.store(true, Ordering::Relaxed);
            }
            return Ok(RemoteWatcher::attach(existing));
        }
        let (tx, _) = broadcast::channel(fanout);
        let remote = Arc::new(RemotePane {
            pane: pane.to_string(),
            conversation: AtomicBool::new(conversation),
            shadow: Mutex::new(Shadow::default()),
            history: Mutex::new(History::default()),
            tx,
            requests: self.requests.clone(),
        });
        panes.insert(pane.to_string(), Arc::downgrade(&remote));
        drop(panes);
        self.request(json!({
            "t": "watch", "pane": pane, "scrollback": true, "conversation": conversation
        }))?;
        Ok(RemoteWatcher::attach(remote))
    }

    fn hold(&self, pane: &str) -> Option<PeerHold> {
        self.panes
            .lock()
            .unwrap()
            .get(pane)
            .and_then(Weak::upgrade)
            .map(|pane| PeerHold { _pane: pane })
    }

    fn pane(&self, message: &Value) -> Option<Arc<RemotePane>> {
        let id = message["pane"].as_str()?;
        self.panes.lock().unwrap().get(id).and_then(Weak::upgrade)
    }

    fn grid_reset(&self, message: &Value) {
        let Some(pane) = self.pane(message) else { return };
        let cols = message["cols"].as_u64().unwrap_or_default() as u16;
        let rows = message["rows"].as_u64().unwrap_or_default() as u16;
        let rows_data: Vec<RowRuns> = from_value(message.get("rows_data"));
        let links: Vec<String> = from_value(message.get("links"));
        let styles = self.state.lock().unwrap();
        let update =
            pane.shadow
                .lock()
                .unwrap()
                .reset(cols, rows, &rows_data, cursor(message), links, &styles.styles);
        drop(styles);
        pane.emit(RemoteEvent::Update(update));
    }

    fn grid_patch(&self, message: &Value) {
        let Some(pane) = self.pane(message) else { return };
        let rows: Vec<RowRuns> = from_value(message.get("rows"));
        let links: Vec<String> = from_value(message.get("links"));
        let styles = self.state.lock().unwrap();
        let update = pane
            .shadow
            .lock()
            .unwrap()
            .patch(&rows, cursor(message), links, &styles.styles);
        drop(styles);
        if let Some(update) = update {
            pane.emit(RemoteEvent::Update(update));
        }
    }

    fn scrollback(&self, message: &Value) {
        let Some(pane) = self.pane(message) else { return };
        let rows: Vec<RowRuns> = from_value(message.get("rows"));
        let (cols, _) = pane.geometry();
        let styles = self.state.lock().unwrap();
        let doc = ScrollbackDoc {
            from_top: message["from_top"].as_u64().unwrap_or_default() as u32,
            rows: rows
                .iter()
                .map(|row| RowDiff {
                    row: row.row,
                    cells: crate::shadow::decode_row(&row.runs, &styles.styles, cols),
                })
                .collect(),
            total_rows: message["total_rows"].as_u64().unwrap_or_default() as u32,
            complete: message["complete"].as_bool().unwrap_or(true),
            capped: message["capped"].as_bool().unwrap_or(false),
        };
        drop(styles);
        let mut history = pane.history.lock().unwrap();
        let before = history.end();
        history.absorb(&doc);
        let delta = history.since(before);
        drop(history);
        if let Some(delta) = delta {
            pane.emit(RemoteEvent::Scrollback(delta));
        }
    }

    fn passthrough(&self, message: &Value) {
        if let Some(pane) = self.pane(message) {
            pane.emit(RemoteEvent::Passthrough(message.clone()));
        }
    }

    fn error(&self, message: &Value) {
        let code = message["code"].as_str().unwrap_or("error").to_string();
        let text = message["message"].as_str().unwrap_or_default().to_string();
        match self.pane(message) {
            Some(pane) => pane.emit(RemoteEvent::Error { code, message: text }),
            None => debug!(node = %self.node_id, %code, %text, "a peer reported an error"),
        }
    }

    fn pong(&self, message: &Value) -> bool {
        let Some(n) = message["n"].as_u64() else {
            return false;
        };
        let mut state = self.state.lock().unwrap();
        match state.pings.remove(&n) {
            Some(sent) => {
                state.rtt_ms = Some(sent.elapsed().as_secs_f64() * 1000.0);
                true
            }
            None => false,
        }
    }
}

fn cursor(message: &Value) -> Cursor {
    message
        .get("cursor")
        .cloned()
        .and_then(|c| serde_json::from_value(c).ok())
        .unwrap_or_default()
}

/// The link's own round trip, measured over the same socket the frames use, so a client's
/// `rtt_ms` for a peer is the number that actually explains why its pane feels slower — and the
/// tick that notices a peer has stopped answering, or has been revoked since it dialled in.
async fn keepalive(link: Arc<PeerLink>, interval: Duration, store: Store) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut n = 0u64;
    let mut wedged = 0u64;
    loop {
        ticker.tick().await;
        if revoked(&store, &link.pubkey).await {
            link.close("revoked");
            return;
        }
        n += 1;
        let silent = {
            let mut state = link.state.lock().unwrap();
            // An answer that never came is not a measurement; keeping one past the deadline it is
            // evidence for would only grow the map.
            state.pings.retain(|id, _| n.saturating_sub(*id) <= MISSED_PONGS);
            state.pings.keys().any(|id| n.saturating_sub(*id) >= MISSED_PONGS)
        };
        if silent {
            warn!(node = %link.node_id, name = %link.name, "a peer stopped answering keepalives");
            link.close("it stopped answering keepalives");
            return;
        }
        match link.requests.try_send(json!({ "t": "ping", "n": n }).to_string()) {
            // Only a ping that left the hub is evidence about the peer, so only that one is held
            // against the deadline above.
            Ok(()) => {
                wedged = 0;
                link.state.lock().unwrap().pings.insert(n, Instant::now());
            }
            // A full queue is a moment of congestion on a link that is otherwise alive, and
            // ending the task here froze `rtt_ms` at its last reading for the life of the link —
            // which reads to a client as a node that is nearby and merely slow. A queue that
            // stays full for as long as the pong deadline is the wedged link its bound names.
            Err(mpsc::error::TrySendError::Full(_)) => {
                wedged += 1;
                debug!(node = %link.node_id, wedged, "the outbound queue to a peer is full");
                if wedged >= MISSED_PONGS {
                    warn!(node = %link.node_id, name = %link.name, "a peer's outbound queue never drained");
                    link.close("its outbound queue never drained");
                    return;
                }
            }
            Err(mpsc::error::TrySendError::Closed(_)) => return,
        }
    }
}

/// Whether this hub still serves that key. Asked on every tick because the operator's `kampr mesh
/// revoke` runs in a different process and only writes SQLite: nothing tells a live link.
///
/// A row that is *gone* counts as revoked — `kampr mesh forget` is the same decision spelled
/// differently — but a database that cannot be read counts as nothing, because dropping every
/// mesh link on a transient sqlite error is worse than asking again on the next tick.
async fn revoked(store: &Store, pubkey: &str) -> bool {
    match store.mesh().node(pubkey).await {
        Ok(node) => !node.is_some_and(|node| node.active()),
        Err(e) => {
            warn!(error = %e, "could not check whether a peer is still enrolled");
            false
        }
    }
}

/// One attachment being pulled off a peer, a chunk at a time.
///
/// **Nothing here ever holds the whole record.** The hub asks for [`ATT_WINDOW`] chunks, and
/// grants one more for each chunk it has handed downstream — so the memory it spends is the
/// window, whether the attachment is 68 bytes or the 2.22 MB largest measured (#247), and the
/// rate it pulls at is the rate the client is reading at.
///
/// Dropping it is the cancellation: the client that walked away is the last thing keeping this
/// alive, and the peer is told to stop rather than left streaming into a hub that will discard it.
pub struct Transfer {
    link: Arc<PeerLink>,
    rid: u64,
    header: AttHeader,
    rx: mpsc::Receiver<AttEvent>,
    remaining: u64,
    done: bool,
}

impl Transfer {
    pub fn header(&self) -> &AttHeader {
        &self.header
    }

    /// The next chunk, `None` at a clean end. A peer that stalls ends the transfer rather than
    /// hanging the request, on the same bound a manage op waits on.
    pub async fn next_chunk(&mut self) -> Option<Result<Vec<u8>, FetchError>> {
        if self.done {
            return None;
        }
        let event = match tokio::time::timeout(MANAGE_TIMEOUT, self.rx.recv()).await {
            Ok(Some(event)) => event,
            Ok(None) => return Some(self.fail(FetchError::Truncated)),
            Err(_) => return Some(self.fail(RelayError::NoAnswer.into())),
        };
        match event {
            AttEvent::Chunk(data) => {
                // The peer's header was a claim and this is the arithmetic. A peer that keeps
                // sending past the size it announced is one this hub stops reading from.
                let Some(left) = self.remaining.checked_sub(data.len() as u64) else {
                    return Some(self.fail(FetchError::Refused("more bytes than it announced".into())));
                };
                self.remaining = left;
                if let Err(e) = self
                    .link
                    .request(json!({ "t": "att.more", "rid": self.rid, "n": 1 }))
                {
                    return Some(self.fail(e.into()));
                }
                Some(Ok(data))
            }
            AttEvent::End => {
                self.done = true;
                self.link.end_transfer(self.rid, false);
                match self.remaining {
                    0 => None,
                    _ => Some(Err(FetchError::Truncated)),
                }
            }
            AttEvent::Refused(code) => Some(self.fail(FetchError::Refused(code))),
            AttEvent::Open(_) => Some(self.fail(FetchError::Refused("out of order".into()))),
        }
    }

    fn fail(&mut self, e: FetchError) -> Result<Vec<u8>, FetchError> {
        self.done = true;
        self.link.end_transfer(self.rid, true);
        Err(e)
    }
}

impl Drop for Transfer {
    fn drop(&mut self) {
        if !self.done {
            self.link.end_transfer(self.rid, true);
        }
    }
}

/// A relayed pane kept open while one watcher is handed over to the next. See
/// [`Peers::hold_while`].
#[derive(Debug)]
pub struct PeerHold {
    _pane: Arc<RemotePane>,
}

#[derive(Debug)]
struct RemotePane {
    pane: String,
    conversation: AtomicBool,
    shadow: Mutex<Shadow>,
    history: Mutex<History>,
    tx: broadcast::Sender<RemoteEvent>,
    requests: mpsc::Sender<String>,
}

impl RemotePane {
    fn emit(&self, event: RemoteEvent) {
        let _ = self.tx.send(event);
    }

    fn fail(&self, code: &str, message: &str) {
        self.emit(RemoteEvent::Error {
            code: code.to_string(),
            message: message.to_string(),
        });
    }

    fn geometry(&self) -> (u16, u16) {
        self.shadow.lock().unwrap().geometry()
    }
}

/// The last watcher going away is what stops the peer streaming this pane. One `watch` per pane
/// per link, however many clients are looking at it — the WAN hop carries one copy.
impl Drop for RemotePane {
    fn drop(&mut self) {
        let _ = self
            .requests
            .try_send(json!({ "t": "unwatch", "pane": self.pane }).to_string());
    }
}

#[derive(Debug)]
pub struct RemoteWatcher {
    pane: Arc<RemotePane>,
    rx: broadcast::Receiver<RemoteEvent>,
    initial: Vec<RemoteEvent>,
    ready: bool,
    sent_history: u32,
}

impl RemoteWatcher {
    fn attach(pane: Arc<RemotePane>) -> Self {
        // Subscribing under the shadow lock is what stops the grid a joiner is handed and the
        // stream that follows it from interleaving.
        let shadow = pane.shadow.lock().unwrap();
        let rx = pane.tx.subscribe();
        let ready = shadow.is_ready();
        let mut initial = Vec::new();
        if ready {
            initial.push(RemoteEvent::Update(shadow.full()));
        }
        drop(shadow);
        let history = pane.history.lock().unwrap();
        let sent_history = history.end();
        if !history.is_empty() {
            initial.push(RemoteEvent::Scrollback(history.doc()));
        }
        drop(history);
        Self {
            pane,
            rx,
            initial,
            ready,
            sent_history,
        }
    }

    /// What this pane looked like at the moment of joining. Empty when the hub has not yet been
    /// sent a frame for it, which is the one case a client waits for.
    pub fn initial(&mut self) -> Vec<RemoteEvent> {
        std::mem::take(&mut self.initial)
    }

    pub fn is_ready(&self) -> bool {
        self.ready
    }

    pub fn history_sent(&self) -> u32 {
        self.sent_history
    }

    /// A watcher that falls behind is caught up from the hub's shadow — one full grid, out of
    /// memory, with no round trip to the peer.
    pub async fn recv(&mut self) -> Option<RemoteEvent> {
        loop {
            match self.rx.recv().await {
                Ok(event) => {
                    self.ready |= matches!(&event, RemoteEvent::Update(u) if u.is_reset());
                    return Some(event);
                }
                Err(broadcast::error::RecvError::Lagged(dropped)) => {
                    debug!(pane = %self.pane.pane, dropped, "hub fan-out lagged; resetting from the shadow");
                    let shadow = self.pane.shadow.lock().unwrap();
                    if shadow.is_ready() {
                        return Some(RemoteEvent::Update(shadow.full()));
                    }
                }
                Err(broadcast::error::RecvError::Closed) => return None,
            }
        }
    }

    /// The full grid the hub holds, for a client whose queue had to be purged.
    pub fn resync(&self) -> Option<PaneUpdate> {
        let shadow = self.pane.shadow.lock().unwrap();
        shadow.is_ready().then(|| shadow.full())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_auth::MeshRole;

    const KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    async fn enrolled() -> Store {
        let store = Store::open_memory().await.expect("a store");
        store
            .mesh()
            .enrol(KEY, "01JA", "laptop", MeshRole::Peer, None, kampr_auth::now())
            .await
            .expect("an enrolment");
        store
    }

    fn link(requests: mpsc::Sender<String>) -> Arc<PeerLink> {
        Arc::new(PeerLink {
            node_id: "01JA".into(),
            name: "laptop".into(),
            pubkey: KEY.into(),
            build: "0.1.0".into(),
            requests,
            state: Mutex::new(LinkState::default()),
            panes: Mutex::new(HashMap::new()),
            manages: Mutex::new(HashMap::new()),
            transfers: Mutex::new(HashMap::new()),
            next_request: AtomicU64::new(1),
            closed: Arc::new(tokio::sync::Notify::new()),
            closed_reason: Mutex::new(None),
            superseded: AtomicBool::new(false),
        })
    }

    /// The waiter is registered before the request is sent, so the send failing has to unregister
    /// it. It used to return through `?`, and every manage op against a congested link left a
    /// sender behind that nothing would ever remove.
    #[tokio::test]
    async fn a_manage_op_that_never_left_the_hub_leaves_no_waiter_behind() {
        let (requests, rx) = mpsc::channel(1);
        drop(rx);
        let link = link(requests);

        let error = link
            .manage(json!({ "t": "manage", "op": "pane.close" }))
            .await
            .expect_err("nothing can be sent on a closed link");
        assert!(matches!(error, RelayError::Wedged), "{error}");
        assert!(
            link.manages.lock().unwrap().is_empty(),
            "a manage op that was never sent is still waiting for an answer",
        );
    }

    #[tokio::test]
    async fn one_full_outbound_queue_does_not_end_the_keepalives() {
        let store = enrolled().await;
        let (requests, mut rx) = mpsc::channel(2);
        requests.try_send("a queued watch".into()).expect("room");
        requests.try_send("a queued input".into()).expect("room");
        let link = link(requests);

        // The first tick is due immediately and finds nowhere to put its ping.
        let task = tokio::spawn(keepalive(link.clone(), Duration::from_millis(500), store));
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert_eq!(rx.recv().await.expect("the queue"), "a queued watch");
        assert_eq!(rx.recv().await.expect("the queue"), "a queued input");

        let text = tokio::time::timeout(Duration::from_secs(2), rx.recv())
            .await
            .expect("the keepalives ended at the first full queue")
            .expect("a ping");
        assert_eq!(serde_json::from_str::<Value>(&text).expect("json")["t"], "ping",);
        task.abort();
    }
}
