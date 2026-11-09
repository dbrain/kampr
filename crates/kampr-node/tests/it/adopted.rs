//! A node hosting a child node over a command's stdio, and that child reached from a hub two hops
//! up (docs/19 "Adoption" and "Through a hub"). The child is the real `kampr serve --peer-stdio`.

use crate::attachment::Harness;
use crate::native::Client;
use crate::streaming;
use crate::upload;
use kampr_auth::{MeshRole, Role};
use kampr_journal::attach::{FileRef, Source};
use kampr_node::child::{Child, ChildSpec, ChildState, child_node_id};
use kampr_node::{Config, Node};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio_tungstenite::tungstenite;

const SOON: Duration = Duration::from_secs(30);

struct Chain {
    top: streaming::Client,
    middle: Harness,
    child: Option<Child>,
    grand_id: String,
    grand: tempfile::TempDir,
    keys: tempfile::TempDir,
}

impl Drop for Chain {
    fn drop(&mut self) {
        drop(self.child.take());
        kill_keepers(&self.grand.path().join("native"));
    }
}

fn kill_keepers(root: &Path) {
    for session in kampr_keep::list(root).unwrap_or_default() {
        if let Some(keeper) = session.keeper() {
            unsafe {
                libc::kill(-(keeper.child as i32), libc::SIGKILL);
                libc::kill(keeper.keeper as i32, libc::SIGKILL);
            }
        }
    }
}

impl Chain {
    async fn start() -> Self {
        let top = streaming::Client::start_with(|config| config.mesh.accept = true).await;
        let middle = Harness::start().await;
        link(&top.h.node, &top.h.origin, &middle.node).await;

        let grand = tempfile::Builder::new()
            .prefix("kad")
            .tempdir_in("/tmp")
            .expect("a grandchild home");
        grandchild_config(grand.path());
        let keys = tempfile::tempdir().expect("a key dir");
        let grand_id = child_node_id(middle.node.node_id(), "grand");
        let mut chain = Self {
            top,
            middle,
            child: None,
            grand_id,
            grand,
            keys,
        };
        chain.spawn();
        chain.reachable_at_top(true).await;
        chain
    }

    fn spec(&self) -> ChildSpec {
        let dir = |name: &str| self.grand.path().join(name).display().to_string();
        ChildSpec {
            argv: vec![
                kampr_testkit::kampr().display().to_string(),
                "serve".into(),
                "--peer-stdio".into(),
            ],
            env: vec![
                ("KAMPR_CONFIG_DIR".into(), dir("config")),
                ("KAMPR_STATE_DIR".into(), dir("state")),
                ("KAMPR_LOG".into(), "debug".into()),
            ],
            node_id: self.grand_id.clone(),
            node_name: "grand".into(),
            key: self.keys.path().join("grand.key"),
        }
    }

    fn spawn(&mut self) {
        self.child = Some(kampr_node::child::spawn(&self.middle.node, self.spec()).expect("a child"));
    }

    async fn reachable_at_top(&self, want: bool) {
        let top = self.top.h.node.clone();
        let id = self.grand_id.clone();
        settles("the grandchild's reachability at the top", move || {
            top.herd().node(&id).is_some_and(|n| n.is_reachable() == want)
        })
        .await;
    }

    async fn client(&self) -> Client {
        let token = self.top.h.token_for(Role::Full).await;
        let url = self.top.h.origin.replacen("http", "ws", 1) + "/ws";
        let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("kampr.token.{token}").parse().unwrap(),
        );
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("a websocket");
        Client::over(socket, token)
    }

    async fn native_pane(&self, client: &mut Client, script: &str) -> String {
        client
            .send(json!({
                "t": "manage", "op": "native.create", "node": self.grand_id,
                "args": ["sh", "-c", script], "cols": 100, "rows": 24,
            }))
            .await;
        let ack = client.reply("managed").await;
        assert_eq!(ack["ok"], true, "{ack}");
        let id = ack["id"].as_str().expect("a pane id").to_string();
        assert!(id.starts_with(&format!("{}/n:", self.grand_id)), "{id}");
        client.pane(&id, |_| true).await;
        id
    }

    fn grand_home(&self) -> PathBuf {
        self.grand.path().join("journals")
    }
}

