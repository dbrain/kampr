use crate::dial::{self, Socket};
use crate::frames::{ConvoPage, Event, Failure, Hello, Managed, NodeCaps, Pending, Role};
use crate::herd::Herd;
use crate::pane::PaneState;
use crate::resolve::Session;
use futures_util::{SinkExt, StreamExt};
use kampr_core::Backoff;
use kampr_core::scrollback::ScrollbackDoc;
use kampr_core::wire::{Cursor, HerdDelta, NodeEntry, PaneEntry, RowRuns, Styles};
use kampr_mesh::shadow::{StyleTable, decode_row};
use kampr_term::RowDiff;
use serde_json::{Value, json};
use std::collections::{BTreeMap, HashMap};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::{Duration, Instant};
use tokio::sync::{broadcast, mpsc, oneshot};
use tokio_tungstenite::tungstenite::Message;
use tracing::{debug, warn};

/// How long a connection must stand before it counts as a success worth resetting the backoff
/// for. A socket that closed as soon as it opened is a refusal wearing a connection's clothes.
const SETTLED_AFTER: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct Policy {
    pub backoff: Backoff,
    pub connect_timeout: Duration,
    pub manage_timeout: Duration,
    /// How many events may go un-read before the slowest consumer is told it lagged. A consumer
    /// that lags redraws from [`State`], which is authoritative — the same repair the protocol's
    /// own backpressure rule uses.
    pub event_capacity: usize,
}

impl Default for Policy {
    fn default() -> Self {
        Self {
            backoff: Backoff {
                initial: Duration::from_millis(250),
                max: Duration::from_secs(10),
            },
            connect_timeout: Duration::from_secs(15),
            manage_timeout: Duration::from_secs(10),
            event_capacity: 1024,
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ManageError {
    #[error("the node is not connected")]
    Offline,
    #[error("{op} was not acknowledged")]
    NoAck { op: String },
    /// Every refusal is acknowledged, including the ones a node decides before it looks at the op.
    /// It is an error here so that a caller cannot mistake arrival for success.
    #[error("{op}: {message}")]
    Refused {
        op: String,
        code: String,
        message: String,
    },
}

/// Which of the three greeting frames is next. The third is pushed unasked, and a client that
/// took it for the answer to its own `prefs` write would apply somebody else's zoom.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Greeting {
    Hello,
    Herd,
    Prefs,
    Done,
}

/// Everything the client knows, and the only authoritative copy of it.
///
/// [`Event`]s say *that* something changed; this says what is true. A consumer that misses an
/// event redraws from here.
#[derive(Debug)]
pub struct State {
    pub hello: Option<Hello>,
    pub role: Role,
    pub herd: Herd,
    /// Per-pane, per-device render preferences, as the node last published them. Values are
    /// opaque JSON: `1.6` and `"1.6"` both round-trip and a client reads either.
    pub prefs: BTreeMap<String, Value>,
    pub connected: bool,
    panes: HashMap<String, PaneState>,
    styles: StyleTable,
    greeting: Greeting,
}

impl Default for State {
    fn default() -> Self {
        Self {
            hello: None,
            role: Role::Readonly,
            herd: Herd::default(),
            prefs: BTreeMap::new(),
            connected: false,
            panes: HashMap::new(),
            styles: StyleTable::default(),
            greeting: Greeting::Hello,
        }
    }
}

impl State {
    pub fn pane(&self, id: &str) -> Option<&PaneState> {
        self.panes.get(id)
    }

    /// What this node claims it can do. Every affordance is gated on one of these, and what a
    /// node does not claim is hidden rather than disabled.
    pub fn caps(&self) -> crate::frames::Caps {
        self.hello.as_ref().map(|h| h.caps.clone()).unwrap_or_default()
    }

    pub fn node_name(&self) -> &str {
        self.hello.as_ref().map_or("", |h| h.node_name.as_str())
    }
}

struct Watch {
    scrollback: bool,
    conversation: bool,
}

struct Inner {
    session: Session,
    config: Policy,
    state: Mutex<State>,
    events: broadcast::Sender<Event>,
    link: Mutex<Option<mpsc::UnboundedSender<String>>>,
    watched: Mutex<HashMap<String, Watch>>,
    waiting: Mutex<HashMap<u64, oneshot::Sender<Managed>>>,
    next_rid: AtomicU64,
}

/// A node, kept connected.
///
/// Dropping it closes the socket and stops the supervisor.
pub struct Client {
    inner: Arc<Inner>,
    supervisor: tokio::task::JoinHandle<()>,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.supervisor.abort();
    }
}

impl Client {
    pub fn start(session: Session) -> Self {
        Self::with_policy(session, Policy::default())
    }

