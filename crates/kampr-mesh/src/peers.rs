use crate::handshake::Accepted;
use crate::shadow::{History, Shadow, StyleTable};
use crate::transport::{Incoming, Outgoing};
use kampr_core::registry::PaneUpdate;
use kampr_core::scrollback::ScrollbackDoc;
use kampr_core::wire::{Cursor, ErrorCode, HerdDelta, NodeEntry, PaneEntry, RowRuns, Styles};
use kampr_term::RowDiff;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
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

    pub fn link_for(&self, id: &str) -> Option<Arc<PeerLink>> {
        let node = Self::node_of(id);
        self.links
            .lock()
            .unwrap()
            .iter()
            .find(|link| link.serves(node))
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

    /// Keystrokes and answers: fire and forget, exactly as they are locally. The peer's own
    /// session decides whether they are allowed, and says so on its own error channel.
    pub fn relay(&self, addressed_to: &str, message: Value) -> Result<(), RelayError> {
        self.resolve(addressed_to)?.request(message)
    }

    pub async fn manage(&self, addressed_to: &str, message: Value) -> Result<Value, RelayError> {
        self.resolve(addressed_to)?.manage(message).await
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
            next_request: AtomicU64::new(1),
            closed: Arc::new(tokio::sync::Notify::new()),
        });
        self.links.lock().unwrap().push(link.clone());
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
        let pinger = tokio::spawn(ping(link.clone(), self.config.ping_interval));

        let closed = link.closed.clone();
        let reason = loop {
            tokio::select! {
                text = incoming.recv() => {
                    let Some(text) = text else { break "the peer closed the link".to_string() };
                    self.receive(&link, &text);
                }
                () = closed.notified() => break "the link was closed by this hub".to_string(),
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
        self.remembered.lock().unwrap().insert(
            link.pubkey.clone(),
            Remembered {
                nodes: state.nodes.clone(),
                panes: state.panes.clone(),
                detail: detail.clone(),
            },
        );
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
                let mut state = link.state.lock().unwrap();
                state.nodes = from_value(message.get("nodes"));
                state.panes = from_value(message.get("panes"));
                drop(state);
                self.publish();
            }
            "herd.patch" => {
                let added: HerdDelta = from_one(message.get("added"));
                let changed: HerdDelta = from_one(message.get("changed"));
                let removed: Vec<String> = from_value(message.get("removed_ids"));
                let mut state = link.state.lock().unwrap();
                state.apply(added, changed, &removed);
                drop(state);
                self.publish();
            }
            "styles" => {
                if let Ok(styles) = serde_json::from_value::<Styles>(message) {
                    link.state.lock().unwrap().styles.absorb(&styles);
                }
            }
            "grid.reset" => link.grid_reset(&message),
            "grid.patch" => link.grid_patch(&message),
            "scrollback" => link.scrollback(&message),
            "pending" | "convo" | "convo.turn" => link.passthrough(&message),
            "error" => link.error(&message),
            "managed" => link.managed(message),
            // A fresh round trip is a change to the herd like any other: it is what a client
            // renders to say how far away a node is.
            "pong" => {
                let measured = link.pong(&message);
                if measured {
                    self.publish();
                }
            }
            // `hello` carries nothing the handshake did not already establish, and an unknown `t`
            // is ignored rather than refused — the same forward-compatibility rule as everywhere.
            _ => {}
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
    next_request: AtomicU64,
    closed: Arc<tokio::sync::Notify>,
}

impl PeerLink {
    pub fn nodes(&self) -> Vec<NodeEntry> {
        self.state.lock().unwrap().nodes.clone()
    }

    pub fn rtt_ms(&self) -> Option<f64> {
        self.state.lock().unwrap().rtt_ms
    }

    pub fn serves(&self, node_id: &str) -> bool {
        let state = self.state.lock().unwrap();
        // Before the peer's first `herd` message the handshake id is all there is, and a client
        // may address it in that window.
        self.node_id == node_id || state.nodes.iter().any(|n| n.id == node_id)
    }

    pub fn close(&self, reason: &str) {
        debug!(node = %self.node_id, %reason, "closing a mesh link");
        self.closed.notify_waiters();
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
        self.request(message)?;
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

    fn watch(
        self: &Arc<Self>,
        pane: &str,
        conversation: bool,
        fanout: usize,
    ) -> Result<RemoteWatcher, RelayError> {
        let mut panes = self.panes.lock().unwrap();
        if let Some(existing) = panes.get(pane).and_then(Weak::upgrade) {
            return Ok(RemoteWatcher::attach(existing));
        }
        let (tx, _) = broadcast::channel(fanout);
        let remote = Arc::new(RemotePane {
            pane: pane.to_string(),
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
/// `rtt_ms` for a peer is the number that actually explains why its pane feels slower.
async fn ping(link: Arc<PeerLink>, interval: Duration) {
    let mut ticker = tokio::time::interval(interval);
    ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut n = 0u64;
    loop {
        ticker.tick().await;
        n += 1;
        {
            let mut state = link.state.lock().unwrap();
            state.pings.insert(n, Instant::now());
            // An answer that never came is not a measurement; keeping them would grow the map.
            state.pings.retain(|id, _| n.saturating_sub(*id) < 4);
        }
        if link.request(json!({ "t": "ping", "n": n })).is_err() {
            return;
        }
    }
}

#[derive(Debug)]
struct RemotePane {
    pane: String,
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
