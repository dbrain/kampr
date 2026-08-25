//! The hub half, driven by a scripted peer.
//!
//! The peer here speaks the ordinary v1 protocol over an in-process link, which is exactly what a
//! real peer's client session does over a WebSocket — so these tests exercise the relay, the
//! shadow and the herd merge without a socket, a terminal or a herd.

use kampr_auth::{MeshRole, Store};
use kampr_core::registry::PaneUpdate;
use kampr_core::wire::ErrorCode;
use kampr_mesh::handshake::Accepted;
use kampr_mesh::transport::{Incoming, Link, Outgoing, Receiver, Sender, pair};
use kampr_mesh::{PeerState, Peers, PeersConfig, RemoteEvent};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

/// The hub's enrolment table. A link is only served while its key is in here, so every test needs
/// one — and the one about revocation writes to it while the link is up, as an operator does.
async fn store() -> Store {
    Store::open_memory().await.expect("a store")
}

struct Peer {
    link: Link<Sender, Receiver>,
}

impl Peer {
    async fn send(&mut self, message: Value) {
        assert!(self.link.out.send(message.to_string()).await);
    }

    /// The next request the hub made, or a panic naming what it did instead of asking.
    async fn request(&mut self) -> Value {
        let text = tokio::time::timeout(Duration::from_secs(2), self.link.incoming.recv())
            .await
            .expect("the hub said nothing")
            .expect("the hub closed the link");
        serde_json::from_str(&text).expect("the hub sent JSON")
    }

    /// The next request that is not a keepalive.
    async fn request_but_ping(&mut self) -> Value {
        loop {
            let message = self.request().await;
            if message["t"] != "ping" {
                return message;
            }
        }
    }

    async fn close(self) {
        drop(self.link);
    }

    /// Whether the hub hung up on this peer. `None` from the read half is the socket closing,
    /// which is all a refused or revoked peer is ever told.
    async fn hung_up(&mut self) -> bool {
        tokio::time::timeout(Duration::from_secs(2), async {
            while self.link.incoming.recv().await.is_some() {}
        })
        .await
        .is_ok()
    }
}

/// One authenticated peer, enrolled exactly as the handshake would have left it.
async fn join(peers: &Arc<Peers>, store: &Store, pubkey: &str, node_id: &str, name: &str) -> Peer {
    let node = store
        .mesh()
        .enrol(pubkey, node_id, name, MeshRole::Peer, None, kampr_auth::now())
        .await
        .expect("an enrolment");
    let (hub_side, peer_side) = pair();
    let (out, incoming) = hub_side.split();
    let accepted = Accepted {
        node,
        build: "0.1.0".into(),
        enrolled: false,
        store: store.clone(),
    };
    let peers = peers.clone();
    tokio::spawn(async move { peers.serve(accepted, out, incoming).await });
    Peer { link: peer_side }
}

fn peers() -> Arc<Peers> {
    // Keepalives are measured elsewhere; here they would only be noise in the request stream.
    peers_ticking(Duration::from_secs(3600))
}

fn peers_ticking(ping_interval: Duration) -> Arc<Peers> {
    Peers::new(PeersConfig {
        ping_interval,
        pane_fanout: 8,
    })
}

fn herd(node_id: &str, panes: &[&str]) -> Value {
    json!({
        "t": "herd",
        "nodes": [{ "id": node_id, "name": node_id, "kind": "local", "online": true,
                    "rtt_ms": 0.5, "herdr_version": "0.8.2", "build": "0.1.0" }],
        "panes": panes.iter().map(|pane| json!({
            "id": format!("{node_id}/{pane}"), "node_id": node_id,
            "cols": 8, "rows": 2, "agent_status": "unknown",
        })).collect::<Vec<_>>(),
    })
}

fn reset(pane: &str, text: &str) -> Value {
    json!({
        "t": "grid.reset", "pane": pane, "cols": 8, "rows": 2,
        "rows_data": [{ "row": 0, "runs": [{ "s": 0, "x": text }] }],
        "cursor": { "col": 0, "row": 0, "visible": true }, "links": [],
    })
}

fn patch(pane: &str, row: u32, text: &str) -> Value {
    json!({
        "t": "grid.patch", "pane": pane,
        "rows": [{ "row": row, "runs": [{ "s": 0, "x": text }] }],
        "cursor": { "col": 0, "row": row, "visible": true },
    })
}

/// Waits for the herd to satisfy a predicate, so a test never races the publish.
async fn settle(peers: &Arc<Peers>, want: impl Fn(&kampr_mesh::PeerHerd) -> bool) {
    let mut herd = peers.subscribe();
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if want(&peers.herd()) {
                return;
            }
            herd.changed().await.expect("the herd channel stays open");
        }
    })
    .await
    .expect("the herd never reached the expected state");
}

