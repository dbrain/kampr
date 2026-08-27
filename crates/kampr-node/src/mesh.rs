use crate::session;
use crate::state::{BUILD, Node};
use axum::extract::ws::{Message, WebSocket};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use kampr_auth::{Entry, MeshRole};
use kampr_mesh::dial::{DialPolicy, Hub};
use kampr_mesh::{HubIdentity, Incoming, Link, Outgoing, Presence};
use std::collections::HashMap;
use std::sync::{Arc, Weak};
use std::time::Duration;
use tokio::task::JoinHandle;
use tracing::{info, warn};

/// How often the node re-reads its enrolled hubs, so a `kampr mesh join` run against a live node
/// takes effect without a restart — and a `kampr mesh leave` drops the link.
const HUB_POLL: Duration = Duration::from_secs(10);

/// How long an inbound handshake may take before the socket is hung up. It holds one of the
/// handshake permits for exactly this long, so a socket that opens and then says nothing costs a
/// bounded wait rather than a permit forever.
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(15);

/// Aborts a task when this guard is dropped, including when the task holding it is itself
/// cancelled.
///
/// `tokio::spawn` **detaches**: dropping a `JoinHandle` leaves the task running. For anything
/// holding one half of a socket that means the connection outlives the session that owns it, and
/// the far end never sees it close.
pub struct AbortOnDrop(pub tokio::task::AbortHandle);

impl Drop for AbortOnDrop {
    fn drop(&mut self) {
        self.0.abort();
    }
}

/// A task that stops when its handle is dropped.
///
/// A bare `JoinHandle` **detaches** on drop, which for an outbound mesh link means the socket
/// outlives the node that opened it — a stopped node still serving its panes to a hub.
struct Supervised(JoinHandle<()>);

impl Drop for Supervised {
    fn drop(&mut self) {
        self.0.abort();
    }
}

pub struct WsOut(SplitSink<WebSocket, Message>);

pub struct WsIn(SplitStream<WebSocket>, Arc<Heard>);

/// Whether anything has arrived from the far end since it was last asked.
///
/// A pong is the only frame a frozen peer still produces — its websocket library answers one
/// without waking the application — so this counts *every* frame rather than only the text ones
/// `recv` passes up. Read and cleared by the writer's keepalive; see [`Outgoing::ping`].
#[derive(Default)]
pub struct Heard(std::sync::atomic::AtomicBool);

impl Heard {
    fn note(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn take(&self) -> bool {
        self.0.swap(false, std::sync::atomic::Ordering::Relaxed)
    }
}

impl Outgoing for WsOut {
    async fn send(&mut self, text: String) -> bool {
        self.0.send(Message::text(text)).await.is_ok()
    }

    async fn close(&mut self) {
        let _ = self.0.close().await;
    }

