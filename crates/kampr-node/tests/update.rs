//! Release discovery, end to end: a real node, a real websocket client, and a stub standing in
//! for GitHub so nothing here reaches the internet.
//!
//! Driven from the wire rather than from the checker, because the whole point of the feature is
//! that the answer rides beside `build` in the herd model. A test that asked the checker directly
//! would agree with the checker and say nothing about what a phone is handed.

use axum::response::IntoResponse;
use futures_util::StreamExt;
use kampr_auth::Role;
use kampr_node::{BUILD, Config, Node, http};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// Stands in for `api.github.com`, and counts what it was asked — the cadence is half the
/// requirement and it is invisible from the answer alone.
struct Github {
    base: String,
    asks: Arc<AtomicUsize>,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Github {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Github {
    async fn serving(tag: &'static str) -> Self {
        Self::start(tag, false).await
    }

    /// The answer is real, but it is one hop away and the hop leaves https.
    async fn redirecting_off_https(tag: &'static str) -> Self {
        Self::start(tag, true).await
    }

    async fn start(tag: &'static str, redirect: bool) -> Self {
        let asks = Arc::new(AtomicUsize::new(0));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let base = format!("http://{}", listener.local_addr().expect("an address"));
        // Only the route that actually answers counts, so a redirect that was not followed leaves
        // the count at zero.
        let answer = {
            let counter = asks.clone();
            move || {
                let counter = counter.clone();
                async move {
                    counter.fetch_add(1, Ordering::SeqCst);
                    axum::Json(json!({ "tag_name": tag, "name": tag, "draft": false }))
                }
            }
        };
        let latest = base.clone();
        let app = axum::Router::new()
            .route("/moved", axum::routing::get(answer.clone()))
            .route(
                "/repos/{owner}/{repo}/releases/latest",
                axum::routing::get(move || {
                    let latest = latest.clone();
                    let answer = answer.clone();
                    async move {
                        match redirect {
                            true => axum::response::Redirect::temporary(&format!("{latest}/moved"))
                                .into_response(),
                            false => answer().await.into_response(),
                        }
                    }
                }),
            );
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self { base, asks, server }
    }

    fn asked(&self) -> usize {
        self.asks.load(Ordering::SeqCst)
    }
}

struct Harness {
    node: Arc<Node>,
    origin: String,
    home: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.node.shutdown();
        self.server.abort();
    }
}

impl Harness {
    async fn start(tweak: impl FnOnce(&mut Config)) -> Self {
        Self::start_in(tempfile::tempdir().expect("a home"), tweak).await
    }

    /// Keeps the home directory across two starts, so the cache on disk is the same file both
    /// times — which is what makes a restart loop cost one request rather than one per restart.
    async fn start_in(home: tempfile::TempDir, tweak: impl FnOnce(&mut Config)) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let config_dir = home.path().join("config");
        let state_dir = home.path().join("state");
        std::fs::create_dir_all(&state_dir).expect("a state dir");

        let mut config = Config::bootstrap("front");
        config.config_dir = config_dir.display().to_string();
        config.state_dir = state_dir.display().to_string();
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        // No herd of any kind: this node must not find the operator's.
        config.herdr.socket = home.path().join("herdr.sock").display().to_string();
        config.herdr.binary = home.path().join("no-such-herdr").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        tweak(&mut config);
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
            node,
            origin,
            home,
            server,
        }
    }

    fn state_dir(&self) -> PathBuf {
        self.home.path().join("state")
    }

    async fn token(&self) -> String {
        let pairing = self
            .node
            .auth
            .create_pairing(Role::Full, kampr_auth::Delivery::Console)
            .await
            .expect("a pairing");
        if !pairing.armed {
            assert!(self.node.auth.arm_pairing(&pairing.code).await.expect("armed"));
        }
        let body = json!({ "code": pairing.code, "device_name": "phone" });
        post(&self.origin, "/auth/pair", &body.to_string()).await["token"]
            .as_str()
            .expect("a token")
            .to_string()
    }

    async fn connect(&self) -> Socket {
        let token = self.token().await;
        let url = self.origin.replacen("http", "ws", 1) + "/ws";
        let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("kampr.token.{token}").parse().unwrap(),
        );
        tokio_tungstenite::connect_async(request)
            .await
            .map(|(socket, _)| socket)
            .expect("a websocket")
    }

    /// This node's own entry out of the first `herd` frame a client is sent.
    async fn own_node(&self) -> Value {
        let mut socket = self.connect().await;
        let id = self.node.node_id().to_string();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let frame = tokio::time::timeout_at(deadline, socket.next())
                .await
                .expect("a frame before the deadline")
                .expect("the socket stayed open")
                .expect("a readable frame");
            let tungstenite::Message::Text(text) = frame else {
                continue;
            };
            let message: Value = serde_json::from_str(&text).expect("json");
            if message["t"] != "herd" {
                continue;
            }
            let entry = message["nodes"]
                .as_array()
                .expect("nodes")
                .iter()
                .find(|n| n["id"] == id.as_str())
                .unwrap_or_else(|| panic!("this node is not in its own herd: {message}"))
                .clone();
            return entry;
        }
    }
}

