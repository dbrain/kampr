use crate::convo::{self, ConvoCtx};
use crate::manage::{self, ManageOp, Manager};
use crate::outbox::Outbox;
use crate::pending;
use crate::state::{BUILD, Node};
use crate::wire::Wire;
use axum::extract::ws::WebSocket;
use base64::Engine;
use kampr_auth::{Device, Entry, Role};
use kampr_core::provider::Input;
use kampr_core::wire::{ClientMsg, ErrorCode, PROTOCOL, PendingSource, ServerMsg};
use kampr_mesh::{Incoming, Outgoing};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::debug;

const SCROLLBACK_POLL: Duration = Duration::from_secs(3);

/// How often, and how many times, a blocked pane is re-read for a question its screen had not
/// finished painting. Bounded so a harness whose dialog Kampr cannot parse costs a handful of
/// reads rather than one every half second for as long as it sits there.
const PENDING_RETRY: Duration = Duration::from_millis(500);
const PENDING_ATTEMPTS: u32 = 12;

/// How often a live session re-reads its own device row. The broadcast covers a revocation made
/// in this process; this covers one made by `kampr setup` in another, and a Tier 0 expiry that
/// nothing announces at all.
const DEVICE_RECHECK: Duration = Duration::from_secs(2);

const MAX_PREFS_BYTES: usize = 2048;

/// Every `t` [`ClientMsg`] decodes, kept beside it because the enum's own refusal of an unknown
/// variant is indistinguishable from a malformed known one.
const CLIENT_VERBS: [&str; 7] = [
    "watch",
    "unwatch",
    "input",
    "answer",
    "convo.load",
    "resync",
    "ping",
];

pub async fn run(socket: WebSocket, node: Arc<Node>, device: Device, peer: String) {
    let link = crate::mesh::split(socket);
    let (out, incoming) = link.split();
    run_on(out, incoming, node, device, peer).await;
}

