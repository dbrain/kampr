//! A hub serving a peer's files end to end: a real hub node's HTTP route, a real `Peers` link, and
//! a real peer node's session on the far end of it — everything but the socket between them.

use crate::streaming::{self, Client, MIB};
use crate::upload::{self, listing};
use kampr_auth::{MeshRole, Role};
use kampr_journal::attach::{FileRef, Source};
use kampr_mesh::handshake::Accepted;
use kampr_mesh::transport::pair;
use kampr_node::session::{self, Caller};
use kampr_node::{Config, Node};
use std::sync::Arc;
use std::time::Duration;

const KEY: &str = "3333333333333333333333333333333333333333333333333333333333333333";

struct Herd {
    c: Client,
    peer: Arc<Node>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    _state: tempfile::TempDir,
}

impl Drop for Herd {
    fn drop(&mut self) {
        for task in &self.tasks {
            task.abort();
        }
        self.peer.shutdown();
    }
}

impl Herd {
    /// `role` is the hub's device row *on the peer*, which is the second gate an upload or a path
    /// passes — the hub's own device row for the client is the first.
    async fn start(role: Role) -> Self {
        Self::start_with(role, |_| {}).await
    }

    async fn start_with(role: Role, tweak: impl FnOnce(&mut Config)) -> Self {
        let c = Client::start().await;
        let state = tempfile::tempdir().expect("a state dir");
        let home = state.path().join("home");
        std::fs::create_dir_all(&home).expect("a home");
        let mut config = Config::bootstrap("peer");
        config.muffit.enabled = Some(false);
        config.update.check = false;
        config.journals.home = home.display().to_string();
        config.herdr.socket = state.path().join("no-herdr.sock").display().to_string();
        config.native.root = state.path().join("native").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        tweak(&mut config);
        let peer = Node::start(config, state.path()).await.expect("a peer");

        let device = peer
            .auth
            .store()
            .create_device("hub", role, kampr_auth::now(), None, None, None)
            .await
            .expect("a device");
        let store = c.h.node.auth.store().clone();
        let enrolled = store
            .mesh()
            .enrol(
                KEY,
                peer.node_id(),
                "peer",
                MeshRole::Peer,
                None,
                kampr_auth::now(),
            )
            .await
            .expect("an enrolment");
        let (hub_side, peer_side) = pair();
        let (out, incoming) = hub_side.split();
        let accepted = Accepted {
            node: enrolled,
            build: "0.1.0".into(),
            enrolled: false,
            store,
        };
        let peers = c.h.node.peers.clone();
        let serve = tokio::spawn(async move { peers.serve(accepted, out, incoming).await });
        let (out, incoming) = peer_side.split();
        let session = tokio::spawn(session::run_on(
            out,
            incoming,
            peer.clone(),
            device,
            "mesh:test".into(),
            Caller::Hub,
        ));
        for _ in 0..500 {
            if c.h.node.peers.can_serve_attachments(peer.node_id()) {
                return Self {
                    c,
                    peer,
                    tasks: vec![serve, session],
                    _state: state,
                };
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("the hub never heard the peer say it serves attachments");
    }

    fn peer_id(&self) -> String {
        self.peer.node_id().to_string()
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peers_file_past_the_old_ceiling_crosses_the_mesh_whole_and_by_range() {
    let herd = Herd::start(Role::Full).await;
    assert_ne!(herd.peer_id(), herd.c.h.node.node_id());
    let dir = tempfile::tempdir().expect("a dir");
    let (path, bytes) = streaming::a_big_file(dir.path(), 20 * MIB);
    streaming::assert_ranges(&herd.c, &herd.peer_id(), &path, &bytes).await;
}

#[tokio::test(flavor = "multi_thread")]
async fn the_tail_of_a_peers_huge_file_crosses_the_mesh_without_the_rest() {
    let herd = Herd::start(Role::Full).await;
    let dir = tempfile::tempdir().expect("a dir");
    let len: u64 = 12 * 1024 * 1024 * 1024;
    let path = streaming::a_sparse_file(dir.path(), len, b"the very end");
    let tail = streaming::within_seconds(herd.c.get(
        &herd.peer_id(),
        &streaming::file(&path),
        &[("Range", &format!("bytes={}-", len - 12))],
    ))
    .await;
    assert_eq!(tail.status, 206, "{}", tail.text());
    assert_eq!(
        tail.header("content-range"),
        Some(format!("bytes {}-{}/{len}", len - 12, len - 1).as_str())
    );
    assert_eq!(tail.body, b"the very end");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peers_directory_crosses_the_mesh_as_a_zip_of_unknown_length() {
    let herd = Herd::start(Role::Full).await;
    let dir = tempfile::tempdir().expect("a dir");
    let (root, files) = streaming::a_tree(dir.path());
    let zip = herd
        .c
        .get(&herd.peer_id(), &Source::Zip(FileRef::new(&root)), &[])
        .await;
    assert_eq!(zip.status, 200, "{}", zip.text());
    assert_eq!(zip.header("content-type"), Some("application/zip"));
    assert_eq!(zip.header("content-length"), None);
    assert_eq!(
        zip.header("content-disposition"),
        Some("attachment; filename=\"tree.zip\"")
    );
    streaming::assert_tree(&streaming::unzip(&zip.body), &files);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peers_staged_and_unstaged_diffs_cross_the_mesh() {
    let herd = Herd::start(Role::Full).await;
    let dir = tempfile::tempdir().expect("a dir");
    let root = dir.path().canonicalize().expect("a root");
    let git = |args: &[&str]| {
        let ok = std::process::Command::new("git")
            .arg("-C")
            .arg(&root)
            .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
            .args(args)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .status()
            .expect("git")
            .success();
        assert!(ok, "git {args:?}");
    };
    git(&["init", "-q", "-b", "main"]);
    std::fs::write(root.join("a.md"), "one\n").expect("write");
    git(&["add", "-A"]);
    git(&["commit", "-qm", "first"]);
    std::fs::write(root.join("a.md"), "two\n").expect("write");
    git(&["add", "-A"]);
    std::fs::write(root.join("a.md"), "three\n").expect("write");

    let a = FileRef::new(root.join("a.md"));
    let staged = herd.c.get(&herd.peer_id(), &Source::Staged(a.clone()), &[]).await;
    let unstaged = herd.c.get(&herd.peer_id(), &Source::Unstaged(a), &[]).await;
    assert!(
        staged.text().contains("+two") && !staged.text().contains("three"),
        "{}",
        staged.text()
    );
    assert!(
        unstaged.text().contains("-two") && unstaged.text().contains("+three"),
        "{}",
        unstaged.text()
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_uploaded_into_a_directory_on_a_peer() {
    let herd = Herd::start(Role::Full).await;
    let dir = tempfile::tempdir().expect("a dir");
    let root = dir.path().canonicalize().expect("a root");
    upload::assert_uploads(&herd.c, &herd.peer_id(), &root).await;
}

/// The hub's device row for the client may write; the peer's row for the hub may not. The second
/// gate is the one that holds, for a read and for a write alike.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_will_not_take_input_from_the_hub_refuses_its_uploads_and_its_paths() {
    let herd = Herd::start(Role::Readonly).await;
    let dir = tempfile::tempdir().expect("a dir");
    let root = dir.path().canonicalize().expect("a root");
    let refused = upload::upload(&herd.c, &herd.peer_id(), &root, "x.txt", b"x").await;
    assert_eq!(refused.status, 403, "{}", refused.text());
    assert!(listing(&root).is_empty());

    let (path, _) = streaming::a_big_file(&root, 1024);
    let read = herd.c.get(&herd.peer_id(), &streaming::file(&path), &[]).await;
    assert_eq!(read.status, 404, "{}", read.text());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peer_drops_a_relayed_upload_that_stops_moving_though_the_hub_still_holds_it() {
    let herd = Herd::start_with(Role::Full, |config| config.limits.upload_idle_secs = 1).await;
    let dir = tempfile::tempdir().expect("a dir");
    let root = dir.path().canonicalize().expect("a root");
    let mut held = herd
        .c
        .h
        .node
        .peers
        .upload(&herd.peer_id(), &root.display().to_string(), "stuck.bin")
        .await
        .expect("an upload");
    held.send(b"a start").await.expect("a chunk");
    assert_eq!(listing(&root).len(), 1, "the peer should be writing a temp file");
    for _ in 0..50 {
        if listing(&root).is_empty() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(listing(&root).is_empty(), "{:?}", listing(&root));
    drop(held);
}
