use crate::convo::{self, ConvoCtx};
use crate::manage::{self, ManageOp, Managed, Manager, Settle};
use crate::outbox::{Frame, Outbox};
use crate::pending;
use crate::state::{BUILD, Node};
use crate::wire::Wire;
use axum::extract::ws::WebSocket;
use base64::Engine;
use kampr_auth::{Device, Entry, Role};
use kampr_core::PaneRegistry;
use kampr_core::provider::Input;
use kampr_core::registry::PaneHold;
use kampr_core::wire::{ClientMsg, ErrorCode, PROTOCOL, PendingSource, ServerMsg};
use kampr_mesh::peers::PeerHold;
use kampr_mesh::{Incoming, Outgoing, Peers};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{debug, warn};

/// The least time between two `scrollback` frames on one socket.
///
/// Not a poll any more — the ring says when it has moved ([`kampr_core::PaneRegistry::
/// scrollback_changes`]) and this socket is woken then. What it still is, is a **floor**: a busy
/// pane's ring moves ten times a second and laying twenty thousand rows out per socket per move
/// is a tokio worker with no `.await` in it. Three seconds is the cadence the poll it replaced
/// ran at, so the rendering cost per watcher is exactly what it was, and the frame now arrives
/// *when there is one* rather than up to three seconds later.
const SCROLLBACK_FLOOR: Duration = Duration::from_secs(3);

/// How often, and how many times, a blocked pane is re-read for a question its screen had not
/// finished painting. Bounded so a harness whose dialog Kampr cannot parse costs a handful of
/// reads rather than one every half second for as long as it sits there.
const PENDING_RETRY: Duration = Duration::from_millis(500);
const PENDING_ATTEMPTS: u32 = 12;

/// How often a live session re-reads its own device row. The broadcast covers a revocation made
/// in this process; this covers one made by `kampr setup` in another, and a Tier 0 expiry that
/// nothing announces at all.
const DEVICE_RECHECK: Duration = Duration::from_secs(2);

/// How many consecutive re-reads of the session's own device row may fail before the socket is
/// closed.
///
/// A transient database error is not a revocation and one must stay survivable — but nothing here
/// can tell transient from permanent, and *every* condition that keeps the store unreadable (a
/// full disk, a corrupt WAL, the file replaced under a restore, permissions changed) looks the
/// same from this side. Failing open on all of them kept every connected device online for as
/// long as it lasted, revoked ones included, which turns the product's kill switch into a
/// plausible-looking success. A session that has gone this many checks without an answer closes
/// instead, and a client that reconnects is refused at the door for the same reason.
const DEVICE_READ_FAILURES: u32 = 3;

/// Keepalives a client may leave unanswered before the node stops serving it.
///
/// Three, the same as the mesh link's, and for the same reason: one missed answer is a scheduling
/// hiccup on a loaded phone, two is a coincidence, three is a peer that is not there. What it
/// bounds is real — an abandoned watcher costs herdr what a live one costs (#284).
const MISSED_PONGS: u32 = 3;

const MAX_PREFS_BYTES: usize = 2048;

/// Attachments this node will be chunking towards one hub at a time.
///
/// Each holds one decoded record until the hub has read it, and the ceiling on one is 8 MiB, so
/// this is the memory a hub can make a peer hold — and the hub opens one per client request, so
/// without a bound a herd's worth of phones tapping screenshots is a peer's whole heap.
const CONCURRENT_TRANSFERS: usize = 4;

/// Every `t` [`ClientMsg`] decodes, kept beside it because the enum's own refusal of an unknown
/// variant is indistinguishable from a malformed known one.
/// Turns in a page of a launched conversation. Larger than the pane's own page because a
/// subagent's whole conversation is usually shorter than one exchange of its parent's, and a
/// reader who opened one asked for all of it.
const SUB_PAGE: usize = 200;

const CLIENT_VERBS: [&str; 10] = [
    "watch",
    "unwatch",
    "answer.submit",
    "input",
    "paste",
    "answer",
    "convo.load",
    "convo.sub",
    "resync",
    "ping",
];

pub async fn run(socket: WebSocket, node: Arc<Node>, device: Device, peer: String) {
    let (link, heard) = crate::mesh::split(socket);
    let (out, incoming) = link.split();
    run_on_watched(out, incoming, node, device, peer, Caller::Client, Some(heard)).await;
}

/// One client session, over any framed link.
///
/// Generic over the transport because a **hub is a client of a peer**: it sends `watch` and
/// `input` and receives grids, and it does so over a socket the peer dialled outbound. Serving it
/// with this code rather than a second implementation is what makes the relay free — the
/// read-only refusal, the device re-read before every write, the audit line, the bounded queue and
/// its purge rule all apply at the mesh hop because they are the same code.
pub async fn run_on<O: Outgoing, I: Incoming>(
    out: O,
    incoming: I,
    node: Arc<Node>,
    device: Device,
    peer: String,
    caller: Caller,
) {
    run_on_watched(out, incoming, node, device, peer, caller, None).await;
}

/// [`run_on`], plus the liveness half for a transport that can be lied to.
///
/// **A peer that freezes rather than closing is invisible from every other angle** (#284). Its
/// kernel stays alive and ACKs each of TCP's window probes with a zero window, which resets the
/// probe counter, so the connection never times out and a write never errors; the writer never
/// breaks, `outbox.close()` is never reached, and the node serves its watches for ever — measured
/// still held after twenty-five minutes, costing herdr exactly what a live watcher costs and
/// holding one of the node's socket permits. The application backpressure cannot notice either,
/// because `pump_pane` *purges* a congested pane's frames rather than queueing them, so the
/// bounded queue never reaches the cap that would close it.
///
/// So the node asks. Two questions, because the two states fail differently: a **ping**, for a
/// peer with nothing queued for it, whose socket is idle and will never time out on its own; and a
/// **deadline on the send itself**, for a peer that is being written to, where the write is what
/// hangs and the ticker below is never polled again. `heard` is `None` for a transport that has
/// its own liveness or cannot be lied to.
///
/// The ping is what the tests pin, both cases. The send deadline is not reproduced by any of them
/// — at test volumes `pump_pane`'s purge keeps the socket from filling — and it stands instead on
/// #284's `ss` capture of a real one in that state and on the `select!` below being unable to
/// reach the ticker from inside a pending `send`. It is the only guard covering that state.
#[allow(clippy::too_many_arguments)]
async fn run_on_watched<O: Outgoing, I: Incoming>(
    mut out: O,
    mut incoming: I,
    node: Arc<Node>,
    device: Device,
    peer: String,
    caller: Caller,
    heard: Option<Arc<crate::mesh::Heard>>,
) {
    let outbox = Arc::new(Outbox::new(node.config.limits.client_queue));
    let wire = Arc::new(Wire::new(outbox.clone()));

    let every = Duration::from_secs(node.config.limits.client_keepalive_secs.max(1));
    let patience = every * MISSED_PONGS;
    let writer = tokio::spawn({
        let outbox = outbox.clone();
        let who = peer.clone();
        async move {
            let mut ticker = tokio::time::interval(every);
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            ticker.tick().await;
            let mut unanswered = 0u32;
            loop {
                tokio::select! {
                    frame = outbox.next() => {
                        let Some(frame) = frame else { break };
                        match tokio::time::timeout(patience, out.send(frame.json)).await {
                            Ok(true) => {}
                            Ok(false) => break,
                            Err(_) => {
                                warn!(peer = %who, "a client stopped reading; dropping it");
                                break;
                            }
                        }
                    }
                    _ = ticker.tick(), if heard.is_some() => {
                        if heard.as_ref().is_some_and(|h| h.take()) {
                            unanswered = 0;
                        } else {
                            unanswered += 1;
                            if unanswered >= MISSED_PONGS {
                                warn!(peer = %who, "a client stopped answering keepalives");
                                break;
                            }
                        }
                        match tokio::time::timeout(patience, out.ping()).await {
                            Ok(true) => {}
                            _ => break,
                        }
                    }
                }
            }
            outbox.close();
            out.close().await;
        }
    });
    // The writer owns the sending half of the socket, so it has to die with this session — and a
    // session can be *cancelled* rather than returning, which is what happens when a node stops
    // while a mesh link is up. Without this the socket stays open and the far end never notices.
    let _writer_guard = crate::mesh::AbortOnDrop(writer.abort_handle());

    let mut session = Session {
        node: node.clone(),
        wire: wire.clone(),
        device,
        peer,
        caller,
        panes: HashMap::new(),
        held: HashMap::new(),
        matched: HashMap::new(),
        sending: HashMap::new(),
        unreadable: 0,
    };
    // **Subscribed before the herd is read, and the read is the subscription's own.** `greet`
    // awaits a database round trip after it has sent the model, and a rebuild landing in that
    // window used to be seen by neither: the client held V while the feed started at V+1, so the
    // V→V+1 delta was never sent and every later patch diffed against a model the client had
    // never been given. `state.rs` calls out the same trap one file over.
    let mut herd_rx = node.subscribe_herd();
    let greeting = herd_rx.borrow_and_update().clone();
    session.greet(&greeting).await;
    let herd_task = tokio::spawn(herd_updates(herd_rx, greeting, wire.clone()));
    let _herd_guard = crate::mesh::AbortOnDrop(herd_task.abort_handle());

    // The book is the node's rather than the device's, so a save made on the phone has to reach
    // the desktop looking at the same node right now — not at its next connection.
    let mut book_rx = node.subscribe_book();
    book_rx.borrow_and_update();

    let mut changes = node.auth.device_changes();
    let mut recheck = tokio::time::interval(DEVICE_RECHECK);
    recheck.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    recheck.tick().await;

    loop {
        tokio::select! {
            text = incoming.recv() => {
                let Some(text) = text else { break };
                if !session.dispatch(&text).await {
                    break;
                }
            }
            changed = changes.recv() => {
                let mine = match changed {
                    Ok(id) => id == session.device.id,
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                    // Lagged: something changed and we do not know what, so look.
                    Err(_) => true,
                };
                if mine && !session.refresh().await {
                    break;
                }
            }
            _ = recheck.tick() => {
                if !session.refresh().await {
                    break;
                }
            }
            changed = book_rx.changed() => {
                if changed.is_err() {
                    break;
                }
                session.publish_book().await;
            }
        }
        if outbox.is_closed() {
            break;
        }
    }

    session.audit("session.closed", None, None);
    for handle in session.panes.into_values() {
        handle.stop();
    }
    herd_task.abort();
    outbox.close();
    let _ = writer.await;
}

struct PaneHandle {
    pane: String,
    outbox: Arc<Outbox>,
    tasks: Vec<JoinHandle<()>>,
    scrollback: bool,
    conversation: bool,
    convo: convo::Open,
    /// What this pane's conversation cost to open, on loan from the node for as long as this
    /// handle holds the pane and handed back when it drops. See [`crate::warm`].
    warm: convo::Warmth,
    warmth: Arc<crate::warm::ConvoWarmth>,
    /// The launched conversation this client has open on the pane, followed while it has it.
    followed: convo::Followed,
    _hold: Option<Hold>,
}

/// What keeps a pane open across a handover, on whichever side of the mesh streams it. Dropping it
/// is what releases the pane, so it lives in the [`PaneHandle`] that replaced the old one and goes
/// when that handle does — an unwatch, a disconnect, or the next handover, which takes its own
/// hold before this one is stopped.
enum Hold {
    Local { _pane: PaneHold },
    Peer { _pane: PeerHold },
}

/// Where the pane about to be handed over is streamed from, and therefore who can hold it open.
enum Streams<'a> {
    Local(&'a PaneRegistry, &'a str),
    Peer(&'a Peers, &'a str),
}