/// Async on purpose: the stub GitHub and the node share this test's runtime, so a blocking read
/// here would stop the very server it is waiting on.
async fn post(origin: &str, path: &str, body: &str) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (host, port) = split(origin);
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect((host.as_str(), port)).await.expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or_default();
    serde_json::from_str(body.trim()).unwrap_or(Value::Null)
}

fn split(origin: &str) -> (String, u16) {
    let rest = origin.trim_start_matches("http://");
    let (host, port) = rest.split_once(':').expect("host:port");
    (host.to_string(), port.parse().expect("a port"))
}

/// The checker publishes into the herd from its own task, so the first `herd` frame can legally
/// precede it; this waits for the field rather than for a frame.
async fn settle(harness: &Harness, want: Option<&str>) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
    loop {
        let entry = harness.own_node().await;
        let seen = entry["update"].as_str();
        if seen == want || tokio::time::Instant::now() >= deadline {
            return entry;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
}

/// Waits until the checker has written its answer to disk, which is what a restart reads.
async fn cached(state_dir: &Path) -> Value {
    let path = state_dir.join("update.json");
    for _ in 0..100 {
        if let Ok(text) = std::fs::read_to_string(&path)
            && let Ok(value) = serde_json::from_str::<Value>(&text)
        {
            return value;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("the checker never wrote {}", path.display());
}

#[tokio::test]
async fn a_node_that_is_behind_names_the_release_that_supersedes_it() {
    let github = Github::serving("v99.9.9").await;
    let harness = Harness::start(|config| {
        config.update.api = github.base.clone();
        config.update.repo = "dbrain/kampr".into();
    })
    .await;

    let entry = settle(&harness, Some("99.9.9")).await;
    assert_eq!(
        entry["update"], "99.9.9",
        "the node is on {BUILD} and the release is 99.9.9, and it said nothing: {entry}"
    );
    assert_eq!(
        entry["build"], BUILD,
        "the answer has to sit beside the build it judges"
    );
    assert_eq!(github.asked(), 1, "one release check, not one per herd rebuild");
}

/// The quiet case, and the one the client is built around: a current node must not put a field on
/// the wire at all, or every machine in the herd renders an update line saying nothing.
#[tokio::test]
async fn a_node_on_the_latest_release_says_nothing() {
    let tag: &'static str = Box::leak(format!("v{BUILD}").into_boxed_str());
    let github = Github::serving(tag).await;
    let harness = Harness::start(|config| config.update.api = github.base.clone()).await;

    let cache = cached(&harness.state_dir()).await;
    assert_eq!(cache["latest"], tag, "the check did not land: {cache}");
    let entry = harness.own_node().await;
    assert!(
        entry.as_object().expect("an object").get("update").is_none(),
        "a node running the latest release still claimed one was available: {entry}"
    );
}

/// Degrading to silence rather than to an error is the requirement: a node with no route out
/// still serves its herd, still names its build, and simply has nothing to add.
#[tokio::test]
async fn a_node_that_cannot_reach_github_is_silent_and_still_serves() {
    // Reserved and dropped, so the port is one nothing is listening on.
    let dead = {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        listener.local_addr().expect("an address").port()
    };
    let harness = Harness::start(|config| config.update.api = format!("http://127.0.0.1:{dead}")).await;

    let entry = harness.own_node().await;
    assert_eq!(entry["build"], BUILD, "the herd stopped working: {entry}");
    assert!(
        entry.as_object().expect("an object").get("update").is_none(),
        "a failed check leaked onto the wire: {entry}"
    );
    // `detail` is the herdr socket's business — this node has no herd. What must not be in it is
    // the release check, which is not a fault of the node and must never be reported as one.
    let detail = entry["detail"].as_str().unwrap_or_default().to_lowercase();
    for word in ["release", "update", "github", "version"] {
        assert!(
            !detail.contains(word),
            "a failed release check surfaced as a fault on the node: {entry}"
        );
    }
    let cache = cached(&harness.state_dir()).await;
    assert_eq!(
        cache["ok"], false,
        "a failure recorded itself as a good answer: {cache}"
    );
}

/// The off switch has to mean the request is never made. Hiding the answer while still asking is
/// exactly the thing the operator turned it off to prevent.
#[tokio::test]
async fn the_check_off_means_no_request_at_all() {
    let github = Github::serving("v99.9.9").await;
    let harness = Harness::start(|config| {
        config.update.api = github.base.clone();
        config.update.check = false;
    })
    .await;

    let entry = harness.own_node().await;
    assert!(
        entry.as_object().expect("an object").get("update").is_none(),
        "a node whose check is off still reported an update: {entry}"
    );
    tokio::time::sleep(Duration::from_millis(500)).await;
    assert_eq!(
        github.asked(),
        0,
        "the check is off and the node asked GitHub anyway, which is the whole point of the switch"
    );
    assert!(
        !harness.state_dir().join("update.json").exists(),
        "a node whose check is off wrote a release cache"
    );
}

/// A node under a supervisor that is crash-looping must not become a request per restart, so the
/// cadence is held on disk rather than in the process.
#[tokio::test]
async fn a_restart_inside_the_day_reuses_the_cached_answer() {
    let github = Github::serving("v99.9.9").await;
    let home = tempfile::tempdir().expect("a home");
    let base = github.base.clone();
    let mut first = Harness::start_in(home, |config| config.update.api = base.clone()).await;
    settle(&first, Some("99.9.9")).await;
    assert_eq!(github.asked(), 1);
    let home = std::mem::replace(&mut first.home, tempfile::tempdir().expect("a spare"));
    drop(first);

    let base = github.base.clone();
    let second = Harness::start_in(home, |config| config.update.api = base.clone()).await;
    let entry = settle(&second, Some("99.9.9")).await;
    assert_eq!(
        entry["update"], "99.9.9",
        "a restarted node lost the answer it had already paid for: {entry}"
    );
    assert_eq!(
        github.asked(),
        1,
        "the second start asked GitHub again, so a supervised restart loop is a request loop"
    );
}

/// A bootstrapped node checks. The default is what almost every operator will run, and a silent
/// flip to off would take the feature away without anyone noticing.
#[test]
fn discovery_is_on_by_default_and_points_at_this_repository() {
    let config = Config::bootstrap("front");
    assert!(config.update.check);
    assert_eq!(config.update.repo, "dbrain/kampr");
    assert_eq!(
        config.update.latest_release_url(),
        "https://api.github.com/repos/dbrain/kampr/releases/latest"
    );
}

/// A redirect chooses the next URL, and nothing about the request that provoked it constrains
/// where it points — `--proto`, and a config validated to https, both govern the first hop only.
/// The release check is one version string, but it is the one request this node makes to a host
/// it did not choose, and a hop that leaves https must not be followed.
#[tokio::test]
async fn a_redirect_off_https_is_refused_rather_than_followed() {
    let github = Github::redirecting_off_https("v99.9.9").await;
    let harness = Harness::start(|config| config.update.api = github.base.clone()).await;

    let cache = cached(&harness.state_dir()).await;
    assert_eq!(
        cache["ok"], false,
        "the node followed a redirect off https and treated the answer as good: {cache}"
    );
    assert_eq!(
        github.asked(),
        0,
        "the redirect was followed, so anything that can answer this node's release check can \
         also choose the transport it is answered over"
    );
    let entry = harness.own_node().await;
    assert!(
        entry.as_object().expect("an object").get("update").is_none(),
        "a release named over a redirect the node should not have taken reached the wire: {entry}"
    );
}