    pub fn with_policy(session: Session, config: Policy) -> Self {
        let (events, _) = broadcast::channel(config.event_capacity);
        let inner = Arc::new(Inner {
            session,
            config,
            state: Mutex::new(State::default()),
            events,
            link: Mutex::new(None),
            watched: Mutex::new(HashMap::new()),
            waiting: Mutex::new(HashMap::new()),
            next_rid: AtomicU64::new(1),
        });
        let supervisor = tokio::spawn(supervise(inner.clone()));
        Self { inner, supervisor }
    }

    pub fn session(&self) -> &Session {
        &self.inner.session
    }

    /// A consumer that sees [`broadcast::error::RecvError::Lagged`] has missed frames and must
    /// redraw from [`Client::state`] rather than trying to catch up.
    pub fn events(&self) -> broadcast::Receiver<Event> {
        self.inner.events.subscribe()
    }

    pub fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().expect("the client state lock")
    }

    pub fn connected(&self) -> bool {
        self.inner.link.lock().expect("the link lock").is_some()
    }

    /// Records the watch and issues it. It is re-issued on every reconnection, so a caller states
    /// what it wants to see once.
    pub fn watch(&self, pane: &str, scrollback: bool, conversation: bool) -> bool {
        self.inner.watched.lock().expect("watched").insert(
            pane.to_string(),
            Watch {
                scrollback,
                conversation,
            },
        );
        self.send(json!({
            "t": "watch", "pane": pane,
            "scrollback": scrollback, "conversation": conversation
        }))
    }

    pub fn unwatch(&self, pane: &str) -> bool {
        self.inner.watched.lock().expect("watched").remove(pane);
        self.send(json!({ "t": "unwatch", "pane": pane }))
    }

    /// Text is what herdr's `pane.send_text` takes. Everything its key grammar rejects — Home,
    /// End, PageUp, PageDown, Insert, Delete — goes as its escape sequence through here rather
    /// than through [`Client::keys`] (probes #8/#9).
    pub fn input(&self, pane: &str, text: &str) -> bool {
        self.send(json!({ "t": "input", "pane": pane, "text": text }))
    }

    pub fn keys(&self, pane: &str, keys: &[&str]) -> bool {
        self.send(json!({ "t": "input", "pane": pane, "keys": keys }))
    }

    /// The node decides whether a submit key follows, per harness, so only a key that was offered
    /// in `pending.options` may be sent and an Enter is never synthesised.
    pub fn answer(&self, pane: &str, key: &str) -> bool {
        self.send(json!({ "t": "answer", "pane": pane, "key": key }))
    }

    pub fn convo_load(&self, pane: &str, before: Option<&str>) -> bool {
        self.send(json!({ "t": "convo.load", "pane": pane, "before": before }))
    }

    /// A merge, not a replacement: it names the keys it is changing and a `null` value removes
    /// one. With no `pane` it stores nothing and just asks for the current set.
    pub fn write_prefs(&self, pane: &str, prefs: Value) -> bool {
        self.send(json!({ "t": "prefs", "pane": pane, "prefs": prefs }))
    }

