//! A file from the device, written into a directory on the node: streamed to a temp file beside
//! its destination, renamed only when the last byte is in, never over a file that is already there.

use crate::raw::{self, Reply};
use crate::streaming::{Client, MIB, pattern};
use kampr_auth::Role;
use std::path::Path;

pub(crate) fn upload_path(node: &str, dir: &str, name: &str) -> String {
    let q = |s: &str| url::form_urlencoded::byte_serialize(s.as_bytes()).collect::<String>();
    format!("/api/upload/{node}/_?dir={}&name={}", q(dir), q(name))
}

pub(crate) async fn upload(c: &Client, node: &str, dir: &Path, name: &str, body: &[u8]) -> Reply {
    upload_as(c, &c.full, node, &dir.display().to_string(), name, body).await
}

pub(crate) async fn upload_as(
    c: &Client,
    token: &str,
    node: &str,
    dir: &str,
    name: &str,
    body: &[u8],
) -> Reply {
    raw::send(
        &c.h.origin,
        "POST",
        &upload_path(node, dir, name),
        &[
            ("Authorization", token),
            ("Content-Type", "application/octet-stream"),
        ],
        body,
    )
    .await
}

pub(crate) fn listing(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .expect("a dir")
        .map(|e| e.expect("an entry").file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// Everything one node has to say about uploads into `dir`, wherever the node is.
pub(crate) async fn assert_uploads(c: &Client, node: &str, dir: &Path) {
    let body = pattern(5 * MIB + 17);
    let first = upload(c, node, dir, "notes.tar.gz", &body).await;
    assert_eq!(first.status, 201, "{}", first.text());
    let written = dir.join("notes.tar.gz");
    assert_eq!(first.json()["path"], written.display().to_string());
    assert_eq!(first.json()["bytes"], body.len());
    assert!(std::fs::read(&written).expect("the upload") == body);

    for n in 1..=2 {
        let again = upload(c, node, dir, "notes.tar.gz", format!("v{n}").as_bytes()).await;
        assert_eq!(again.status, 201, "{}", again.text());
        let path = dir.join(format!("notes.tar ({n}).gz"));
        assert_eq!(again.json()["path"], path.display().to_string());
        assert_eq!(
            std::fs::read(&path).expect("the second upload"),
            format!("v{n}").as_bytes()
        );
    }
    assert!(
        std::fs::read(&written).expect("the first upload") == body,
        "an upload overwrote a file"
    );
    let dotfile = upload(c, node, dir, ".env", b"a").await;
    assert_eq!(dotfile.status, 201);
    assert_eq!(
        upload(c, node, dir, ".env", b"b").await.json()["path"],
        dir.join(".env (1)").display().to_string()
    );

    let long = "x".repeat(256);
    for bad in [
        "",
        ".",
        "..",
        "a/b",
        "../escape",
        "/etc/passwd",
        "nul\0byte",
        long.as_str(),
    ] {
        let refused = upload(c, node, dir, bad, b"x").await;
        assert_eq!(refused.status, 400, "{bad:?}: {}", refused.text());
    }

    let file = dir.join("notes.tar.gz").display().to_string();
    let absent = dir.join("absent").display().to_string();
    for missing in [absent.as_str(), file.as_str(), "relative/dir"] {
        let refused = upload_as(c, &c.full, node, missing, "x.txt", b"x").await;
        assert_eq!(refused.status, 404, "{missing}: {}", refused.text());
    }

    let readonly = format!("Bearer {}", c.h.token_for(Role::Readonly).await);
    let refused = upload_as(c, &readonly, node, &dir.display().to_string(), "ro.txt", b"x").await;
    assert_eq!(refused.status, 403, "{}", refused.text());
    assert!(!dir.join("ro.txt").exists());

    let before = listing(dir);
    raw::abandon(
        &c.h.origin,
        &upload_path(node, &dir.display().to_string(), "partial.bin"),
        &[("Authorization", c.full.as_str())],
        4 * MIB,
        &pattern(MIB),
    )
    .await;
    for _ in 0..50 {
        if listing(dir) == before {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    assert_eq!(listing(dir), before, "an abandoned upload left something behind");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_is_uploaded_into_a_directory_on_this_node() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let root = dir.path().canonicalize().expect("a root");
    assert_uploads(&c, c.h.node.node_id(), &root).await;
    assert!(
        listing(&c.h.node.state_dir.join("uploads")).is_empty(),
        "an upload is still remembered"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_into_a_directory_this_node_cannot_write_is_the_same_404() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let locked = dir.path().join("locked");
    std::fs::create_dir(&locked).expect("a dir");
    std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o500)).expect("chmod");
    let refused = upload(&c, c.h.node.node_id(), &locked, "x.txt", b"x").await;
    assert_eq!(refused.status, 404, "{}", refused.text());
    std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o700)).expect("chmod");
}

/// A refusal that answers before reading the body closes a socket with the body still in it, and
/// the kernel resets it, which can throw the answer away before the client reads it: macOS did,
/// on CI, and a browser shows that as a network error rather than the reason. Each kind of early
/// refusal, with a body big enough to be in flight when it is sent, a few times over.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_upload_reaches_the_client_as_its_answer_rather_than_a_reset() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let root = dir.path().canonicalize().expect("a root");
    let body = pattern(8 * MIB);
    let node = c.h.node.node_id();
    let missing = root.join("missing").display().to_string();
    let here = root.display().to_string();
    let readonly = format!("Bearer {}", c.h.token_for(Role::Readonly).await);
    for _ in 0..5 {
        for (token, dir, name, want) in [
            (&c.full, here.as_str(), "a/b", 400),
            (&c.full, missing.as_str(), "x.txt", 404),
            (&readonly, here.as_str(), "x.txt", 403),
        ] {
            let refused = upload_as(&c, token, node, dir, name, &body).await;
            assert_eq!(refused.status, want, "{dir} {name}: {}", refused.text());
        }
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_tilde_directory_is_the_nodes_own_home() {
    let c = Client::start().await;
    let home = c.h.journals_home();
    let answer = upload_as(&c, &c.full, c.h.node.node_id(), "~", "hello.txt", b"hi").await;
    assert_eq!(answer.status, 201, "{}", answer.text());
    assert_eq!(std::fs::read(home.join("hello.txt")).expect("the upload"), b"hi");
}

fn temps(dir: &Path) -> Vec<String> {
    listing(dir)
        .into_iter()
        .filter(|n| n.starts_with(".kampr-upload-"))
        .collect()
}

#[tokio::test(flavor = "multi_thread")]
async fn an_upload_that_stops_moving_is_dropped_and_leaves_nothing() {
    let c = Client::start_with(|config| config.limits.upload_idle_secs = 1).await;
    let dir = tempfile::tempdir().expect("a dir");
    let root = dir.path().canonicalize().expect("a root");
    let answer = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        raw::stall(
            &c.h.origin,
            &upload_path(c.h.node.node_id(), &root.display().to_string(), "stuck.bin"),
            &[("Authorization", c.full.as_str())],
            4 * MIB,
            &pattern(MIB),
        ),
    )
    .await
    .expect("a stalled upload is never dropped");
    assert_eq!(answer.status, 408, "{}", answer.text());
    assert!(listing(&root).is_empty(), "{:?}", listing(&root));
}

#[tokio::test(flavor = "multi_thread")]
async fn what_a_killed_node_was_uploading_is_removed_when_it_starts_again() {
    let state = tempfile::tempdir().expect("a state dir");
    let dir = tempfile::tempdir().expect("a dir");
    let root = dir.path().canonicalize().expect("a root");
    let config = || {
        let mut config = kampr_node::Config::bootstrap("killed");
        config.muffit.enabled = Some(false);
        config.update.check = false;
        config.herdr.socket = state.path().join("no-herdr.sock").display().to_string();
        config.native.root = state.path().join("native").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        config
    };
    let node = kampr_node::Node::start(config(), state.path())
        .await
        .expect("a node");
    let mut staged = kampr_node::upload::Staged::open(&node, &root.display().to_string(), "half.bin")
        .await
        .expect("an upload");
    staged.write(&pattern(MIB)).await.expect("a write");
    let _killed = std::mem::ManuallyDrop::new(staged);
    node.shutdown();
    drop(node);
    assert_eq!(temps(&root).len(), 1, "the kill should have left its temp file");

    let node = kampr_node::Node::start(config(), state.path())
        .await
        .expect("a node");
    assert!(listing(&root).is_empty(), "{:?}", listing(&root));
    node.shutdown();
}
