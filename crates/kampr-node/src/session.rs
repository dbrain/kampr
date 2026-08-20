use crate::caps;
use crate::manage::{self, ManageOp, Manager};
use crate::outbox::Outbox;
use crate::pending;
use crate::state::{BUILD, Node};
use crate::wire::Wire;
use axum::extract::ws::{Message, WebSocket};
use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use kampr_auth::{Device, Entry, Role};
use kampr_core::provider::Input;
use kampr_core::wire::{ClientMsg, PROTOCOL, ServerMsg};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::debug;

const SCROLLBACK_POLL: Duration = Duration::from_secs(3);

pub async fn run(socket: WebSocket, node: Arc<Node>, device: Device, peer: String) {
    let outbox = Arc::new(Outbox::new(node.config.limits.client_queue));
    let wire = Arc::new(Wire::new(outbox.clone()));
    let (mut sink, mut stream) = socket.split();

    let writer = tokio::spawn({
        let outbox = outbox.clone();
        async move {
            while let Some(frame) = outbox.next().await {
                if sink.send(Message::text(frame.json)).await.is_err() {
                    break;
                }
            }
            outbox.close();
            let _ = sink.close().await;
        }
    });

    let mut session = Session {
        node: node.clone(),
        wire: wire.clone(),
        device,
        peer,
        panes: HashMap::new(),
    };
    session.greet();
    let herd_task = tokio::spawn(herd_updates(node.clone(), wire.clone()));

    while let Some(Ok(message)) = stream.next().await {
        match message {
            Message::Text(text) => session.dispatch(&text).await,
            Message::Close(_) => break,
            _ => {}
        }
        if outbox.is_closed() {
            break;
        }
    }

    session.audit("session.closed", None, None);
    for handle in session.panes.into_values() {
        handle.task.abort();
    }
    herd_task.abort();
    outbox.close();
    let _ = writer.await;
}

struct PaneHandle {
    task: JoinHandle<()>,
}

struct Session {
    node: Arc<Node>,
    wire: Arc<Wire>,
    device: Device,
    peer: String,
    panes: HashMap<String, PaneHandle>,
}

impl Session {
    fn greet(&self) {
        self.wire.send_json(&hello(&self.node, &self.device));
        self.wire.send(&self.node.herd().message());
        self.audit("session.opened", None, None);
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
    fn may_write(&self, pane: Option<&str>) -> bool {
        if self.device.role.writes() {
            return true;
        }
        self.wire.error("not_writer", "this device is read-only", pane);
        false
    }

    async fn dispatch(&mut self, text: &str) {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            self.wire.error("bad_request", "not JSON", None);
            return;
        };
        // Unknown `t` values are ignored rather than refused — that is how a v1 client survives a
        // later node, and it has to work in this direction too.
        match value["t"].as_str() {
            Some("manage") => self.manage(value).await,
            Some("caps") => self.caps().await,
            Some("prefs") => self.prefs(&value).await,
            Some(_) => match serde_json::from_value::<ClientMsg>(value) {
                Ok(msg) => self.client_msg(msg).await,
                Err(e) => {
                    self.wire.error("bad_request", &e.to_string(), None);
                }
            },
            None => {
                self.wire.error("bad_request", "message has no `t`", None);
            }
        }
    }

    async fn client_msg(&mut self, msg: ClientMsg) {
        match msg {
            ClientMsg::Watch { pane, scrollback, .. } => self.watch(&pane, scrollback).await,
            ClientMsg::Unwatch { pane } => self.unwatch(&pane),
            ClientMsg::Input {
                pane,
                text,
                b64,
                keys,
            } => self.input(&pane, text, b64, keys).await,
            ClientMsg::Answer { pane, key } => self.answer(&pane, &key).await,
            ClientMsg::ConvoLoad { .. } => {
                self.wire
                    .error("unsupported", "this node serves no conversations yet", None);
            }
            ClientMsg::Resync => self.resync().await,
            ClientMsg::Ping { n } => {
                self.wire.send(&ServerMsg::Pong { n });
            }
        }
    }