/// Stops whatever is streaming this pane and hands back what keeps it open until the replacement
/// has attached.
///
/// **A re-watch must not be a re-open.** The old pump owns the watcher for the pane and `stop`
/// aborts it, so if it was the only one the pane goes with it. Locally that is the emulator, the
/// ring and the spawned `observe` behind them, and what the new pump opens is a *fresh* pane whose
/// flush publishes a blank grid at the pane's real geometry over content the client is already
/// looking at (#252). For a peer's pane it is the hub's shadow, the history it has stitched, and
/// one `unwatch` plus one `watch` back across the WAN.
///
/// The two differ in how reliably they bite. Locally the old pump is stopped synchronously, so it
/// fires every time. Over the mesh the pump is a task and `JoinHandle::abort` is not synchronous,
/// so the aborted pump usually still holds the pane when the replacement watches — one relayed
/// pane survives a resync on its own. `resync` spawns a pump per pane, though, and the second
/// spawn displaces the first from the scheduler slot that was polling it first: from two panes
/// upwards every pane of every resync is re-opened.
fn hand_over(
    panes: &mut HashMap<String, PaneHandle>,
    outbox: &Outbox,
    streams: Streams<'_>,
    pane: &str,
) -> Option<Hold> {
    let stop = || {
        if let Some(old) = panes.remove(pane) {
            old.stop();
        }
    };
    let hold = match streams {
        Streams::Local(registry, local) => registry
            .hold_while(local, stop)
            .map(|pane| Hold::Local { _pane: pane }),
        Streams::Peer(peers, global) => peers
            .hold_while(global, stop)
            .map(|pane| Hold::Peer { _pane: pane }),
    };
    // The previous handle stopped this pane in the outbox, and a straggling frame from its pump
    // must not outlive that. Reopening it is the last thing before the new pump exists.
    outbox.resume_pane(pane);
    hold
}

impl PaneHandle {
    /// Aborting is not enough on its own. `JoinHandle::abort` lands at the task's next await, and
    /// a pump that has already taken an update off its watcher reaches the outbox without one — so
    /// the pane is stopped in the outbox too, which is where a push and a stop are serialised.
    fn stop(&self) {
        for task in &self.tasks {
            task.abort();
        }
        self.outbox.stop_pane(&self.pane);
    }
}

/// Same reason as the writer guard: a session that is cancelled rather than closed must not leave
/// pane pumps running against a socket nobody is reading.
///
/// And the conversation goes back to the node on the way out, so that the next watcher of this
/// pane — this client returning to it, or the same client on the socket it opens after a
/// reconnect — is served off a parse and a fold that already exist (#409). Synchronously, and
/// before this returns, because the re-watch that wants it is the very next thing the session
/// does: `hand_over` drops the handle it is replacing and takes the pane's warmth back one line
/// later.
impl Drop for PaneHandle {
    fn drop(&mut self) {
        self.stop();
        self.warmth.put(&self.pane, self.warm.clone());
    }
}

/// One terminal view's standing claim on a pane's geometry, for as long as that view is open.
///
/// A lease is *scoped*, and that is the whole of "newest holder wins": a viewer displaced by a
/// later one still runs this on the way out and it lands on nothing, so the earlier viewer never
/// takes the later one's hold down with it and never fights back for the pane.
///
/// The peer arm sends the same scoped release down the mesh, because a hub's link to a peer
/// outlives any one phone that was looking through it — without this a client that closed its
/// laptop would leave a pane on another host held at that laptop's size until the link dropped.
enum MatchLease {
    Local {
        node: Arc<Node>,
        /// herdr's own id, which is what [`crate::holds::PaneHolds`] is keyed by.
        local: String,
        token: crate::holds::HoldToken,
    },
    Peer {
        node: Arc<Node>,
        target: String,
        global: String,
        token: u64,
    },
}

impl MatchLease {
    fn token(&self) -> u64 {
        match self {
            Self::Local { token, .. } | Self::Peer { token, .. } => *token,
        }
    }
}

impl Drop for MatchLease {
    fn drop(&mut self) {
        match self {
            Self::Local { node, local, token } => {
                node.holds.let_go(local, *token);
            }
            Self::Peer {
                node,
                target,
                global,
                token,
            } => {
                let (node, target, global, token) = (node.clone(), target.clone(), global.clone(), *token);
                // Spawned rather than awaited, because this runs from a `Drop` that is itself
                // often running inside a cancelled task. The runtime outlives the session.
                tokio::spawn(async move {
                    let release = json!({ "t": "manage", "op": "pane.size",
                                          "at": global, "mode": "release", "lease": token });
                    if let Err(e) = node.peers.manage(&target, release).await {
                        warn!(pane = %global, error = %e, "letting go of a matched pane on a peer");
                    }
                });
            }
        }
    }
}

/// Who is on the other end of a session.
///
/// The whole of the difference is `att.*`: a browser has `GET /api/attachment` and must use it,
/// because bytes on the socket that carries frames is the thing that route exists to avoid. A hub
/// has no inbound path to a peer (ADR 0007), so for it the socket is the only way, and the
/// chunking and the bulk lane are what make that safe.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Caller {
    Client,
    Hub,
}

struct Session {
    node: Arc<Node>,
    wire: Arc<Wire>,
    device: Device,
    peer: String,
    caller: Caller,
    panes: HashMap<String, PaneHandle>,
    /// What this client is holding from each pane's conversation, kept past the pump that sent it
    /// and past the `unwatch` that stopped the pump. See [`convo::Held`].
    held: HashMap<String, convo::Held>,
    /// The panes this client's terminal views are holding at their own geometry, by global id.
    ///
    /// **This map is the release-on-disconnect.** Dropping a lease lets its hold go, and the map
    /// is dropped on every path out of a session — the ordinary close below, a `break` in the
    /// dispatch loop, the keepalive giving up on a peer that froze rather than closed (#284), and
    /// the whole task being cancelled when the node stops. A closed laptop is a socket that stops
    /// answering, and that is what ends the hold rather than anything the client remembered to
    /// send. See [ADR 0013](../../../docs/adr/0013-a-standing-intent-to-match-the-view.md).
    matched: HashMap<String, MatchLease>,
    sending: HashMap<u64, Sending>,
    unreadable: u32,
}

/// One attachment this node is chunking towards a hub: the credits it is waiting for, and a task
/// that dies with the entry.
struct Sending {
    credit: mpsc::Sender<u32>,
    _task: crate::mesh::AbortOnDrop,
}

impl Session {
    /// `hello`, the herd, this device's stored preferences, and the fleet book — the last two
    /// unasked, because there is no other way for a client to learn the zoom it left a pane at or
    /// the commands it is about to offer. A client that has to ask has already painted.
    ///
    /// The book is last because it is the only one of the four that is not about this device: it
    /// is the node's, so that it is the same list on the operator's phone and on their desktop.
    async fn greet(&self, herd: &crate::herd::HerdModel) {
        self.wire.send_json(&hello(&self.node, &self.device, self.caller));
        self.wire.send(&herd.message());
        self.wire.send_json(&self.stored_prefs().await);
        self.publish_book().await;
        self.audit("session.opened", None, None);
    }

    async fn publish_book(&self) {
        self.wire
            .send_json(&crate::book::frame(self.node.auth.store()).await);
    }

    async fn stored_prefs(&self) -> Value {
        let panes = self
            .node
            .auth
            .store()
            .pane_prefs(&self.device.id)
            .await
            .unwrap_or_else(|_| json!({}));
        json!({ "t": "prefs", "panes": panes })
    }

    fn audit(&self, action: &str, pane: Option<&str>, detail: Option<Value>) {
        let mut entry = Entry::new(action)
            .device(&self.device.id, &self.device.name, self.device.role.as_str())
            .peer(&self.peer);
        if let Some(pane) = pane {
            entry = entry.pane(pane);
        }
        if let Some(detail) = detail {
            entry = entry.detail(detail);
        }
        self.node.auth.audit().record(&entry);
    }

    /// `readonly` receives every server → client message and is refused every write. The refusal
    /// is here rather than at the transport, so a read-only device keeps its stream.
    fn may_write(&self, verb: &str, pane: Option<&str>) -> bool {
        if self.device.role.writes() {
            return true;
        }
        self.audit_refused(verb, pane, ErrorCode::NotWriter, json!({}));
        self.wire
            .error(ErrorCode::NotWriter, "this device is read-only", pane);
        false
    }

    /// A device that is refused used to leave no trace at all — not what it tried, not on what,
    /// not that it tried again (probe #125). The loop rule lives in [`kampr_auth::Refusals`], so
    /// a client retrying forever costs a line per doubling rather than a line per attempt.
    fn audit_refused(&self, verb: &str, pane: Option<&str>, code: ErrorCode, mut detail: Value) {
        detail["code"] = serde_json::to_value(code).expect("an error code serialises");
        self.node
            .auth
            .record_refusal(&self.device, &self.peer, verb, pane, detail);
    }

    /// Re-reads this session's device. `false` means the socket must close: the row is gone,
    /// revoked, past its expiry — or unreadable for [`DEVICE_READ_FAILURES`] checks running, which
    /// is a store that cannot be asked whether this device is still allowed.
    async fn refresh(&mut self) -> bool {
        let device = match self.node.auth.store().device(&self.device.id).await {
            Ok(device) => {
                self.unreadable = 0;
                device
            }
            Err(e) => {
                self.unreadable += 1;
                if self.unreadable < DEVICE_READ_FAILURES {
                    debug!(error = %e, "could not re-read the session device");
                    return true;
                }
                tracing::error!(
                    error = %e,
                    device = %self.device.id,
                    checks = self.unreadable,
                    "the device store cannot say whether this device is still authorised; closing"
                );
                self.audit(
                    "session.unverifiable",
                    None,
                    Some(json!({ "checks": self.unreadable })),
                );
                return false;
            }
        };
        match device.filter(|d| d.active(kampr_auth::now())) {
            Some(device) => {
                if device.role != self.device.role {
                    self.device = device;
                    // Enforcement alone leaves a demoted device holding affordances that no
                    // longer work, and a promoted one waiting for a reconnect it has no reason
                    // to make. `hello` is the first message on a connection, so the change
                    // travels on its own frame.
                    self.wire.send(&ServerMsg::RoleChanged {
                        role: wire_role(self.device.role),
                    });
                    self.audit(
                        "session.role_changed",
                        None,
                        Some(json!({ "role": self.device.role.as_str() })),
                    );
                } else {
                    self.device = device;
                }
                true
            }
            None => {
                self.wire
                    .error(ErrorCode::Revoked, "this device is no longer authorised", None);
                self.audit("session.revoked", None, None);
                false
            }
        }
    }