    /// Asks what this node can be told to make. The answer arrives as [`Event::Caps`], and the
    /// kinds in it come from the node rather than from a list compiled into a client.
    pub fn request_caps(&self) -> bool {
        self.send(json!({ "t": "caps" }))
    }

    pub fn resync(&self) -> bool {
        self.send(json!({ "t": "resync" }))
    }

    pub fn ping(&self, n: u64) -> bool {
        self.send(json!({ "t": "ping", "n": n }))
    }

    /// One `manage` op, awaited to its ack.
    ///
    /// `op` is the body — `{"op": "...", "at": "..."}` — and the `t` and the `rid` are this
    /// client's to add. The herd is **not** mutated here: the structure change arrives as an
    /// ordinary `herd.patch` and the node stays authoritative.
    pub async fn manage(&self, op: Value) -> Result<Managed, ManageError> {
        let name = op["op"].as_str().unwrap_or_default().to_string();
        let rid = self.inner.next_rid.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = oneshot::channel();
        let mut message = op;
        message["t"] = json!("manage");
        message["rid"] = json!(rid);
        self.inner.waiting.lock().expect("waiting").insert(rid, tx);
        if !self.send(message) {
            self.inner.waiting.lock().expect("waiting").remove(&rid);
            return Err(ManageError::Offline);
        }
        let ack = match tokio::time::timeout(self.inner.config.manage_timeout, rx).await {
            Ok(Ok(ack)) => ack,
            _ => {
                self.inner.waiting.lock().expect("waiting").remove(&rid);
                return Err(ManageError::NoAck { op: name });
            }
        };
        match ack.ok {
            true => Ok(ack),
            false => Err(ManageError::Refused {
                op: ack.op,
                code: ack.code.unwrap_or_default(),
                message: ack.message.unwrap_or_default(),
            }),
        }
    }

    /// `false` means the socket is down and nothing was sent. Nothing is queued across an outage
    /// on purpose: replaying a keystroke into a shell minutes after it was typed is worse than
    /// dropping it.
    pub fn send(&self, message: Value) -> bool {
        self.inner.send(message)
    }
}

impl Inner {
    fn send(&self, message: Value) -> bool {
        let link = self.link.lock().expect("the link lock");
        match link.as_ref() {
            Some(tx) => tx.send(message.to_string()).is_ok(),
            None => false,
        }
    }

    fn emit(&self, event: Event) {
        let _ = self.events.send(event);
    }

    /// A fresh socket starts a fresh greeting and a fresh style table — ids are only promised to
    /// be stable for the life of a connection. The **grids are kept**: their cells carry resolved
    /// colours, so a new table cannot invalidate them, and they are what is on screen until the
    /// `grid.reset` swaps them.
    fn opening(&self) {
        let mut state = self.state.lock().expect("state");
        state.styles = StyleTable::default();
        state.greeting = Greeting::Hello;
        state.connected = false;
    }

    fn closed(&self, reason: &str) {
        {
            let mut state = self.state.lock().expect("state");
            state.connected = false;
            state.herd.stale = true;
            for pane in state.panes.values_mut() {
                pane.set_stale(true);
            }
        }
        self.waiting.lock().expect("waiting").clear();
        self.emit(Event::Disconnected {
            reason: reason.to_string(),
        });
    }

    fn resubscribe(&self) {
        let watched: Vec<(String, bool, bool)> = self
            .watched
            .lock()
            .expect("watched")
            .iter()
            .map(|(pane, w)| (pane.clone(), w.scrollback, w.conversation))
            .collect();
        for (pane, scrollback, conversation) in watched {
            self.send(json!({
                "t": "watch", "pane": pane,
                "scrollback": scrollback, "conversation": conversation
            }));
        }
    }