    async fn watch(&mut self, pane: &str, scrollback: bool) {
        let Some(local) = self.node.local_pane(pane) else {
            self.wire
                .error("unknown_pane", "not a pane on this node", Some(pane));
            return;
        };
        if self.node.herd().pane(pane).is_none() {
            self.wire.error("unknown_pane", "no such pane", Some(pane));
            return;
        }
        if let Some(old) = self.panes.remove(pane) {
            old.task.abort();
        }
        let task = tokio::spawn(pump_pane(PaneStreamCtx {
            registry: self.node.registry.clone(),
            herdr: self.node.herdr.clone(),
            herd: self.node.subscribe_herd(),
            wire: self.wire.clone(),
            global: pane.to_string(),
            local,
            scrollback,
        }));
        self.panes.insert(pane.to_string(), PaneHandle { task });
    }

    fn unwatch(&mut self, pane: &str) {
        if let Some(handle) = self.panes.remove(pane) {
            handle.task.abort();
            self.wire.outbox().purge_pane(pane);
        }
    }

    async fn input(
        &mut self,
        pane: &str,
        text: Option<String>,
        b64: Option<String>,
        keys: Option<Vec<String>>,
    ) {
        if !self.may_write(Some(pane)) {
            return;
        }
        let Some(local) = self.node.local_pane(pane) else {
            self.wire
                .error("unknown_pane", "not a pane on this node", Some(pane));
            return;
        };
        let supplied = [text.is_some(), b64.is_some(), keys.is_some()]
            .iter()
            .filter(|s| **s)
            .count();
        if supplied != 1 {
            self.wire.error(
                "bad_request",
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
                        self.wire
                            .error("bad_request", "b64 must decode to valid UTF-8", Some(pane));
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
        if let Err(e) = self.node.registry.write(&local, input).await {
            self.wire.error("herdr_unavailable", &e.to_string(), Some(pane));
        }
    }

    async fn answer(&mut self, pane: &str, key: &str) {
        if !self.may_write(Some(pane)) {
            return;
        }
        let Some(local) = self.node.local_pane(pane) else {
            self.wire
                .error("unknown_pane", "not a pane on this node", Some(pane));
            return;
        };
        if key.is_empty() || key.chars().count() > 2 {
            self.wire.error(
                "bad_request",
                "an answer key is one or two characters",
                Some(pane),
            );
            return;
        }
        self.audit("answer", Some(pane), Some(json!({ "key": key })));
        if let Err(e) = self
            .node
            .registry
            .write(&local, Input::Bytes(key.as_bytes().to_vec()))
            .await
        {
            self.wire.error("herdr_unavailable", &e.to_string(), Some(pane));
        }
    }

    /// Every watched pane restarts, which is exactly what the protocol promises: a fresh `herd`
    /// and one `grid.reset` per pane, with no patch that could reference state the client threw
    /// away.
    async fn resync(&mut self) {
        self.wire.send(&self.node.herd().message());
        let panes: Vec<String> = self.panes.keys().cloned().collect();
        for pane in panes {
            self.watch(&pane, true).await;
        }
    }

    async fn manage(&mut self, value: Value) {
        let Ok(op) = serde_json::from_value::<ManageOp>(value) else {
            self.wire.error("bad_request", "unreadable manage op", None);
            return;
        };
        if !self.may_write(op.at.as_deref()) {
            return;
        }
        self.audit(
            "manage",
            op.at.as_deref(),
            Some(json!({ "op": op.op, "label": op.label, "kind": op.kind, "name": op.name })),
        );
        let manager = Manager {
            herdr: &self.node.herdr,
            node_id: self.node.node_id(),
            binary: &self.node.config.herdr.binary,
        };
        match manager.run(&op).await {
            Ok(reply) => {
                let id = manage::created_id(&reply).map(|id| self.node.global_pane(&id));
                let mut ack = json!({ "t": "managed", "op": op.op, "ok": true });
                if let Some(id) = id {
                    ack["id"] = json!(id);
                }
                if op.op == "layout.export" {
                    ack["layout"] = reply["layout"].clone();
                }
                self.wire.send_json(&ack);
            }
            Err(e) => {
                self.wire.send_json(&json!({
                    "t": "managed", "op": op.op, "ok": false,
                    "code": e.code(), "message": e.to_string()
                }));
                self.wire.error(e.code(), &e.to_string(), op.at.as_deref());
            }
        }
    }

    async fn caps(&self) {
        let kinds = caps::agent_kinds(&self.node.herdr).await;
        let sessions = caps::sessions(&self.node.config.herdr.binary).await;
        self.wire.send_json(&json!({
            "t": "caps",
            "node": self.node.node_id(),
            "agent_kinds": kinds,
            "sessions": sessions,
        }));
    }

    /// Per-pane render preferences, kept per device so a phone and a desktop can disagree about
    /// zoom on the same pane.
    async fn prefs(&self, value: &Value) {
        let store = self.node.auth.store();
        match (value["pane"].as_str(), value.get("prefs")) {
            (Some(pane), Some(prefs)) if !prefs.is_null() => {
                let _ = store
                    .set_pane_prefs(&self.device.id, pane, prefs, kampr_auth::now())
                    .await;
            }
            _ => {}
        }
        let all = store.pane_prefs(&self.device.id).await.unwrap_or(json!({}));
        self.wire.send_json(&json!({ "t": "prefs", "panes": all }));
    }
}

fn hello(node: &Node, device: &Device) -> Value {
    let hello = ServerMsg::Hello(kampr_core::wire::Hello {
        protocol: PROTOCOL,
        node_id: node.config.node_id.clone(),
        node_name: node.config.node_name.clone(),
        build: BUILD.to_string(),
        role: match device.role {
            Role::Full => kampr_core::wire::Role::Full,
            Role::Readonly => kampr_core::wire::Role::Readonly,
        },
        caps: kampr_core::wire::Caps {
            push: false,
            scrollback: true,
            conversation: false,
        },
    });
    let mut value = serde_json::to_value(hello).expect("hello serialises");
    // `manage` is not on the v1 `Caps` struct and `security` is not on `hello` at all; both are
    // additive, and unknown fields are ignored by construction.
    value["caps"]["manage"] = json!(true);
    let tier = node.auth.tier();
    value["security"] = json!({
        "tier": tier.tier,
        "origin": tier.origin,
        "encrypted": tier.secure_context,
        "unencrypted_banner": !tier.secure_context,
        "passkeys": tier.passkeys,
        "push": tier.push,
        "installable": tier.installable,
        "unlocks": tier.unlocks,
    });
    value["device"] = json!({
        "id": device.id,
        "name": device.name,
        "expires_at": device.expires_at,
    });
    value
}

async fn herd_updates(node: Arc<Node>, wire: Arc<Wire>) {
    let mut rx = node.subscribe_herd();
    let mut last = node.herd();
    loop {
        if rx.changed().await.is_err() {
            return;
        }
        let current = rx.borrow_and_update().clone();
        if let Some(patch) = current.diff(&last)
            && !wire.send(&patch)
        {
            return;
        }
        last = current;
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
            wire.error("herdr_unavailable", &e.to_string(), Some(&global));
            return;
        }
    };
    if watcher.is_ready() && !wire.send_update(&global, watcher.initial()) {
        return;
    }

    let mut blocked = false;
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
                    if !send_pending(&herdr, &wire, &global, &local, blocked).await {
                        return;
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
    if doc.total_rows == *sent_rows && doc.from_top <= *sent_rows {
        return true;
    }
    // The ring restarted — a gap it could not stitch, or a width change — so the client's copy is
    // no longer adjacent to what the node holds and the whole thing goes again.
    if doc.from_top <= *sent_rows {
        doc.rows.retain(|r| r.row >= *sent_rows);
        doc.from_top = (*sent_rows).min(doc.total_rows);
    }
    *sent_rows = doc.total_rows;
    wire.send_scrollback(global, &doc)
}

/// Claude publishes nothing about a pending request until after it is answered (probe #42), so
/// the question comes off the screen and `source` says so. A cleared prompt is the same message
/// with no question, which is the only way a client can tell the strip to go away.
async fn send_pending(
    herdr: &kampr_herdr::Herdr,
    wire: &Wire,
    global: &str,
    local: &str,
    blocked: bool,
) -> bool {
    if !blocked {
        return wire.send_json(&json!({
            "t": "pending", "pane": global, "question": null, "options": [], "source": "screen"
        }));
    }
    let Some(found) = pending::read(herdr, local).await else {
        return true;
    };
    wire.send_json(&json!({
        "t": "pending", "pane": global,
        "question": found.question, "options": found.options, "source": "screen"
    }))
}