    /// `false` closes the socket.
    async fn dispatch(&mut self, text: &str) -> bool {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            self.wire.error(ErrorCode::BadRequest, "not JSON", None);
            return true;
        };
        let tag = value["t"].as_str().map(str::to_string);
        // Anything that types into a terminal re-reads the device first. The handshake snapshot
        // is a cache, and a revoked token or a demoted role has to bite between two frames rather
        // than at the next connection.
        // `att.fetch` is here because the file-id form reads any path on this machine, and the
        // whole argument for serving one is that it is equivalent to typing — so it is gated like
        // typing. An operator demoting a hub writes SQLite in another process and tells this
        // session nothing, and a gate that waited for the next recheck is a two-second window on
        // an arbitrary read.
        if matches!(
            tag.as_deref(),
            Some("manage" | "input" | "paste" | "answer" | "att.fetch")
        ) && !self.refresh().await
        {
            return false;
        }
        // Unknown `t` values are ignored rather than refused — that is how a v1 client survives a
        // later node, and it has to work in this direction too. The tagged enum refuses a variant
        // it does not know, so the vocabulary is checked *before* it, and only a verb this node
        // does know is allowed to fail decoding.
        match tag.as_deref() {
            Some("manage") => self.manage(value).await,
            Some("caps") => self.caps().await,
            Some("prefs") => self.prefs(&value).await,
            // Only a hub asks for these, and a session that is not one has no verb for them —
            // which is the same thing it does with any other `t` it does not know.
            Some("att.fetch" | "att.more" | "att.stop") if self.caller == Caller::Hub => {
                self.attachment(&value);
            }
            Some(known) if CLIENT_VERBS.contains(&known) => {
                match serde_json::from_value::<ClientMsg>(value) {
                    Ok(msg) => self.client_msg(msg).await,
                    Err(e) => {
                        self.wire.error(ErrorCode::BadRequest, &e.to_string(), None);
                    }
                }
            }
            Some(unknown) => debug!(t = %unknown, "ignoring a message this node has no verb for"),
            None => {
                self.wire.error(ErrorCode::BadRequest, "message has no `t`", None);
            }
        }
        true
    }

    async fn client_msg(&mut self, msg: ClientMsg) {
        match msg {
            ClientMsg::Watch {
                pane,
                scrollback,
                conversation,
            } => self.watch(&pane, scrollback, conversation).await,
            ClientMsg::Unwatch { pane } => self.unwatch(&pane),
            ClientMsg::Input {
                pane,
                text,
                b64,
                keys,
            } => self.input(&pane, text, b64, keys).await,
            ClientMsg::Paste { pane, b64, name } => self.paste(&pane, &b64, name.as_deref()).await,
            ClientMsg::Answer { pane, key } => self.answer(&pane, &key).await,
            ClientMsg::AnswerSubmit { pane } => self.answer_submit(&pane).await,
            ClientMsg::ConvoLoad { pane, before } => self.convo_load(&pane, before.as_deref()),
            ClientMsg::ConvoSub { pane, id, before } => self.convo_sub(&pane, &id, before.as_deref()),
            ClientMsg::Resync => self.resync().await,
            ClientMsg::Ping { n } => {
                self.wire.send(&ServerMsg::Pong { n });
            }
        }
    }

    async fn watch(&mut self, pane: &str, scrollback: bool, conversation: bool) {
        let Some((session, local)) = self.node.resolve(pane) else {
            self.watch_peer(pane, scrollback, conversation).await;
            return;
        };
        if self.node.herd().pane(pane).is_none() {
            self.wire
                .error(ErrorCode::UnknownPane, "no such pane", Some(pane));
            return;
        }
        if !session.online() {
            self.wire.error(
                ErrorCode::NodeOffline,
                "the herdr serving this pane is not reachable",
                Some(pane),
            );
        }
        // Watching a pane is reading a terminal, and with `scrollback` it is reading its history
        // and with `conversation` its transcript. For a read-only device it is the *only* thing it
        // can do, so it is the only thing there is to record.
        self.audit(
            "watch",
            Some(pane),
            Some(json!({ "scrollback": scrollback, "conversation": conversation })),
        );
        let hold = hand_over(
            &mut self.panes,
            self.wire.outbox(),
            Streams::Local(&session.registry, &local),
            pane,
        );
        let mut tasks = vec![tokio::spawn(pump_pane(PaneStreamCtx {
            registry: session.registry.clone(),
            herdr: session.herdr.clone(),
            herd: self.node.subscribe_herd(),
            wire: self.wire.clone(),
            global: pane.to_string(),
            local: local.clone(),
            scrollback,
        }))];
        // Taken from the node rather than made here: on a re-watch this is the transcript already
        // found, already parsed and already folded, and taking it is what makes it this handle's
        // alone until it drops (#409).
        let warm = self.node.convo_warmth.take(pane);
        let convo = warm.lock().unwrap().journal.clone();
        let followed = convo::followed();
        if conversation {
            let provider = session.provider.clone();
            let for_identity = self.node.journals();
            let for_marker = self.node.journals();
            let marker_provider = session.provider.clone();
            let held = self
                .held
                .entry(pane.to_string())
                .or_insert_with(convo::held)
                .clone();
            tasks.push(tokio::spawn(convo::pump_convo(ConvoCtx {
                state_dir: self.node.state_dir.clone(),
                journals: self.node.journals(),
                panes: session.registry.clone(),
                herd: self.node.subscribe_herd(),
                identity: Box::new(move |local| convo::identity(&for_identity, &provider, local)),
                describe: Box::new(move |local| convo::marker_of(&for_marker, &marker_provider, local)),
                wire: self.wire.clone(),
                global: pane.to_string(),
                local,
                warm: warm.clone(),
                held,
                followed: followed.clone(),
            })));
        }
        self.panes.insert(
            pane.to_string(),
            PaneHandle {
                pane: pane.to_string(),
                outbox: self.wire.outbox().clone(),
                tasks,
                scrollback,
                conversation,
                convo,
                warm,
                warmth: self.node.convo_warmth.clone(),
                followed,
                _hold: hold,
            },
        );
    }

    /// A pane on another host. The hub holds one relay per pane however many clients are looking
    /// at it, so this costs the WAN hop nothing beyond the first watcher.
    async fn watch_peer(&mut self, pane: &str, scrollback: bool, conversation: bool) {
        if self.node.peers.state(pane) == kampr_mesh::PeerState::Unknown {
            self.wire.error(
                ErrorCode::UnknownPane,
                "no node in this herd serves that pane",
                Some(pane),
            );
            return;
        }
        self.audit(
            "watch",
            Some(pane),
            Some(json!({ "scrollback": scrollback, "conversation": conversation, "peer": true })),
        );
        let hold = hand_over(
            &mut self.panes,
            self.wire.outbox(),
            Streams::Peer(&self.node.peers, pane),
            pane,
        );
        let tasks = vec![tokio::spawn(crate::relay::pump_peer_pane(
            crate::relay::PeerPaneCtx {
                peers: self.node.peers.clone(),
                wire: self.wire.clone(),
                global: pane.to_string(),
                conversation,
            },
        ))];
        self.panes.insert(
            pane.to_string(),
            PaneHandle {
                pane: pane.to_string(),
                outbox: self.wire.outbox().clone(),
                tasks,
                scrollback,
                conversation,
                convo: convo::open(),
                // A pane on another host is followed by the node that owns it — this side only
                // relays, so there is nothing here to poll and nothing to keep warm.
                warm: convo::warmth(),
                warmth: self.node.convo_warmth.clone(),
                followed: convo::followed(),
                _hold: hold,
            },
        );
    }

    /// Anything addressed at a pane this process does not serve goes to the node that does,
    /// exactly as the client sent it. The peer's own session decides whether it is allowed and
    /// answers on its own error channel, so a relayed refusal reads like a local one.
    fn relay_to_peer(&self, pane: &str, message: Value) {
        if let Err(e) = self.node.peers.relay(pane, message) {
            self.wire.error(e.code(), &e.to_string(), Some(pane));
        }
    }

    /// Pages backwards through a transcript the pump already has open. A pane that is not watched
    /// with `conversation` has nothing to page, which is `not_found` rather than `unsupported` —
    /// the node implements the op.
    /// A page of a conversation this pane's agent launched.
    ///
    /// **Containment is proved against the pane's own transcript, not against the handle.** The
    /// id arrives from the network, so the only thing that makes it safe is that the file it
    /// resolves to has to sit under the session tree of the transcript *this node* derived for
    /// this pane — which the request had no say in. Everything else is the ordinary refusal.
    fn convo_sub(&self, pane: &str, id: &str, before: Option<&str>) {
        if self.node.resolve(pane).is_none() {
            self.relay_to_peer(
                pane,
                json!({ "t": "convo.sub", "pane": pane, "id": id, "before": before }),
            );
            return;
        }
        let Some(transcript) = self.panes.get(pane).and_then(|handle| {
            let guard = handle.convo.lock().unwrap();
            guard.as_ref().map(|open| open.path().to_path_buf())
        }) else {
            self.wire
                .error(ErrorCode::NotFound, "this pane has no conversation", Some(pane));
            return;
        };
        match self
            .node
            .journals()
            .open_sub(id, &transcript)
            .map(|opened| crate::pasted::Shown::over(opened, &self.node.state_dir))
        {
            Ok(mut journal) => {
                if journal.poll().is_err() {
                    self.wire
                        .error(ErrorCode::NotFound, "that conversation is unreadable", Some(pane));
                    return;
                }
                let page = journal.page_before(before, SUB_PAGE);
                if !self.wire.send(&ServerMsg::convo_sub(pane, id, page)) {
                    return;
                }
                // Kept, so the pump follows it while the reader has it open. One at a time: asking
                // for another replaces this, which is what closing one and opening the next does.
                if let Some(handle) = self.panes.get(pane) {
                    *handle.followed.lock().unwrap() = Some((id.to_string(), journal));
                }
            }
            Err(e) => {
                debug!(pane = %pane, error = %e, "refusing a launched conversation");
                self.wire
                    .error(ErrorCode::NotFound, "no such conversation", Some(pane));
            }
        }
    }

    fn convo_load(&self, pane: &str, before: Option<&str>) {
        // A transcript lives on the host that runs the harness, so paging one is a question for
        // the node that owns the pane rather than for the hub relaying it.
        if self.node.resolve(pane).is_none() {
            self.relay_to_peer(pane, json!({ "t": "convo.load", "pane": pane, "before": before }));
            return;
        }
        let page = self
            .panes
            .get(pane)
            // Older turns from the transcript already on the screen: a page that merges.
            .and_then(|handle| convo::page(&handle.convo, pane, before, false));
        match page {
            Some(page) => {
                self.wire.send(&page);
            }
            None => {
                self.wire.error(
                    ErrorCode::NotFound,
                    "no conversation open for this pane",
                    Some(pane),
                );
            }
        }
    }

    /// The three verbs a hub uses to pull an attachment off this node: ask, grant, cancel.
    ///
    /// Everything expensive is on a task of its own. `att.fetch` walks project directories and
    /// reads a transcript, and doing that here would stop this node answering the hub's keystrokes
    /// for as long as it took.
    fn attachment(&mut self, message: &Value) {
        let Some(rid) = message["rid"].as_u64() else {
            return;
        };
        match message["t"].as_str().unwrap_or_default() {
            "att.more" => {
                if let Some(sending) = self.sending.get(&rid) {
                    let _ = sending.credit.try_send(message["n"].as_u64().unwrap_or(1) as u32);
                }
            }
            "att.stop" => {
                self.sending.remove(&rid);
            }
            _ => self.send_attachment(rid, message),
        }
    }

    fn send_attachment(&mut self, rid: u64, message: &Value) {
        let (Some(pane), Some(id)) = (message["pane"].as_str(), message["id"].as_str()) else {
            return;
        };
        // The same line the HTTP route holds: an id that names a path names *any* path on this
        // machine, so it is answered only for a hub whose device may send input here — one that has
        // been demoted to read-only gets the single refusal, which is what the route it is relaying
        // for gives too.
        //
        // **Every path-shaped variant, not the one that existed first.** This is #304's shape: a
        // verb that reaches the filesystem while the gate in front of it enumerates one case.
        let decoded = kampr_journal::Source::decode(id);
        let file = match &decoded {
            Ok(kampr_journal::Source::File(file)) => Some(file.clone()),
            _ => None,
        };
        let path_form = matches!(
            decoded,
            Ok(kampr_journal::Source::File(_) | kampr_journal::Source::Diff(_))
        );
        if path_form && !self.device.role.writes() {
            self.audit_refused(
                "att.fetch",
                Some(pane),
                ErrorCode::NotWriter,
                json!({ "rid": rid }),
            );
            self.wire
                .send_json(&json!({ "t": "att.error", "rid": rid, "code": "not_found" }));
            return;
        }
        self.sending.retain(|_, sending| !sending.credit.is_closed());
        if self.sending.len() >= CONCURRENT_TRANSFERS {
            debug!(
                rid,
                "refusing an attachment: this node is already sending as many as it will"
            );
            self.wire
                .send_json(&json!({ "t": "att.error", "rid": rid, "code": "busy" }));
            return;
        }
        let window = message["window"]
            .as_u64()
            .unwrap_or(1)
            .clamp(1, u64::from(kampr_mesh::ATT_WINDOW)) as u32;
        let (credit, granted) = mpsc::channel(kampr_mesh::ATT_WINDOW as usize);
        let task = tokio::spawn(pump_attachment(AttachmentCtx {
            node: self.node.clone(),
            outbox: self.wire.outbox().clone(),
            rid,
            pane: pane.to_string(),
            id: id.to_string(),
            file,
            window,
            granted,
        }));
        self.sending.insert(
            rid,
            Sending {
                credit,
                _task: crate::mesh::AbortOnDrop(task.abort_handle()),
            },
        );
    }

    fn unwatch(&mut self, pane: &str) {
        // Before the handle, because a pane nobody is watching is a pane nobody has a terminal
        // view of — and a hold that outlived its view is exactly the write rule 3 forbids.
        self.matched.remove(pane);
        if let Some(handle) = self.panes.remove(pane) {
            handle.stop();
            self.audit("unwatch", Some(pane), None);
        }
    }

    async fn input(
        &mut self,
        pane: &str,
        text: Option<String>,
        b64: Option<String>,
        keys: Option<Vec<String>>,
    ) {
        if !self.may_write("input", Some(pane)) {
            return;
        }
        let Some((session, local)) = self.node.resolve(pane) else {
            self.audit("input", Some(pane), Some(json!({ "peer": true })));
            self.relay_to_peer(
                pane,
                json!({ "t": "input", "pane": pane, "text": text, "b64": b64, "keys": keys }),
            );
            return;
        };
        let supplied = [text.is_some(), b64.is_some(), keys.is_some()]
            .iter()
            .filter(|s| **s)
            .count();
        if supplied != 1 {
            self.wire.error(
                ErrorCode::BadRequest,
                "input takes exactly one of text, b64, keys",
                Some(pane),
            );
            return;
        }
        let input = match (text, b64, keys) {
            (Some(text), _, _) => Input::Bytes(text.into_bytes()),
            // `b64` is a convenience for control characters, not a raw-byte escape hatch:
            // `pane.send_text` takes a JSON string, so bytes that are not valid UTF-8 have no
            // representation on the wire to herdr and are refused rather than mangled.
            (_, Some(b64), _) => {
                match base64::engine::general_purpose::STANDARD
                    .decode(&b64)
                    .ok()
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                {
                    Some(text) => Input::Bytes(text.into_bytes()),
                    None => {
                        self.wire.error(
                            ErrorCode::BadRequest,
                            "b64 must decode to valid UTF-8",
                            Some(pane),
                        );
                        return;
                    }
                }
            }
            (_, _, Some(keys)) => Input::Keys(keys),
            _ => unreachable!("exactly one was supplied"),
        };
        // How much was typed, never what. `keys` is herdr's key grammar and probe #7 says that
        // grammar includes single characters, so recording the names typed a password into the
        // one file an operator hands to somebody else during an investigation.
        let detail = match &input {
            Input::Bytes(bytes) => json!({ "bytes": bytes.len() }),
            Input::Keys(keys) => json!({ "keys": keys.len() }),
        };
        self.audit("input", Some(pane), Some(detail));
        if let Err(e) = session.registry.write(&local, input).await {
            self.wire
                .error(offline_code(&session), &e.to_string(), Some(pane));
        }
    }

    /// Bytes from a client, written to a file on the pane's own node and typed in as its path.
    ///
    /// **This is typing, and it is gated as typing** — the same `may_write` and the same
    /// dispatch-time device refresh, because the alternative is the shape of #304: a verb that
    /// reaches the filesystem while the arm that re-reads the device row does not cover it.
    ///
    /// A pane on a peer is relayed rather than written here. The file has to land on the machine
    /// the harness will read it from, so the frame travels and that node writes it — a path this
    /// node chose would name nothing on the machine that matters.
    async fn paste(&mut self, pane: &str, b64: &str, name: Option<&str>) {
        if !self.may_write("paste", Some(pane)) {
            return;
        }
        if self.node.resolve(pane).is_none() {
            self.audit("paste", Some(pane), Some(json!({ "peer": true })));
            self.relay_to_peer(
                pane,
                json!({ "t": "paste", "pane": pane, "b64": b64, "name": name }),
            );
            return;
        }
        // The length is read off the base64 before anything is allocated, so a body claiming a
        // gigabyte costs a comparison rather than a gigabyte.
        if kampr_journal::attach::decoded_len(b64) > kampr_journal::attach::MAX_BYTES {
            self.wire.error(
                ErrorCode::BadRequest,
                "that is larger than this node will take",
                Some(pane),
            );
            return;
        }
        let Ok(body) = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .or_else(|_| base64::engine::general_purpose::STANDARD_NO_PAD.decode(b64))
        else {
            self.wire
                .error(ErrorCode::BadRequest, "the body was not base64", Some(pane));
            return;
        };
        let written = crate::paste::write(&self.node.state_dir, &body, name);
        let path = match written {
            Ok(path) => path,
            Err(e) => {
                self.wire.error(ErrorCode::BadRequest, &e.to_string(), Some(pane));
                return;
            }
        };
        self.audit(
            "paste",
            Some(pane),
            Some(json!({ "bytes": body.len(), "path": path.display().to_string() })),
        );
        self.input(pane, Some(crate::paste::typed(&path)), None, None)
            .await;
    }

    async fn answer(&mut self, pane: &str, key: &str) {
        if !self.may_write("answer", Some(pane)) {
            return;
        }
        let Some((session, local)) = self.node.resolve(pane) else {
            self.audit("answer", Some(pane), Some(json!({ "key": key, "peer": true })));
            // The submit key is per harness and the *owning* node knows which harness this pane
            // is running, so the answer is relayed as an answer rather than pre-expanded here.
            self.relay_to_peer(pane, json!({ "t": "answer", "pane": pane, "key": key }));
            return;
        };
        if key.is_empty() || key.chars().count() > 2 {
            self.wire.error(
                ErrorCode::BadRequest,
                "an answer key is one or two characters",
                Some(pane),
            );
            return;
        }
        // The node decides whether a submit key follows, per harness — the client sends only the
        // key it was offered.
        let agent = self.node.herd().pane(pane).and_then(|p| p.agent.clone());
        self.audit("answer", Some(pane), Some(json!({ "key": key })));
        let mut keystrokes = vec![key.to_string()];
        keystrokes.extend(submit_key(agent.as_deref()).map(str::to_string));
        for stroke in keystrokes {
            if let Err(e) = session
                .registry
                .write(&local, Input::Bytes(stroke.into_bytes()))
                .await
            {
                self.wire
                    .error(offline_code(&session), &e.to_string(), Some(pane));
                return;
            }
        }
    }

    /// Commits a question that takes several answers, after the operator has ticked what they want.
    ///
    /// The keys are the node's because they are a measurement (#421): on Claude a digit toggles a
    /// checkbox and Enter toggles the *focused* row rather than submitting — the dialog draws its
    /// own `Submit` control in a header row, reached sideways — so the sequence is right-arrow and
    /// then Enter. A harness nobody has measured this on is refused rather than guessed at, which
    /// leaves the operator answering at the pane instead of pressing something that does nothing.
    async fn answer_submit(&mut self, pane: &str) {
        if !self.may_write("answer.submit", Some(pane)) {
            return;
        }
        let Some((session, local)) = self.node.resolve(pane) else {
            self.audit("answer.submit", Some(pane), Some(json!({ "peer": true })));
            self.relay_to_peer(pane, json!({ "t": "answer.submit", "pane": pane }));
            return;
        };
        let agent = self.node.herd().pane(pane).and_then(|p| p.agent.clone());
        let Some(keys) = commit_keys(agent.as_deref()) else {
            self.wire.error(
                ErrorCode::BadRequest,
                "nobody has measured what commits a multiple-answer question on this harness, so \
                 it has to be submitted at the pane",
                Some(pane),
            );
            return;
        };
        self.audit("answer.submit", Some(pane), Some(json!({ "agent": agent })));
        for stroke in keys {
            if let Err(e) = session
                .registry
                .write(&local, Input::Bytes(stroke.as_bytes().to_vec()))
                .await
            {
                self.wire
                    .error(offline_code(&session), &e.to_string(), Some(pane));
                return;
            }
        }
    }

    /// Every watched pane restarts, which is exactly what the protocol promises: a fresh `herd`
    /// and one `grid.reset` per pane, with no patch that could reference state the client threw
    /// away.
    async fn resync(&mut self) {
        self.wire.send(&self.node.herd().message());
        let watched: Vec<(String, bool, bool)> = self
            .panes
            .iter()
            .map(|(id, h)| (id.clone(), h.scrollback, h.conversation))
            .collect();
        for (pane, scrollback, conversation) in watched {
            self.watch(&pane, scrollback, conversation).await;
        }
    }

    async fn manage(&mut self, value: Value) {
        // `rid` is an opaque correlation token: a caller that sends one gets it back on the
        // `managed` ack. A browser never needs it; a hub relaying for several clients at once
        // does, and it is additive to the protocol either way.
        let rid = value.get("rid").cloned();
        let mut raw = value.clone();
        let Ok(mut op) = serde_json::from_value::<ManageOp>(value) else {
            self.refuse(
                raw["op"].as_str().unwrap_or_default(),
                None,
                ErrorCode::BadRequest,
                "unreadable manage op",
                rid.as_ref(),
            );
            return;
        };
        // A refused op is acknowledged too: a client that clears its in-flight state on the ack —
        // which is what the protocol tells it to do — hangs forever on an `error` alone.
        if !self.device.role.writes() {
            self.audit_refused(
                "manage",
                op.at.as_deref(),
                ErrorCode::NotWriter,
                json!({ "op": op.op }),
            );
            self.refuse(
                &op.op,
                op.at.as_deref(),
                ErrorCode::NotWriter,
                "this device is read-only",
                rid.as_ref(),
            );
            return;
        }
        // `op` and a label say that something ran. `cwd`, `env`, `args`, `path` and `branch` are
        // what it ran, where, and against which tree — which is the whole question the log exists
        // to answer.
        self.audit("manage", op.at.as_deref(), Some(manage_detail(&op)));
        // **Before routing, deliberately.** The book is this node's and names no host, so a
        // `fleet.save` carrying a `node` must not be relayed to it — the operator's list would
        // then live on whichever machine their last pane happened to be on. It reaches herdr for
        // nothing either, so it must not wake one.
        if crate::book::is_book_op(&op.op) {
            let result = crate::book::apply(self.node.auth.store(), &op, kampr_auth::now()).await;
            match result {
                Ok(reply) => {
                    let mut ack = json!({ "t": "managed", "op": op.op, "ok": true });
                    if let Some(entry) = reply["entry"].as_str() {
                        ack["id"] = json!(entry);
                    }
                    if let Some(rid) = &rid {
                        ack["rid"] = rid.clone();
                    }
                    self.wire.send_json(&ack);
                    self.node.book_changed();
                }
                Err(e) => self.refuse(&op.op, None, e.code(), &e.to_string(), rid.as_ref()),
            }
            return;
        }
        // **A release is scoped to this view's own hold, and only this view's.** The panel's
        // release still lets go of whatever is standing — that is an operator asking by name —
        // but a terminal view turning matching off is answering for its own claim, and a viewer
        // that has already been displaced by a newer one must not take the newer one's hold down
        // with it on the way past.
        if op.op == "pane.size"
            && op.mode.as_deref() == Some("release")
            && let Some(token) = op
                .at
                .as_deref()
                .and_then(|at| self.matched.get(at))
                .map(MatchLease::token)
        {
            op.lease = Some(token);
            raw["lease"] = json!(token);
        }

        // Every session is its own herdr server, so an op is addressed at the session that owns
        // its target rather than at whichever socket the node happens to have started with.
        let target = op.at.as_deref().or(op.node.as_deref());
        let session = match target.and_then(|t| self.node.route(t)) {
            Some(session) => session,
            None if target.is_none() => self.node.primary(),
            None => {
                self.manage_peer(target.unwrap_or_default(), &op, raw, rid).await;
                return;
            }
        };
        let manager = Manager {
            herdr: &session.herdr,
            node_id: &session.node_id,
            binary: &self.node.config.herdr.binary,
            holds: &self.node.holds,
            fleet: &session.fleet,
            provider: &session.provider,
        };
        match manager.run(&op).await {
            Ok(Managed { reply, settle }) => {
                // A session is named rather than addressed, so its name is the id; everything
                // else herdr creates is a container this node qualifies with its own id.
                let id = match reply["session"].as_str() {
                    Some(name) => Some(name.to_string()),
                    None => manage::created_id(&op.op, &reply).map(|id| session.global_pane(&id)),
                };
                let mut ack = json!({ "t": "managed", "op": op.op, "ok": true });
                if let Some(id) = id {
                    ack["id"] = json!(id);
                }
                if op.op == "layout.export" {
                    ack["layout"] = reply["layout"].clone();
                }
                // What the op measured, which is not the same thing as whether it ran. `pane.size`
                // claims the PTY, resizes, releases and then *checks* — and an ack that carried
                // only `ok` said `true` about a resize herdr had already taken back (#19).
                for (field, value) in manage::measured(&op.op, &reply) {
                    ack[field] = value;
                }
                self.note_match(&op, &reply);
                if let Some(rid) = &rid {
                    ack["rid"] = rid.clone();
                }
                match settle {
                    Some(settle) => self.spawn_settle(settle, &op, ack, rid),
                    None => {
                        self.wire.send_json(&ack);
                    }
                }
                self.note_fleet_run(&op).await;
            }
            Err(e) => self.refuse(&op.op, op.at.as_deref(), e.code(), &e.to_string(), rid.as_ref()),
        }
    }

    /// A `fleet.run` the node accepted, written into this node's book.
    ///
    /// **On this node's book whether the run is local or relayed**, because a fan-out is one op
    /// per host and the operator has one list. Deduplication is what collapses the four ops one
    /// run produces into the one entry; the book is a list of commands, not of runs.
    ///
    /// After the ack rather than before it, so an op the node refused — an empty argv, a missing
    /// cohort — is not written down as a command that ran.
    async fn note_fleet_run(&self, op: &ManageOp) {
        if op.op != "fleet.run" {
            return;
        }
        if crate::book::record_run(self.node.auth.store(), op, kampr_auth::now()).await {
            self.node.book_changed();
        }
    }

    /// A session op's ack waits for the host to catch up, and that wait is up to five seconds
    /// long — but `dispatch` is sequential, so serving it here means this socket answers nothing
    /// else in the meantime: no input to any other pane, no watch, no caps, because somebody
    /// stopped a session. The herdr call itself stays inline and in dispatch order, so two
    /// creates still cannot race; only the waiting moves, and the ack still follows its *own*
    /// settle.
    ///
    /// Acks of different ops may therefore interleave. The wire already allows for that — `rid`
    /// exists because a hub relays for several clients at once — and the client correlates a
    /// `managed` by its `op`, never by arrival order.
    fn spawn_settle(&mut self, settle: Settle, op: &ManageOp, ack: Value, rid: Option<Value>) {
        let node = self.node.clone();
        let wire = self.wire.clone();
        let name = op.op.clone();
        let at = op.at.clone();
        let task = tokio::spawn(async move {
            match settle.wait().await {
                Ok(()) => {
                    // A named session is its own node, and the discovery loop would otherwise
                    // take up to `DISCOVERY_POLL` to notice one appear or go. The ack is what a
                    // client acts on, so it has to mean the herd already knows — this is the op
                    // that changed the session set, and it is the only thing in the process that
                    // knows it did.
                    //
                    // **And a reconcile that could not read the host has not made that true.** The
                    // session list is a spawned process; one that times out under load leaves the
                    // herd naming a session that has stopped, and nothing else would look again
                    // for another `DISCOVERY_POLL`. So this retries rather than acking a promise
                    // it did not keep.
                    reconcile_hard(&node).await;
                    wire.send_json(&ack);
                }
                Err(e) => refuse_on(
                    &wire,
                    &name,
                    at.as_deref(),
                    e.code(),
                    &e.to_string(),
                    rid.as_ref(),
                ),
            }
        });
        // **Detached on purpose.** `reconcile` and the `sessions_changed` inside the settle are
        // node-wide truths, not this socket's: they are what stops every *other* client seeing the
        // session set the op already changed. Tying the task to the socket that asked would mean a
        // client hanging up inside the 52-303 ms window (#241) left the herd stale for everyone
        // until the next discovery poll. The wait is bounded by `SESSION_SETTLE` and the ack lands
        // in an outbox that is already gone, which costs nothing.
        drop(task);
    }

    fn refuse(&self, op: &str, at: Option<&str>, code: ErrorCode, message: &str, rid: Option<&Value>) {
        refuse_on(&self.wire, op, at, code, message, rid);
    }

    /// A structural op against a pane on another host. Unlike input this one has an answer, so
    /// the hub waits for the peer's `managed` and hands it back — with the *caller's* correlation
    /// token, never the one the hub minted for its own bookkeeping.
    async fn manage_peer(&mut self, target: &str, op: &ManageOp, mut raw: Value, rid: Option<Value>) {
        if let Some(object) = raw.as_object_mut() {
            object.remove("rid");
        }
        let mut reply = match self.node.peers.manage(target, raw).await {
            Ok(reply) => reply,
            Err(kampr_mesh::RelayError::Unknown(_)) => {
                let message = format!("{target} is not on a node this herd serves");
                self.refuse(
                    &op.op,
                    op.at.as_deref(),
                    ErrorCode::UnknownPane,
                    &message,
                    rid.as_ref(),
                );
                return;
            }
            Err(e) => {
                self.refuse(&op.op, op.at.as_deref(), e.code(), &e.to_string(), rid.as_ref());
                return;
            }
        };
        match rid {
            Some(rid) => reply["rid"] = rid,
            None => {
                if let Some(object) = reply.as_object_mut() {
                    object.remove("rid");
                }
            }
        }
        let ran = reply["ok"] == true;
        if ran {
            self.note_peer_match(target, op, &reply);
        }
        self.wire.send_json(&reply);
        if ran {
            self.note_fleet_run(op).await;
        }
    }

    /// Takes ownership of a matched hold the node just took on this client's behalf, and gives one
    /// up when the client says it is done with the view.
    ///
    /// Keyed by the *global* pane id because that is what an `unwatch` and a `release` name; the
    /// local id inside the lease is what herdr and [`crate::holds::PaneHolds`] are keyed by.
    fn note_match(&mut self, op: &ManageOp, reply: &Value) {
        if op.op != "pane.size" {
            return;
        }
        let Some(at) = op.at.clone() else { return };
        match (reply["lease"].as_u64(), reply["pane_id"].as_str()) {
            (Some(token), Some(local)) => {
                self.matched.insert(
                    at,
                    MatchLease::Local {
                        node: self.node.clone(),
                        local: local.to_string(),
                        token,
                    },
                );
            }
            // A `release` this session's own lease answered for. Dropping it here rather than
            // before the op is what lets the ack say `was_held: true` honestly.
            _ if op.mode.as_deref() == Some("release") => {
                self.matched.remove(&at);
            }
            _ => {}
        }
    }

    fn note_peer_match(&mut self, target: &str, op: &ManageOp, reply: &Value) {
        if op.op != "pane.size" {
            return;
        }
        let Some(at) = op.at.clone() else { return };
        match reply["lease"].as_u64() {
            Some(token) => {
                self.matched.insert(
                    at.clone(),
                    MatchLease::Peer {
                        node: self.node.clone(),
                        target: target.to_string(),
                        global: at,
                        token,
                    },
                );
            }
            None if op.mode.as_deref() == Some("release") => {
                self.matched.remove(&at);
            }
            None => {}
        }
    }

    async fn caps(&self) {
        self.wire.send_json(&self.node.caps().await);
    }

    /// Per-pane render preferences, kept per device so a phone and a desktop can disagree about
    /// zoom on the same pane. Read-only devices keep theirs too — refusing them here would cost a
    /// real feature and fix nothing, because the defect is the unbounded write, not the role.
    ///
    /// So both bounds are on the write itself: the pane has to be one this node actually serves,
    /// and the blob has to fit. Otherwise any device, of any role, fills the disk one arbitrary
    /// pane id at a time.
    async fn prefs(&self, value: &Value) {
        if let (Some(pane), Some(Value::Object(incoming))) = (value["pane"].as_str(), value.get("prefs")) {
            if self.node.herd().pane(pane).is_none() {
                self.wire
                    .error(ErrorCode::UnknownPane, "no such pane on this node", Some(pane));
                return;
            }
            let stored = self.stored_prefs().await;
            let mut merged = match &stored["panes"][pane] {
                Value::Object(existing) => existing.clone(),
                _ => serde_json::Map::new(),
            };
            // A write names the keys it is changing and nothing else — a client that sets zoom
            // must not thereby forget the view. `null` is how one key is cleared, since a merge
            // leaves no other way back.
            for (key, value) in incoming {
                match value {
                    Value::Null => {
                        merged.remove(key);
                    }
                    value => {
                        merged.insert(key.clone(), value.clone());
                    }
                }
            }
            let merged = Value::Object(merged);
            // The bound is on what is stored, not on what arrived: merging is what can grow it.
            if merged.to_string().len() > MAX_PREFS_BYTES {
                self.wire.error(
                    ErrorCode::BadRequest,
                    "preferences for one pane must fit in 2 KiB",
                    Some(pane),
                );
                return;
            }
            let _ = self
                .node
                .auth
                .store()
                .set_pane_prefs(&self.device.id, pane, &merged, kampr_auth::now())
                .await;
        }
        self.wire.send_json(&self.stored_prefs().await);
    }
}

