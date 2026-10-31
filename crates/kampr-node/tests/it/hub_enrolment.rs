//! `kampr mesh join` and `leave` write the hub row from another process; a running node dials or
//! drops it on that write, with no herdr and no hub needed to see it try.

use kampr_auth::MeshRole;
use kampr_node::{Config, Node};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

const KEY: &str = "4444444444444444444444444444444444444444444444444444444444444444";

#[tokio::test]
async fn a_hub_joined_or_left_by_another_process_is_dialled_or_dropped_at_once() {
    let state = tempfile::tempdir().unwrap();
    let mut config = Config::bootstrap("joiner");
    config.muffit.enabled = Some(false);
    config.update.check = false;
    config.journals.home = state.path().display().to_string();
    config.herdr.socket = state.path().join("no-herdr.sock").display().to_string();
    config.herdr.enabled = Some(false);
    config.native.root = state.path().join("native").display().to_string();
    let node = Node::start(config, state.path()).await.expect("a node");
    let hub = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("ws://{}/mesh", hub.local_addr().unwrap());
    // The node's first look at its hubs has run and found none.
    tokio::time::sleep(Duration::from_millis(300)).await;

    let cli = kampr_auth::Store::open(&Config::state_db(state.path()))
        .await
        .expect("the store, as the CLI opens it");
    cli.mesh()
        .enrol(KEY, "01JHUB", "hub", MeshRole::Hub, Some(&url), kampr_auth::now())
        .await
        .expect("the hub row");
    let (mut link, _) = tokio::time::timeout(Duration::from_secs(1), hub.accept())
        .await
        .expect("the joined hub was not dialled promptly")
        .unwrap();

    cli.mesh().forget(KEY).await.expect("forgotten").expect("a row");
    let mut buf = [0u8; 4096];
    let closed = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            match link.read(&mut buf).await {
                Ok(0) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await;
    assert!(closed.is_ok(), "the hub that was left kept its link");
    node.shutdown();
}