    async fn serve(self: &Arc<Self>, socket: Socket) -> String {
        let (mut sink, mut stream) = socket.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<String>();
        self.opening();
        *self.link.lock().expect("the link lock") = Some(tx);
        self.resubscribe();
        let writer = tokio::spawn(async move {
            while let Some(text) = rx.recv().await {
                if sink.send(Message::text(text)).await.is_err() {
                    break;
                }
            }
        });
        let reason = loop {
            match stream.next().await {
                Some(Ok(Message::Text(text))) => {
                    if let Some(close) = self.receive(&text) {
                        break close;
                    }
                }
                Some(Ok(Message::Close(_))) | None => break "the node closed the socket".into(),
                Some(Err(e)) => break e.to_string(),
                Some(Ok(_)) => {}
            }
        };
        writer.abort();
        *self.link.lock().expect("the link lock") = None;
        reason
    }

    /// `Some` is a reason to drop the socket and dial again.
    ///
    /// **An unknown `t` is ignored, not an error.** That is how a client older than a node keeps
    /// working, and it is the same rule the node applies in the other direction.
    fn receive(&self, text: &str) -> Option<String> {
        let Ok(message) = serde_json::from_str::<Value>(text) else {
            debug!("the node sent something that is not JSON");
            return None;
        };
        match message["t"].as_str().unwrap_or_default() {
            "hello" => self.hello(message),
            "role" => self.role(&message),
            "herd" => self.herd(message),
            "herd.patch" => self.herd_patch(&message),
            "prefs" => self.prefs(&message),
            "styles" => return self.styles(message),
            "grid.reset" => self.grid_reset(&message),
            "grid.patch" => self.grid_patch(&message),
            "scrollback" => self.scrollback(&message),
            "convo" => self.convo(message),
            "convo.turn" => self.convo_turn(&message),
            "convo.facets" => self.convo_facets(&message),
            "convo.composer" => self.convo_composer(&message),
            "pending" => self.pending(message),
            "caps" => self.node_caps(message),
            "managed" => self.managed(message),
            "error" => self.error(message),
            "pong" => {
                if let Some(n) = message["n"].as_u64() {
                    self.emit(Event::Pong { n });
                }
            }
            _ => {}
        }
        None
    }

    fn hello(&self, message: Value) {
        let Ok(hello) = serde_json::from_value::<Hello>(message) else {
            warn!("a node greeted this client with a hello it could not read");
            return;
        };
        {
            let mut state = self.state.lock().expect("state");
            state.role = hello.role;
            state.hello = Some(hello.clone());
            state.connected = true;
            state.greeting = Greeting::Herd;
        }
        self.emit(Event::Connected(Box::new(hello)));
    }

    /// **Not a second greeting.** Re-running one would throw the herd and the preferences away
    /// over a permission change, so this carries the role and nothing else.
    fn role(&self, message: &Value) {
        let Some(role) = message.get("role").cloned() else {
            return;
        };
        let Ok(role) = serde_json::from_value::<Role>(role) else {
            return;
        };
        {
            let mut state = self.state.lock().expect("state");
            state.role = role;
            if let Some(hello) = state.hello.as_mut() {
                hello.role = role;
            }
        }
        self.emit(Event::Role(role));
    }

    fn herd(&self, message: Value) {
        let nodes: Vec<NodeEntry> = decode(message.get("nodes"));
        let panes: Vec<PaneEntry> = decode(message.get("panes"));
        {
            let mut state = self.state.lock().expect("state");
            state.herd.apply(nodes, panes);
            if state.greeting == Greeting::Herd {
                state.greeting = Greeting::Prefs;
            }
        }
        self.emit(Event::Herd);
    }

    fn herd_patch(&self, message: &Value) {
        let added: HerdDelta = decode(message.get("added"));
        let changed: HerdDelta = decode(message.get("changed"));
        let removed: Vec<String> = decode(message.get("removed_ids"));
        self.state
            .lock()
            .expect("state")
            .herd
            .apply_patch(added, changed, &removed);
        self.emit(Event::Herd);
    }