/// The `managed` ack and the `error` frame that follows it, in the order the protocol names
/// them, with the caller's correlation token on the ack wherever there is one.
/// Reconciles until the host actually answers, or until the budget is spent.
///
/// Bounded rather than patient: an ack held open for ever is worse than one that arrives with the
/// herd a poll behind, and `DISCOVERY_POLL` is still the backstop. Four attempts over roughly a
/// second covers a list that lost one race with a loaded machine, which is the whole of the
/// failure this exists for.
async fn reconcile_hard(node: &Node) {
    const TRIES: usize = 4;
    const GAP: Duration = Duration::from_millis(250);
    for attempt in 0..TRIES {
        if node.sessions.reconcile().await {
            return;
        }
        if attempt + 1 < TRIES {
            tokio::time::sleep(GAP).await;
        }
    }
    tracing::warn!("could not read the session list after a session op; the herd is a poll behind");
}

fn refuse_on(wire: &Wire, op: &str, at: Option<&str>, code: ErrorCode, message: &str, rid: Option<&Value>) {
    let mut ack = json!({ "t": "managed", "op": op, "ok": false,
                          "code": code, "message": message });
    if let Some(rid) = rid {
        ack["rid"] = rid.clone();
    }
    wire.send_json(&ack);
    wire.error(code, message, at);
}