/// The middle dials the top as any peer dials its hub.
async fn link(top: &Arc<Node>, origin: &str, middle: &Arc<Node>) {
    let now = kampr_auth::now();
    let middle_key = middle.identity().expect("a key").public_hex();
    top.auth
        .store()
        .mesh()
        .enrol(&middle_key, middle.node_id(), "middle", MeshRole::Peer, None, now)
        .await
        .expect("the middle enrolled at the top");
    let top_key = top.identity().expect("a key").public_hex();
    middle
        .auth
        .store()
        .mesh()
        .enrol(
            &top_key,
            top.node_id(),
            "top",
            MeshRole::Hub,
            Some(&kampr_mesh::mesh_url(origin)),
            now,
        )
        .await
        .expect("the top enrolled at the middle");
    let (top, id) = (top.clone(), middle.node_id().to_string());
    settles("the middle linked to the top", move || {
        top.peers.link_for(&id).is_some()
    })
    .await;
}

fn grandchild_config(home: &Path) {
    let mut config = Config::bootstrap("grand");
    config.muffit.enabled = Some(false);
    config.update.check = false;
    config.journals.home = home.join("journals").display().to_string();
    config.herdr.socket = home.join("no-herdr.sock").display().to_string();
    config.herdr.binary = home.join("no-such-herdr").display().to_string();
    config.herdr.sessions = Some(Vec::new());
    config.native.root = home.join("native").display().to_string();
    config.native.keeper = kampr_testkit::keeper().display().to_string();
    config.native.launch = "detached".into();
    config.fleet.path = "/usr/bin:/bin".into();
    std::fs::create_dir_all(home.join("journals")).expect("a journals home");
    config.save(&home.join("config")).expect("a config");
}