    async fn ping(&mut self) -> bool {
        self.0.send(Message::Ping(Default::default())).await.is_ok()
    }
}

impl Incoming for WsIn {
    async fn recv(&mut self) -> Option<String> {
        loop {
            match self.0.next().await? {
                Ok(Message::Text(text)) => {
                    self.1.note();
                    return Some(text.to_string());
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => self.1.note(),
            }
        }
    }
}

pub fn split(socket: WebSocket) -> (Link<WsOut, WsIn>, Arc<Heard>) {
    let (sink, stream) = socket.split();
    let heard = Arc::new(Heard::default());
    (Link::new(WsOut(sink), WsIn(stream, heard.clone())), heard)
}

/// The hub half: a node dialled in, so authenticate it and then let it drive this node's panes.
///
/// The refusal path is deliberately as loud as the acceptance path in the audit log. An
/// unenrolled node reaching a hub is either an operator halfway through a join or somebody
/// knocking, and both are worth a line.
pub async fn accept(
    socket: WebSocket,
    node: Arc<Node>,
    peer: String,
    permit: tokio::sync::OwnedSemaphorePermit,
) {
    let identity = match node.identity() {
        Ok(identity) => identity,
        Err(e) => {
            warn!(error = %e, "this node has no mesh identity, so it cannot be a hub");
            return;
        }
    };
    let (mut link, _heard) = split(socket);
    let presence = node.presence();
    let accepted = tokio::time::timeout(
        HANDSHAKE_TIMEOUT,
        kampr_mesh::accept(
            &mut link,
            &identity,
            &presence,
            node.auth.store(),
            kampr_auth::now(),
        ),
    )
    .await;
    // The permit bounds argon2, and the handshake is done with it either way. A link that is up
    // holds nothing: a handful of settled peers must not close the door on the next one.
    drop(permit);
    let accepted = match accepted {
        Ok(accepted) => accepted,
        Err(_) => {
            warn!(%peer, "a mesh handshake never finished; hanging up");
            node.auth.audit().record(
                &Entry::new("mesh.refused").peer(&peer).detail(
                    serde_json::json!({ "code": "timeout", "reason": "the handshake did not finish" }),
                ),
            );
            link.out.close().await;
            return;
        }
    };
    match accepted {
        Ok(accepted) => {
            node.auth.mesh_settled(&peer);
            node.auth
                .audit()
                .record(&Entry::new("mesh.joined").peer(&peer).detail(serde_json::json!({
                    "node": accepted.node.node_id,
                    "name": accepted.node.name,
                    "fingerprint": accepted.node.fingerprint(),
                    "enrolled": accepted.enrolled,
                })));
            let (out, incoming) = link.split();
            node.peers.serve(accepted, out, incoming).await;
        }
        Err(e) => {
            warn!(%peer, error = %e, code = e.code(), "refused a mesh link");
            node.auth.audit().record(
                &Entry::new("mesh.refused")
                    .peer(&peer)
                    .detail(serde_json::json!({ "code": e.code(), "reason": e.to_string() })),
            );
        }
    }
}

/// The peer half: keeps one supervised outbound link to every enrolled hub.
///
/// A hub that is unreachable costs one parked task. Nothing here can affect this node's own
/// clients, its herdr sessions, or any other hub.
/// Held as a [`Weak`] because this task lives *in* the node it dials for. A strong reference here
/// would be a cycle that keeps the node — and every emulator under it — alive forever.
pub async fn dial_hubs(node: Weak<Node>) {
    let mut running: HashMap<String, Supervised> = HashMap::new();
    let mut poll = tokio::time::interval(HUB_POLL);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    loop {
        poll.tick().await;
        let Some(node) = node.upgrade() else { return };
        let hubs = match node.auth.store().mesh().nodes(MeshRole::Hub).await {
            Ok(hubs) => hubs,
            Err(e) => {
                warn!(error = %e, "could not read the enrolled hubs");
                continue;
            }
        };
        let wanted: HashMap<String, Hub> = hubs
            .into_iter()
            .filter(|hub| hub.active() && hub.url.is_some())
            .map(|hub| {
                (
                    hub.pubkey.clone(),
                    Hub {
                        url: hub.url.clone().unwrap_or_default(),
                        name: hub.name.clone(),
                        key: Some(hub.pubkey.clone()),
                        join: None,
                    },
                )
            })
            .collect();

        running.retain(|key, _| {
            let keep = wanted.contains_key(key);
            if !keep {
                info!(hub = %key, "no longer enrolled with this hub; dropping the link");
            }
            keep
        });
        // A node with nothing to dial never asks for a key, so it never writes one.
        if wanted.keys().all(|key| running.contains_key(key)) {
            continue;
        }
        let Ok(identity) = node.identity() else {
            continue;
        };
        let presence = node.presence();
        let weak = Arc::downgrade(&node);
        for (key, hub) in wanted {
            if running.contains_key(&key) {
                continue;
            }
            let identity = identity.clone();
            let presence = presence.clone();
            let weak = weak.clone();
            running.insert(
                key,
                Supervised(tokio::spawn(async move {
                    kampr_mesh::supervise(hub, identity, presence, DialPolicy::default(), {
                        move |hub, out, incoming| {
                            let weak = weak.clone();
                            async move {
                                if let Some(node) = weak.upgrade() {
                                    serve_hub(node, hub, out, incoming).await;
                                }
                            }
                        }
                    })
                    .await;
                })),
            );
        }
    }
}

/// A hub is a client of this node, and is served by the code that serves a browser.
///
/// It gets a device row of its own so that everything the session layer already does — the
/// read-only refusal, the re-read before every write, the audit line naming who typed — applies
/// to a hub without a second implementation. The row carries no token, so nothing can present it
/// as a bearer credential.
async fn serve_hub(
    node: Arc<Node>,
    hub: HubIdentity,
    out: kampr_mesh::dial::WsOut,
    incoming: kampr_mesh::dial::WsIn,
) {
    let name = format!("hub {}", hub.node_name);
    let device = match node
        .auth
        .store()
        .mesh()
        .hub_device(&hub.key, &name, kampr_auth::now())
        .await
    {
        Ok(device) => device,
        Err(e) => {
            warn!(error = %e, "could not record the hub as a device");
            return;
        }
    };
    if !device.active(kampr_auth::now()) {
        warn!(hub = %hub.node_name, "this hub's device is revoked here; refusing to serve it");
        return;
    }
    info!(hub = %hub.node_name, fingerprint = %hub.fingerprint(), "serving a hub");
    session::run_on(
        out,
        incoming,
        node,
        device,
        format!("mesh:{}", hub.fingerprint()),
        session::Caller::Hub,
    )
    .await;
}

impl Node {
    pub fn presence(&self) -> Presence {
        Presence {
            node_id: self.config.node_id.clone(),
            node_name: self.config.node_name.clone(),
            build: BUILD.to_string(),
        }
    }
}