fn manage_detail(op: &ManageOp) -> Value {
    let mut detail = json!({ "op": op.op });
    let fields: [(&str, Value); 15] = [
        ("node", json!(op.node)),
        ("entry", json!(op.entry)),
        ("label", json!(op.label)),
        ("cwd", json!(op.cwd)),
        ("env", op.env.clone().unwrap_or(Value::Null)),
        ("direction", json!(op.direction)),
        ("mode", json!(op.mode)),
        ("kind", json!(op.kind)),
        ("name", json!(op.name)),
        ("args", json!(op.args)),
        ("branch", json!(op.branch)),
        ("base", json!(op.base)),
        ("path", json!(op.path)),
        ("cols", json!(op.cols)),
        ("rows", json!(op.rows)),
    ];
    for (key, value) in fields {
        if !value.is_null() {
            detail[key] = value;
        }
    }
    detail
}

fn wire_role(role: Role) -> kampr_core::wire::Role {
    match role {
        Role::Full => kampr_core::wire::Role::Full,
        Role::Readonly => kampr_core::wire::Role::Readonly,
    }
}

fn hello(node: &Node, device: &Device, caller: Caller) -> Value {
    let hello = ServerMsg::Hello(kampr_core::wire::Hello {
        protocol: PROTOCOL,
        node_id: node.config.node_id.clone(),
        node_name: node.config.node_name.clone(),
        build: BUILD.to_string(),
        role: wire_role(device.role),
        caps: kampr_core::wire::Caps {
            // Reality, not intent. Push needs a secure context *and* a VAPID key, and a client
            // that trusts this to hide the control must never see it claimed where it cannot
            // work — `security.push` says whether the origin allows it, this says whether the
            // node can actually do it.
            push: node.push.available(),
            scrollback: true,
            // Both this and every pane's `has_conversation` are answered from the same adapter
            // registry, so a pane can never claim a conversation the node cannot serve.
            conversation: node.journals().serves_any(),
        },
    });
    let mut value = serde_json::to_value(hello).expect("hello serialises");
    // `manage` is not on the v1 `Caps` struct and `security` is not on `hello` at all; both are
    // additive, and unknown fields are ignored by construction.
    value["caps"]["manage"] = json!(true);
    // A client that knows about the mesh can show a per-node latency and a version skew; one that
    // does not ignores the field and sees a herd it cannot tell apart, which is still correct.
    value["caps"]["mesh"] = json!(node.config.mesh.accept);
    // A hub reads this to decide whether it may keep an `att` on a block it relays. It is said
    // only to a hub because `att.fetch` is answered only for one: a browser has the HTTP route,
    // and the point of that route is that bytes never share a queue with terminal frames.
    if caller == Caller::Hub {
        value["caps"]["attachments"] = json!(true);
    }
    let tier = node.auth.tier();
    // A browser needs the application server key before it may call `pushManager.subscribe`, and
    // it is a public value — sending it with `hello` saves a round trip on the one path where a
    // round trip is most likely to be interrupted.
    if let Some(key) = node.push.public_key() {
        value["push"] = json!({ "key": key });
    }
    value["security"] = json!({
        "tier": tier.tier,
        "origin": tier.origin,
        "encrypted": tier.secure_context,
        "unencrypted_banner": !tier.secure_context,
        "passkeys": tier.passkeys,
        "push": tier.push,
        "installable": tier.installable,
        "unlocks": tier.locked(),
    });
    value["device"] = json!({
        "id": device.id,
        "name": device.name,
        "expires_at": device.expires_at,
    });
    value
}