async fn settles(what: &str, done: impl Fn() -> bool) {
    let deadline = tokio::time::Instant::now() + SOON;
    while !done() {
        assert!(tokio::time::Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

async fn reset_where(client: &mut Client, id: &str, want: impl Fn(&Value) -> bool) -> Value {
    let id = id.to_string();
    client
        .until(&format!("a grid.reset of {id} as wanted"), |c| {
            c.resets.get(&id).filter(|r| want(r)).cloned()
        })
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_hosted_node_is_published_as_its_hosts_child_all_the_way_up() {
    let chain = Chain::start().await;
    let middle_id = chain.middle.node.node_id().to_string();
    for (at, node) in [("middle", &chain.middle.node), ("top", &chain.top.h.node)] {
        let entry = node
            .herd()
            .node(&chain.grand_id)
            .cloned()
            .expect("the grandchild");
        assert_eq!(
            entry.parent.as_deref(),
            Some(middle_id.as_str()),
            "at the {at}: {entry:?}"
        );
    }
    let middle = chain
        .top
        .h
        .node
        .herd()
        .node(&middle_id)
        .cloned()
        .expect("the middle");
    assert_eq!(middle.parent, None, "{middle:?}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_grandchilds_native_pane_is_watched_typed_into_and_re_laid_through_two_hubs() {
    let chain = Chain::start().await;
    let mut client = chain.client().await;
    let script = "printf 'W%0149d\\n' 0; printf 'P%0129d' 0; exec cat";
    let id = chain.native_pane(&mut client, script).await;

    client
        .send(json!({ "t": "watch", "pane": id, "scrollback": true }))
        .await;
    let plain = reset_where(&mut client, &id, |_| true).await;
    assert_eq!(plain["cols"], json!(100), "{plain}");
    let marker = format!("two-hops-{}", std::process::id());
    client
        .send(json!({ "t": "input", "pane": id, "text": format!("{marker}\r") }))
        .await;
    client.shows(&id, &marker).await;

    let mut narrow = chain.client().await;
    narrow
        .send(json!({ "t": "watch", "pane": id, "scrollback": true, "reflow": { "cols": 40 } }))
        .await;
    let reset = reset_where(&mut narrow, &id, |r| r["reflow"] == "on" || r["cols"] != 100).await;
    assert_eq!(
        (
            reset["reflow"].clone(),
            reset["cols"].clone(),
            reset["rows"].clone()
        ),
        (json!("on"), json!(40), json!(24)),
        "{reset} {:?}",
        narrow.others
    );
    let screen = narrow.shows(&id, "P0").await;
    assert!(screen.iter().all(|r| r.chars().count() <= 40), "{screen:#?}");

    narrow
        .send(json!({ "t": "find", "pane": id, "query": "P0", "backward": true }))
        .await;
    let found = narrow.reply("find").await;
    assert_eq!(found["pane"], json!(id), "{found}");
    assert!(found["total"].as_u64().is_some_and(|n| n >= 1), "{found}");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_grandchilds_files_are_read_and_written_on_the_grandchild_not_on_its_host() {
    let chain = Chain::start().await;
    std::fs::write(chain.grand_home().join("note.txt"), "the grandchild's").expect("a note");
    std::fs::write(chain.middle.journals_home().join("note.txt"), "the middle's").expect("a note");

    let note = Source::File(FileRef::new("~/note.txt"));
    let read = chain.top.get(&chain.grand_id, &note, &[]).await;
    assert_eq!((read.status, read.text()), (200, "the grandchild's".to_string()));

    let written = upload::upload_as(&chain.top, &chain.top.full, &chain.grand_id, "~", "up.txt", b"up").await;
    assert_eq!(written.status, 201, "{}", written.text());
    assert!(chain.grand_home().join("up.txt").is_file());
    assert!(!chain.middle.journals_home().join("up.txt").exists());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_child_that_ends_leaves_its_sessions_running_and_a_supervised_one_comes_back() {
    let mut chain = Chain::start().await;
    let mut client = chain.client().await;
    let id = chain.native_pane(&mut client, "exec cat").await;
    let child = chain.child.take().expect("a child");
    assert_eq!(child.state(), ChildState::Up);
    child.stop().await;
    chain.reachable_at_top(false).await;
    let sessions = kampr_keep::list(&chain.grand.path().join("native")).expect("the sessions");
    assert_eq!(sessions.len(), 1);
    assert!(sessions[0].alive(), "the session went with the node that made it");

    let spec = chain.spec();
    let supervised =
        kampr_node::child::supervise(&chain.middle.node, spec, || async { Ok(true) }).expect("a child");
    let mut state = supervised.subscribe();
    let _ = tokio::time::timeout(SOON, state.wait_for(|s| *s == ChildState::Up))
        .await
        .expect("up again");
    chain.reachable_at_top(true).await;
    let pid = supervised.pid().expect("a pid");
    unsafe { libc::kill(pid as i32, libc::SIGKILL) };
    let _ = tokio::time::timeout(SOON, state.wait_for(|s| *s != ChildState::Up))
        .await
        .expect("down");
    let _ = tokio::time::timeout(SOON, state.wait_for(|s| *s == ChildState::Up))
        .await
        .expect("respawned");
    assert_ne!(supervised.pid(), Some(pid));
    chain.reachable_at_top(true).await;
    client.send(json!({ "t": "watch", "pane": id })).await;
    reset_where(&mut client, &id, |_| true).await;

    let key = chain.spec().key;
    let pubkey = kampr_auth::NodeIdentity::load(&key)
        .unwrap()
        .unwrap()
        .public_hex();
    supervised.release(&chain.middle.node).await.expect("released");
    chain.reachable_at_top(false).await;
    let row = chain
        .middle
        .node
        .auth
        .store()
        .mesh()
        .node(&pubkey)
        .await
        .unwrap()
        .expect("a row");
    assert!(!row.active(), "{row:?}");
}
