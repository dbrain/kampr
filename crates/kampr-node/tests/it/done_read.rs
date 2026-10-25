//! A finish read on one device is read on all of them.
//!
//! herdr's `done` only falls when the pane is focused at the desk, which Kampr may never do (rule
//! 3), so a pane read on the desktop used to stay `done` everywhere else — and the phone went on
//! saying "6 agents finished" about work the operator had already looked at. The node now keeps
//! what was read and publishes such a pane as `idle`, which every client and the push watcher
//! already treat as nothing to report.

use kampr_auth::Role;
use kampr_core::provider::{AgentStatus, PaneInfo};
use kampr_core::wire::PaneEntry;
use kampr_mesh::transport::{Incoming, Outgoing};
use kampr_node::herd::HerdModel;
use kampr_node::session::Caller;
use kampr_node::{Config, Node};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const PANE: &str = "01J/w1:p1";

async fn node_at(home: &Path) -> Arc<Node> {
    let state_dir = home.join("state");
    std::fs::create_dir_all(&state_dir).expect("a state dir");
    let mut config = Config::bootstrap("done-read");
    config.muffit.enabled = Some(false);
    config.update.check = false;
    config.config_dir = home.join("config").display().to_string();
    config.state_dir = state_dir.display().to_string();
    config.herdr.socket = home.join("herdr.sock").display().to_string();
    config.native.root = home.join("native").display().to_string();
    config.herdr.binary = home.join("no-such-herdr").display().to_string();
    config.herdr.sessions = Some(Vec::new());
    let node = Node::start(config, &state_dir).await.expect("a node");
    // The node's own rebuild publishes an empty herd for a node with no herdr, and one landing
    // after `publish_herd` would take the finished pane away under the test.
    node.shutdown();
    let mut settling = node.subscribe_herd();
    while tokio::time::timeout(Duration::from_millis(200), settling.changed())
        .await
        .is_ok()
    {}
    node
}

fn finished(at: Option<&str>) -> HerdModel {
    let mut pane = PaneEntry::new(
        "01J",
        &PaneInfo {
            pane_id: "w1:p1".into(),
            agent: Some("claude".into()),
            agent_status: AgentStatus::Done,
            ..PaneInfo::default()
        },
        false,
    );
    pane.updated_at = at.map(str::to_string);
    HerdModel {
        nodes: Vec::new(),
        panes: vec![pane],
    }
}

struct Client {
    out: kampr_mesh::transport::Sender,
    from_node: kampr_mesh::transport::Receiver,
    session: tokio::task::JoinHandle<()>,
}

impl Drop for Client {
    fn drop(&mut self) {
        self.session.abort();
    }
}

async fn connect(node: &Arc<Node>, name: &str) -> Client {
    let device = node
        .auth
        .store()
        .create_device(name, Role::Full, kampr_auth::now(), None, None, None)
        .await
        .expect("a device");
    let (node_side, client_side) = kampr_mesh::transport::pair();
    let (out, incoming) = node_side.split();
    let session = tokio::spawn(kampr_node::session::run_on(
        out,
        incoming,
        node.clone(),
        device,
        "test".into(),
        Caller::Client,
    ));
    let (out, from_node) = client_side.split();
    Client {
        out,
        from_node,
        session,
    }
}

impl Client {
    async fn send(&mut self, msg: Value) {
        assert!(self.out.send(msg.to_string()).await, "the session hung up");
    }

    /// Every frame this client sent before it has been handled.
    async fn settled(&mut self) {
        self.send(json!({ "t": "ping", "n": 7 })).await;
        let pong = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(text) = self.from_node.recv().await {
                if serde_json::from_str::<Value>(&text).is_ok_and(|m| m["t"] == "pong") {
                    return;
                }
            }
        })
        .await;
        assert!(pong.is_ok(), "the session never answered a ping");
    }

    /// Waits for the node to tell this client the pane is `want`, and names what it said instead.
    async fn sees(&mut self, want: &str) -> Result<(), Vec<String>> {
        let mut seen = Vec::new();
        let found = tokio::time::timeout(Duration::from_secs(5), async {
            while let Some(text) = self.from_node.recv().await {
                let msg: Value = serde_json::from_str(&text).expect("a server message");
                if let Some(status) = status_in(&msg) {
                    seen.push(status.clone());
                    if status == want {
                        return true;
                    }
                }
            }
            false
        })
        .await
        .unwrap_or(false);
        match found {
            true => Ok(()),
            false => Err(seen),
        }
    }
}