/// A herd outage has to be *visible*: without this a client watching a pane through a herdr that
/// died just froze, with `online` still true and no error at all (probe #70).
async fn herd_updates(
    mut rx: tokio::sync::watch::Receiver<Arc<crate::herd::HerdModel>>,
    mut last: Arc<crate::herd::HerdModel>,
    wire: Arc<Wire>,
) {
    loop {
        if rx.changed().await.is_err() {
            return;
        }
        let current = rx.borrow_and_update().clone();
        for (id, online) in current.reachability_changes(&last) {
            let sent = if online {
                // The protocol's own recovery rule: a reconnect re-sends the whole herd, so a
                // client that gave up on patches lands back on solid ground.
                wire.send(&current.message())
            } else {
                wire.node_error(
                    ErrorCode::HerdrUnavailable,
                    &current
                        .node(&id)
                        .and_then(|n| n.detail.clone())
                        .unwrap_or_else(|| format!("{id} is not reachable")),
                    &id,
                ) && wire.node_error(ErrorCode::NodeOffline, &format!("{id} is offline"), &id)
            };
            if !sent {
                return;
            }
        }
        if let Some(patch) = current.diff(&last)
            && !wire.send(&patch)
        {
            return;
        }
        last = current;
    }
}

fn pane_fault(herd: &crate::herd::HerdModel, pane: &str) -> Option<String> {
    herd.pane(pane).and_then(|p| p.detail.clone())
}

/// A write that failed while the session is down is the herd being unreachable, not a bad pane.
fn offline_code(session: &crate::sessions::SessionNode) -> ErrorCode {
    if session.online() {
        ErrorCode::HerdrUnavailable
    } else {
        ErrorCode::NodeOffline
    }
}

struct AttachmentCtx {
    node: Arc<Node>,
    outbox: Arc<Outbox>,
    rid: u64,
    pane: String,
    id: String,
    /// Set when the id named a path rather than a record, and already gated on the hub's role by
    /// the session that decoded it.
    file: Option<kampr_journal::FileRef>,
    window: u32,
    granted: mpsc::Receiver<u32>,
}

/// One attachment, chunked towards a hub at the rate the hub asks for it.
///
/// **The ceiling is the transcript's own.** `attach::fetch` reads the decoded length off the
/// record's base64 and refuses past 8 MiB *before* decoding, so a record claiming a gigabyte costs
/// a comparison here exactly as it does on the local HTTP route — this path adds no second
/// judgement about size and inherits that one.
///
/// **Chunks go down the bulk lane and everything else does not.** A frame enqueued while this is
/// running overtakes every chunk already queued, so a pane on this link repaints during a transfer
/// rather than after it. `att.end` rides the same lane as the chunks, or it would arrive first.
async fn pump_attachment(ctx: AttachmentCtx) {
    let AttachmentCtx {
        node,
        outbox,
        rid,
        pane,
        id,
        file,
        window,
        mut granted,
    } = ctx;
    let refuse = |code: &str| {
        outbox.push(Frame::plain(
            json!({ "t": "att.error", "rid": rid, "code": code }).to_string(),
        ));
    };
    // The same two file reads the local route makes, on the same blocking pool and for the same
    // reason: a miss walks every project directory and reads both ends of up to 64 transcripts.
    let read = tokio::task::spawn_blocking({
        let node = node.clone();
        move || {
            // A path has no transcript behind it, so the lookup that resolves one is skipped
            // rather than made and ignored — a pane with no agent on it can still serve a file.
            if let Some(file) = file {
                return Some(file.fetch(&node.config.journal_home()));
            }
            let transcript = crate::http::transcript_of(&node, &pane)?;
            Some(kampr_journal::attach::fetch(&node.journals(), &id, &transcript))
        }
    })
    .await;
    let found = match read {
        Ok(Some(Ok(found))) => found,
        Ok(Some(Err(kampr_journal::JournalError::TooLarge(bytes)))) => {
            debug!(rid, bytes, "refusing a relayed attachment past the ceiling");
            return refuse("too_large");
        }
        // Every other refusal is one answer here for the same reason it is one answer over HTTP:
        // an escape, a stale id and an id for somebody else's transcript are the same sentence.
        Ok(other) => {
            debug!(rid, found = other.is_some(), "refusing a relayed attachment");
            return refuse("not_found");
        }
        Err(_) => return refuse("not_found"),
    };

    let mut open = json!({
        "t": "att.open", "rid": rid, "bytes": found.data.len(), "kind": found.kind,
    });
    if let Some(mime) = &found.mime {
        open["mime"] = json!(mime);
    }
    if let Some(name) = &found.name {
        open["name"] = json!(name);
    }
    if !outbox.push(Frame::plain(open.to_string())) {
        return;
    }
    let mut credit = window;
    for (seq, chunk) in found.data.chunks(kampr_mesh::ATT_CHUNK_BYTES).enumerate() {
        while credit == 0 {
            // `None` is the hub cancelling or the session ending. Either way nobody wants the
            // rest of these bytes.
            let Some(more) = granted.recv().await else { return };
            credit += more;
        }
        credit -= 1;
        let frame = json!({
            "t": "att.chunk", "rid": rid, "seq": seq,
            "b64": base64::engine::general_purpose::STANDARD.encode(chunk),
        });
        if !outbox.push_bulk(Frame::plain(frame.to_string())) {
            return;
        }
    }
    outbox.push_bulk(Frame::plain(json!({ "t": "att.end", "rid": rid }).to_string()));
}

/// Everything one pane's stream needs, and nothing else — so the pump can be driven in a test
/// against a fake provider rather than only against a live herd.
pub struct PaneStreamCtx {
    pub registry: Arc<kampr_core::PaneRegistry>,
    pub herdr: kampr_herdr::Herdr,
    pub herd: tokio::sync::watch::Receiver<Arc<crate::herd::HerdModel>>,
    pub wire: Arc<Wire>,
    pub global: String,
    pub local: String,
    pub scrollback: bool,
}

/// One pane's stream for one client.
///
/// The backpressure rule lives here: before encoding an update, the pump asks whether the client
/// is keeping up. If it is not, this pane's queued grid frames are dropped and the pane rejoins at
/// a fresh `grid.reset` — never a queue of patches the client can never drain. Re-joining also
/// clears the registry's own broadcast backlog, so the two bounds do not fight each other.
pub async fn pump_pane(ctx: PaneStreamCtx) {
    let PaneStreamCtx {
        registry,
        herdr,
        mut herd,
        wire,
        global,
        local,
        scrollback,
    } = ctx;
    let mut watcher = match registry.watch(&local).await {
        Ok(w) => w,
        Err(e) => {
            wire.error(ErrorCode::HerdrUnavailable, &e.to_string(), Some(&global));
            return;
        }
    };
    if watcher.is_ready() && !wire.send_update(&global, watcher.initial()) {
        return;
    }

    // A pane that cannot be streamed carries its reason in the herd, and the herd is state rather
    // than news: a client joining a pane that has been unstreamable for a week would otherwise be
    // handed the state with nothing saying it had arrived. So the state is read once here and the
    // transitions are announced below, which is exactly how an outage is already told.
    let mut fault = pane_fault(&herd.borrow(), &global);
    if let Some(detail) = &fault
        && !wire.error(ErrorCode::StreamUnavailable, detail, Some(&global))
    {
        return;
    }

    // **The state a watcher arrives into, not only the changes after it.** `blocked` used to start
    // `false` and the strip was published only on a *transition*, so opening a pane that was
    // already blocked showed nothing at all until the next one — and a dialog standing still
    // produces no provider revisions, so that wait was bounded by the herd's reconcile sweep at
    // thirty seconds. The operator's phone exists to answer blocked agents; it cannot be the last
    // thing to hear that one is. This is the same reading the fault above already does, and for
    // the same reason: the herd is state, and a client joining is arriving at it.
    let mut blocked = herd.borrow().pane(&global).map(|p| p.agent_status)
        == Some(kampr_core::provider::AgentStatus::Blocked);
    // A pane can be reported blocked before its dialog has finished painting, and the question is
    // read off the screen (probe #42). Latching on the first read would leave a blocked agent with
    // no prompt strip at all until its status happened to change again, so an unproductive read is
    // retried for a short while rather than accepted.
    let mut asking = 0u32;
    // What this socket has been told, so a re-read that finds the same dialog costs no frame.
    let mut asked: Option<pending::Pending> = None;
    let mut ask = tokio::time::interval(PENDING_RETRY);
    ask.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    if blocked {
        asking = PENDING_ATTEMPTS;
        // Exactly one screen read, and only for a pane herdr already calls blocked. An unblocked
        // pane costs nothing here: `send_pending` finds nothing, matches what this socket has
        // been told, and sends no frame — so this does not add a read or a frame per watch, and
        // #448's `multi` re-arm is left exactly as it was.
        match send_pending(&herdr, &wire, &global, &local, true, &mut asked).await {
            None => return,
            Some(published) => asking = keep_asking(published, &asked, asking),
        }
    }
    // The ring's own signal, so this socket is told when there is history to send instead of
    // asking herdr for it again on a timer. A pane with no accumulator behind it has none, and
    // nothing on a quiet pane ever fires: the old timer rendered the whole ring every three
    // seconds per socket whether or not a row had been added to it.
    let mut history_moved = registry.scrollback_changes(&local);
    // Armed, because the pane may already have a ring: a client joining a pane that has scrolled
    // is arriving at history, and there is no change coming to tell it so.
    let mut history_due = scrollback;
    let mut history_at = tokio::time::Instant::now();
    let mut sent_rows = 0u32;

    loop {
        tokio::select! {
            update = watcher.recv() => {
                // **Never a bare `return`.** This is the one place the node learns that a pane's
                // frames have stopped for a reason that is not the socket going away, and for as
                // long as it was silent the pane sat frozen on an old screen while its own
                // conversation kept answering over the same healthy connection — the whole of the
                // browser report behind #268. The pane is dropped either way; saying so is what
                // separates it from a pane nobody is typing in.
                let Ok(update) = update else {
                    wire.error(
                        ErrorCode::StreamUnavailable,
                        "this pane's frames stopped arriving; reopen it to restart the stream",
                        Some(&global),
                    );
                    return;
                };
                if wire.outbox().congested() {
                    let dropped = wire.outbox().purge_pane(&global);
                    debug!(pane = %global, dropped, "client is behind; resetting instead of buffering");
                    match registry.watch(&local).await {
                        Ok(fresh) => watcher = fresh,
                        Err(_) => return,
                    }
                    if !wire.send_update(&global, watcher.initial()) {
                        return;
                    }
                    continue;
                }
                if !wire.send_update(&global, &update) {
                    return;
                }
            }
            _ = ask.tick(), if asking > 0 => {
                asking -= 1;
                match send_pending(&herdr, &wire, &global, &local, true, &mut asked).await {
                    None => return,
                    Some(published) => asking = keep_asking(published, &asked, asking),
                }
            }
            // Two arms rather than one, and the split is what keeps the wake cancel-safe: the
            // first only *records* that the ring moved, so losing the race to another branch
            // cannot lose the news with it.
            _ = ring_moved(&mut history_moved), if scrollback && !history_due => {
                history_due = true;
            }
            _ = tokio::time::sleep_until(history_at), if scrollback && history_due => {
                history_due = false;
                match send_history(&registry, &wire, &global, &local, &mut sent_rows).await {
                    None => return,
                    // The floor is spent only on a frame that actually went. A render that
                    // found nothing new — the arm this loop starts with, before the ring has
                    // anything in it — must not push the first real one three seconds out.
                    Some(sent) => {
                        if sent {
                            history_at = tokio::time::Instant::now() + SCROLLBACK_FLOOR;
                        }
                    }
                }
            }
            changed = herd.changed() => {
                if changed.is_err() {
                    return;
                }
                let (status, detail) = {
                    let model = herd.borrow_and_update();
                    (model.pane(&global).map(|p| p.agent_status), pane_fault(&model, &global))
                };
                if detail != fault {
                    fault = detail;
                    // Recovery says nothing: the herd entry clearing is what takes the notice
                    // down, and an `error` frame has no form that means "never mind".
                    if let Some(detail) = &fault
                        && !wire.error(ErrorCode::StreamUnavailable, detail, Some(&global))
                    {
                        return;
                    }
                }
                let now_blocked = status == Some(kampr_core::provider::AgentStatus::Blocked);
                if now_blocked != blocked {
                    blocked = now_blocked;
                    asking = if blocked { PENDING_ATTEMPTS } else { 0 };
                    match send_pending(&herdr, &wire, &global, &local, blocked, &mut asked).await {
                        None => return,
                        Some(published) => asking = keep_asking(published, &asked, asking),
                    }
                }
            }
        }
    }
}

