//! Named herdr sessions found as their sockets come and go, against a `herdr` that answers only
//! `session list --json`, out of the same directory herdr itself keeps them in (probe #49, #240).

use kampr_node::{Config, Node};
use std::os::unix::fs::PermissionsExt;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

fn fake_herdr(home: &Path) -> std::path::PathBuf {
    let path = home.join("herdr");
    let sessions = home.join("cfg/herdr/sessions");
    let script = format!(
        r#"#!/bin/sh
[ "$1 $2" = "session list" ] || exit 1
printf '{{"sessions":['
sep=
for dir in {sessions}/*/; do
  [ -e "$dir/herdr.sock" ] || continue
  name=$(basename "$dir")
  printf '%s{{"name":"%s","running":true,"socket_path":"%sherdr.sock"}}' "$sep" "$name" "$dir"
  sep=,
done
printf ']}}\n'
"#,
        sessions = sessions.display()
    );
    std::fs::write(&path, script).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
    path
}

async fn node(home: &Path) -> Arc<Node> {
    let state_dir = home.join("state");
    std::fs::create_dir_all(home.join("cfg/herdr/sessions")).unwrap();
    std::fs::create_dir_all(&state_dir).unwrap();
    let mut config = Config::bootstrap("discovery");
    config.muffit.enabled = Some(false);
    config.update.check = false;
    config.state_dir = state_dir.display().to_string();
    config.herdr.binary = fake_herdr(home).display().to_string();
    config.herdr.socket = home.join("cfg/herdr/herdr.sock").display().to_string();
    config.herdr.enabled = Some(true);
    config.native.root = home.join("native").display().to_string();
    config.journals.home = home.display().to_string();
    Node::start(config, &state_dir).await.expect("a node")
}

async fn served_within(node: &Node, name: &str, wanted: bool, within: Duration) {
    let deadline = tokio::time::Instant::now() + within;
    loop {
        if node.sessions.all().iter().any(|s| s.name == name) == wanted {
            return;
        }
        if tokio::time::Instant::now() >= deadline {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("session {name} served = {} after {within:?}", !wanted);
}

/// A session's socket appearing or going is the news, and the 15 s listing behind it is only the
/// net for a server killed without removing its socket (probe #427).
#[tokio::test]
async fn a_named_session_is_served_as_its_socket_appears_and_dropped_as_it_goes() {
    let home = tempfile::Builder::new().prefix("kd").tempdir_in("/tmp").unwrap();
    let node = node(home.path()).await;
    // The first listing at startup has run and found nothing.
    tokio::time::sleep(Duration::from_millis(300)).await;
    served_within(&node, "agents", false, Duration::ZERO).await;

    let dir = home.path().join("cfg/herdr/sessions/agents");
    std::fs::create_dir(&dir).unwrap();
    tokio::time::sleep(Duration::from_millis(100)).await;
    let socket = std::os::unix::net::UnixListener::bind(dir.join("herdr.sock")).unwrap();
    served_within(&node, "agents", true, Duration::from_millis(1000)).await;

    drop(socket);
    std::fs::remove_file(dir.join("herdr.sock")).unwrap();
    served_within(&node, "agents", false, Duration::from_millis(1000)).await;
    node.shutdown();
}