fn status_in(msg: &Value) -> Option<String> {
    [&msg["panes"], &msg["added"]["panes"], &msg["changed"]["panes"]]
        .into_iter()
        .filter_map(Value::as_array)
        .flatten()
        .find(|p| p["id"] == PANE)
        .and_then(|p| p["agent_status"].as_str())
        .map(str::to_string)
}

fn read(at: Option<&str>) -> Value {
    json!({ "t": "done.read", "pane": PANE, "at": at })
}

#[tokio::test]
async fn a_finish_read_on_one_device_is_idle_on_every_other() {
    let home = tempfile::tempdir().unwrap();
    let node = node_at(home.path()).await;
    node.publish_herd(finished(Some("2026-10-14T09:00:00Z")));
    let mut desktop = connect(&node, "desktop").await;
    let mut phone = connect(&node, "phone").await;
    desktop
        .sees("done")
        .await
        .expect("the desktop is told it finished");
    phone.sees("done").await.expect("the phone is told it finished");

    desktop.send(read(Some("2026-10-14T09:00:00Z"))).await;

    phone
        .sees("idle")
        .await
        .expect("read on the desktop, so nothing is waiting to be read on the phone");
}

/// Read is keyed by the finish, so the *next* finish is news again rather than the pane going
/// quiet for good.
#[tokio::test]
async fn the_next_finish_of_a_read_pane_is_unread_again() {
    let home = tempfile::tempdir().unwrap();
    let node = node_at(home.path()).await;
    node.publish_herd(finished(Some("2026-10-14T09:00:00Z")));
    let mut desktop = connect(&node, "desktop").await;
    desktop.sees("done").await.unwrap();
    desktop.send(read(Some("2026-10-14T09:00:00Z"))).await;
    desktop.sees("idle").await.unwrap();

    node.publish_herd(finished(Some("2026-10-14T09:30:00Z")));

    desktop
        .sees("done")
        .await
        .expect("a pane that finished again has something new to read");
}

/// A read that names an older finish than the one the herd holds is about work the operator has
/// not seen: the pane finished again between the look and the frame arriving.
#[tokio::test]
async fn a_read_of_an_older_finish_leaves_the_newer_one_unread() {
    let home = tempfile::tempdir().unwrap();
    let node = node_at(home.path()).await;
    node.publish_herd(finished(Some("2026-10-14T09:30:00Z")));
    let mut desktop = connect(&node, "desktop").await;
    desktop.sees("done").await.unwrap();

    desktop.send(read(Some("2026-10-14T09:00:00Z"))).await;
    desktop.settled().await;
    node.publish_herd(finished(Some("2026-10-14T09:30:00Z")));

    assert_eq!(
        node.herd().pane(PANE).map(|p| p.agent_status),
        Some(AgentStatus::Done)
    );
}

/// A node restarts on every upgrade, and a ledger that forgot would re-raise every finish the
/// operator had already read — and buzz the phone about all of them at once.
#[tokio::test]
async fn what_was_read_survives_the_node_restarting() {
    let home = tempfile::tempdir().unwrap();
    {
        let node = node_at(home.path()).await;
        node.publish_herd(finished(Some("2026-10-14T09:00:00Z")));
        let mut desktop = connect(&node, "desktop").await;
        desktop.sees("done").await.unwrap();
        desktop.send(read(Some("2026-10-14T09:00:00Z"))).await;
        desktop.sees("idle").await.unwrap();
    }

    let node = node_at(home.path()).await;
    node.publish_herd(finished(Some("2026-10-14T09:00:00Z")));
    let mut phone = connect(&node, "phone").await;

    phone
        .sees("idle")
        .await
        .expect("the read outlives the process that took it");
}