/// Waits for the pane's history ring to move.
///
/// **Never returns for a pane that has no ring behind it, and never again once the accumulator
/// has gone.** A `select!` arm that answered immediately would spin this loop for the life of the
/// stream, which is the shape a closed `watch` already produced once here (`state.rs`'s `moved`).
async fn ring_moved(rx: &mut Option<tokio::sync::watch::Receiver<u64>>) {
    if let Some(rx) = rx.as_mut()
        && rx.changed().await.is_ok()
    {
        return;
    }
    *rx = None;
    std::future::pending().await
}

/// `None` if the socket is gone; otherwise whether a frame actually went.
async fn send_history(
    registry: &kampr_core::PaneRegistry,
    wire: &Wire,
    global: &str,
    local: &str,
    sent_rows: &mut u32,
) -> Option<bool> {
    let Ok(Some(mut doc)) = registry.scrollback(local).await else {
        return Some(false);
    };
    // `total_rows` is a depth, so the ring ends here; `sent_rows` is the same index, one message
    // ago.
    let end = doc.from_top + doc.total_rows;
    if end == *sent_rows && doc.from_top <= *sent_rows {
        return Some(false);
    }
    // The ring restarted — a gap it could not stitch, or a width change — so the client's copy is
    // no longer adjacent to what the node holds and the whole thing goes again.
    if doc.from_top <= *sent_rows {
        doc.rows.retain(|r| r.row >= *sent_rows);
        doc.from_top = (*sent_rows).min(end);
        doc.total_rows = end - doc.from_top;
    }
    *sent_rows = end;
    wire.send_scrollback(global, &doc).then_some(true)
}

/// How many more times to re-read the screen, after one attempt at publishing a prompt.
///
/// **A multiple-answer dialog is the one that has to be re-read for as long as it stands**, and it
/// is the reason this is a function rather than an assignment at each of the two call sites — the
/// blocked *edge* zeroed the counter on a successful publish and the retry branch re-armed it, so
/// the shape that needed re-reading was the one shape that never got it. Every press on such a
/// dialog is a tick (#421), the checkboxes move under the operator, and nothing else on the wire
/// ever says which are ticked.
///
/// Every other dialog latches on its first successful read exactly as before: the bound exists so
/// a harness whose dialog Kampr cannot parse costs a handful of reads rather than one every half
/// second for as long as it sits there. And the re-read is free to the wire either way —
/// [`send_pending`] compares against what this socket was last told and sends nothing when the
/// screen has not moved.
fn keep_asking(published: bool, asked: &Option<pending::Pending>, asking: u32) -> u32 {
    match (published, asked.as_ref().is_some_and(|p| p.multi)) {
        (true, true) => PENDING_ATTEMPTS,
        (true, false) => 0,
        (false, _) => asking,
    }
}

/// Claude publishes nothing about a pending request until after it is answered (probe #42), so
/// the question comes off the screen and `source` says so. A cleared prompt is the same message
/// with no question, which is the only way a client can tell the strip to go away.
///
/// `None` means the socket is gone. `Some(published)` says whether a prompt actually went out —
/// a blocked pane whose screen has no readable dialog yet publishes nothing and is asked again.
///
/// `last` is what this socket has already been told, and it is what makes a **re-read** free to
/// the wire: a question whose screen has not moved sends nothing. That matters for exactly one
/// shape. A single-answer dialog never changes — the next press closes it — but a multiple-answer
/// one changes on **every press**, because a press is a tick (#421), and a client drawing the
/// checkboxes off the first reading would show the same two ticked for as long as the dialog stood
/// however many times the operator pressed.
async fn send_pending(
    herdr: &kampr_herdr::Herdr,
    wire: &Wire,
    global: &str,
    local: &str,
    blocked: bool,
    last: &mut Option<pending::Pending>,
) -> Option<bool> {
    let found = match blocked {
        true => pending::read(herdr, local).await,
        false => None,
    };
    if blocked && found.is_none() {
        return Some(false);
    }
    if found == *last {
        return Some(found.is_some());
    }
    *last = found.clone();
    let sent = wire.send(&ServerMsg::Pending {
        pane: global.to_string(),
        question: found.as_ref().map(|f| f.question.clone()),
        header: found.as_ref().and_then(|f| f.header.clone()),
        multi: found.as_ref().is_some_and(|f| f.multi),
        options: found.map(|f| f.options).unwrap_or_default(),
        source: PendingSource::Screen,
    });
    sent.then_some(true)
}

/// What the node sends after the answer key, per harness — the wire says the node decides and a
/// client sends only the key it was offered.
///
/// Claude selects on the bare digit: verified live twice on 2.1.237, against its trust prompt and
/// against a real Bash permission prompt whose own footer reads "Enter to confirm". Codex holds at
/// "Press enter to confirm" until it gets one (probe #43). A harness nobody has probed gets
/// nothing rather than a guess.
const SUBMIT_KEYS: &[(&str, &str)] = &[("codex", "\r")];

/// What commits a question that takes **several** answers, per harness.
///
/// Measured on Claude 2.1.258 (#421) and on nothing else. Two digits ticked two boxes and the tool
/// did not complete; `\r` toggled the focused row instead of submitting and it still did not; the
/// dialog draws `\u{2190}  \u{2610} Test suites  \u{2714} Submit  \u{2192}` above the question, and right-arrow then
/// Enter completed it with the ticked answers. A harness with no row here has not been probed, and
/// is refused rather than sent a guess into a live dialog.
const COMMIT_KEYS: &[(&str, &[&str])] = &[("claude", &["\u{1b}[C", "\r"])];

fn commit_keys(agent: Option<&str>) -> Option<&'static [&'static str]> {
    let agent = agent?;
    COMMIT_KEYS
        .iter()
        .find(|(harness, _)| *harness == agent)
        .map(|(_, keys)| *keys)
}