/// Waits until the hub is serving `count` links, so a test never races `serve`'s own spawn.
async fn linked(peers: &Arc<Peers>, count: usize) {
    tokio::time::timeout(Duration::from_secs(2), async {
        while peers.links().len() != count {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .expect("the hub never reached that many links");
}

fn text_of(update: &PaneUpdate) -> Vec<String> {
    update
        .rows()
        .iter()
        .map(|row| row.cells.iter().map(|c| c.ch).collect::<String>())
        .collect()
}

/// Waits until the hub has absorbed a scrollback message, so a handover is measured against a
/// history that is actually there.
async fn settled_history(watcher: &mut kampr_mesh::RemoteWatcher) {
    loop {
        match tokio::time::timeout(Duration::from_secs(2), watcher.recv())
            .await
            .expect("the watcher went quiet")
            .expect("the watcher closed")
        {
            RemoteEvent::Scrollback(_) => return,
            _ => continue,
        }
    }
}

async fn next_update(watcher: &mut kampr_mesh::RemoteWatcher) -> PaneUpdate {
    loop {
        match tokio::time::timeout(Duration::from_secs(2), watcher.recv())
            .await
            .expect("the watcher went quiet")
            .expect("the watcher closed")
        {
            RemoteEvent::Update(update) => return update,
            _ => continue,
        }
    }
}

#[tokio::test]
async fn a_peers_nodes_join_the_herd_marked_as_peers() {
    let peers = peers();
    let store = store().await;
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let herd = peers.herd();
    assert_eq!(herd.nodes[0].id, "01JA");
    assert_eq!(
        herd.nodes[0].kind, "peer",
        "a local node to itself is a peer to us"
    );
    assert!(herd.nodes[0].online);
    assert_eq!(herd.nodes[0].build.as_deref(), Some("0.1.0"));
    assert_eq!(herd.panes[0].id, "01JA/w1:p1");
    assert_eq!(peers.state("01JA/w1:p1"), PeerState::Live);
    assert_eq!(peers.state("01JZ/w1:p1"), PeerState::Unknown);
}

/// A peer answers for its own version, and the hub carries the answer without touching it. Both
/// halves matter: the hub must not drop `update` on the way through, and it must not fill one in
/// for a peer that said nothing — a peer whose operator turned the check off would otherwise be
/// judged by a request they declined.
#[tokio::test]
async fn a_peers_own_verdict_on_its_version_crosses_the_hub_untouched() {
    let peers = peers();
    let store = store().await;
    let mut stale = join(&peers, &store, KEY, "01JA", "laptop").await;
    let mut quiet = join(&peers, &store, OTHER, "01JB", "desk").await;
    let mut says = herd("01JA", &["w1:p1"]);
    says["nodes"][0]["update"] = json!("0.1.2");
    stale.send(says).await;
    quiet.send(herd("01JB", &["w1:p1"])).await;
    settle(&peers, |h| h.nodes.len() == 2).await;

    let herd = peers.herd();
    let stale = herd
        .nodes
        .iter()
        .find(|n| n.id == "01JA")
        .expect("the stale peer");
    let quiet = herd
        .nodes
        .iter()
        .find(|n| n.id == "01JB")
        .expect("the quiet peer");
    assert_eq!(
        stale.update.as_deref(),
        Some("0.1.2"),
        "the hub dropped what the peer said about its own version"
    );
    assert_eq!(
        quiet.update, None,
        "the hub answered a version question on behalf of a peer that did not answer it"
    );
}

#[tokio::test]
async fn a_pane_is_watched_once_upstream_however_many_clients_look_at_it() {
    let peers = peers();
    let store = store().await;
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let mut first = peers.watch("01JA/w1:p1", false).expect("a live peer");
    let request = peer.request_but_ping().await;
    assert_eq!(request["t"], "watch");
    assert_eq!(request["pane"], "01JA/w1:p1");

    peer.send(reset("01JA/w1:p1", "hello")).await;
    assert_eq!(text_of(&next_update(&mut first).await), ["hello   ", "        "]);

    // A second client joins the same pane: no second request, and it renders at once from the
    // hub's shadow rather than waiting for the peer to repaint.
    let mut second = peers.watch("01JA/w1:p1", false).expect("a live peer");
    let initial = second.initial();
    assert_eq!(initial.len(), 1);
    let RemoteEvent::Update(update) = &initial[0] else {
        panic!("a joiner is handed a grid");
    };
    assert_eq!(text_of(update), ["hello   ", "        "]);
    assert!(update.is_reset(), "and it is a reset, never a patch");

    peer.send(patch("01JA/w1:p1", 1, "again")).await;
    assert_eq!(text_of(&next_update(&mut first).await), ["again   "]);
    assert_eq!(text_of(&next_update(&mut second).await), ["again   "]);

    drop(first);
    drop(second);
    assert_eq!(
        peer.request_but_ping().await["t"],
        "unwatch",
        "the last watcher going away is what stops the peer streaming"
    );
}

#[tokio::test]
async fn input_for_a_peer_pane_is_relayed_as_the_client_sent_it() {
    let peers = peers();
    let store = store().await;
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    peers
        .relay(
            "01JA/w1:p1",
            json!({ "t": "input", "pane": "01JA/w1:p1", "text": "ls\r" }),
        )
        .expect("a live peer");
    let request = peer.request_but_ping().await;
    assert_eq!(request["t"], "input");
    assert_eq!(request["text"], "ls\r");
}

#[tokio::test]
async fn a_pane_on_a_node_nobody_serves_is_unknown_rather_than_offline() {
    let peers = peers();
    let error = peers.watch("01JZ/w1:p1", false).expect_err("nobody serves it");
    assert_eq!(error.code(), ErrorCode::UnknownPane);
}

#[tokio::test]
async fn a_peer_dropping_costs_its_own_panes_and_nothing_else() {
    let peers = peers();
    let store = store().await;
    let mut laptop = join(&peers, &store, KEY, "01JA", "laptop").await;
    let mut workshop = join(&peers, &store, OTHER, "01JB", "workshop").await;
    laptop.send(herd("01JA", &["w1:p1"])).await;
    workshop.send(herd("01JB", &["w1:p1"])).await;
    settle(&peers, |h| h.panes.len() == 2).await;

    let mut watching_laptop = peers.watch("01JA/w1:p1", false).expect("live");
    let mut watching_workshop = peers.watch("01JB/w1:p1", false).expect("live");
    laptop.request_but_ping().await;
    workshop.request_but_ping().await;
    laptop.send(reset("01JA/w1:p1", "gone")).await;
    workshop.send(reset("01JB/w1:p1", "fine")).await;
    next_update(&mut watching_laptop).await;
    next_update(&mut watching_workshop).await;

    laptop.close().await;
    settle(&peers, |h| h.nodes.iter().any(|n| n.id == "01JA" && !n.online)).await;

    // The dropped node is still *listed*, offline, with a reason — emptying it out of the herd is
    // the one thing a user cannot act on.
    let herd = peers.herd();
    let laptop_node = herd.nodes.iter().find(|n| n.id == "01JA").expect("still listed");
    assert!(!laptop_node.online);
    assert!(
        laptop_node
            .detail
            .as_deref()
            .unwrap_or_default()
            .contains("laptop")
    );
    assert!(laptop_node.rtt_ms.is_none());
    assert!(
        herd.panes.iter().any(|p| p.id == "01JA/w1:p1"),
        "its panes stay listed so the node does not vanish"
    );
    assert_eq!(peers.state("01JA/w1:p1"), PeerState::Offline);

    // Its watcher is told, rather than left on a grid that will never move again.
    let told = tokio::time::timeout(Duration::from_secs(2), watching_laptop.recv())
        .await
        .expect("the watcher was told something");
    match told {
        Some(RemoteEvent::Error { code, .. }) => assert_eq!(code, "node_offline"),
        None => {}
        other => panic!("expected an offline error, got {other:?}"),
    }
    assert_eq!(
        peers.watch("01JA/w1:p1", false).unwrap_err().code(),
        ErrorCode::NodeOffline
    );

    // The other peer never noticed.
    assert_eq!(peers.state("01JB/w1:p1"), PeerState::Live);
    workshop.send(patch("01JB/w1:p1", 0, "still")).await;
    assert_eq!(text_of(&next_update(&mut watching_workshop).await), ["still   "]);
}

#[tokio::test]
async fn a_peer_that_comes_back_is_online_again_with_no_help() {
    let peers = peers();
    let store = store().await;
    let mut laptop = join(&peers, &store, KEY, "01JA", "laptop").await;
    laptop.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;
    laptop.close().await;
    settle(&peers, |h| h.nodes.iter().any(|n| !n.online)).await;

    let mut again = join(&peers, &store, KEY, "01JA", "laptop").await;
    again.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| h.nodes.iter().all(|n| n.online)).await;

    let herd = peers.herd();
    assert_eq!(herd.nodes.len(), 1, "one node, not one per connection");
    assert_eq!(herd.panes.len(), 1);
    assert!(peers.watch("01JA/w1:p1", false).is_ok());
}

#[tokio::test]
async fn history_is_stitched_by_index_and_a_joiner_gets_all_of_it() {
    let peers = peers();
    let store = store().await;
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;
    let mut first = peers.watch("01JA/w1:p1", false).expect("live");
    peer.request_but_ping().await;
    peer.send(reset("01JA/w1:p1", "live")).await;

    peer.send(json!({
        "t": "scrollback", "pane": "01JA/w1:p1", "from_top": 0,
        "rows": [{ "row": 0, "runs": [{ "s": 0, "x": "one" }] },
                 { "row": 1, "runs": [{ "s": 0, "x": "two" }] }],
        "total_rows": 2, "complete": true, "capped": false,
    }))
    .await;
    peer.send(json!({
        "t": "scrollback", "pane": "01JA/w1:p1", "from_top": 2,
        "rows": [{ "row": 2, "runs": [{ "s": 0, "x": "three" }] }],
        "total_rows": 1, "complete": true, "capped": false,
    }))
    .await;

    let mut delivered = Vec::new();
    while delivered.len() < 2 {
        if let Some(RemoteEvent::Scrollback(doc)) = tokio::time::timeout(Duration::from_secs(2), first.recv())
            .await
            .expect("the watcher went quiet")
        {
            delivered.push(doc);
        }
    }
    assert_eq!(delivered[0].from_top, 0);
    assert_eq!(delivered[0].total_rows, 2);
    assert_eq!(delivered[1].from_top, 2, "a delta, not the whole ring again");
    assert_eq!(delivered[1].total_rows, 1);

    let mut joiner = peers.watch("01JA/w1:p1", false).expect("live");
    let history = joiner
        .initial()
        .into_iter()
        .find_map(|event| match event {
            RemoteEvent::Scrollback(doc) => Some(doc),
            _ => None,
        })
        .expect("a joiner is handed the history the hub holds");
    assert_eq!(history.from_top, 0);
    assert_eq!(history.total_rows, 3, "stitched, not the last message alone");
}

/// The handshake authenticates a *key* and binds it to one node id. Everything after it — the
/// `herd` message included — is the peer's own words about itself, so an enrolled machine can name
/// any node it likes and be believed. On a shared hub that is one host asking for another's
/// terminals: its watches, its keystrokes and its manage ops.
#[tokio::test]
async fn a_peer_cannot_claim_another_peers_node_id() {
    let peers = peers();
    let store = store().await;
    let mut laptop = join(&peers, &store, KEY, "01JA", "laptop").await;
    laptop.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let mut impostor = join(&peers, &store, OTHER, "01JB", "workshop").await;
    let mut claim = herd("01JB", &["w1:p1"]);
    claim["nodes"].as_array_mut().unwrap().push(json!({
        "id": "01JA", "name": "not the laptop", "kind": "local", "online": true,
        "herdr_version": "0.8.2", "build": "0.1.0",
    }));
    claim["panes"].as_array_mut().unwrap().push(json!({
        "id": "01JA/w1:p1", "node_id": "01JA", "cols": 8, "rows": 2, "agent_status": "unknown",
    }));
    impostor.send(claim).await;
    settle(&peers, |h| h.nodes.iter().any(|n| n.id == "01JB")).await;

    let herd = peers.herd();
    assert_eq!(
        herd.nodes.iter().filter(|n| n.id == "01JA").count(),
        1,
        "the herd listed the laptop twice, once as somebody else",
    );
    assert_eq!(herd.panes.iter().filter(|p| p.id == "01JA/w1:p1").count(), 1);
    assert_eq!(
        peers.link_for("01JA/w1:p1").expect("a live link").pubkey,
        KEY,
        "the laptop's pane resolved to the link that merely claimed it",
    );

    peers
        .relay(
            "01JA/w1:p1",
            json!({ "t": "input", "pane": "01JA/w1:p1", "text": "sudo rm -rf /\r" }),
        )
        .expect("a live peer");
    assert_eq!(laptop.request_but_ping().await["t"], "input");
    // And the impostor's own node is untouched: it is refused an id, not disbelieved wholesale.
    assert!(herd.nodes.iter().any(|n| n.id == "01JB"));
    assert_eq!(peers.link_for("01JB/w1:p1").expect("live").pubkey, OTHER);
}

/// Connection order decided this before: the first link to name an id got everything addressed to
/// it, so an impostor only had to be there first.
#[tokio::test]
async fn an_impostor_that_claimed_an_id_first_still_loses_it_to_the_node_that_owns_it() {
    let peers = peers();
    let store = store().await;
    let mut impostor = join(&peers, &store, OTHER, "01JB", "workshop").await;
    let mut claim = herd("01JB", &["w1:p1"]);
    claim["nodes"].as_array_mut().unwrap().push(json!({
        "id": "01JA", "name": "not the laptop", "kind": "local", "online": true,
        "herdr_version": "0.8.2", "build": "0.1.0",
    }));
    claim["panes"].as_array_mut().unwrap().push(json!({
        "id": "01JA/w1:p1", "node_id": "01JA", "cols": 8, "rows": 2, "agent_status": "unknown",
    }));
    impostor.send(claim).await;
    settle(&peers, |h| h.nodes.iter().any(|n| n.id == "01JA")).await;

    let mut laptop = join(&peers, &store, KEY, "01JA", "laptop").await;
    linked(&peers, 2).await;
    laptop.send(herd("01JA", &["w1:p1"])).await;
    // The laptop names itself "01JA"; the impostor called the node it claimed something else.
    settle(&peers, |h| {
        h.nodes.iter().any(|n| n.id == "01JA" && n.name == "01JA")
    })
    .await;

    assert_eq!(peers.link_for("01JA/w1:p1").expect("a live link").pubkey, KEY,);
    let herd = peers.herd();
    assert_eq!(
        herd.nodes.iter().filter(|n| n.id == "01JA").count(),
        1,
        "the claim outlived the arrival of the node that authenticated as it",
    );
    assert!(herd.nodes.iter().all(|n| n.name != "not the laptop"));
    assert_eq!(herd.panes.iter().filter(|p| p.id == "01JA/w1:p1").count(), 1);
}

#[tokio::test]
async fn a_second_link_claiming_a_live_node_id_is_refused() {
    let peers = peers();
    let store = store().await;
    let mut laptop = join(&peers, &store, KEY, "01JA", "laptop").await;
    laptop.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let mut impostor = join(&peers, &store, OTHER, "01JA", "also the laptop").await;
    assert!(
        impostor.hung_up().await,
        "the hub served two links for one node id"
    );
    assert_eq!(peers.links().len(), 1);
    assert_eq!(peers.link_for("01JA").expect("a live link").pubkey, KEY);

    // The link that holds the id keeps holding it.
    peers
        .relay(
            "01JA/w1:p1",
            json!({ "t": "input", "pane": "01JA/w1:p1", "text": "ls\r" }),
        )
        .expect("a live peer");
    assert_eq!(laptop.request_but_ping().await["t"], "input");
}

/// A peer that reconnects before the hub notices the old socket died used to be in the list twice,
/// and every lookup took whichever the scan reached first — sometimes the dead one.
#[tokio::test]
async fn a_peer_that_dials_again_replaces_its_link_rather_than_joining_it() {
    let peers = peers();
    let store = store().await;
    let mut first = join(&peers, &store, KEY, "01JA", "laptop").await;
    first.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let mut again = join(&peers, &store, KEY, "01JA", "laptop").await;
    again.send(herd("01JA", &["w1:p1"])).await;
    assert!(first.hung_up().await, "the hub kept the socket it had replaced");
    settle(&peers, |h| h.nodes.len() == 1).await;

    assert_eq!(
        peers.links().len(),
        1,
        "one link per node, not one per connection"
    );
    let herd = peers.herd();
    assert_eq!(herd.nodes.len(), 1);
    assert!(
        herd.nodes[0].online,
        "the replaced link was remembered as an offline twin"
    );
    assert_eq!(herd.panes.len(), 1);

    peers
        .relay(
            "01JA/w1:p1",
            json!({ "t": "input", "pane": "01JA/w1:p1", "text": "ls\r" }),
        )
        .expect("a live peer");
    assert_eq!(
        again.request_but_ping().await["t"],
        "input",
        "the traffic went to the socket that had already closed",
    );
}

/// `kampr mesh revoke` writes SQLite in another process and prints that a running node drops the
/// link within seconds. Nothing made that true: the authenticated socket stayed up, relaying panes
/// and input, until the node restarted.
#[tokio::test]
async fn a_revoked_peer_loses_the_link_it_already_had() {
    let peers = peers_ticking(Duration::from_millis(50));
    let store = store().await;
    let mut laptop = join(&peers, &store, KEY, "01JA", "laptop").await;
    laptop.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    store
        .mesh()
        .revoke(KEY, kampr_auth::now())
        .await
        .expect("the revocation")
        .expect("the enrolled node");

    settle(&peers, |h| {
        h.nodes.iter().any(|n| {
            n.id == "01JA" && !n.online && n.detail.as_deref().unwrap_or_default().contains("revoked")
        })
    })
    .await;
    assert!(laptop.hung_up().await, "a revoked peer kept its socket");
    assert!(peers.links().is_empty());
    assert_eq!(
        peers.watch("01JA/w1:p1", false).unwrap_err().code(),
        ErrorCode::NodeOffline,
    );
}

/// Unanswered pings were retained for four rounds and then forgotten, so a peer that stopped
/// answering entirely was never disconnected — it kept its place in the herd, its panes, and the
/// last round trip it ever measured.
#[tokio::test]
async fn a_peer_that_stops_answering_keepalives_is_dropped() {
    let peers = peers_ticking(Duration::from_millis(50));
    let store = store().await;
    let mut laptop = join(&peers, &store, KEY, "01JA", "laptop").await;
    laptop.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    settle(&peers, |h| {
        h.nodes.iter().any(|n| {
            n.id == "01JA" && !n.online && n.detail.as_deref().unwrap_or_default().contains("keepalives")
        })
    })
    .await;
    assert!(peers.links().is_empty());
}

/// One `watch` per pane per link is what keeps the WAN hop carrying one copy, and it used to mean
/// the first viewer decided for everybody: a second viewer asking for the transcript was attached
/// to a stream that was never asked for one. Agent panes default to the conversation view, so that
/// viewer lands on the product's default surface with nothing in it.
#[tokio::test]
async fn a_second_viewer_asking_for_the_conversation_is_sent_one() {
    let peers = peers();
    let store = store().await;
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let _terminal = peers.watch("01JA/w1:p1", false).expect("a live peer");
    let opened = peer.request_but_ping().await;
    assert_eq!(opened["t"], "watch");
    assert_eq!(opened["conversation"], false);

    let _conversation = peers.watch("01JA/w1:p1", true).expect("a live peer");
    let upgraded = peer.request_but_ping().await;
    assert_eq!(
        upgraded["t"], "watch",
        "the second viewer never asked for a transcript"
    );
    assert_eq!(upgraded["pane"], "01JA/w1:p1");
    assert_eq!(upgraded["conversation"], true);

    // And it is asked for once: a third viewer wanting the same thing costs no round trip.
    let _third = peers.watch("01JA/w1:p1", true).expect("a live peer");
    peers
        .relay(
            "01JA/w1:p1",
            json!({ "t": "input", "pane": "01JA/w1:p1", "text": "ls\r" }),
        )
        .expect("a live peer");
    assert_eq!(
        peer.request_but_ping().await["t"],
        "input",
        "the hub asked for the same transcript twice",
    );
}

/// `styles.from` is a `u32` straight off the peer's frame, and the table was resized to it. Forty
/// bytes of JSON — well under any size ceiling — asked the hub for four billion entries, and the
/// hub is the front node the whole herd is reached through.
#[tokio::test]
async fn a_styles_message_that_skips_past_the_table_closes_the_link() {
    let peers = peers();
    let store = store().await;
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    peer.send(json!({ "t": "styles", "from": 1_000_000, "styles": [] }))
        .await;
    assert!(
        peer.hung_up().await,
        "the hub sized a table from a number the peer chose",
    );
    settle(&peers, |h| h.nodes.iter().any(|n| !n.online)).await;
}

/// The node-side twin of this was #252: a registry that holds a `Weak`, and a caller that stops
/// the old watcher before the new one attaches. A `RemotePane` is kept alive by its watchers
/// alone, so the last one dropping takes the hub's shadow of the pane, the history it has
/// stitched, and the upstream `watch` with it — and what the replacement gets is a *fresh* pane:
/// a blank grid over content a viewer was already looking at, no history at all, and a second
/// crossing of the WAN per pane per resync.
#[tokio::test]
async fn a_relayed_pane_keeps_its_history_when_the_last_viewer_is_replaced() {
    let peers = peers();
    let store = store().await;
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let mut first = peers.watch("01JA/w1:p1", false).expect("a live peer");
    assert_eq!(peer.request_but_ping().await["t"], "watch");
    peer.send(reset("01JA/w1:p1", "hello")).await;
    peer.send(json!({
        "t": "scrollback", "pane": "01JA/w1:p1", "from_top": 0,
        "rows": [{ "row": 0, "runs": [{ "s": 0, "x": "one" }] }],
        "total_rows": 1, "complete": true, "capped": false,
    }))
    .await;
    assert_eq!(text_of(&next_update(&mut first).await), ["hello   ", "        "]);
    settled_history(&mut first).await;

    // The resync: this viewer is the last one, and it is stopped before its replacement watches.
    let hold = peers.hold_while("01JA/w1:p1", || drop(first));
    let mut second = peers.watch("01JA/w1:p1", false).expect("a live peer");
    drop(hold);

    let initial = second.initial();
    let grid = initial.iter().find_map(|event| match event {
        RemoteEvent::Update(update) => Some(update),
        _ => None,
    });
    assert_eq!(
        grid.map(text_of),
        Some(vec!["hello   ".to_string(), "        ".to_string()]),
        "the replacement was handed a blank pane instead of the grid the hub already held",
    );
    let history = initial.iter().find_map(|event| match event {
        RemoteEvent::Scrollback(doc) => Some(doc),
        _ => None,
    });
    assert_eq!(
        history.map(|doc| doc.total_rows),
        Some(1),
        "the stitched history was thrown away and re-asked for",
    );

    // Nothing crossed the link for any of it. `input` is a fence: it is the next request the hub
    // makes, so anything the handover sent would arrive ahead of it.
    peers
        .relay(
            "01JA/w1:p1",
            json!({ "t": "input", "pane": "01JA/w1:p1", "text": "ls\r" }),
        )
        .expect("a live peer");
    assert_eq!(
        peer.request_but_ping().await["t"],
        "input",
        "the handover cost the WAN a second watch",
    );
}

/// The other half of the hold: a pane held across a handover nobody came back for is still a pane
/// nobody is watching, and the hub must stop the peer streaming it. Held forever it would cost the
/// hub a shadow and a history for every pane it ever showed, and the peer a stream nobody reads.
#[tokio::test]
async fn a_relayed_pane_nobody_came_back_for_is_still_unwatched_upstream() {
    let peers = peers();
    let store = store().await;
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let mut only = peers.watch("01JA/w1:p1", false).expect("a live peer");
    assert_eq!(peer.request_but_ping().await["t"], "watch");
    peer.send(reset("01JA/w1:p1", "hello")).await;
    next_update(&mut only).await;

    let hold = peers.hold_while("01JA/w1:p1", || drop(only));
    assert!(hold.is_some(), "a live pane is holdable");
    drop(hold);

    assert_eq!(
        peer.request_but_ping().await["t"],
        "unwatch",
        "a hold that outlived every viewer kept the peer streaming a pane nobody reads",
    );
}

/// A hold must not short-circuit what `watch` decides. The first viewer of an agent pane settles
/// what the peer sends, so a replacement that wants the transcript still has to ask for it — and
/// across a handover the pane it re-attaches to is the *old* one, which was opened without.
#[tokio::test]
async fn a_viewer_replaced_across_a_handover_still_asks_for_the_transcript() {
    let peers = peers();
    let store = store().await;
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let terminal = peers.watch("01JA/w1:p1", false).expect("a live peer");
    let opened = peer.request_but_ping().await;
    assert_eq!(opened["t"], "watch");
    assert_eq!(opened["conversation"], false);

    let hold = peers.hold_while("01JA/w1:p1", || drop(terminal));
    let _conversation = peers.watch("01JA/w1:p1", true).expect("a live peer");
    drop(hold);

    let upgraded = peer.request_but_ping().await;
    assert_eq!(upgraded["t"], "watch", "{upgraded}");
    assert_eq!(upgraded["pane"], "01JA/w1:p1");
    assert_eq!(
        upgraded["conversation"], true,
        "the replacement was re-attached to a stream that carries no transcript: {upgraded}",
    );
}

/// `hold_while` runs its `stop` whatever it finds, so a caller can order the swap the same way for
/// every pane it holds — including one on a peer that has gone since it was watched.
#[tokio::test]
async fn holding_a_pane_no_link_serves_still_stops_the_watcher_it_was_given() {
    let peers = peers();
    let mut stopped = false;
    let hold = peers.hold_while("01JZ/w1:p1", || stopped = true);
    assert!(hold.is_none(), "nobody serves that pane");
    assert!(stopped, "the old watcher was left running");
}

// ---------------------------------------------------------------------------------------------
// Attachments, which are the one thing on this link that is bulk rather than a frame.

const CEILING: u64 = 8 * 1024 * 1024;

fn b64(bytes: &[u8]) -> String {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD.encode(bytes)
}

/// The hub asking, and everything the peer said back, as one task — so the test can play the peer
/// against a fetch that is genuinely concurrent with it.
fn fetching(peers: &Arc<Peers>, pane: &str) -> tokio::task::JoinHandle<Result<(u64, Vec<u8>), String>> {
    let peers = peers.clone();
    let pane = pane.to_string();
    tokio::spawn(async move {
        let mut transfer = peers
            .fetch_attachment(&pane, "an-id", CEILING)
            .await
            .map_err(|e| e.to_string())?;
        let bytes = transfer.header().bytes;
        let mut body = Vec::new();
        while let Some(chunk) = transfer.next_chunk().await {
            body.extend_from_slice(&chunk.map_err(|e| e.to_string())?);
        }
        Ok((bytes, body))
    })
}

/// A peer that has said it answers `att.fetch`, which is what a hub keys both the promise and the
/// request on.
async fn join_serving_attachments(peers: &Arc<Peers>, store: &Store) -> Peer {
    let mut peer = join(peers, store, KEY, "01JA", "laptop").await;
    peer.send(json!({ "t": "hello", "node_id": "01JA", "caps": { "attachments": true } }))
        .await;
    for _ in 0..200 {
        if peers.can_serve_attachments("01JA") {
            return peer;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the hub never took the peer at its word");
}

/// A hub that asked for the whole record at once would put 2.22 MB (#247) on the socket carrying
/// every pane's frames, at the WAN hop — the head-of-line problem the attachment route is HTTP to
/// avoid. It asks a chunk at a time instead, and *grants* the next one only once it has handed the
/// last downstream, so what the hub holds is the window and never the record.
#[tokio::test]
async fn an_attachment_crosses_the_link_a_chunk_at_a_time_and_each_one_is_asked_for() {
    let store = store().await;
    let peers = peers();
    let mut peer = join_serving_attachments(&peers, &store).await;
    let pulled = fetching(&peers, "01JA/w1:p1");

    let asked = peer.request_but_ping().await;
    assert_eq!(asked["t"], "att.fetch", "{asked}");
    assert_eq!(asked["pane"], "01JA/w1:p1");
    assert_eq!(asked["id"], "an-id");
    assert_eq!(
        asked["window"],
        json!(kampr_mesh::ATT_WINDOW),
        "the peer was not told how far ahead it may run: {asked}",
    );
    let rid = asked["rid"].as_u64().expect("an rid");

    peer.send(json!({
        "t": "att.open", "rid": rid, "bytes": 6, "kind": "image", "mime": "image/png"
    }))
    .await;
    peer.send(json!({ "t": "att.chunk", "rid": rid, "seq": 0, "b64": b64(b"abc") }))
        .await;
    assert_eq!(
        peer.request_but_ping().await,
        json!({ "t": "att.more", "rid": rid, "n": 1 }),
        "the hub took a chunk without granting one back, so the window only ever shrinks",
    );
    peer.send(json!({ "t": "att.chunk", "rid": rid, "seq": 1, "b64": b64(b"def") }))
        .await;
    assert_eq!(peer.request_but_ping().await["t"], "att.more");
    peer.send(json!({ "t": "att.end", "rid": rid })).await;

    let (bytes, body) = pulled.await.expect("the fetch task").expect("a transfer");
    assert_eq!(bytes, 6);
    assert_eq!(body, b"abcdef");
}

/// The ceiling is read off a claim, before anything is pulled — the same shape the local route
/// has, where the decoded length comes off the record's base64 rather than off a decode.
#[tokio::test]
async fn an_attachment_past_the_ceiling_is_refused_before_a_byte_of_it_is_pulled() {
    let store = store().await;
    let peers = peers();
    let mut peer = join_serving_attachments(&peers, &store).await;
    let pulled = fetching(&peers, "01JA/w1:p1");

    let rid = peer.request_but_ping().await["rid"].as_u64().expect("an rid");
    peer.send(json!({
        "t": "att.open", "rid": rid, "bytes": CEILING + 1, "kind": "image"
    }))
    .await;

    let refusal = pulled.await.expect("the fetch task").expect_err("a refusal");
    assert!(refusal.contains("larger than"), "{refusal}");
    assert_eq!(
        peer.request_but_ping().await,
        json!({ "t": "att.stop", "rid": rid }),
        "the hub asked for a chunk of something it had already refused",
    );
}

/// A peer that announces less than it sends is a peer this hub stops reading from. The claim is
/// how the ceiling is enforced, so nothing may arrive past it.
#[tokio::test]
async fn a_peer_that_sends_more_than_it_announced_is_cut_off() {
    let store = store().await;
    let peers = peers();
    let mut peer = join_serving_attachments(&peers, &store).await;
    let pulled = fetching(&peers, "01JA/w1:p1");

    let rid = peer.request_but_ping().await["rid"].as_u64().expect("an rid");
    peer.send(json!({ "t": "att.open", "rid": rid, "bytes": 2, "kind": "image" }))
        .await;
    peer.send(json!({ "t": "att.chunk", "rid": rid, "seq": 0, "b64": b64(b"far too much") }))
        .await;

    let refusal = pulled.await.expect("the fetch task").expect_err("a refusal");
    assert!(refusal.contains("more bytes than it announced"), "{refusal}");
}

/// A body that stops short is a body that stops short: the client is promised a `Content-Length`
/// before the first chunk goes out, so a short read has to be an error rather than a clean end.
#[tokio::test]
async fn an_attachment_that_ends_early_is_an_error_and_not_a_short_body() {
    let store = store().await;
    let peers = peers();
    let mut peer = join_serving_attachments(&peers, &store).await;
    let pulled = fetching(&peers, "01JA/w1:p1");

    let rid = peer.request_but_ping().await["rid"].as_u64().expect("an rid");
    peer.send(json!({ "t": "att.open", "rid": rid, "bytes": 9, "kind": "image" }))
        .await;
    peer.send(json!({ "t": "att.chunk", "rid": rid, "seq": 0, "b64": b64(b"abc") }))
        .await;
    peer.send(json!({ "t": "att.end", "rid": rid })).await;

    let refusal = pulled.await.expect("the fetch task").expect_err("a refusal");
    assert!(refusal.contains("stopped sending"), "{refusal}");
}

/// The client hung up. Nothing downstream will ever read these bytes, so the peer is told rather
/// than left pushing a megabyte into a hub that discards it.
#[tokio::test]
async fn a_client_that_walks_away_stops_the_peer_sending() {
    let store = store().await;
    let peers = peers();
    let mut peer = join_serving_attachments(&peers, &store).await;

    let transfer = {
        let asked = tokio::spawn({
            let peers = peers.clone();
            async move { peers.fetch_attachment("01JA/w1:p1", "an-id", CEILING).await }
        });
        let rid = peer.request_but_ping().await["rid"].as_u64().expect("an rid");
        peer.send(json!({ "t": "att.open", "rid": rid, "bytes": 3, "kind": "image" }))
            .await;
        (asked.await.expect("the fetch task").expect("a transfer"), rid)
    };
    let (transfer, rid) = transfer;
    drop(transfer);

    assert_eq!(
        peer.request_but_ping().await,
        json!({ "t": "att.stop", "rid": rid }),
        "the peer was left streaming to nobody",
    );
}

/// A peer that goes quiet must end the request rather than hold a client's socket open for ever.
/// The bound is the one a manage op already waits on.
#[tokio::test]
async fn a_peer_that_stalls_ends_the_transfer_rather_than_hanging_it() {
    let store = store().await;
    let peers = peers();
    let mut peer = join_serving_attachments(&peers, &store).await;
    // The clock stops here rather than at the top: sqlx acquires on tokio time too, and a store
    // opened under a paused one never finishes.
    tokio::time::pause();
    let pulled = fetching(&peers, "01JA/w1:p1");

    let rid = peer.request_but_ping().await["rid"].as_u64().expect("an rid");
    peer.send(json!({ "t": "att.open", "rid": rid, "bytes": 6, "kind": "image" }))
        .await;
    peer.send(json!({ "t": "att.chunk", "rid": rid, "seq": 0, "b64": b64(b"abc") }))
        .await;
    // …and then says nothing at all about the second half.

    let refusal = tokio::time::timeout(Duration::from_secs(120), pulled)
        .await
        .expect("the fetch never gave up")
        .expect("the fetch task")
        .expect_err("a refusal");
    assert!(refusal.contains("did not answer"), "{refusal}");
}

/// A peer that never answers the first ask is the same bounded wait, one step earlier.
#[tokio::test]
async fn a_peer_that_never_opens_the_attachment_ends_the_request() {
    let store = store().await;
    let peers = peers();
    let mut peer = join_serving_attachments(&peers, &store).await;
    tokio::time::pause();
    let pulled = fetching(&peers, "01JA/w1:p1");
    let _asked = peer.request_but_ping().await;

    let refusal = tokio::time::timeout(Duration::from_secs(120), pulled)
        .await
        .expect("the fetch never gave up")
        .expect("the fetch task")
        .expect_err("a refusal");
    assert!(refusal.contains("did not answer"), "{refusal}");
}

/// A link that drops mid-body ends the transfer with it. Waiting out the deadline would hold a
/// client's socket open for ten seconds after the answer was already known.
#[tokio::test]
async fn a_link_that_drops_mid_attachment_ends_the_transfer_with_it() {
    let store = store().await;
    let peers = peers();
    let mut peer = join_serving_attachments(&peers, &store).await;
    let pulled = fetching(&peers, "01JA/w1:p1");

    let rid = peer.request_but_ping().await["rid"].as_u64().expect("an rid");
    peer.send(json!({ "t": "att.open", "rid": rid, "bytes": 6, "kind": "image" }))
        .await;
    peer.send(json!({ "t": "att.chunk", "rid": rid, "seq": 0, "b64": b64(b"abc") }))
        .await;
    peer.close().await;

    let refusal = tokio::time::timeout(Duration::from_secs(5), pulled)
        .await
        .expect("the transfer outlived the link it was on")
        .expect("the fetch task")
        .expect_err("a refusal");
    assert!(refusal.contains("stopped sending"), "{refusal}");
}

/// The promise a hub relays is keyed on this: a build that does not answer `att.fetch` must not
/// have an attachment button rendered for it, and only the peer's own `hello` says which it is.
#[tokio::test]
async fn a_peer_promises_nothing_about_attachments_until_its_hello_says_so() {
    let store = store().await;
    let peers = peers();
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;

    assert!(
        !peers.can_serve_attachments("01JA/w1:p1"),
        "a peer that has said nothing was taken at a word it never gave",
    );
    peer.send(json!({ "t": "hello", "node_id": "01JA", "caps": { "scrollback": true } }))
        .await;
    settle(&peers, |_| true).await;
    assert!(
        !peers.can_serve_attachments("01JA/w1:p1"),
        "a build with no `att.fetch` was promised anyway",
    );

    peer.send(json!({ "t": "hello", "node_id": "01JA", "caps": { "attachments": true } }))
        .await;
    for _ in 0..100 {
        if peers.can_serve_attachments("01JA/w1:p1") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("a peer that says it serves attachments is still not believed");
}

/// Offline is one of the three reasons the button must be absent, and it is the one that arrives
/// while a client is looking at the pane.
#[tokio::test]
async fn a_peer_that_has_gone_promises_nothing_about_attachments() {
    let store = store().await;
    let peers = peers();
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    peer.send(json!({ "t": "hello", "node_id": "01JA", "caps": { "attachments": true } }))
        .await;
    for _ in 0..100 {
        if peers.can_serve_attachments("01JA/w1:p1") {
            break;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(peers.can_serve_attachments("01JA/w1:p1"));

    peer.close().await;
    for _ in 0..100 {
        if !peers.can_serve_attachments("01JA/w1:p1") {
            return;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    panic!("a node that has left the herd is still promised to serve its attachments");
}

/// The promise and the request are keyed on the same fact, so a build with no `att.fetch` is
/// refused here rather than left waiting out a deadline for an answer that is never coming.
#[tokio::test]
async fn a_peer_with_no_attachment_route_is_refused_rather_than_asked() {
    let store = store().await;
    let peers = peers();
    let mut peer = join(&peers, &store, KEY, "01JA", "laptop").await;
    for _ in 0..200 {
        if peers.link_for("01JA").is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let refusal = peers
        .fetch_attachment("01JA/w1:p1", "an-id", CEILING)
        .await
        .err()
        .expect("a refusal");
    assert!(refusal.to_string().contains("no attachment route"), "{refusal}");
    assert_eq!(
        tokio::time::timeout(Duration::from_millis(200), peer.request_but_ping())
            .await
            .ok(),
        None,
        "the hub asked a peer it already knew could not answer",
    );
}
