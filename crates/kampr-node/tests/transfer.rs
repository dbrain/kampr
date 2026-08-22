//! What the first load actually costs on the wire.
//!
//! No herd is needed: this drives the static-asset surface of a node whose herdr socket
//! deliberately does not exist.

use kampr_node::{Config, Node, http};
use std::path::PathBuf;
use std::sync::Arc;

struct Harness {
    origin: String,
    node: Arc<Node>,
    _home: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.node.shutdown();
        self.server.abort();
    }
}

impl Harness {
    async fn start() -> Self {
        let home = tempfile::tempdir().expect("a home");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let config_dir = home.path().join("config");
        let state_dir = home.path().join("state");
        std::fs::create_dir_all(&state_dir).expect("a state dir");

        let mut config = Config::bootstrap("transfer");
        // Nothing in this suite reaches the internet: the release check is the one thing in a
        // node that would, and a test that phoned GitHub would be one with a rate limit.
        config.update.check = false;
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.herdr.socket = home.path().join("herdr.sock").display().to_string();
        config.herdr.binary = home.path().join("no-such-herdr").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        config.save(&config_dir).expect("a config");

        let origin = config.origin();
        let node = Node::start(config, &state_dir).await.expect("a node");
        let server = tokio::spawn({
            let app = http::router(node.clone());
            async move {
                let _ = http::serve_on(listener, app).await;
            }
        });
        Self {
            origin,
            node,
            _home: home,
            server,
        }
    }
}

/// Head and body, kept as bytes: a compressed body is not text.
async fn fetch(origin: &str, path: &str, accept_encoding: Option<&str>) -> (String, Vec<u8>) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let authority = origin.trim_start_matches("http://");
    let (host, port) = authority.split_once(':').expect("host:port");
    let encoding = accept_encoding.map_or(String::new(), |e| format!("Accept-Encoding: {e}\r\n"));
    let request = format!("GET {path} HTTP/1.1\r\nHost: {authority}\r\n{encoding}Connection: close\r\n\r\n");
    let mut stream = tokio::net::TcpStream::connect((host, port.parse::<u16>().unwrap()))
        .await
        .expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a header terminator");
    let head = String::from_utf8_lossy(&response[..split]).to_lowercase();
    (head, response[split + 4..].to_vec())
}

/// The biggest thing the bundle ships, which is the wasm module and is the whole first load.
fn heaviest_asset() -> Option<(String, u64)> {
    let dist = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("dist");
    std::fs::read_dir(dist)
        .ok()?
        .flatten()
        .filter_map(|e| {
            let path = e.path();
            let size = path.metadata().ok()?.len();
            let name = path.file_name()?.to_str()?.to_string();
            path.is_file().then_some((name, size))
        })
        .max_by_key(|(_, size)| *size)
}

/// Twelve megabytes of wasm over a phone's link, on every release, because content-hashed
/// filenames make it a fresh URL each time. `application/wasm` is not in nginx's default
/// `gzip_types` either, so a reverse proxy does not save the operator.
#[tokio::test(flavor = "multi_thread")]
async fn the_bundle_is_compressed_on_the_way_out() {
    let Some((asset, on_disk)) = heaviest_asset() else {
        eprintln!("skipping: no client bundle staged in crates/kampr-node/dist");
        return;
    };
    let h = Harness::start().await;

    let (plain_head, plain) = fetch(&h.origin, &format!("/{asset}"), None).await;
    assert!(plain_head.contains("200 ok"), "{plain_head}");
    assert!(
        !plain_head.contains("content-encoding"),
        "a client that asked for nothing must be sent the bytes: {plain_head}"
    );
    assert_eq!(plain.len() as u64, on_disk, "{asset} arrived truncated");

    let (head, body) = fetch(&h.origin, &format!("/{asset}"), Some("gzip, br, zstd, deflate")).await;
    assert!(head.contains("200 ok"), "{head}");
    assert!(
        head.contains("content-encoding: br"),
        "brotli is the best encoding every phone browser offers: {head}"
    );
    assert!(
        head.contains("vary: accept-encoding"),
        "a shared cache must not serve brotli to a client that cannot read it: {head}"
    );
    eprintln!(
        "{asset}: {} bytes identity, {} bytes brotli ({:.1}% of the original)",
        plain.len(),
        body.len(),
        100.0 * body.len() as f64 / plain.len() as f64
    );
    assert!(
        body.len() * 2 < plain.len(),
        "{asset} compressed to {} of {} bytes, which is not worth the CPU",
        body.len(),
        plain.len()
    );

    // A client that only speaks gzip still gets gzip rather than the whole thing.
    let (gzip_head, gzip) = fetch(&h.origin, &format!("/{asset}"), Some("gzip")).await;
    assert!(gzip_head.contains("content-encoding: gzip"), "{gzip_head}");
    assert!(gzip.len() < plain.len(), "{gzip_head}");
    eprintln!("{asset}: {} bytes gzip", gzip.len());
}

/// The websocket upgrade shares the router with the assets, and a compression layer that touched
/// it would break every client on the node.
#[tokio::test(flavor = "multi_thread")]
async fn compression_leaves_the_health_check_and_the_upgrade_alone() {
    let h = Harness::start().await;
    let (head, body) = fetch(&h.origin, "/healthz", Some("gzip, br")).await;
    assert!(head.contains("200 ok"), "{head}");
    assert_eq!(body, b"ok", "a two-byte body is not worth an encoding: {head}");
}