fn submit_key(agent: Option<&str>) -> Option<&'static str> {
    let agent = agent?;
    SUBMIT_KEYS
        .iter()
        .find(|(harness, _)| *harness == agent)
        .map(|(_, key)| *key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_core::provider::{PaneEvent, PaneInfo, PaneStream, Provider, RawScrollback};
    use kampr_core::registry::RegistryConfig;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::sync::{mpsc, watch};

    const PANE: &str = "01J/w1:p1";
    const LOCAL: &str = "w1:p1";

    /// Announces the pane's geometry on every open and then says nothing more of its own, which
    /// is the provider's shape: the `Reset` goes out as soon as `observe` has been spawned, and
    /// the observer's first frame is as far behind it as the machine is busy.
    struct Scripted {
        opens: AtomicUsize,
        feeds: std::sync::Mutex<Vec<mpsc::Sender<PaneEvent>>>,
        topology: watch::Sender<u64>,
        /// What `pane.read recent` would answer. `None` is a pane with no ring yet, which is
        /// where a watcher starts.
        history: std::sync::Mutex<Option<RawScrollback>>,
    }

    #[async_trait::async_trait]
    impl Provider for Scripted {
        async fn list_panes(&self) -> anyhow::Result<Vec<PaneInfo>> {
            Ok(vec![PaneInfo {
                pane_id: LOCAL.into(),
                cols: Some(20),
                rows: 3,
                ..PaneInfo::default()
            }])
        }

        async fn watch_pane(&self, _pane_id: &str) -> anyhow::Result<PaneStream> {
            let (tx, rx) = mpsc::channel(32);
            tx.try_send(PaneEvent::Reset { cols: 20, rows: 3 }).unwrap();
            self.feeds.lock().unwrap().push(tx);
            self.opens.fetch_add(1, Ordering::SeqCst);
            Ok(PaneStream::new(rx))
        }

        async fn write_pane(&self, _pane_id: &str, _input: Input) -> anyhow::Result<()> {
            Ok(())
        }

        async fn read_scrollback(&self, _pane_id: &str) -> anyhow::Result<Option<RawScrollback>> {
            Ok(self.history.lock().unwrap().clone())
        }

        fn topology(&self) -> watch::Receiver<u64> {
            self.topology.subscribe()
        }
    }

    /// A herdr socket that answers exactly one method — `pane.read visible` — and counts how
    /// often it was asked.
    ///
    /// The count is the assertion that matters as much as the frame: a `multi` dialog re-arms its
    /// twelve read attempts on **every** read (#421/#448), so a change that published the state a
    /// watcher arrives into had to be proved not to have bought that with a second standing poll.
    struct FakeHerdr {
        reads: Arc<AtomicUsize>,
        socket: std::path::PathBuf,
        _dir: tempfile::TempDir,
    }

    impl FakeHerdr {
        fn showing(screen: &'static str) -> Self {
            let dir = tempfile::tempdir().expect("tempdir");
            let socket = dir.path().join("herdr.sock");
            let listener = tokio::net::UnixListener::bind(&socket).expect("bind");
            let reads = Arc::new(AtomicUsize::new(0));
            tokio::spawn({
                let reads = reads.clone();
                async move {
                    while let Ok((stream, _)) = listener.accept().await {
                        let reads = reads.clone();
                        tokio::spawn(async move {
                            use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
                            let mut reader = tokio::io::BufReader::new(stream);
                            let mut line = String::new();
                            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                                return;
                            }
                            let request: Value = serde_json::from_str(&line).unwrap_or_default();
                            if request["method"] == "pane.read" {
                                reads.fetch_add(1, Ordering::SeqCst);
                            }
                            let reply = json!({ "id": "kampr",
                                "result": { "read": { "text": screen, "truncated": false } } });
                            let mut stream = reader.into_inner();
                            let _ = stream.write_all(format!("{reply}\n").as_bytes()).await;
                        });
                    }
                }
            });
            Self {
                reads,
                socket,
                _dir: dir,
            }
        }

        fn reads(&self) -> usize {
            self.reads.load(Ordering::SeqCst)
        }
    }

    struct Harness {
        provider: Arc<Scripted>,
        registry: Arc<PaneRegistry>,
        outbox: Arc<Outbox>,
        wire: Arc<Wire>,
        panes: HashMap<String, PaneHandle>,
        herds: Vec<watch::Sender<Arc<crate::herd::HerdModel>>>,
        /// What the herd already says about the pane when a watcher arrives, and where it is
        /// read from — both of them the state, not a change.
        arrives_at: kampr_core::provider::AgentStatus,
        herdr: Option<Arc<FakeHerdr>>,
        scrollback: bool,
    }

    impl Harness {
        fn new() -> Self {
            let provider = Arc::new(Scripted {
                opens: AtomicUsize::new(0),
                feeds: std::sync::Mutex::default(),
                topology: watch::channel(0).0,
                history: std::sync::Mutex::default(),
            });
            let registry = PaneRegistry::with_config(
                provider.clone(),
                RegistryConfig {
                    // Short enough that a blank flush would land inside the test rather than
                    // after it; the real 300 ms is a whole `observe` spawn wide.
                    reset_flush_after: Duration::from_millis(60),
                    first_grid_wait: Duration::from_millis(500),
                    history: kampr_core::registry::HistoryPolicy {
                        row_budget: 4,
                        fastest: Duration::from_millis(20),
                        quiet: Duration::from_millis(40),
                        idle: Duration::from_millis(200),
                    },
                    ..RegistryConfig::default()
                },
            );
            let outbox = Arc::new(Outbox::new(256));
            let wire = Arc::new(Wire::new(outbox.clone()));
            Self {
                provider,
                registry,
                outbox,
                wire,
                panes: HashMap::new(),
                herds: Vec::new(),
                arrives_at: kampr_core::provider::AgentStatus::Unknown,
                herdr: None,
                scrollback: false,
            }
        }

        /// The herd as it already stands when the watcher arrives — the state, never a change.
        /// Nothing in the test sends a second model, so anything the client is told about the
        /// prompt was published because this pump *read* the herd rather than watched it move.
        fn already(mut self, status: kampr_core::provider::AgentStatus, screen: &'static str) -> Self {
            self.arrives_at = status;
            self.herdr = Some(Arc::new(FakeHerdr::showing(screen)));
            self
        }

        fn with_history(mut self) -> Self {
            self.scrollback = true;
            self
        }

        /// What the pane's ring will hand back from now on.
        fn scrolls(&self, lines: &[&str]) {
            *self.provider.history.lock().unwrap() = Some(RawScrollback {
                text: lines.iter().map(|l| format!("{l}\n")).collect(),
                cols: Some(20),
                viewport_rows: 1,
                truncated: false,
            });
        }

        fn standing(&self) -> Arc<crate::herd::HerdModel> {
            Arc::new(crate::herd::HerdModel {
                nodes: Vec::new(),
                panes: vec![kampr_core::wire::PaneEntry::new(
                    "01J",
                    &PaneInfo {
                        pane_id: LOCAL.into(),
                        cols: Some(20),
                        rows: 3,
                        agent_status: self.arrives_at,
                        ..PaneInfo::default()
                    },
                    false,
                )],
            })
        }

        /// Exactly what `watch` does once its checks have passed.
        fn watch(&mut self) {
            let hold = hand_over(
                &mut self.panes,
                &self.outbox,
                Streams::Local(&self.registry, LOCAL),
                PANE,
            );
            let (herd, herd_rx) = watch::channel(self.standing());
            self.herds.push(herd);
            let tasks = vec![tokio::spawn(pump_pane(PaneStreamCtx {
                registry: self.registry.clone(),
                // Nothing that does not name a herdr reaches one; a socket that does not exist
                // proves it for every test but the arrival one.
                herdr: kampr_herdr::Herdr::new(
                    self.herdr
                        .as_ref()
                        .map(|h| h.socket.clone())
                        .unwrap_or_else(|| "/nonexistent/kampr-test.sock".into()),
                ),
                herd: herd_rx,
                wire: self.wire.clone(),
                global: PANE.into(),
                local: LOCAL.into(),
                scrollback: self.scrollback,
            }))];
            self.panes.insert(
                PANE.to_string(),
                PaneHandle {
                    pane: PANE.into(),
                    outbox: self.outbox.clone(),
                    tasks,
                    scrollback: false,
                    conversation: false,
                    convo: convo::open(),
                    warm: convo::warmth(),
                    warmth: Arc::new(crate::warm::ConvoWarmth::default()),
                    followed: convo::followed(),
                    _hold: hold,
                },
            );
        }

        async fn paint(&self, text: &str) {
            let feed = self.provider.feeds.lock().unwrap().last().unwrap().clone();
            feed.send(PaneEvent::Bytes {
                full: true,
                bytes: format!("\x1b[1;1H{text}").into_bytes(),
            })
            .await
            .unwrap();
        }

        /// Every frame of one kind the socket was handed inside a window.
        ///
        /// A window rather than a quiet period: the pane's first grid is up to `first_grid_wait`
        /// behind the watch, and a drain that stopped at the first gap would stop before the pump
        /// had even opened the pane.
        async fn published(&self, tag: &str, window_ms: u64) -> Vec<Value> {
            let deadline = tokio::time::Instant::now() + Duration::from_millis(window_ms);
            let mut found = Vec::new();
            while let Some(left) = deadline.checked_duration_since(tokio::time::Instant::now()) {
                let Ok(Some(frame)) = tokio::time::timeout(left, self.outbox.next()).await else {
                    break;
                };
                let message: Value = serde_json::from_str(&frame.json).unwrap();
                if message["t"] == tag {
                    found.push(message);
                }
            }
            found
        }

        async fn resets(&self) -> Vec<String> {
            let mut texts = Vec::new();
            while let Ok(Some(frame)) =
                tokio::time::timeout(Duration::from_millis(50), self.outbox.next()).await
            {
                let message: Value = serde_json::from_str(&frame.json).unwrap();
                if message["t"] != "grid.reset" {
                    continue;
                }
                texts.push(
                    message["rows_data"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .flat_map(|row| row["runs"].as_array().cloned().unwrap_or_default())
                        .filter_map(|run| run["x"].as_str().map(str::to_string))
                        .collect::<String>()
                        .trim()
                        .to_string(),
                );
            }
            texts
        }
    }

    /// **The state a watcher arrives into, published — not only the changes after it.**
    ///
    /// `blocked` started `false` and the strip went out on a herd *transition*, so a client
    /// opening a pane that was already blocked was told nothing at all. What normally covered
    /// that is a coincidence rather than a mechanism: joining a pane moves the watcher count,
    /// which rebuilds the herd, which looks like a transition from the `false` this pump had
    /// assumed. Nothing here sends a second model — the channel is written once, before the pump
    /// exists — so the prompt below can only have been published because the pump *read* the herd
    /// it arrived at.
    ///
    /// Behind that coincidence is a dialog standing still, which produces no provider revision
    /// and no herd rebuild of its own: the backstop is `HERD_RECONCILE`, thirty seconds, and the
    /// operator's phone exists to answer blocked agents.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_pane_that_was_already_blocked_gets_its_prompt_from_the_state_not_from_a_change() {
        let mut h = Harness::new().already(
            kampr_core::provider::AgentStatus::Blocked,
            "\nDo you want to make this edit?\n\n 1. Yes\n 2. No\n",
        );
        h.watch();

        let published = h.published("pending", 2000).await;
        let strip = published
            .first()
            .expect("no prompt strip for an already-blocked pane");
        assert_eq!(strip["pane"], PANE);
        assert_eq!(strip["question"], "Do you want to make this edit?");
        assert_eq!(strip["options"].as_array().unwrap().len(), 2);
        assert_eq!(strip["source"], "screen");
    }

    /// And the other half, which is what stops the fix above from being a second standing poll.
    ///
    /// A pane nobody is blocked on must cost **no read at all** on arrival, and send no frame:
    /// `pane.read[visible]` is already 2/s per socket for as long as a `multi` dialog stands
    /// (#421, #448), and the cheapest way to make that worse is to read a screen per watch.
    #[tokio::test(flavor = "multi_thread")]
    async fn arriving_at_a_pane_nobody_is_blocked_on_costs_no_read_and_no_frame() {
        let mut h = Harness::new().already(
            kampr_core::provider::AgentStatus::Idle,
            "\nDo you want to make this edit?\n\n 1. Yes\n 2. No\n",
        );
        let herdr = h.herdr.clone().expect("a fake herdr");
        h.watch();

        assert!(
            h.published("pending", 2000).await.is_empty(),
            "a pane nobody is blocked on published a strip"
        );
        assert_eq!(
            herdr.reads(),
            0,
            "arriving at an unblocked pane read its screen; #448 counts that per socket"
        );
    }

    /// **History reaches a watching socket because the ring moved, not because a timer went off.**
    ///
    /// The pump used to poll `read_scrollback` on its own three-second timer, **per watching
    /// socket** — the same read [`accumulate_history`] was already making, so three phones on one
    /// pane were three duplicated streams of it (#448) and none of them made the ring any more
    /// current. Now the accumulator says when its document moved and the socket renders what is
    /// already in hand.
    ///
    /// The pane starts with no ring at all, which is where a watcher starts and is what makes
    /// this test load-bearing: the frame below cannot have come from the arm the pump opens with,
    /// because at that moment there was nothing to send.
    #[tokio::test(flavor = "multi_thread")]
    async fn history_reaches_a_watcher_when_the_ring_moves_rather_than_on_a_timer_of_its_own() {
        let mut h = Harness::new().with_history();
        h.watch();
        // Past the pump's opening arm, with the ring still empty: whatever it rendered then, it
        // had nothing to render.
        assert!(
            h.published("scrollback", 300).await.is_empty(),
            "a pane with no ring sent a history frame"
        );

        h.scrolls(&["one", "two", "v"]);
        let frames = h.published("scrollback", 2000).await;
        let first = frames
            .first()
            .expect("the ring gained rows and no watcher was told");
        assert_eq!(first["pane"], PANE);
        assert_eq!(first["from_top"], 0);
        assert_eq!(first["total_rows"], 2, "{first}");
    }

    /// A resync re-watches every pane the client holds. Each of those is a stop and a start
    /// against the same registry, and the stop drops the pane's only watcher — so without a hold
    /// across the swap the pane is re-opened rather than re-attached, and what the client is sent
    /// is an empty grid at the pane's real geometry over the one it was already looking at.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_re_watched_pane_is_re_attached_rather_than_re_opened() {
        for attempt in 0..5 {
            let mut h = Harness::new();
            h.watch();
            tokio::time::sleep(Duration::from_millis(40)).await;
            h.paint("hello").await;
            tokio::time::sleep(Duration::from_millis(40)).await;
            assert_eq!(h.resets().await, ["hello"], "attempt {attempt}");

            h.watch();
            tokio::time::sleep(Duration::from_millis(200)).await;

            let resets = h.resets().await;
            assert_eq!(
                h.provider.opens.load(Ordering::SeqCst),
                1,
                "attempt {attempt}: a re-watch re-attaches to the pane; it does not re-open it"
            );
            assert!(!resets.is_empty(), "attempt {attempt}: the re-watch repaints");
            assert!(
                resets.iter().all(|text| text == "hello"),
                "attempt {attempt}: a blank grid over a pane that had one: {resets:?}"
            );
        }
    }

    /// The list gates the tagged enum, so a verb added to one and not the other either goes
    /// unanswered or reintroduces the refusal of an unknown `t`. Serde names the variants it
    /// knows in its own error, which is the only source that cannot drift.
    #[test]
    fn the_verb_list_is_exactly_what_the_enum_decodes() {
        let error =
            serde_json::from_value::<kampr_core::wire::ClientMsg>(serde_json::json!({ "t": "no.such.verb" }))
                .expect_err("an unknown variant")
                .to_string();
        let listed: Vec<String> = error.split('`').skip(3).step_by(2).map(str::to_string).collect();
        assert_eq!(listed, CLIENT_VERBS, "{error}");
    }

    #[test]
    fn only_the_harnesses_that_need_a_submit_key_get_one() {
        assert_eq!(submit_key(Some("codex")), Some("\r"), "probe #43");
        assert_eq!(
            submit_key(Some("claude")),
            None,
            "claude selects on the bare digit, verified live against both its dialogs"
        );
        assert_eq!(
            submit_key(Some("gemini")),
            None,
            "an unprobed harness gets no guess"
        );
        assert_eq!(submit_key(None), None);
    }

    /// The other table, and it is a different question with a different answer. A bare digit
    /// *answers* a single-answer dialog and *ticks* a multiple-answer one; `\r` then toggles the
    /// focused row rather than committing, and the dialog's own Submit control is reached sideways
    /// (#421). Two digits and an Enter left the tool uncompleted; right-arrow and Enter completed
    /// it with the ticked answers.
    #[test]
    fn committing_a_multiple_answer_question_is_a_different_key_from_answering_a_single_one() {
        assert_eq!(commit_keys(Some("claude")), Some(["\u{1b}[C", "\r"].as_slice()));
        assert_ne!(
            commit_keys(Some("claude")).map(|k| k.to_vec()),
            Some(vec!["\r"]),
            "Enter alone toggles the focused row and leaves the dialog standing",
        );
        assert_eq!(
            commit_keys(Some("codex")),
            None,
            "nobody has raised a multiple-answer question on codex, so there is nothing to send"
        );
        assert_eq!(
            commit_keys(Some("gemini")),
            None,
            "an unprobed harness gets no guess"
        );
        assert_eq!(commit_keys(None), None);
    }
}
