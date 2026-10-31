//! `kampr mesh join` and `leave` write the hub row from another process; a running node dials or
//! drops it on that write, with no herdr and no hub needed to see it try.

use kampr_auth::MeshRole;
use kampr_node::{Config, Node};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::net::TcpListener;

const KEY: &str = "4444444444444444444444444444444444444444444444444444444444444444";

/// What the store watch, the hub reads and the dials said, so a failure on a host nobody here
/// can run says which of them stopped. Global, because the store watch logs from its own thread.
fn captured() -> Arc<Mutex<Vec<String>>> {
    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::SubscriberExt;
    let lines = Arc::new(Mutex::new(Vec::new()));
    let writer = {
        let lines = lines.clone();
        move || Line(lines.clone())
    };
    let layer = tracing_subscriber::fmt::layer()
        .with_writer(writer)
        .with_ansi(false)
        .with_thread_names(true)
        .with_filter(tracing_subscriber::EnvFilter::new(
            "kampr_node::mesh=debug,kampr_node::device_watch=debug,kampr_mesh=debug",
        ));
    let _ = tracing::subscriber::set_global_default(tracing_subscriber::registry().with(layer));
    lines
}

struct Line(Arc<Mutex<Vec<String>>>);

impl std::io::Write for Line {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0
            .lock()
            .unwrap()
            .push(String::from_utf8_lossy(buf).trim_end().to_string());
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn said(lines: &Mutex<Vec<String>>) -> String {
    lines.lock().unwrap().join("\n")
}

#[tokio::test]
async fn a_hub_joined_or_left_by_another_process_is_dialled_or_dropped_at_once() {
    let log = captured();
    let state = tempfile::tempdir().unwrap();
    let mut config = Config::bootstrap("joiner");
    config.muffit.enabled = Some(false);
    config.update.check = false;
    // The node's mesh key is made on its first dial, and only this test's directory may get one.
    config.config_dir = state.path().join("config").display().to_string();
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
    let Ok(accepted) = tokio::time::timeout(Duration::from_secs(1), hub.accept()).await else {
        panic!(
            "the joined hub was not dialled promptly; the node said:\n{}",
            said(&log)
        );
    };
    let (mut link, _) = accepted.unwrap();

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
    assert!(
        closed.is_ok(),
        "the hub that was left kept its link; the node said:\n{}",
        said(&log)
    );
    node.shutdown();
}