/// One client session, over any framed link.
///
/// Generic over the transport because a **hub is a client of a peer**: it sends `watch` and
/// `input` and receives grids, and it does so over a socket the peer dialled outbound. Serving it
/// with this code rather than a second implementation is what makes the relay free — the
/// read-only refusal, the device re-read before every write, the audit line, the bounded queue and
/// its purge rule all apply at the mesh hop because they are the same code.
pub async fn run_on<O: Outgoing, I: Incoming>(
    mut out: O,
    mut incoming: I,
    node: Arc<Node>,
    device: Device,
    peer: String,
) {
    let outbox = Arc::new(Outbox::new(node.config.limits.client_queue));
    let wire = Arc::new(Wire::new(outbox.clone()));

    let writer = tokio::spawn({
        let outbox = outbox.clone();
        async move {
            while let Some(frame) = outbox.next().await {
                if !out.send(frame.json).await {
                    break;
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
        panes: HashMap::new(),
        toaster: crate::toast::Toaster::default(),
    };
    session.greet().await;
    let herd_task = tokio::spawn(herd_updates(node.clone(), wire.clone()));
    let _herd_guard = crate::mesh::AbortOnDrop(herd_task.abort_handle());

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
    tasks: Vec<JoinHandle<()>>,
    scrollback: bool,
    conversation: bool,
    convo: convo::Open,
}

impl PaneHandle {
    fn stop(&self) {
        for task in &self.tasks {
            task.abort();
        }
    }
}

/// Same reason as the writer guard: a session that is cancelled rather than closed must not leave
/// pane pumps running against a socket nobody is reading.
impl Drop for PaneHandle {
    fn drop(&mut self) {
        self.stop();
    }
}

struct Session {
    node: Arc<Node>,
    wire: Arc<Wire>,
    device: Device,
    peer: String,
    panes: HashMap<String, PaneHandle>,
    toaster: crate::toast::Toaster,
}

impl Session {
    /// `hello`, the herd, and this device's stored preferences — unasked, because there is no
    /// other way for a client to learn the zoom it left a pane at. A client that has to ask has
    /// already rendered the pane at the wrong size.
    async fn greet(&self) {
        self.wire.send_json(&hello(&self.node, &self.device));
        self.wire.send(&self.node.herd().message());
        self.wire.send_json(&self.stored_prefs().await);
        self.audit("session.opened", None, None);
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
    /// revoked, or past its expiry. A transient database error is not a revocation, so it leaves
    /// the session alone and the next tick tries again.
    async fn refresh(&mut self) -> bool {
        let device = match self.node.auth.store().device(&self.device.id).await {
            Ok(device) => device,
            Err(e) => {
                debug!(error = %e, "could not re-read the session device");
                return true;
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
        if matches!(tag.as_deref(), Some("manage" | "input" | "answer" | "notify")) && !self.refresh().await {
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
            Some("notify") => self.notify(&value).await,
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
            ClientMsg::Answer { pane, key } => self.answer(&pane, &key).await,
            ClientMsg::ConvoLoad { pane, before } => self.convo_load(&pane, before.as_deref()),
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
        if let Some(old) = self.panes.remove(pane) {
            old.stop();
        }
        // Watching a pane is reading a terminal, and with `scrollback` it is reading its history
        // and with `conversation` its transcript. For a read-only device it is the *only* thing it
        // can do, so it is the only thing there is to record.
        self.audit(
            "watch",
            Some(pane),
            Some(json!({ "scrollback": scrollback, "conversation": conversation })),
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
        let convo = convo::open();
        if conversation {
            let provider = session.provider.clone();
            tasks.push(tokio::spawn(convo::pump_convo(ConvoCtx {
                journals: self.node.journals(),
                panes: session.registry.clone(),
                herd: self.node.subscribe_herd(),
                snapshot: Box::new(move |local| convo::announced(&provider, local)),
                wire: self.wire.clone(),
                global: pane.to_string(),
                local,
                journal: convo.clone(),
            })));
        }
        self.panes.insert(
            pane.to_string(),
            PaneHandle {
                tasks,
                scrollback,
                conversation,
                convo,
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
        if let Some(old) = self.panes.remove(pane) {
            old.stop();
        }
        self.audit(
            "watch",
            Some(pane),
            Some(json!({ "scrollback": scrollback, "conversation": conversation, "peer": true })),
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
                tasks,
                scrollback,
                conversation,
                convo: convo::open(),
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
            .and_then(|handle| convo::page(&handle.convo, pane, before));
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

    fn unwatch(&mut self, pane: &str) {
        if let Some(handle) = self.panes.remove(pane) {
            handle.stop();
            self.wire.outbox().purge_pane(pane);
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
        let detail = match &input {
            Input::Bytes(bytes) => json!({ "bytes": bytes.len() }),
            Input::Keys(keys) => json!({ "keys": keys }),
        };
        self.audit("input", Some(pane), Some(detail));
        if let Err(e) = session.registry.write(&local, input).await {
            self.wire
                .error(offline_code(&session), &e.to_string(), Some(pane));
        }
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
        let raw = value.clone();
        let Ok(op) = serde_json::from_value::<ManageOp>(value) else {
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
        };
        match manager.run(&op).await {
            Ok(reply) => {
                // A session is named rather than addressed, so its name is the id; everything
                // else herdr creates is a container this node qualifies with its own id.
                let id = match reply["session"].as_str() {
                    Some(name) => Some(name.to_string()),
                    None => manage::created_id(&reply).map(|id| session.global_pane(&id)),
                };
                let mut ack = json!({ "t": "managed", "op": op.op, "ok": true });
                if let Some(id) = id {
                    ack["id"] = json!(id);
                }
                if op.op == "layout.export" {
                    ack["layout"] = reply["layout"].clone();
                }
                if let Some(rid) = rid {
                    ack["rid"] = rid;
                }
                self.wire.send_json(&ack);
            }
            Err(e) => self.refuse(&op.op, op.at.as_deref(), e.code(), &e.to_string(), rid.as_ref()),
        }
    }

    /// The `managed` ack and the `error` frame that follows it, in the order the protocol names
    /// them, with the caller's correlation token on the ack wherever there is one.
    fn refuse(&self, op: &str, at: Option<&str>, code: ErrorCode, message: &str, rid: Option<&Value>) {
        let mut ack = json!({ "t": "managed", "op": op, "ok": false,
                              "code": code, "message": message });
        if let Some(rid) = rid {
            ack["rid"] = rid.clone();
        }
        self.wire.send_json(&ack);
        self.wire.error(code, message, at);
    }

    /// A structural op against a pane on another host. Unlike input this one has an answer, so
    /// the hub waits for the peer's `managed` and hands it back — with the *caller's* correlation
    /// token, never the one the hub minted for its own bookkeeping.
    async fn manage_peer(&self, target: &str, op: &ManageOp, mut raw: Value, rid: Option<Value>) {
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
        self.wire.send_json(&reply);
    }

    /// Probe #50, the reverse direction: a phone raising a toast on the operator's desktop.
    ///
    /// Writer-only, because it puts text on someone's screen; attributed to this device by the
    /// node rather than by the client; and answered honestly when the herdr session is headless
    /// and there is no desk to show it on (probe #77).
    async fn notify(&mut self, value: &Value) {
        let pane = value["pane"].as_str();
        if !self.may_write("notify", pane) {
            return;
        }
        let Some(title) = value["title"].as_str().map(str::trim).filter(|t| !t.is_empty()) else {
            self.wire
                .error(ErrorCode::BadRequest, "notify needs a title", pane);
            return;
        };
        let session = pane
            .and_then(|p| self.node.route(p))
            .unwrap_or_else(|| self.node.primary());
        self.audit("notify", pane, Some(json!({ "title": title })));
        let shown = self
            .toaster
            .show(&session.herdr, &self.device.name, title, value["body"].as_str())
            .await;
        let (ok, reason) = match &shown {
            crate::toast::Toast::Shown => (true, None),
            crate::toast::Toast::NoDesk(reason) => (false, Some(reason.as_str())),
            crate::toast::Toast::TooSoon => (false, Some("rate_limited")),
            crate::toast::Toast::Refused(reason) => (false, Some(reason.as_str())),
        };
        self.wire
            .send_json(&json!({ "t": "notified", "ok": ok, "reason": reason, "pane": pane }));
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

fn manage_detail(op: &ManageOp) -> Value {
    let mut detail = json!({ "op": op.op });
    let fields: [(&str, Value); 12] = [
        ("node", json!(op.node)),
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

fn hello(node: &Node, device: &Device) -> Value {
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
async fn herd_updates(node: Arc<Node>, wire: Arc<Wire>) {
    let mut rx = node.subscribe_herd();
    let mut last = node.herd();
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
                wire.error(
                    ErrorCode::HerdrUnavailable,
                    &current
                        .node(&id)
                        .and_then(|n| n.detail.clone())
                        .unwrap_or_else(|| format!("{id} is not reachable")),
                    None,
                ) && wire.error(ErrorCode::NodeOffline, &format!("{id} is offline"), None)
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

/// A write that failed while the session is down is the herd being unreachable, not a bad pane.
fn offline_code(session: &crate::sessions::SessionNode) -> ErrorCode {
    if session.online() {
        ErrorCode::HerdrUnavailable
    } else {
        ErrorCode::NodeOffline
    }
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

    let mut blocked = false;
    // A pane can be reported blocked before its dialog has finished painting, and the question is
    // read off the screen (probe #42). Latching on the first read would leave a blocked agent with
    // no prompt strip at all until its status happened to change again, so an unproductive read is
    // retried for a short while rather than accepted.
    let mut asking = 0u32;
    let mut ask = tokio::time::interval(PENDING_RETRY);
    ask.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut history = tokio::time::interval(SCROLLBACK_POLL);
    history.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    let mut sent_rows = 0u32;

    loop {
        tokio::select! {
            update = watcher.recv() => {
                let Ok(update) = update else { return };
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
                match send_pending(&herdr, &wire, &global, &local, true).await {
                    None => return,
                    Some(published) => {
                        if published {
                            asking = 0;
                        }
                    }
                }
            }
            _ = history.tick(), if scrollback => {
                if !send_history(&registry, &wire, &global, &local, &mut sent_rows).await {
                    return;
                }
            }
            changed = herd.changed() => {
                if changed.is_err() {
                    return;
                }
                let status = herd
                    .borrow_and_update()
                    .pane(&global)
                    .map(|p| p.agent_status);
                let now_blocked = status == Some(kampr_core::provider::AgentStatus::Blocked);
                if now_blocked != blocked {
                    blocked = now_blocked;
                    asking = if blocked { PENDING_ATTEMPTS } else { 0 };
                    match send_pending(&herdr, &wire, &global, &local, blocked).await {
                        None => return,
                        Some(published) => {
                            if published {
                                asking = 0;
                            }
                        }
                    }
                }
            }
        }
    }
}

async fn send_history(
    registry: &kampr_core::PaneRegistry,
    wire: &Wire,
    global: &str,
    local: &str,
    sent_rows: &mut u32,
) -> bool {
    let Ok(Some(mut doc)) = registry.scrollback(local).await else {
        return true;
    };
    // `total_rows` is a depth, so the ring ends here; `sent_rows` is the same index, one message
    // ago.
    let end = doc.from_top + doc.total_rows;
    if end == *sent_rows && doc.from_top <= *sent_rows {
        return true;
    }
    // The ring restarted — a gap it could not stitch, or a width change — so the client's copy is
    // no longer adjacent to what the node holds and the whole thing goes again.
    if doc.from_top <= *sent_rows {
        doc.rows.retain(|r| r.row >= *sent_rows);
        doc.from_top = (*sent_rows).min(end);
        doc.total_rows = end - doc.from_top;
    }
    *sent_rows = end;
    wire.send_scrollback(global, &doc)
}

/// Claude publishes nothing about a pending request until after it is answered (probe #42), so
/// the question comes off the screen and `source` says so. A cleared prompt is the same message
/// with no question, which is the only way a client can tell the strip to go away.
///
/// `None` means the socket is gone. `Some(published)` says whether a prompt actually went out —
/// a blocked pane whose screen has no readable dialog yet publishes nothing and is asked again.
async fn send_pending(
    herdr: &kampr_herdr::Herdr,
    wire: &Wire,
    global: &str,
    local: &str,
    blocked: bool,
) -> Option<bool> {
    let found = match blocked {
        true => pending::read(herdr, local).await,
        false => None,
    };
    if blocked && found.is_none() {
        return Some(false);
    }
    let sent = wire.send(&ServerMsg::Pending {
        pane: global.to_string(),
        question: found.as_ref().map(|f| f.question.clone()),
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

fn submit_key(agent: Option<&str>) -> Option<&'static str> {
    let agent = agent?;
    SUBMIT_KEYS
        .iter()
        .find(|(harness, _)| *harness == agent)
        .map(|(_, key)| *key)
}

#[cfg(test)]
mod tests {
    use super::{CLIENT_VERBS, submit_key};

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
}