    /// The third greeting frame arrives unasked on every connection, even when nothing is stored.
    /// It is **not** the answer to this client's own write, and a client that treated it as one
    /// would resolve its first write against whatever it happened to be holding.
    fn prefs(&self, message: &Value) {
        let panes = match message.get("panes") {
            Some(Value::Object(map)) => map
                .iter()
                .map(|(pane, value)| (pane.clone(), value.clone()))
                .collect(),
            _ => BTreeMap::new(),
        };
        let greeting = {
            let mut state = self.state.lock().expect("state");
            state.prefs = panes;
            let greeting = state.greeting == Greeting::Prefs;
            if greeting {
                state.greeting = Greeting::Done;
            }
            greeting
        };
        self.emit(Event::Prefs { greeting });
    }

    fn styles(&self, message: Value) -> Option<String> {
        let Ok(styles) = serde_json::from_value::<Styles>(message) else {
            return None;
        };
        let absorbed = self.state.lock().expect("state").styles.absorb(&styles);
        match absorbed {
            true => None,
            // Ids are minted append-only by one encoder per connection, so a batch that starts
            // past the end of the table cannot be honestly filled in. A fresh socket is a fresh
            // table, which is the only repair there is.
            false => Some(format!(
                "a styles message started at {} and this connection's table is shorter",
                styles.from
            )),
        }
    }

    fn grid_reset(&self, message: &Value) {
        let Some(pane) = message["pane"].as_str() else {
            return;
        };
        let cols = message["cols"].as_u64().unwrap_or_default() as u16;
        let rows = message["rows"].as_u64().unwrap_or_default() as u16;
        let rows_data: Vec<RowRuns> = decode(message.get("rows_data"));
        // **A reset carries the whole link table and replaces the pane's.** Appending it instead
        // puts every later id out by the length of the previous table, which resolves links to
        // the wrong URL rather than failing visibly.
        let links: Vec<String> = decode(message.get("links"));
        let update = {
            let mut state = self.state.lock().expect("state");
            let State { styles, panes, .. } = &mut *state;
            let entry = panes.entry(pane.to_string()).or_default();
            entry.set_stale(false);
            entry
                .shadow_mut()
                .reset(cols, rows, &rows_data, cursor(message), links, styles)
        };
        self.emit(Event::Grid {
            pane: pane.to_string(),
            update,
        });
    }

    fn grid_patch(&self, message: &Value) {
        let Some(pane) = message["pane"].as_str() else {
            return;
        };
        let rows: Vec<RowRuns> = decode(message.get("rows"));
        // A patch carries only the entries discovered since the last message, so it **appends**.
        let links: Vec<String> = decode(message.get("links"));
        let update = {
            let mut state = self.state.lock().expect("state");
            let State { styles, panes, .. } = &mut *state;
            let entry = panes.entry(pane.to_string()).or_default();
            entry.shadow_mut().patch(&rows, cursor(message), links, styles)
        };
        if let Some(update) = update {
            self.emit(Event::Grid {
                pane: pane.to_string(),
                update,
            });
        }
    }

    fn scrollback(&self, message: &Value) {
        let Some(pane) = message["pane"].as_str() else {
            return;
        };
        let rows: Vec<RowRuns> = decode(message.get("rows"));
        let delta = {
            let mut state = self.state.lock().expect("state");
            let State { styles, panes, .. } = &mut *state;
            let entry = panes.entry(pane.to_string()).or_default();
            let (cols, _) = entry.geometry();
            let doc = ScrollbackDoc {
                from_top: message["from_top"].as_u64().unwrap_or_default() as u32,
                rows: rows
                    .iter()
                    .map(|row| RowDiff {
                        row: row.row,
                        cells: decode_row(&row.runs, styles, cols),
                    })
                    .collect(),
                total_rows: message["total_rows"].as_u64().unwrap_or_default() as u32,
                complete: message["complete"].as_bool().unwrap_or(true),
                capped: message["capped"].as_bool().unwrap_or(false),
                era: message["era"].as_u64().unwrap_or_default() as u32,
            };
            let history = entry.history_mut();
            let before = history.end();
            // The ring the node holds was replaced rather than added to, so what this client holds
            // is not its ancestor however adjacent the indices look (probe #498). The whole
            // document goes on; a delta past the old end would leave the era before it underneath.
            match history.absorb(&doc) {
                true => Some(history.doc()),
                false => history.since(before),
            }
        };
        if let Some(doc) = delta {
            self.emit(Event::Scrollback {
                pane: pane.to_string(),
                doc,
            });
        }
    }

