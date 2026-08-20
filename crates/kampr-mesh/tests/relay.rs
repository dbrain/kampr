//! The hub half, driven by a scripted peer.
//!
//! The peer here speaks the ordinary v1 protocol over an in-process link, which is exactly what a
//! real peer's client session does over a WebSocket — so these tests exercise the relay, the
//! shadow and the herd merge without a socket, a terminal or a herd.

use kampr_auth::{MeshNode, MeshRole};
use kampr_core::registry::PaneUpdate;
use kampr_mesh::handshake::Accepted;
use kampr_mesh::transport::{Incoming, Link, Outgoing, Receiver, Sender, pair};
use kampr_mesh::{PeerState, Peers, PeersConfig, RemoteEvent};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const KEY: &str = "1111111111111111111111111111111111111111111111111111111111111111";
const OTHER: &str = "2222222222222222222222222222222222222222222222222222222222222222";

fn accepted(pubkey: &str, node_id: &str, name: &str) -> Accepted {
    Accepted {
        node: MeshNode {
            pubkey: pubkey.into(),
            node_id: node_id.into(),
            name: name.into(),
            role: MeshRole::Peer,
            url: None,
            created_at: 0,
            last_seen_at: None,
            revoked_at: None,
        },
        build: "0.1.0".into(),
        enrolled: false,
    }
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
}

fn join(peers: &Arc<Peers>, pubkey: &str, node_id: &str, name: &str) -> Peer {
    let (hub_side, peer_side) = pair();
    let (out, incoming) = hub_side.split();
    let accepted = accepted(pubkey, node_id, name);
    let peers = peers.clone();
    tokio::spawn(async move { peers.serve(accepted, out, incoming).await });
    Peer { link: peer_side }
}

fn peers() -> Arc<Peers> {
    Peers::new(PeersConfig {
        // Keepalives are measured elsewhere; here they would only be noise in the request stream.
        ping_interval: Duration::from_secs(3600),
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

fn text_of(update: &PaneUpdate) -> Vec<String> {
    update
        .rows()
        .iter()
        .map(|row| row.cells.iter().map(|c| c.ch).collect::<String>())
        .collect()
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
    let mut peer = join(&peers, KEY, "01JA", "laptop");
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let herd = peers.herd();
    assert_eq!(herd.nodes[0].id, "01JA");
    assert_eq!(herd.nodes[0].kind, "peer", "a local node to itself is a peer to us");
    assert!(herd.nodes[0].online);
    assert_eq!(herd.nodes[0].build.as_deref(), Some("0.1.0"));
    assert_eq!(herd.panes[0].id, "01JA/w1:p1");
    assert_eq!(peers.state("01JA/w1:p1"), PeerState::Live);
    assert_eq!(peers.state("01JZ/w1:p1"), PeerState::Unknown);
}

#[tokio::test]
async fn a_pane_is_watched_once_upstream_however_many_clients_look_at_it() {
    let peers = peers();
    let mut peer = join(&peers, KEY, "01JA", "laptop");
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    let mut first = peers.watch("01JA/w1:p1").expect("a live peer");
    let request = peer.request_but_ping().await;
    assert_eq!(request["t"], "watch");
    assert_eq!(request["pane"], "01JA/w1:p1");

    peer.send(reset("01JA/w1:p1", "hello")).await;
    assert_eq!(text_of(&next_update(&mut first).await), ["hello   ", "        "]);

    // A second client joins the same pane: no second request, and it renders at once from the
    // hub's shadow rather than waiting for the peer to repaint.
    let mut second = peers.watch("01JA/w1:p1").expect("a live peer");
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
    let mut peer = join(&peers, KEY, "01JA", "laptop");
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;

    peers
        .relay("01JA/w1:p1", json!({ "t": "input", "pane": "01JA/w1:p1", "text": "ls\r" }))
        .expect("a live peer");
    let request = peer.request_but_ping().await;
    assert_eq!(request["t"], "input");
    assert_eq!(request["text"], "ls\r");
}

#[tokio::test]
async fn a_pane_on_a_node_nobody_serves_is_unknown_rather_than_offline() {
    let peers = peers();
    let error = peers.watch("01JZ/w1:p1").expect_err("nobody serves it");
    assert_eq!(error.code(), "unknown_pane");
}

#[tokio::test]
async fn a_peer_dropping_costs_its_own_panes_and_nothing_else() {
    let peers = peers();
    let mut laptop = join(&peers, KEY, "01JA", "laptop");
    let mut workshop = join(&peers, OTHER, "01JB", "workshop");
    laptop.send(herd("01JA", &["w1:p1"])).await;
    workshop.send(herd("01JB", &["w1:p1"])).await;
    settle(&peers, |h| h.panes.len() == 2).await;

    let mut watching_laptop = peers.watch("01JA/w1:p1").expect("live");
    let mut watching_workshop = peers.watch("01JB/w1:p1").expect("live");
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
    assert!(laptop_node.detail.as_deref().unwrap_or_default().contains("laptop"));
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
    assert_eq!(peers.watch("01JA/w1:p1").unwrap_err().code(), "node_offline");

    // The other peer never noticed.
    assert_eq!(peers.state("01JB/w1:p1"), PeerState::Live);
    workshop.send(patch("01JB/w1:p1", 0, "still")).await;
    assert_eq!(text_of(&next_update(&mut watching_workshop).await), ["still   "]);
}

#[tokio::test]
async fn a_peer_that_comes_back_is_online_again_with_no_help() {
    let peers = peers();
    let mut laptop = join(&peers, KEY, "01JA", "laptop");
    laptop.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;
    laptop.close().await;
    settle(&peers, |h| h.nodes.iter().any(|n| !n.online)).await;

    let mut again = join(&peers, KEY, "01JA", "laptop");
    again.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| h.nodes.iter().all(|n| n.online)).await;

    let herd = peers.herd();
    assert_eq!(herd.nodes.len(), 1, "one node, not one per connection");
    assert_eq!(herd.panes.len(), 1);
    assert!(peers.watch("01JA/w1:p1").is_ok());
}

#[tokio::test]
async fn history_is_stitched_by_index_and_a_joiner_gets_all_of_it() {
    let peers = peers();
    let mut peer = join(&peers, KEY, "01JA", "laptop");
    peer.send(herd("01JA", &["w1:p1"])).await;
    settle(&peers, |h| !h.panes.is_empty()).await;
    let mut first = peers.watch("01JA/w1:p1").expect("live");
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
        if let Some(RemoteEvent::Scrollback(doc)) =
            tokio::time::timeout(Duration::from_secs(2), first.recv())
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

    let mut joiner = peers.watch("01JA/w1:p1").expect("live");
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