    fn convo(&self, message: Value) {
        if let Ok(page) = serde_json::from_value::<ConvoPage>(message) {
            self.emit(Event::Convo(page));
        }
    }

    fn convo_turn(&self, message: &Value) {
        let Some(pane) = message["pane"].as_str() else {
            return;
        };
        self.emit(Event::ConvoTurn {
            pane: pane.to_string(),
            turns: decode(message.get("turns")),
        });
    }

    fn convo_facets(&self, message: &Value) {
        let Some(pane) = message["pane"].as_str() else {
            return;
        };
        let Ok(facets) = serde_json::from_value(message["facets"].clone()) else {
            return;
        };
        self.emit(Event::ConvoFacets {
            pane: pane.to_string(),
            facets,
        });
    }

    fn convo_composer(&self, message: &Value) {
        let Some(pane) = message["pane"].as_str() else {
            return;
        };
        self.emit(Event::ConvoComposer {
            pane: pane.to_string(),
            text: message["text"].as_str().map(str::to_string),
            clear: message["clear"].as_str().map(str::to_string),
        });
    }

    fn pending(&self, message: Value) {
        if let Ok(pending) = serde_json::from_value::<Pending>(message) {
            self.emit(Event::Pending(pending));
        }
    }

    fn node_caps(&self, message: Value) {
        if let Ok(caps) = serde_json::from_value::<NodeCaps>(message) {
            self.emit(Event::Caps(caps));
        }
    }

    fn managed(&self, message: Value) {
        let rid = message["rid"].as_u64();
        let Ok(ack) = serde_json::from_value::<Managed>(message) else {
            return;
        };
        if let Some(waiter) = rid.and_then(|rid| self.waiting.lock().expect("waiting").remove(&rid)) {
            // A caller that stopped waiting is not an error: the ack and the timeout crossed.
            let _ = waiter.send(ack);
            return;
        }
        self.emit(Event::Managed(ack));
    }

    /// An unrecognised code renders its `message`. The vocabulary is open — a hub forwards a
    /// peer's codes verbatim — so failing on one this build has never seen would hide the
    /// diagnosis the node went to the trouble of sending.
    fn error(&self, message: Value) {
        let Ok(failure) = serde_json::from_value::<Failure>(message) else {
            return;
        };
        self.emit(Event::Error(failure));
    }
}

async fn supervise(inner: Arc<Inner>) {
    let mut backoff = inner.config.backoff.start();
    loop {
        let dialled = dial::connect(
            &inner.session.origin,
            &inner.session.token,
            inner.config.connect_timeout,
        )
        .await;
        match dialled {
            Ok(socket) => {
                let at = Instant::now();
                let reason = inner.serve(socket).await;
                inner.closed(&reason);
                if at.elapsed() >= SETTLED_AFTER {
                    backoff.reset();
                }
            }
            Err(e) => inner.closed(&e.to_string()),
        }
        backoff.sleep().await;
    }
}

fn decode<T: serde::de::DeserializeOwned + Default>(value: Option<&Value>) -> T {
    value
        .cloned()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

fn cursor(message: &Value) -> Cursor {
    message
        .get("cursor")
        .cloned()
        .and_then(|c| serde_json::from_value(c).ok())
        .unwrap_or_default()
}
