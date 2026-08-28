//! End-to-end against a real Herdr.
//!
//! Every test here runs in a throwaway named session created and destroyed by the test itself.
//! `default` is never touched. When `herdr` is not on PATH the suite reports a skip rather than a
//! failure, so it stays honest on a machine that has no herd.

use base64::Engine;
use futures_util::{SinkExt, StreamExt};
use kampr_auth::Role;
use kampr_node::{Config, Node, http};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Session {
    name: String,
    socket: PathBuf,
}

impl Session {
    async fn start(tag: &str) -> Option<Self> {
        which("herdr")?;
        let name = format!("kampr-it-{tag}-{}", std::process::id());
        assert_ne!(name, "default");
        let socket = herdr_home().join("sessions").join(&name).join("herdr.sock");
        std::process::Command::new("herdr")
            .args(["server", "--session", &name])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .ok()?;
        for _ in 0..100 {
            if socket.exists() {
                tokio::time::sleep(Duration::from_millis(300)).await;
                return Some(Self { name, socket });
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    fn herdr(&self) -> kampr_herdr::Herdr {
        kampr_herdr::Herdr::new(&self.socket)
    }

    /// Stops the server without deleting the session, so it can be started again on the same
    /// socket — a herdr outage rather than a herd that went away.
    async fn stop(&self) {
        let _ = self.herdr().call::<Value>("server.stop", json!({})).await;
        for _ in 0..50 {
            if !self.socket.exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    // The server is stopped over its own socket in `Drop`, not by reaping a child: it outlives
    // this handle by design, exactly as the one `start` spawns does.
    #[allow(clippy::zombie_processes)]
    async fn respawn(&self) {
        std::process::Command::new("herdr")
            .args(["server", "--session", &self.name])
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("herdr server");
        for _ in 0..100 {
            if self.socket.exists() {
                tokio::time::sleep(Duration::from_millis(300)).await;
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("herdr never came back on {}", self.socket.display());
    }

    async fn call(&self, method: &str, params: Value) -> Value {
        self.herdr()
            .call::<Value>(method, params)
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Some(dir) = self.socket.parent() {
            forget_session(dir);
        }
    }
}

/// A session a test made, removed whether that test passes, fails or panics. Teardown at the end
/// of a body is skipped by both a panic and an abort, and an aborted run once left thirty-seven
/// of these — which is not tidiness but correctness, because a node serves every session it can
/// find (#97), so one left behind is in the next test's herd.
struct CreatedSession {
    dir: PathBuf,
}

impl CreatedSession {
    fn named(name: &str) -> Self {
        Self {
            dir: herdr_home().join("sessions").join(name),
        }
    }

    fn dir(&self) -> &Path {
        &self.dir
    }
}

impl Drop for CreatedSession {
    fn drop(&mut self) {
        forget_session(&self.dir);
        assert!(
            std::thread::panicking() || !self.dir.exists(),
            "left {:?} behind",
            self.dir
        );
    }
}

/// Never leave a herdr behind. `server.stop` over the session's own socket first — a test that
/// panicked before its `session.stop` still has one running, and removing the directory under it
/// leaves the herdr — then the socket, then the directory. A stopped session keeps its directory
/// until something removes it (#242), and a herdr asked to stop still owns that directory for a
/// moment: one removal races it, which is what leaves a throwaway session listed forever. So wait
/// for the socket to go, then keep removing until the directory stays gone.
fn forget_session(dir: &Path) {
    if !dir.exists() {
        return;
    }
    let socket = dir.join("herdr.sock");
    if socket.exists() {
        // `block_on` panics inside a runtime, and every caller of this is inside one.
        std::thread::spawn({
            let socket = socket.clone();
            move || {
                let runtime = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .expect("teardown runtime");
                runtime.block_on(async {
                    let _ = kampr_herdr::Herdr::new(&socket)
                        .call::<Value>("server.stop", json!({}))
                        .await;
                });
            }
        })
        .join()
        .ok();
    }
    for _ in 0..100 {
        if !socket.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    for _ in 0..25 {
        let _ = std::fs::remove_dir_all(dir);
        std::thread::sleep(Duration::from_millis(200));
        if !dir.exists() {
            break;
        }
    }
}

fn which(binary: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|path| {
        std::env::split_paths(&path)
            .map(|dir| dir.join(binary))
            .find(|candidate| candidate.is_file())
    })
}

fn herdr_home() -> PathBuf {
    std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(std::env::var("HOME").expect("HOME")).join(".config"))
        .join("herdr")
}

struct Harness {
    _session: Session,
    _state: tempfile::TempDir,
    node: Arc<Node>,
    origin: String,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Harness {
    async fn start_with(tag: &str, tweak: impl FnOnce(&mut Config)) -> Option<Self> {
        let session = Session::start(tag).await?;
        session
            .call("workspace.create", json!({ "label": "kampr", "cwd": "/tmp" }))
            .await;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.ok()?;
        let port = listener.local_addr().ok()?.port();
        let state = tempfile::tempdir().ok()?;
        let config_dir = state.path().join("config");

        let mut config = Config::bootstrap("testnode");
        // Nothing in this suite reaches the internet: the release check is the one thing in a
        // node that would, and a test that phoned GitHub would be one with a rate limit.
        config.update.check = false;
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.herdr.socket = session.socket.display().to_string();
        // Serve only this test's own session. The node discovers every herdr running on the
        // machine by default, and another test's throwaway herd is not this test's herd.
        config.herdr.sessions = Some(Vec::new());
        config.auth.audit = true;
        config.limits.client_queue = 32;
        tweak(&mut config);
        config.save(&config_dir).ok()?;

        let origin = config.origin();
        let node = Node::start(config, state.path()).await.ok()?;
        let server = tokio::spawn({
            let app = http::router(node.clone());
            async move {
                let _ = http::serve_on(listener, app).await;
            }
        });
        // The first herd model is built by a background task; a watch with no panes is not ready.
        for _ in 0..50 {
            if node.herd().panes.iter().any(|p| p.node_id == node.node_id()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Some(Self {
            _session: session,
            _state: state,
            node,
            origin,
            server,
        })
    }

    async fn token(&self, role: Role) -> String {
        let pairing = self
            .node
            .auth
            .create_pairing(role, kampr_auth::Delivery::Console)
            .await
            .unwrap();
        if !pairing.armed {
            assert!(self.node.auth.arm_pairing(&pairing.code).await.unwrap());
        }
        let body = json!({ "code": pairing.code, "device_name": "integration" });
        let response = post(&format!("{}/auth/pair", self.origin), &body).await;
        response["token"].as_str().expect("a token").to_string()
    }

    async fn connect(&self, token: &str) -> Socket {
        let url = self.origin.replacen("http", "ws", 1) + "/ws";
        let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("kampr.token.{token}").parse().unwrap(),
        );
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("websocket upgrade");
        socket
    }

    /// Whether the herd says this pane has a transcript, once the sweep has had time to say so.
    async fn claims_a_conversation(&self, pane: &str) -> bool {
        let mut claimed = false;
        for _ in 0..24 {
            tokio::time::sleep(Duration::from_millis(250)).await;
            if let Some(entry) = self.node.herd().pane(pane) {
                claimed |= serde_json::to_value(entry).unwrap()["has_conversation"] == true;
            }
        }
        claimed
    }

    /// The pane of this harness's session rooted at `cwd`, once herdr has reported it.
    async fn pane_with_cwd(&self, cwd: &str) -> Option<String> {
        for _ in 0..100 {
            let found = self
                .node
                .herd()
                .panes
                .iter()
                .find(|p| p.node_id == self.node.node_id() && p.cwd.as_deref() == Some(cwd))
                .map(|p| p.id.clone());
            if found.is_some() {
                return found;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    /// A pane of *this harness's own* session. The node discovers every named herdr session on
    /// the machine, so the herd carries other tests' panes — and another test's throwaway session
    /// is not what a test means by "the pane".
    fn pane_id(&self) -> String {
        self.node
            .herd()
            .panes
            .iter()
            .find(|p| p.node_id == self.node.node_id())
            .expect("a pane on this harness's own session")
            .id
            .clone()
    }

    /// Whether the node has noticed that its own herdr is gone. A test that stops herdr and acts
    /// immediately is racing the poll loop, and proves whatever the race decided.
    async fn offline(&self, seconds: u64) -> bool {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
        while tokio::time::Instant::now() < deadline {
            let herd = self.node.herd();
            if herd
                .nodes
                .iter()
                .any(|n| n.id == self.node.node_id() && !n.online)
            {
                return true;
            }
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
        false
    }
}

async fn post(url: &str, body: &Value) -> Value {
    post_as(url, body, None).await.1
}

/// `host` and `origin` are what a DNS-rebinding attacker controls: a domain they own, pointed at
/// this node's address, so the browser sends both as their own.
async fn post_from(url: &str, body: &Value, host: &str, origin: Option<&str>) -> String {
    let (real_host, port, path) = split(url);
    let payload = body.to_string();
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}\r\nContent-Type: application/json\r\n\
         {origin}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    raw_with_status(&real_host, port, &request).await.0
}

async fn head_of(url: &str, body: &Value) -> String {
    let (host, port, path) = split(url);
    let payload = body.to_string();
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect((host.as_str(), port)).await.expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    text.split_once("\r\n\r\n")
        .map_or(text.clone(), |(h, _)| h.to_string())
}

async fn with_cookie(method: &str, url: &str, token: &str, origin: Option<&str>) -> String {
    let (host, port, path) = split(url);
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nCookie: kampr_session={token}\r\n\
         {origin}Content-Length: 0\r\nConnection: close\r\n\r\n"
    );
    raw_with_status(&host, port, &request).await.0
}

async fn post_as(url: &str, body: &Value, forwarded_for: Option<&str>) -> (String, Value) {
    let (host, port, path) = split(url);
    let payload = body.to_string();
    let forwarded = forwarded_for.map_or(String::new(), |f| format!("X-Forwarded-For: {f}\r\n"));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
         {forwarded}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let (status, body) = raw_with_status(&host, port, &request).await;
    (status, Value::from(body))
}

async fn get(url: &str, token: Option<&str>) -> (String, Value) {
    let (host, port, path) = split(url);
    let auth = token.map_or(String::new(), |t| format!("Authorization: Bearer {t}\r\n"));
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\n{auth}Connection: close\r\n\r\n");
    let (status, body) = raw_with_status(&host, port, &request).await;
    (status, Value::from(body))
}

fn split(url: &str) -> (String, u16, String) {
    let rest = url.trim_start_matches("http://");
    let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
    let (host, port) = authority.split_once(':').unwrap_or((authority, "80"));
    (host.to_string(), port.parse().unwrap(), format!("/{path}"))
}

async fn raw_with_status(host: &str, port: u16, request: &str) -> (String, Json) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let mut stream = TcpStream::connect((host, port)).await.expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text.lines().next().unwrap_or_default().to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or_default();
    (status, Json(body.trim().to_string()))
}

struct Json(String);

impl From<Json> for Value {
    fn from(json: Json) -> Self {
        serde_json::from_str(&json.0).unwrap_or(Value::Null)
    }
}

/// Drains until the peer closes. `true` means the node hung up inside the window.
async fn closed(socket: &mut Socket, seconds: u64) -> bool {
    tokio::time::timeout(Duration::from_secs(seconds), async {
        while let Some(Ok(message)) = socket.next().await {
            if matches!(message, tungstenite::Message::Close(_)) {
                return;
            }
        }
    })
    .await
    .is_ok()
}

async fn recv(socket: &mut Socket, timeout: Duration) -> Option<Value> {
    let message = tokio::time::timeout(timeout, socket.next()).await.ok()??.ok()?;
    match message {
        tungstenite::Message::Text(text) => serde_json::from_str(&text).ok(),
        _ => None,
    }
}

async fn until(socket: &mut Socket, tag: &str, seconds: u64) -> Value {
    until_seen(socket, tag, seconds).await.0
}

/// [`until`], plus every `t` that arrived before it — for the claims that are about what the node
/// did *not* send.
async fn until_seen(socket: &mut Socket, tag: &str, seconds: u64) -> (Value, Vec<String>) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == tag {
            return (message, seen);
        }
        seen.push(message["t"].as_str().unwrap_or("?").to_string());
    }
    panic!("never saw {tag}; saw {seen:?}");
}

/// Like [`until`], but for a message about one pane: the node serves every herdr session on the
/// machine, so another test's session going away arrives here as an unrelated `error`.
async fn until_pane(socket: &mut Socket, tag: &str, pane: &str, seconds: u64) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == tag && message["pane"] == pane {
            return message;
        }
        seen.push(message["t"].as_str().unwrap_or("?").to_string());
    }
    panic!("never saw {tag} for {pane}; saw {seen:?}");
}

async fn send(socket: &mut Socket, value: Value) {
    socket
        .send(tungstenite::Message::text(value.to_string()))
        .await
        .expect("send");
}

macro_rules! harness {
    ($tag:expr) => {
        harness!($tag, |_| {})
    };
    ($tag:expr, $tweak:expr) => {
        match Harness::start_with($tag, $tweak).await {
            Some(h) => h,
            None => {
                eprintln!("skipping: no herdr on PATH");
                return;
            }
        }
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn a_paired_device_drives_a_pane_end_to_end() {
    let h = harness!("drive");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;

    let hello = until(&mut socket, "hello", 10).await;
    assert_eq!(hello["protocol"], 1);
    assert_eq!(hello["role"], "full");
    assert_eq!(hello["caps"]["manage"], true);
    assert_eq!(hello["node_name"], "testnode");
    assert_eq!(hello["security"]["tier"], 0, "a loopback IP origin is Tier 0");
    assert_eq!(hello["security"]["passkeys"], false);

    let herd = until(&mut socket, "herd", 10).await;
    let pane = herd["panes"][0]["id"].as_str().expect("a pane").to_string();
    assert!(pane.starts_with(hello["node_id"].as_str().unwrap()));
    assert!(herd["panes"][0]["rows"].as_u64().unwrap() > 0);
    // `cols` is absent until a wrap has proved the PTY width. The layout rect is not it: headless,
    // a pane whose rect reads 47 is really 93 wide (probe #68).
    assert!(herd["panes"][0]["cols"].as_u64().is_none_or(|cols| cols > 0));
    // A brand-new workspace's pane really is nearly empty (probe #212), and empty is not a fault:
    // `detail` is absent for every pane a node can actually stream.
    assert!(
        herd["panes"]
            .as_array()
            .unwrap()
            .iter()
            .all(|p| p["detail"].is_null()),
        "a healthy node marked a pane unstreamable: {herd}"
    );

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let reset = until(&mut socket, "grid.reset", 15).await;
    assert_eq!(reset["pane"], pane.as_str());
    assert_eq!(
        reset["rows_data"].as_array().unwrap().len(),
        reset["rows"].as_u64().unwrap() as usize
    );

    let marker = "kampr-echo-marker";
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": format!("echo {marker}\n") }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut echoed = false;
    while tokio::time::Instant::now() < deadline && !echoed {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
            continue;
        }
        let text = message.to_string();
        echoed = text.contains(marker);
    }
    assert!(echoed, "the pane never echoed {marker} back over the wire");

    send(&mut socket, json!({ "t": "ping", "n": 7 })).await;
    assert_eq!(until(&mut socket, "pong", 10).await["n"], 7);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_readonly_device_is_refused_writes_and_still_sees_the_pane() {
    let h = harness!("ro");
    let token = h.token(Role::Readonly).await;
    let mut socket = h.connect(&token).await;

    let hello = until(&mut socket, "hello", 10).await;
    assert_eq!(hello["role"], "readonly");
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let reset = until(&mut socket, "grid.reset", 15).await;
    assert_eq!(
        reset["pane"],
        pane.as_str(),
        "readonly still receives every frame"
    );

    for write in [
        json!({ "t": "input", "pane": pane, "text": "whoami\n" }),
        json!({ "t": "answer", "pane": pane, "key": "1" }),
        json!({ "t": "manage", "op": "workspace.create", "label": "nope" }),
    ] {
        send(&mut socket, write).await;
        let error = until(&mut socket, "error", 10).await;
        assert_eq!(error["code"], "not_writer");
    }

    // And the stream is still live afterwards.
    send(&mut socket, json!({ "t": "ping", "n": 3 })).await;
    assert_eq!(until(&mut socket, "pong", 10).await["n"], 3);
}

#[tokio::test(flavor = "multi_thread")]
async fn manage_ops_reshape_the_herd_and_come_back_as_a_patch() {
    let h = harness!("manage");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let herd = until(&mut socket, "herd", 10).await;
    let before = herd["panes"].as_array().unwrap().len();
    let pane = herd["panes"][0]["id"].as_str().unwrap().to_string();

    send(
        &mut socket,
        json!({ "t": "manage", "op": "pane.split", "at": pane, "direction": "right", "ratio": 0.5 }),
    )
    .await;
    let ack = until(&mut socket, "managed", 15).await;
    assert_eq!(ack["ok"], true, "{ack}");
    assert!(ack["id"].as_str().unwrap().contains(":p"), "{ack}");

    let patch = until(&mut socket, "herd.patch", 20).await;
    assert!(!patch["added"]["panes"].as_array().unwrap().is_empty(), "{patch}");
    assert!(h.node.herd().panes.len() > before);

    send(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": pane, "label": "build" }),
    )
    .await;
    assert_eq!(until(&mut socket, "managed", 15).await["ok"], true);

    send(&mut socket, json!({ "t": "manage", "op": "nonsense.op" })).await;
    let refusal = until(&mut socket, "error", 10).await;
    assert_eq!(refusal["code"], "unsupported");
}

#[tokio::test(flavor = "multi_thread")]
async fn capability_discovery_comes_from_the_host_not_a_list() {
    let h = harness!("caps");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    send(&mut socket, json!({ "t": "caps" })).await;
    let caps = until(&mut socket, "caps", 15).await;
    let kinds: Vec<&str> = caps["agent_kinds"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|k| k.as_str())
        .collect();
    assert!(kinds.contains(&"claude"), "{kinds:?}");
    assert!(kinds.len() > 3, "the host reported {} kinds", kinds.len());

    let sessions: Vec<&str> = caps["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert!(sessions.contains(&h._session.name.as_str()), "{sessions:?}");

    // A session this node does not serve must not be advertised as one a client can open a pane
    // on — that promise is what made the capability a lie.
    let served: Vec<&str> = caps["sessions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|s| s["served"] == true)
        .filter_map(|s| s["name"].as_str())
        .collect();
    assert_eq!(
        served,
        [h._session.name.as_str()],
        "this node serves only its own session: {caps}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_message_is_ignored_and_a_bad_one_is_refused() {
    let h = harness!("unknown");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    // Unknown `t` and unknown fields are how a v1 client survives a v1.1 node.
    send(&mut socket, json!({ "t": "telepathy", "wish": "coffee" })).await;
    send(&mut socket, json!({ "t": "ping", "n": 1, "unknown_field": true })).await;
    assert_eq!(until(&mut socket, "pong", 10).await["n"], 1);

    send(&mut socket, json!({ "t": "input", "pane": h.pane_id() })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "bad_request");

    send(
        &mut socket,
        json!({ "t": "input", "pane": h.pane_id(), "b64": "//8=" }),
    )
    .await;
    let refusal = until(&mut socket, "error", 10).await;
    assert_eq!(refusal["code"], "bad_request");
    assert!(refusal["message"].as_str().unwrap().contains("UTF-8"));

    send(&mut socket, json!({ "t": "watch", "pane": "01JNOTOURS/w9:p9" })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "unknown_pane");
}

#[tokio::test(flavor = "multi_thread")]
async fn the_http_surface_refuses_what_it_should() {
    let h = harness!("http");
    let (status, body) = get(&format!("{}/api/node", h.origin), None).await;
    assert!(status.contains("200"), "{status}");
    assert_eq!(body["security"]["tier"], 0);
    assert_eq!(body["security"]["passkeys"], false);
    assert_eq!(body["security"]["unlocks"][0], "passkeys");

    let (status, _) = get(&format!("{}/api/devices", h.origin), None).await;
    assert!(status.contains("401"), "{status}");

    let (status, _) = get(&format!("{}/api/devices", h.origin), Some("kmp_bogus")).await;
    assert!(status.contains("401"), "{status}");

    let token = h.token(Role::Full).await;
    let (status, body) = get(&format!("{}/api/devices", h.origin), Some(&token)).await;
    assert!(status.contains("200"), "{status}");
    assert_eq!(body["devices"].as_array().unwrap().len(), 1);

    // A passkey ceremony on an IP origin is refused with a reason, not a broken ceremony.
    let refusal = post(
        &format!("{}/auth/webauthn/authenticate/start", h.origin),
        &json!({}),
    )
    .await;
    assert!(
        refusal["reason"]
            .as_str()
            .unwrap_or_default()
            .contains("registrable domain"),
        "{refusal}"
    );
}

/// The same-origin gate used to build its allowlist from the request's own `Host`, so a
/// rebinding attacker satisfied it with two headers they wrote themselves.
#[tokio::test(flavor = "multi_thread")]
async fn an_origin_the_attacker_chose_never_satisfies_the_same_origin_gate() {
    let h = harness!("origin");
    let pairing = h
        .node
        .auth
        .create_pairing(Role::Full, kampr_auth::Delivery::Authenticated)
        .await
        .unwrap();
    let body = json!({ "code": pairing.code, "device_name": "rebound" });

    let status = post_from(
        &format!("{}/auth/pair", h.origin),
        &body,
        "kampr.rebind.example",
        Some("http://kampr.rebind.example"),
    )
    .await;
    assert!(
        status.contains("403"),
        "Origin == Host is the attacker's own claim, not a same-origin proof: {status}"
    );

    // And the ordinary path still works, so the gate is a gate rather than a wall.
    let good = post(&format!("{}/auth/pair", h.origin), &body).await;
    assert!(good["token"].is_string(), "{good}");
}

/// A cookie is the one credential a browser attaches by itself, and the origin gate exempts every
/// `GET` outside `/ws` — so as long as a cookie authenticates anything, `GET /api/devices` is one
/// deployment decision away from being a cross-origin read of the whole inventory. Nothing sets
/// this cookie and no client sends one, so the honest fix is that it is not a credential at all.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_cookie_is_not_a_credential() {
    let h = harness!("cookie");
    let token = h.token(Role::Full).await;
    h.token(Role::Readonly).await;
    let victim_id = h
        .node
        .auth
        .devices()
        .await
        .unwrap()
        .into_iter()
        .find(|d| d.role == Role::Readonly)
        .unwrap()
        .id;

    let revoke = format!("{}/api/devices/{victim_id}/revoke", h.origin);
    assert!(
        with_cookie("POST", &revoke, &token, None).await.contains("401"),
        "an ambient credential must not be a credential"
    );
    assert!(
        with_cookie("POST", &revoke, &token, Some(&h.origin))
            .await
            .contains("401"),
        "a same-origin cookie is still a cookie"
    );

    // The un-gated GET is the one that mattered: the device inventory, read from another origin.
    let devices = format!("{}/api/devices", h.origin);
    assert!(
        with_cookie("GET", &devices, &token, Some("https://evil.example"))
            .await
            .contains("401"),
        "a cross-origin page must not be able to read the device inventory"
    );

    // And the header the shipped client actually sends still works.
    assert!(
        get(&devices, Some(&token)).await.0.contains("200"),
        "the bearer path is the one the client uses and must be untouched"
    );
}

/// Reconnection only. That a *live* socket dies is a different claim, and it has its own test —
/// this one used to be named as though it covered both.
#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_token_cannot_reconnect_and_a_wrong_code_never_gets_one() {
    let h = harness!("revoke");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let device = h.node.auth.devices().await.unwrap().pop().unwrap();
    h.node.auth.revoke(&device.id, &device).await.unwrap();

    let url = h.origin.replacen("http", "ws", 1) + "/ws";
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("kampr.token.{token}").parse().unwrap(),
    );
    assert!(
        tokio_tungstenite::connect_async(request).await.is_err(),
        "a revoked token must not reconnect"
    );

    let refused = post(
        &format!("{}/auth/pair", h.origin),
        &json!({ "code": "ZZZZ-ZZZZ", "device_name": "attacker" }),
    )
    .await;
    assert!(refused["token"].is_null(), "{refused}");

    // The audit log records the write actions, at 0600.
    let audit = Config::audit_path(h._state.path());
    let text = std::fs::read_to_string(&audit).unwrap();
    assert!(text.lines().any(|l| l.contains("\"pairing.redeemed\"")), "{text}");
    assert!(text.lines().any(|l| l.contains("\"device.revoked\"")), "{text}");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(&audit).unwrap().permissions().mode() & 0o777,
            0o600
        );
    }
}

/// The kill switch has to reach the socket that is already open. Revoking a device and then
/// checking that a *second* connection is refused proves nothing about the one holding the pane.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_a_device_hangs_up_the_socket_it_is_already_using() {
    let h = harness!("revoke-live");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let device = h.node.auth.devices().await.unwrap().pop().unwrap();
    h.node.auth.revoke(&device.id, &device).await.unwrap();

    assert!(
        closed(&mut socket, 15).await,
        "a revoked device kept its live socket"
    );
}

/// Same defect from the other two directions: the handshake snapshot of the role, and the
/// handshake snapshot of the expiry. Both have to be re-read on the live socket.
#[tokio::test(flavor = "multi_thread")]
async fn a_demotion_and_an_expiry_both_land_on_the_open_socket() {
    let h = harness!("demote-live");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    let device = h.node.auth.devices().await.unwrap().pop().unwrap();
    h.node
        .auth
        .set_role(&device.id, Role::Readonly, &device)
        .await
        .unwrap();
    send(&mut socket, json!({ "t": "input", "pane": pane, "text": "x" })).await;
    assert_eq!(
        until(&mut socket, "error", 10).await["code"],
        "not_writer",
        "a demoted device kept writing against its handshake role"
    );

    // Straight to the database, so nothing in-process is notified — this is the path a Tier 0
    // token passing its expiry mid-session takes.
    h.node
        .auth
        .store()
        .extend_device(&device.id, Some(kampr_auth::now() - 1))
        .await
        .unwrap();
    assert!(
        closed(&mut socket, 15).await,
        "an expired device kept its live socket"
    );
}

/// Enforcement without an announcement is half a fix: a demoted device keeps every write
/// affordance drawn and finds out by being refused. The change has to reach the client on the
/// socket it is already holding — in both directions, because a promotion is the same problem in
/// reverse — and `hello` has to stay the *first* message on a connection rather than quietly
/// becoming one a node re-sends.
#[tokio::test(flavor = "multi_thread")]
async fn a_demotion_and_a_promotion_are_both_announced_on_the_open_socket() {
    let h = harness!("role-frame");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    assert_eq!(until(&mut socket, "hello", 10).await["role"], "full");

    let device = h.node.auth.devices().await.unwrap().pop().unwrap();
    h.node
        .auth
        .set_role(&device.id, Role::Readonly, &device)
        .await
        .unwrap();
    let (demoted, before) = until_seen(&mut socket, "role", 10).await;
    assert_eq!(demoted["role"], "readonly", "{demoted}");
    assert!(
        !before.contains(&"hello".to_string()),
        "a role change re-sent `hello`, which the protocol defines as the first message on a \
         connection: {before:?}"
    );

    h.node
        .auth
        .set_role(&device.id, Role::Full, &device)
        .await
        .unwrap();
    let (promoted, between) = until_seen(&mut socket, "role", 10).await;
    assert_eq!(promoted["role"], "full", "{promoted}");
    assert!(!between.contains(&"hello".to_string()), "{between:?}");

    // And the promotion is real, not just announced.
    send(
        &mut socket,
        json!({ "t": "input", "pane": h.pane_id(), "text": "" }),
    )
    .await;
    send(&mut socket, json!({ "t": "ping", "n": 1 })).await;
    let (_, seen) = until_seen(&mut socket, "pong", 10).await;
    assert!(
        !seen.contains(&"error".to_string()),
        "a promoted device was still refused: {seen:?}"
    );
}

/// Probe #125. A read-only device probing what it can reach is precisely the thing an audit log
/// exists to notice, and it left no trace at all — not on the socket, and not over HTTP. A buggy
/// client retrying in a loop must not be able to write the log full with the answer.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_write_is_audited_and_a_retry_loop_does_not_flood_the_log() {
    let h = harness!("refused");
    let pane = h.pane_id();
    let viewer = h.token(Role::Readonly).await;
    let mut socket = h.connect(&viewer).await;
    until(&mut socket, "hello", 10).await;

    send(&mut socket, json!({ "t": "input", "pane": pane, "text": "x" })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "not_writer");
    send(&mut socket, json!({ "t": "answer", "pane": pane, "key": "1" })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "not_writer");
    send(
        &mut socket,
        json!({ "t": "manage", "op": "pane.close", "at": pane, "rid": "r1" }),
    )
    .await;
    assert_eq!(until(&mut socket, "managed", 10).await["ok"], false);

    // The same gate, the same silence, on the other surface: the device inventory is one a
    // half-trusted device is refused, and that refusal was unlogged too.
    assert!(
        get(&format!("{}/api/devices", h.origin), Some(&viewer))
            .await
            .0
            .contains("403")
    );

    for _ in 0..40 {
        send(&mut socket, json!({ "t": "input", "pane": pane, "text": "x" })).await;
    }
    send(&mut socket, json!({ "t": "ping", "n": 7 })).await;
    until(&mut socket, "pong", 15).await;

    let text = std::fs::read_to_string(Config::audit_path(h._state.path())).unwrap();
    let refusals: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str::<Value>(l).ok())
        .filter(|e| e["action"] == "refused")
        .collect();
    let verb =
        |name: &str| -> Vec<&Value> { refusals.iter().filter(|e| e["detail"]["verb"] == name).collect() };
    for name in ["input", "answer", "manage", "api.devices"] {
        assert!(
            !verb(name).is_empty(),
            "a refused {name} left no audit line at all; the log holds {text}"
        );
    }
    let first = verb("input")[0];
    assert_eq!(first["role"], "readonly", "{first}");
    assert_eq!(
        first["pane"],
        pane.as_str(),
        "a refusal that does not say on what: {first}"
    );
    assert_eq!(first["detail"]["code"], "not_writer", "{first}");
    assert!(
        first["device"].is_string() && first["device_name"] == "integration",
        "{first}"
    );
    assert_eq!(verb("manage")[0]["detail"]["op"], "pane.close");

    // 41 refused inputs. Every one of them on its own line is a log a loop can fill; one line for
    // all of them is a count nobody can read off the log.
    let lines = verb("input").len();
    assert!(
        (2..=8).contains(&lines),
        "41 refusals of one verb wrote {lines} lines"
    );
    let highest = verb("input")
        .iter()
        .filter_map(|e| e["detail"]["attempt"].as_u64())
        .max()
        .unwrap_or(0);
    assert!(
        highest >= 16,
        "the suppressed refusals were never counted; highest attempt was {highest}"
    );
}

/// `X-Forwarded-For` grows left to right: the client's own value sits at the head and each proxy
/// appends what it actually saw. Reading the head hands a rotating header a fresh rate-limit
/// bucket per request — which is exactly what the limiter exists to stop.
#[tokio::test(flavor = "multi_thread")]
async fn a_forged_forwarded_for_buys_no_fresh_rate_limit_bucket() {
    let h = harness!("xff", |c: &mut Config| c.server.trust_proxy = true);
    let mut refused = 0;
    for n in 0..12u8 {
        let (status, _) = post_as(
            &format!("{}/auth/pair", h.origin),
            &json!({ "code": "ZZZZ-ZZZZ", "device_name": "attacker" }),
            Some(&format!("10.0.0.{n}, 203.0.113.9")),
        )
        .await;
        if status.contains("429") {
            refused += 1;
        }
    }
    assert!(
        refused > 0,
        "the limiter must key on what the trusted proxy saw, not on what the client claimed"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn the_http_surface_leaks_neither_paths_nor_the_device_inventory() {
    let h = harness!("leaks");
    let full = h.token(Role::Full).await;
    let readonly = h.token(Role::Readonly).await;

    let (status, body) = get(&format!("{}/api/devices", h.origin), Some(&readonly)).await;
    assert!(
        status.contains("403"),
        "a read-only device must not read the device inventory: {status} {body}"
    );
    assert!(
        get(&format!("{}/api/devices", h.origin), Some(&full))
            .await
            .0
            .contains("200")
    );

    // A token in a response body must never be cached by anything between here and the phone.
    let pairing = h
        .node
        .auth
        .create_pairing(Role::Full, kampr_auth::Delivery::Authenticated)
        .await
        .unwrap();
    let headers = head_of(
        &format!("{}/auth/pair", h.origin),
        &json!({ "code": pairing.code, "device_name": "cached" }),
    )
    .await;
    assert!(
        headers.to_lowercase().contains("cache-control: no-store"),
        "{headers}"
    );
    assert!(
        !headers.contains("connect-src 'self' ws:"),
        "a bare ws: scheme matches any host and is a clean exfiltration channel: {headers}"
    );
}

/// A read-only device receives every frame of every pane and is refused every write — but `prefs`
/// and `caps` were not writes as far as the dispatcher was concerned. One wrote unbounded rows
/// keyed on an arbitrary pane id; the other shelled out to `herdr session list` per message.
#[tokio::test(flavor = "multi_thread")]
async fn prefs_and_caps_are_bounded_rather_than_an_amplifier() {
    let h = harness!("bounds");
    let token = h.token(Role::Readonly).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    // The greeting now carries this device's stored preferences, so the answer to a *write* is
    // the second `prefs` frame on the socket rather than the first.
    until(&mut socket, "prefs", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "zoom": 1.6 } }),
    )
    .await;
    let stored = until(&mut socket, "prefs", 10).await;
    assert_eq!(stored["panes"][&pane]["zoom"], 1.6, "a viewer keeps its own zoom");

    send(
        &mut socket,
        json!({ "t": "prefs", "pane": "01JNOTAPANE/w9:p9", "prefs": { "zoom": 2.0 } }),
    )
    .await;
    let refusal = until(&mut socket, "error", 10).await;
    assert_eq!(
        refusal["code"], "unknown_pane",
        "an arbitrary pane id is a row nobody asked for: {refusal}"
    );

    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "junk": "x".repeat(64 * 1024) } }),
    )
    .await;
    let refusal = until(&mut socket, "error", 10).await;
    assert_eq!(refusal["code"], "bad_request", "{refusal}");

    // Repeated `caps` must not be one `herdr session list` process per message.
    let before = h.node.caps_spawns();
    for _ in 0..8 {
        send(&mut socket, json!({ "t": "caps" })).await;
        until(&mut socket, "caps", 10).await;
    }
    assert!(
        h.node.caps_spawns() - before <= 1,
        "caps shelled out {} times for 8 messages",
        h.node.caps_spawns() - before
    );
}

/// A read-only device that watches every pane exfiltrates every terminal on the host, and used to
/// leave one `session.opened` line behind. And a `manage` entry that omits `cwd`, `args`, `path`
/// and `branch` records that something ran, not what.
#[tokio::test(flavor = "multi_thread")]
async fn the_audit_records_what_was_read_and_what_actually_ran() {
    let h = harness!("audit");
    let pane = h.pane_id();

    let viewer = h.token(Role::Readonly).await;
    let mut watching = h.connect(&viewer).await;
    until(&mut watching, "hello", 10).await;
    send(
        &mut watching,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut watching, "grid.reset", 15).await;
    send(&mut watching, json!({ "t": "unwatch", "pane": pane })).await;

    let writer = h.token(Role::Full).await;
    let mut driving = h.connect(&writer).await;
    until(&mut driving, "hello", 10).await;
    send(
        &mut driving,
        json!({ "t": "manage", "op": "pane.split", "at": pane, "direction": "right", "cwd": "/tmp" }),
    )
    .await;
    until(&mut driving, "managed", 15).await;

    // The `keys` form of `input` is herdr's key grammar, and probe #7 says that grammar includes
    // single characters — so a client that types a password one key at a time types it, and the
    // log is what an operator hands to somebody else during an investigation.
    send(
        &mut driving,
        json!({ "t": "input", "pane": pane, "keys": ["h", "u", "n", "t", "e", "r", "2"] }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let text = std::fs::read_to_string(Config::audit_path(h._state.path())).unwrap();
    let entries: Vec<Value> = text
        .lines()
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect();
    let of = |action: &str| -> Value {
        entries
            .iter()
            .find(|e| e["action"] == action)
            .unwrap_or_else(|| panic!("no {action} entry in {text}"))
            .clone()
    };

    let watch = of("watch");
    assert_eq!(watch["pane"], pane.as_str());
    assert_eq!(watch["role"], "readonly");
    assert_eq!(
        watch["detail"]["scrollback"], true,
        "history is the whole terminal"
    );
    assert_eq!(of("unwatch")["pane"], pane.as_str());

    let managed = of("manage");
    assert_eq!(managed["detail"]["op"], "pane.split");
    assert_eq!(managed["detail"]["cwd"], "/tmp", "{managed}");
    assert_eq!(managed["detail"]["direction"], "right", "{managed}");

    let typed = of("input");
    assert_eq!(typed["pane"], pane.as_str());
    assert_eq!(
        typed["detail"],
        json!({ "keys": 7 }),
        "the log records how much was typed, never what: {typed}",
    );
    assert!(
        !text.contains("hunter2"),
        "typed text reached the audit log in the clear",
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_shell_pane_delivers_its_history_as_absolute_rows() {
    let h = harness!("history");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut socket, "grid.reset", 15).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "seq 1 400\n" }),
    )
    .await;

    let history = until(&mut socket, "scrollback", 25).await;
    assert_eq!(history["pane"], pane.as_str());
    let rows = history["rows"].as_array().unwrap();
    assert!(!rows.is_empty(), "{history}");
    // Absolute ring indices, ascending — `row` is a u32 here rather than a viewport row.
    let indices: Vec<u64> = rows.iter().map(|r| r["row"].as_u64().unwrap()).collect();
    assert!(
        indices.windows(2).all(|w| w[0] < w[1]),
        "history rows are ordered"
    );
    assert_eq!(indices[0], history["from_top"].as_u64().unwrap());
    assert!(history["total_rows"].as_u64().unwrap() >= rows.len() as u64);
    assert!(history["complete"].is_boolean() && history["capped"].is_boolean());

    let text: String = rows
        .iter()
        .flat_map(|r| r["runs"].as_array().cloned().unwrap_or_default())
        .filter_map(|run| run["x"].as_str().map(str::to_string))
        .collect();
    assert!(text.contains("100"), "history did not carry the pane's output");
}

/// The columns a row's runs actually cover, `None` where a column is the right half of the
/// double-width glyph before it. This is the client's own reading of `w` (probe #210) and the
/// only honest way to ask whether a frame puts a wide glyph where a terminal does.
fn columns(runs: &Value) -> Vec<Option<char>> {
    let mut out = Vec::new();
    for run in runs.as_array().cloned().unwrap_or_default() {
        let w = run["w"].as_u64().unwrap_or(1);
        for ch in run["x"].as_str().unwrap_or("").chars() {
            out.push(Some(ch));
            for _ in 1..w {
                out.push(None);
            }
        }
    }
    out
}

/// The row as the client would build it: one entry per column, `None` where the column is the
/// right half of a wide cell, and each cell's `m` entry glued back onto its base.
fn clusters(runs: &Value) -> Vec<Option<String>> {
    let mut out = Vec::new();
    for run in runs.as_array().cloned().unwrap_or_default() {
        let w = run["w"].as_u64().unwrap_or(1);
        let marks = run["m"].as_array().cloned().unwrap_or_default();
        for (i, ch) in run["x"].as_str().unwrap_or("").chars().enumerate() {
            let mut cell = ch.to_string();
            cell.push_str(marks.get(i).and_then(Value::as_str).unwrap_or(""));
            out.push(Some(cell));
            for _ in 1..w {
                out.push(None);
            }
        }
    }
    out
}

fn cluster_text(cols: &[Option<String>]) -> String {
    cols.iter()
        .flatten()
        .cloned()
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn text_of(cols: &[Option<char>]) -> String {
    cols.iter().flatten().collect::<String>().trim_end().to_string()
}

/// Probe #210, end to end against a real herdr rather than against an idea of one: herdr spends
/// two columns on a double-width glyph and addresses the next glyph at col+2, so a frame that
/// spends one leaves a blank behind every wide character for good.
#[tokio::test(flavor = "multi_thread")]
async fn a_wide_glyph_reaches_the_client_in_the_two_columns_herdr_gave_it() {
    let h = harness!("wide");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "printf '%s\\n' 'AB\u{65e5}\u{672c}\u{8a9e}CD' 'XY\u{1f680}ZW'\n" }),
    )
    .await;

    let mut grid: std::collections::HashMap<u64, Vec<Option<char>>> = std::collections::HashMap::new();
    let mut cjk = None;
    let mut emoji = None;
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    while std::time::Instant::now() < deadline && (cjk.is_none() || emoji.is_none()) {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        let rows = match message["t"].as_str() {
            Some("grid.reset") => {
                grid.clear();
                message["rows_data"].clone()
            }
            Some("grid.patch") => message["rows"].clone(),
            _ => continue,
        };
        for row in rows.as_array().cloned().unwrap_or_default() {
            let cols = columns(&row["runs"]);
            grid.insert(row["row"].as_u64().unwrap_or(0), cols);
        }
        cjk = grid
            .values()
            .find(|c| text_of(c) == "AB\u{65e5}\u{672c}\u{8a9e}CD")
            .cloned();
        emoji = grid.values().find(|c| text_of(c) == "XY\u{1f680}ZW").cloned();
    }

    let cjk = cjk.expect("no row came back reading AB\u{65e5}\u{672c}\u{8a9e}CD");
    assert_eq!(cjk[2], Some('\u{65e5}'));
    assert_eq!(
        cjk[3], None,
        "column 3 is the other half of \u{65e5}, not a blank"
    );
    assert_eq!(cjk[4], Some('\u{672c}'), "herdr addresses this glyph at column 5");
    assert_eq!(cjk[6], Some('\u{8a9e}'), "and this one at column 7");
    assert_eq!(cjk[8], Some('C'), "and the text after them at column 9");

    let emoji = emoji.expect("no row came back reading XY\u{1f680}ZW");
    assert_eq!(
        emoji[2],
        Some('\u{1f680}'),
        "one astral glyph, not two surrogate halves"
    );
    assert_eq!(emoji[3], None);
    assert_eq!(emoji[4], Some('Z'));
}

/// Probe #223, end to end against a real herdr rather than against an idea of one: herdr's cell is
/// a grapheme, it keeps the marks on the base and addresses the next glyph at base + the cluster's
/// width — so an emulator that drops the mark loses it for good, because herdr never repaints a
/// cell it believes already matches.
#[tokio::test(flavor = "multi_thread")]
async fn a_combining_mark_reaches_the_client_riding_on_its_base() {
    let h = harness!("marks");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;
    let script = "printf '%b\\n' 'e\\u0301f'                   'ZZ\\U0001F468\\u200d\\U0001F469\\u200d\\U0001F467XY'                   'QQ\\U0001F1EC\\U0001F1E7XY'\n";
    send(&mut socket, json!({ "t": "input", "pane": pane, "text": script })).await;

    let accent = "e\u{301}";
    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}";
    let flag = "\u{1F1EC}\u{1F1E7}";
    let mut grid: std::collections::HashMap<u64, Vec<Option<String>>> = std::collections::HashMap::new();
    let (mut marked, mut joined, mut flagged) = (None, None, None);
    let deadline = std::time::Instant::now() + Duration::from_secs(25);
    while std::time::Instant::now() < deadline && (marked.is_none() || joined.is_none() || flagged.is_none())
    {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        let rows = match message["t"].as_str() {
            Some("grid.reset") => {
                grid.clear();
                message["rows_data"].clone()
            }
            Some("grid.patch") => message["rows"].clone(),
            _ => continue,
        };
        for row in rows.as_array().cloned().unwrap_or_default() {
            grid.insert(row["row"].as_u64().unwrap_or(0), clusters(&row["runs"]));
        }
        let want = |t: &str| grid.values().find(|c| cluster_text(c) == t).cloned();
        marked = want(&format!("{accent}f"));
        joined = want(&format!("ZZ{family}XY"));
        flagged = want(&format!("QQ{flag}XY"));
    }

    let marked = marked.expect("no row came back reading e\u{301}f");
    assert_eq!(
        marked[0].as_deref(),
        Some(accent),
        "the accent rode in on its base"
    );
    assert_eq!(marked[1].as_deref(), Some("f"), "and f is still in column 1");

    let joined = joined.expect("no row came back reading ZZ<family>XY");
    assert_eq!(joined[2].as_deref(), Some(family), "one cell, not three emoji");
    assert_eq!(joined[3], None, "column 3 is the family's other half");
    assert_eq!(joined[4].as_deref(), Some("X"), "herdr addresses X at column 5");

    let flagged = flagged.expect("no row came back reading QQ<flag>XY");
    assert_eq!(
        flagged[2].as_deref(),
        Some(flag),
        "a flag is one cell of two columns"
    );
    assert_eq!(flagged[3], None);
    assert_eq!(flagged[4].as_deref(), Some("X"));
}

#[tokio::test(flavor = "multi_thread")]
async fn resync_repaints_every_watched_pane_and_unwatch_stops_one() {
    let h = harness!("resync");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;

    send(&mut socket, json!({ "t": "resync" })).await;
    until(&mut socket, "herd", 10).await;
    let repaint = until(&mut socket, "grid.reset", 15).await;
    assert_eq!(repaint["pane"], pane.as_str());

    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    // A frame the node had already sent when it read the `unwatch` is late, not a leak — dispatch
    // is sequential, so a pong sent after it is the client's proof that the unwatch has happened
    // and that anything arriving from here on was chosen after the pane was dropped. Without this
    // barrier the resync's own repaint, published a moment before, is counted as the leak.
    send(&mut socket, json!({ "t": "ping", "n": 8 })).await;
    until(&mut socket, "pong", 10).await;

    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "echo after-unwatch\n" }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1500)).await;
    send(&mut socket, json!({ "t": "ping", "n": 9 })).await;

    let mut grids = 0;
    loop {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            break;
        };
        match message["t"].as_str() {
            Some("pong") => break,
            Some("grid.patch" | "grid.reset") => grids += 1,
            _ => {}
        }
    }
    assert_eq!(grids, 0, "an unwatched pane must stop streaming");
}

#[tokio::test(flavor = "multi_thread")]
async fn pane_preferences_are_stored_per_device() {
    let h = harness!("prefs");
    let mut first = h.connect(&h.token(Role::Full).await).await;
    let mut second = h.connect(&h.token(Role::Full).await).await;
    until(&mut first, "hello", 10).await;
    until(&mut second, "hello", 10).await;
    for socket in [&mut first, &mut second] {
        assert_eq!(
            until(socket, "prefs", 10).await["panes"],
            json!({}),
            "the greeting's prefs, before either device has stored any"
        );
    }
    let pane = h.pane_id();

    send(
        &mut first,
        json!({ "t": "prefs", "pane": pane, "prefs": { "zoom": 1.75, "follow": true } }),
    )
    .await;
    let saved = until(&mut first, "prefs", 10).await;
    assert_eq!(saved["panes"][&pane]["zoom"], 1.75);

    send(&mut second, json!({ "t": "prefs" })).await;
    let other = until(&mut second, "prefs", 10).await;
    assert_eq!(
        other["panes"],
        json!({}),
        "a second device does not inherit the first device's zoom"
    );
}

/// The `pending` path end to end.
///
/// Claude publishes nothing about a pending request until after it is answered (probe #42), so the
/// question is read off the screen and `source` says `"screen"`. This drives that with herdr's own
/// `pane.report_agent`: a real pane, a real blocked agent status, a real prompt on the screen.
#[tokio::test(flavor = "multi_thread")]
async fn a_blocked_agent_pane_publishes_the_question_from_the_screen() {
    let h = harness!("pending");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;

    // Put a permission prompt on the screen, the way a harness would.
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane,
                "text": "printf 'Do you want to make this edit?\\n\\n 1. Yes\\n 2. No\\n'\n" }),
    )
    .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;

    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "blocked" }),
        )
        .await;

    let pending = until_pane(&mut socket, "pending", &pane, 25).await;
    assert_eq!(pending["pane"], pane.as_str());
    assert_eq!(
        pending["source"], "screen",
        "probe #42: the transcript is not the source"
    );
    assert_eq!(pending["question"], "Do you want to make this edit?");
    let options = pending["options"].as_array().unwrap();
    assert_eq!(options.len(), 2);
    assert_eq!(options[0]["key"], "1");
    assert_eq!(options[0]["label"], "Yes");

    // Leaving the blocked state clears the strip — the only way a client can know to drop it.
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;
    let cleared = until_pane(&mut socket, "pending", &pane, 25).await;
    assert!(cleared["question"].is_null(), "{cleared}");
    assert!(cleared["options"].as_array().unwrap().is_empty());
}

/// The desk half of W9, against a real herdr: the name Kampr computes reaches herdr's own
/// metadata table and comes back on `pane.get` as `title` (probe #294).
///
/// **The read-back is the assertion, not the ack.** `pane.report_metadata` answers `ok` to a
/// report it silently dropped (probe #295), so a test that watched the call succeed would go green
/// against a reporter that never landed a single name.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_told_to_report_names_puts_one_on_the_pane_at_the_desk() {
    let h = harness!("naming-on", |config: &mut Config| {
        config.naming.report_to_herdr = true;
    });
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    let mut title = Value::Null;
    for _ in 0..60 {
        title = h._session.call("pane.get", json!({ "pane_id": local })).await["pane"]["title"].clone();
        if !title.is_null() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    // The workspace this harness creates is labelled `kampr`, and its pane sits at a shell with no
    // job in it — so the command section drops and the name is what is left.
    assert_eq!(title, "kampr · bash", "pane.get said {title}");

    // The same name goes in as a *token*, which is the only field herdr will sort its agents
    // sidebar on: the sortable builtins are `agent` and `status`, and `title` is not one of them.
    let pane = h._session.call("pane.get", json!({ "pane_id": local })).await;
    assert_eq!(
        pane["pane"]["tokens"][kampr_core::reporter::TOKEN],
        "kampr · bash",
        "pane.get said {pane}"
    );
}

/// The node half of the sidebar sort, end to end and through the config that gates it: two
/// settings on, a real herdr, and the desk put back on the way out.
///
/// **The clear is the half nothing can assert here**, because herdr will not say what view it is
/// holding — there is no `agent.view.get` and `agent.list` is untouched by the view (probe #296).
/// What this proves is that the whole path reaches a real herdr without being refused; that the
/// clear is *sent*, and sent exactly once and never by a node that set nothing, is
/// `kampr-core`'s `reporting.rs`.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_told_to_sort_a_desk_sorts_it_and_puts_it_back_on_the_way_out() {
    let h = harness!("desk-sort", |config: &mut Config| {
        config.naming.report_to_herdr = true;
        config.naming.sort_desk_agents = true;
    });
    assert!(
        h.node.config.naming.desk_agents().is_some(),
        "both settings on is the only combination that sorts anything"
    );
    // Long enough for the sweep that sets the view; a refusal would be a `warn!` and nothing else,
    // so the assertion that matters is the clear below going through on a herdr that took the set.
    tokio::time::sleep(Duration::from_secs(2)).await;
    h.node.restore_desks().await;

    let again = h
        ._session
        .herdr()
        .clear_agent_view()
        .await
        .expect("herdr answers a clear");
    assert!(!again.active);
}

/// The same node with the setting left alone. Looking at somebody's herd writes nothing into it,
/// which is ADR 0002's invariant and the whole reason the other test needs a flag.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_nobody_asked_leaves_the_desk_exactly_as_it_found_it() {
    let h = harness!("naming-off");
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();
    // Long enough for several sweeps of a node that was going to report.
    tokio::time::sleep(Duration::from_secs(3)).await;
    let got = h._session.call("pane.get", json!({ "pane_id": local })).await;
    assert!(got["pane"]["title"].is_null(), "{got}");
}

/// A forged `X-Forwarded-For` must not hand an attacker a fresh rate-limit bucket per guess. The
/// header is believed only when `trust_proxy` is set, and it is never inferred.
#[tokio::test(flavor = "multi_thread")]
async fn a_forged_forwarded_header_does_not_buy_a_fresh_rate_limit_bucket() {
    let h = harness!("proxy");
    assert!(!h.node.config.server.trust_proxy, "never inferred");

    let mut limited = false;
    for n in 0..24 {
        let (status, _) = post_as(
            &format!("{}/auth/pair", h.origin),
            &json!({ "code": "ZZZZ-ZZZZ", "device_name": "attacker" }),
            Some(&format!("10.0.0.{n}")),
        )
        .await;
        if status.contains("429") {
            limited = true;
            break;
        }
    }
    assert!(
        limited,
        "rotating X-Forwarded-For escaped the limiter, so the header was trusted"
    );
}

/// Own-TLS is the alternative to a reverse proxy: the node terminates it itself. It is only half
/// of what moves a node off Tier 0 — a certificate for a *hostname* is the other half — so this
/// proves the transport works and that HTTPS alone still buys no passkeys.
#[tokio::test(flavor = "multi_thread")]
async fn the_node_can_terminate_tls_itself() {
    let Some(session) = Session::start("tls").await else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    session
        .call("workspace.create", json!({ "label": "tls", "cwd": "/tmp" }))
        .await;

    let state = tempfile::tempdir().unwrap();
    let certified = rcgen::generate_simple_self_signed(vec!["kampr.test".into()]).unwrap();
    let cert = state.path().join("cert.pem");
    let key = state.path().join("key.pem");
    std::fs::write(&cert, certified.cert.pem()).unwrap();
    std::fs::write(&key, certified.signing_key.serialize_pem()).unwrap();

    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    drop(listener);

    let mut config = Config::bootstrap("tlsnode");
    // Nothing in this suite reaches the internet: the release check is the one thing in a
    // node that would, and a test that phoned GitHub would be one with a rate limit.
    config.update.check = false;
    config.server.bind = format!("127.0.0.1:{port}");
    config.server.origin = format!("https://kampr.test:{port}");
    config.server.tls.enabled = true;
    config.server.tls.cert = cert.display().to_string();
    config.server.tls.key = key.display().to_string();
    config.herdr.socket = session.socket.display().to_string();
    config.save(&state.path().join("config")).unwrap();

    let node = Node::start(config, state.path()).await.unwrap();
    // A hostname *with* a certificate is Tier 1 — this is the rung Tier 0 cannot reach.
    assert_eq!(node.auth.tier().tier, 1);
    assert!(node.auth.tier().passkeys);
    assert_eq!(node.auth.tier().rp_id.as_deref(), Some("kampr.test"));

    let server = tokio::spawn(http::serve(node.clone()));
    tokio::time::sleep(Duration::from_millis(600)).await;

    let body = tls_get(port, "kampr.test", "/api/node").await;
    assert!(
        body.to_lowercase().contains("strict-transport-security"),
        "a node that has TLS should say so: {body}"
    );
    assert!(body.contains("\"tier\":1"), "{body}");
    assert!(body.contains("\"passkeys\":true"), "{body}");
    assert!(body.contains("tlsnode"), "{body}");
    server.abort();
}

/// A minimal HTTPS GET that pins the node's own self-signed certificate.
async fn tls_get(port: u16, host: &str, path: &str) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let _ = rustls::crypto::ring::default_provider().install_default();

    #[derive(Debug)]
    struct TrustTheNode;
    impl rustls::client::danger::ServerCertVerifier for TrustTheNode {
        fn verify_server_cert(
            &self,
            _e: &rustls::pki_types::CertificateDer<'_>,
            _i: &[rustls::pki_types::CertificateDer<'_>],
            _n: &rustls::pki_types::ServerName<'_>,
            _o: &[u8],
            _t: rustls::pki_types::UnixTime,
        ) -> Result<rustls::client::danger::ServerCertVerified, rustls::Error> {
            Ok(rustls::client::danger::ServerCertVerified::assertion())
        }
        fn verify_tls12_signature(
            &self,
            _m: &[u8],
            _c: &rustls::pki_types::CertificateDer<'_>,
            _d: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn verify_tls13_signature(
            &self,
            _m: &[u8],
            _c: &rustls::pki_types::CertificateDer<'_>,
            _d: &rustls::DigitallySignedStruct,
        ) -> Result<rustls::client::danger::HandshakeSignatureValid, rustls::Error> {
            Ok(rustls::client::danger::HandshakeSignatureValid::assertion())
        }
        fn supported_verify_schemes(&self) -> Vec<rustls::SignatureScheme> {
            rustls::crypto::ring::default_provider()
                .signature_verification_algorithms
                .supported_schemes()
        }
    }

    let config = rustls::ClientConfig::builder()
        .dangerous()
        .with_custom_certificate_verifier(Arc::new(TrustTheNode))
        .with_no_client_auth();
    let connector = tokio_rustls::TlsConnector::from(Arc::new(config));
    let stream = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
    let server_name = rustls::pki_types::ServerName::try_from(host.to_string()).unwrap();
    let mut tls = connector
        .connect(server_name, stream)
        .await
        .expect("tls handshake");
    let request = format!("GET {path} HTTP/1.1\r\nHost: {host}:{port}\r\nConnection: close\r\n\r\n");
    tls.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    let _ = tls.read_to_end(&mut response).await;
    String::from_utf8_lossy(&response).to_string()
}

// ---------------------------------------------------------------------------
// Binding before herdr, and making an outage visible.
// ---------------------------------------------------------------------------

/// Serves on a listener the caller already owns and hands back the origin.
async fn serve_config(
    config: Config,
    state: &tempfile::TempDir,
) -> (Arc<Node>, String, tokio::task::JoinHandle<()>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    let mut config = config;
    config.server.bind = format!("127.0.0.1:{port}");
    config.server.origin = format!("http://127.0.0.1:{port}");
    config.save(&state.path().join("config")).unwrap();
    let origin = config.origin();
    let node = Node::start(config, state.path())
        .await
        .expect("the node must start");
    let server = tokio::spawn({
        let app = http::router(node.clone());
        async move {
            let _ = http::serve_on(listener, app).await;
        }
    });
    (node, origin, server)
}

async fn pair(node: &Arc<Node>, origin: &str) -> String {
    let pairing = node
        .auth
        .create_pairing(Role::Full, kampr_auth::Delivery::Console)
        .await
        .unwrap();
    if !pairing.armed {
        assert!(node.auth.arm_pairing(&pairing.code).await.unwrap());
    }
    let body = json!({ "code": pairing.code, "device_name": "integration" });
    post(&format!("{origin}/auth/pair"), &body).await["token"]
        .as_str()
        .expect("a token")
        .to_string()
}

async fn open(origin: &str, token: &str) -> Socket {
    let url = origin.replacen("http", "ws", 1) + "/ws";
    let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
    request.headers_mut().insert(
        "sec-websocket-protocol",
        format!("kampr.token.{token}").parse().unwrap(),
    );
    tokio_tungstenite::connect_async(request)
        .await
        .expect("upgrade")
        .0
}

/// The whole point of task 1: with no herdr at all the node still binds, still serves, and still
/// says *why* the herd is empty. It used to spend 30 s refusing connections and then exit.
#[tokio::test(flavor = "multi_thread")]
async fn the_port_is_bound_before_herdr_is_needed() {
    let state = tempfile::tempdir().unwrap();
    let mut config = Config::bootstrap("lonely");
    // Nothing in this suite reaches the internet: the release check is the one thing in a
    // node that would, and a test that phoned GitHub would be one with a rate limit.
    config.update.check = false;
    config.herdr.socket = state.path().join("nothing-here.sock").display().to_string();
    // Only the configured session, which does not exist. Otherwise this node discovers every
    // other test's throwaway herd and the herd is not empty for reasons that are not the point.
    config.herdr.sessions = Some(Vec::new());

    let started = tokio::time::Instant::now();
    let (node, origin, server) = serve_config(config, &state).await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "Node::start must not wait on herdr, took {:?}",
        started.elapsed()
    );

    let (status, body) = get(&format!("{origin}/api/node"), None).await;
    assert!(status.contains("200"), "{status}");
    assert_eq!(body["node_name"], "lonely");

    let token = pair(&node, &origin).await;
    let mut socket = open(&origin, &token).await;
    until(&mut socket, "hello", 10).await;
    let herd = until(&mut socket, "herd", 10).await;
    assert_eq!(herd["panes"].as_array().unwrap().len(), 0, "an empty herd");
    assert_eq!(herd["nodes"][0]["online"], false, "and it says it is offline");
    assert!(
        herd["nodes"][0]["detail"]
            .as_str()
            .is_some_and(|d| d.contains("nothing-here.sock")),
        "the node must say why: {}",
        herd["nodes"][0]
    );
    assert!(
        herd["nodes"][0]["herdr_version"].is_null(),
        "no version was ever learned"
    );
    server.abort();
}

/// Probe #70: stopping herdr under a live watcher produced no error and left `online` true. Both
/// documented codes now fire, and the herd comes back on its own.
#[tokio::test(flavor = "multi_thread")]
async fn a_herdr_outage_reaches_the_client_and_recovers() {
    let h = harness!("outage");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;

    h._session.stop().await;

    let mut saw_unavailable = false;
    let mut saw_node_offline = false;
    let mut saw_offline_node = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline && !(saw_unavailable && saw_node_offline && saw_offline_node)
    {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        match message["t"].as_str() {
            // **Both name the node they are about.** Without that a client has nothing to decide
            // with and shows every outage as a modal strip over whatever screen is open — so a
            // node the operator is not looking at interrupts a pane on a different one, on a
            // phone. The node cannot know which pane is on screen; naming itself is what lets the
            // client know whether this is the thing in the operator's hands or the herd.
            Some("error") => match message["code"].as_str() {
                Some("herdr_unavailable") => {
                    saw_unavailable = true;
                    assert!(
                        message["node"].as_str().is_some_and(|n| !n.is_empty()),
                        "herdr_unavailable named no node: {message}"
                    );
                }
                Some("node_offline") => {
                    saw_node_offline = true;
                    assert!(
                        message["node"].as_str().is_some_and(|n| !n.is_empty()),
                        "node_offline named no node: {message}"
                    );
                }
                _ => {}
            },
            Some("herd.patch") => {
                saw_offline_node |= message["changed"]["nodes"]
                    .as_array()
                    .is_some_and(|nodes| nodes.iter().any(|n| n["online"] == false));
            }
            _ => {}
        }
    }
    assert!(saw_unavailable, "an outage must produce herdr_unavailable");
    assert!(saw_node_offline, "and node_offline");
    assert!(saw_offline_node, "and flip herd.nodes[].online");

    h._session.respawn().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut back = false;
    while tokio::time::Instant::now() < deadline && !back {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        back = match message["t"].as_str() {
            Some("herd") => message["nodes"][0]["online"] == true,
            Some("herd.patch") => message["changed"]["nodes"]
                .as_array()
                .is_some_and(|nodes| nodes.iter().any(|n| n["online"] == true)),
            _ => false,
        };
    }
    assert!(back, "the node must come back online on its own");
}

/// The rarely-visited host: the node is up, herdr is not, and the operator taps New. Probe #324
/// says one spawn spelling starts either kind of server and #325 says racing it is harmless, so
/// the op starts the herdr it needs rather than refusing — and waits for an answered call, which
/// per #326 is the only thing that means the herdr it just started can serve the op.
#[tokio::test(flavor = "multi_thread")]
async fn a_manage_op_on_a_node_whose_herdr_is_stopped_starts_it_rather_than_refusing() {
    let h = harness!("wake");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let node = h.node.node_id().to_string();

    h._session.stop().await;
    assert!(
        h.offline(20).await,
        "the node never noticed its herdr had gone, so this proves nothing"
    );

    let created = ok(
        &mut socket,
        json!({ "t": "manage", "op": "workspace.create", "node": node,
                "label": "woken", "cwd": "/tmp" }),
        30,
    )
    .await;

    // The ack is not the claim. Herdr is answering again, and it is holding the workspace the op
    // said it made — a `managed{ok}` for a workspace nothing has is exactly the shape of #233.
    let snapshot = h._session.herdr().snapshot().await.expect("herdr answers again");
    assert!(
        snapshot
            .workspaces
            .iter()
            .any(|w| w.label.as_deref() == Some("woken")),
        "the woken herdr does not have the workspace the op acked: {created}"
    );
}

/// Kampr must not start a herdr nobody asked it to. Watching, polling and reconnecting are not
/// requests; only a manage op is.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_left_alone_never_starts_the_herdr_it_is_missing() {
    let h = harness!("no-wake");
    h._session.stop().await;
    assert!(h.offline(20).await, "the node never noticed the outage");

    tokio::time::sleep(Duration::from_secs(8)).await;
    assert!(
        !h._session.socket.exists(),
        "the node started herdr on its own, with nobody asking for anything"
    );
}

// ---------------------------------------------------------------------------
// Every session on the host.
// ---------------------------------------------------------------------------

/// A named session is a separate herdr server, so it is a separate node — not an invisible one.
#[tokio::test(flavor = "multi_thread")]
async fn every_herdr_session_on_the_host_is_its_own_node() {
    let Some(first) = Session::start("multi-a").await else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    let Some(second) = Session::start("multi-b").await else {
        return;
    };
    first
        .call("workspace.create", json!({ "label": "one", "cwd": "/tmp" }))
        .await;
    second
        .call("workspace.create", json!({ "label": "two", "cwd": "/tmp" }))
        .await;

    let state = tempfile::tempdir().unwrap();
    let mut config = Config::bootstrap("multinode");
    // Nothing in this suite reaches the internet: the release check is the one thing in a
    // node that would, and a test that phoned GitHub would be one with a rate limit.
    config.update.check = false;
    config.herdr.socket = first.socket.display().to_string();
    config.herdr.sessions = Some(vec![second.name.clone()]);
    let (node, origin, server) = serve_config(config, &state).await;

    let base = node.config.node_id.clone();
    let extra = format!("{base}.{}", second.name);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    while tokio::time::Instant::now() < deadline {
        let herd = node.herd();
        if herd.nodes.len() == 2
            && herd.panes.iter().any(|p| p.node_id == extra)
            && herd.panes.iter().any(|p| p.node_id == base)
        {
            break;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }

    let token = pair(&node, &origin).await;
    let mut socket = open(&origin, &token).await;
    until(&mut socket, "hello", 10).await;
    let mut herd = until(&mut socket, "herd", 10).await;
    let both = |herd: &Value| {
        [base.as_str(), extra.as_str()].iter().all(|id| {
            herd["nodes"]
                .as_array()
                .is_some_and(|n| n.iter().any(|n| n["id"] == *id))
                && herd["panes"]
                    .as_array()
                    .is_some_and(|p| p.iter().any(|p| p["node_id"] == *id))
        })
    };
    for _ in 0..60 {
        if both(&herd) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
        send(&mut socket, json!({ "t": "resync" })).await;
        herd = until(&mut socket, "herd", 10).await;
    }

    let ids: Vec<String> = herd["nodes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|n| n["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        ids.contains(&base),
        "the configured session keeps the bare node id: {ids:?}"
    );
    assert!(
        ids.contains(&extra),
        "the second session is its own node: {ids:?}"
    );

    let panes: Vec<String> = herd["panes"]
        .as_array()
        .unwrap()
        .iter()
        .map(|p| p["id"].as_str().unwrap().to_string())
        .collect();
    assert!(
        panes.iter().any(|p| p.starts_with(&format!("{base}/"))),
        "the first session's panes: {panes:?}"
    );
    assert!(
        panes.iter().any(|p| p.starts_with(&format!("{extra}/"))),
        "the second session's panes: {panes:?}"
    );

    // Ids from two sessions must be distinguishable, and a pane must be driven on its own herdr.
    let target = panes
        .iter()
        .find(|p| p.starts_with(&format!("{extra}/")))
        .unwrap()
        .clone();
    send(&mut socket, json!({ "t": "watch", "pane": target })).await;
    let reset = until(&mut socket, "grid.reset", 20).await;
    assert_eq!(reset["pane"], target.as_str());

    // Dropping a session takes its node down without taking the herd with it.
    second.stop().await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut fell_over = false;
    while tokio::time::Instant::now() < deadline && !fell_over {
        fell_over = node.herd().nodes.iter().any(|n| n.id == extra && !n.online);
    }
    assert!(fell_over, "the second session going away is one node offline");
    assert!(
        node.herd().nodes.iter().any(|n| n.id == base && n.online),
        "and never the first"
    );
    server.abort();
}

// ---------------------------------------------------------------------------
// Observing at the PTY's real width.
// ---------------------------------------------------------------------------

/// Probe #68/#84. In a headless session the PTY does not follow the layout rect, so observing at
/// the rect crops every row. The width is derived from what `pane.read visible` renders instead.
#[tokio::test(flavor = "multi_thread")]
async fn a_split_pane_is_observed_at_the_pty_width_not_the_rect() {
    let h = harness!("ptywidth");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    // A line far wider than either rect, so the PTY's own wrap width is on the screen.
    h._session
        .call(
            "pane.send_text",
            json!({ "pane_id": local, "text": "clear; printf '%.0s#' $(seq 1 400); echo\n" }),
        )
        .await;
    let pty = filled_width(&h._session, &local).await;

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let before = await_grid_at(&mut socket, &pane, pty).await;
    assert_eq!(
        before["cols"].as_u64().unwrap(),
        pty as u64,
        "unsplit: the grid is the PTY's width, not the rect"
    );

    let rect_before = rect_width(&h._session, &local).await;
    h._session
        .call(
            "pane.split",
            json!({ "target_pane_id": local, "direction": "right" }),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(3)).await;
    let rect_after = rect_width(&h._session, &local).await;
    assert!(
        rect_after < rect_before,
        "the split must have halved the rect: {rect_before} -> {rect_after}"
    );
    assert_eq!(
        widest_rendered_row(&h._session, &local).await,
        pty,
        "probe #68: the PTY did not follow the rect"
    );

    // The node must follow the PTY, not the rect it was just handed.
    let after = await_grid_at(&mut socket, &pane, pty).await;
    assert_eq!(after["cols"].as_u64().unwrap(), pty as u64);
}

/// Probe #211. The width prober can prove a width nothing ever wrapped at, and the proof used to
/// be sticky. A screen of double-width glyphs is the sharpest case: every row is the PTY's width
/// in columns and half of it in characters, so counting characters called the pane half as wide
/// as it is, and the stream was restarted at that width and stayed there.
///
/// What is asserted is the contract rather than a number — the node must never stream narrower
/// than the PTY. A wrap made by a wide glyph cannot say whether the last column was used or was
/// too narrow to hold one, so a screen with nothing but wide glyphs on it is allowed to read one
/// column wide; it is never allowed to read narrow.
#[tokio::test(flavor = "multi_thread")]
async fn a_screen_of_wide_glyphs_is_not_streamed_at_half_the_pty_width() {
    let h = harness!("cjkwidth");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    h._session
        .call(
            "pane.send_text",
            json!({ "pane_id": local, "text": "clear; printf '%.0s#' $(seq 1 400); echo\n" }),
        )
        .await;
    let pty = filled_width(&h._session, &local).await;

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let before = await_grid_at(&mut socket, &pane, pty).await;
    assert_eq!(before["cols"].as_u64().unwrap(), pty as u64);

    // The phases #211 was driven through, ending on the one that has no ASCII wrap left on it.
    for phase in [
        "clear; for i in $(seq 1 60); do printf '\\r'; printf '=%.0s' $(seq 1 $i); done; echo\n",
        "clear; seq 1 3000\n",
        "clear; printf '%.0s\u{65e5}' $(seq 1 200); echo\n",
    ] {
        h._session
            .call("pane.send_text", json!({ "pane_id": local, "text": phase }))
            .await;
        tokio::time::sleep(Duration::from_secs(5)).await;
    }

    // Four width polls with nothing but wide glyphs on the screen, and then a fresh watch, which
    // repaints from the pane as it is now (probe #209) and so carries the width the node has
    // settled on rather than the one it started at.
    let narrow = drain_resets(&mut socket, &pane, Duration::from_secs(12)).await;
    assert!(
        narrow.iter().all(|&cols| cols >= pty),
        "the node streamed a {pty}-column pane at {narrow:?}"
    );

    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let after = until_pane(&mut socket, "grid.reset", &pane, 30).await["cols"]
        .as_u64()
        .unwrap() as u16;
    assert_eq!(
        after, pty,
        "a pane of wide glyphs came back at {after} columns against a PTY of {pty}"
    );
}

/// The residual of [#218](#), measured out (probe #220): a screen of nothing but wide glyphs
/// reads *identically* on a grid of `2n` and one of `2n + 1` — the same rows and the same logical
/// line, byte for byte at 92 columns and at 93 — because half a glyph will not sit in the last
/// column. So the break cannot settle its own width, and an even-width pane was streamed a column
/// wide for as long as the CJK stayed on it. It does not have to settle it: the ASCII this pane
/// wrapped a moment earlier did, and a break the standing proof agrees with confirms that width
/// rather than widening it.
#[tokio::test(flavor = "multi_thread")]
async fn an_even_width_pane_of_wide_glyphs_keeps_the_width_its_wrap_proved() {
    let h = harness!("cjkeven");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();
    control_cols(&h._session, &local, 92).await;

    // A prompt short enough to leave the bottom of the screen alone. An interactive prompt comes
    // back from `recent_unwrapped` glued to the blanks around it as one logical line no set of
    // rows rebuilds, and the walk stops at the bottom-most line it cannot rebuild — so the whole
    // reading measures nothing and neither rule is exercised.
    for line in ["exec /bin/sh\n", "PS1='$ '\n", "clear\n"] {
        typed(&h._session, &local, line).await;
    }
    for _ in 0..4 {
        typed(&h._session, &local, "printf '%.0s#' $(seq 1 400); echo\n").await;
    }
    let pty = filled_width(&h._session, &local).await;
    assert_eq!(pty, 92, "the controller left the PTY at an even width");

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let before = await_grid_at(&mut socket, &pane, pty).await;
    assert_eq!(before["cols"].as_u64().unwrap(), pty as u64);

    // Enough wide-glyph lines to push every ASCII row out of the read window: one unambiguous
    // break left in it settles the width on its own and proves nothing about the ambiguous ones.
    typed(&h._session, &local, "clear\n").await;
    for _ in 0..8 {
        typed(&h._session, &local, "printf '%.0s\u{65e5}' $(seq 1 200); echo\n").await;
    }
    tokio::time::sleep(Duration::from_secs(6)).await;

    let widths = drain_resets(&mut socket, &pane, Duration::from_secs(12)).await;
    assert!(
        widths.iter().all(|&cols| cols == pty),
        "the node moved a {pty}-column pane to {widths:?}"
    );

    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let after = until_pane(&mut socket, "grid.reset", &pane, 30).await["cols"]
        .as_u64()
        .unwrap() as u16;
    assert_eq!(
        after, pty,
        "a pane of wide glyphs came back at {after} columns against a PTY of {pty}"
    );
}

async fn typed(session: &Session, pane: &str, text: &str) {
    session
        .call("pane.send_text", json!({ "pane_id": pane, "text": text }))
        .await;
    tokio::time::sleep(Duration::from_millis(1200)).await;
}

/// Resizes a pane's PTY to an even width by claiming it with a controller and letting the
/// controller go: the PTY stays where the controller left it and the layout rect never moves
/// (probe #219), which is the only way to reach a width nothing else in this suite produces.
async fn control_cols(session: &Session, pane: &str, cols: u16) {
    let rows = session.call("session.snapshot", json!({})).await["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == pane)
        .and_then(|p| p["scroll"]["viewport_rows"].as_u64())
        .unwrap_or(40);
    let mut controller = std::process::Command::new("herdr")
        .args(["--session", &session.name, "terminal", "session", "control", pane])
        .args(["--cols", &cols.to_string(), "--rows", &rows.to_string()])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("herdr terminal session control");
    tokio::time::sleep(Duration::from_secs(3)).await;
    let _ = controller.kill();
    let _ = controller.wait();
    tokio::time::sleep(Duration::from_secs(1)).await;
}

/// Every width a `grid.reset` carried for this pane over `window`.
async fn drain_resets(socket: &mut Socket, pane: &str, window: Duration) -> Vec<u16> {
    let deadline = tokio::time::Instant::now() + window;
    let mut widths = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "grid.reset" && message["pane"] == pane {
            widths.push(message["cols"].as_u64().unwrap_or(0) as u16);
        }
    }
    widths
}

/// Waits for a `grid.reset` on this pane whose grid is `pty` wide and whose probe line is
/// uncropped. The first reading can miss — a probe that loses its socket call falls back to the
/// rect and the next poll corrects it — so what is asserted is that the node *arrives* at the
/// PTY's width, not that it never guesses.
async fn await_grid_at(socket: &mut Socket, pane: &str, pty: u16) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] != "grid.reset" || message["pane"] != pane {
            continue;
        }
        seen.push(message["cols"].as_u64().unwrap_or(0));
        if longest_hash_run(&message) == pty {
            return message;
        }
    }
    panic!("the grid never came back at {pty} columns uncropped; saw widths {seen:?}");
}

/// Waits for the probe line to reach the screen and returns the width it wrapped at. A wrapped
/// line is what makes the reading exact, and under load the shell can take seconds to produce it.
async fn filled_width(session: &Session, pane: &str) -> u16 {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut widest = 0;
    let mut settled = 0;
    while tokio::time::Instant::now() < deadline {
        let now = widest_rendered_row(session, pane).await;
        settled = if now == widest { settled + 1 } else { 0 };
        widest = widest.max(now);
        // 400 `#` cannot fit on one row of any plausible pane, so anything this wide has wrapped.
        // Two agreeing samples, because a read that lands mid-write catches a half-drawn row.
        if widest > 60 && settled >= 2 {
            return widest;
        }
        tokio::time::sleep(Duration::from_millis(500)).await;
    }
    panic!("the probe line never settled; widest row was {widest}");
}

async fn widest_rendered_row(session: &Session, pane: &str) -> u16 {
    let read = session
        .call(
            "pane.read",
            json!({ "pane_id": pane, "source": "visible", "format": "text" }),
        )
        .await;
    read["read"]["text"]
        .as_str()
        .unwrap_or_default()
        .lines()
        .map(|l| l.chars().count() as u16)
        .max()
        .unwrap_or(0)
}

async fn rect_width(session: &Session, pane: &str) -> u16 {
    let layout = session.call("pane.layout", json!({ "pane_id": pane })).await;
    layout["layout"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == pane)
        .and_then(|p| p["rect"]["width"].as_u64())
        .unwrap() as u16
}

/// The longest unbroken run of `#` in a grid message — the probe line, as the client would see it.
fn longest_hash_run(message: &Value) -> u16 {
    let rows = message["rows_data"]
        .as_array()
        .or_else(|| message["rows"].as_array());
    rows.map(|rows| {
        rows.iter()
            .map(|row| {
                row["runs"]
                    .as_array()
                    .map(|runs| {
                        runs.iter()
                            .filter_map(|r| r["x"].as_str())
                            .collect::<String>()
                            .matches('#')
                            .count() as u16
                    })
                    .unwrap_or(0)
            })
            .max()
            .unwrap_or(0)
    })
    .unwrap_or(0)
}

/// A Claude-shaped transcript, written the way the harness writes one: JSON Lines, a `tool_use`
/// whose result lands in a later record.
///
/// **Stamped from the clock, not from a date somebody typed.** A transcript belongs to the
/// harness process running in the pane, and a node will not serve one whose last word was written
/// before that process existed — so a fixture frozen at a past date describes a conversation no
/// live pane could have had, and would pass only for as long as nobody looked.
fn claude_transcript(cwd: &str, filler: usize) -> (String, String) {
    let opened = time::OffsetDateTime::now_utc().replace_nanosecond(0).unwrap();
    let at = |seconds: i64| {
        (opened + time::Duration::seconds(seconds))
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    };
    let mut lines = Vec::new();
    for n in 0..filler {
        lines.push(json!({
            "type": "user", "uuid": format!("u{n}"), "cwd": cwd,
            "timestamp": at(n as i64),
            "message": { "content": format!("filler {n}") }
        }));
    }
    lines.push(json!({
        "type": "assistant", "uuid": "a-md", "cwd": cwd,
        "timestamp": at(filler as i64 + 1),
        "message": { "content": [
            { "type": "text",
              "text": "Six, and they are…\n\n| Key | Accepted |\n|---|---|\n| `Up` | yes |\n" }
        ] }
    }));
    lines.push(json!({
        "type": "assistant", "uuid": "a-tool", "cwd": cwd,
        "timestamp": at(filler as i64 + 2),
        "message": { "content": [
            { "type": "tool_use", "id": "tu1", "name": "Bash",
              "input": { "command": "herdr pane list --json", "description": "probe key grammar" } }
        ] }
    }));
    let settle = json!({
        "type": "user", "uuid": "u-result", "cwd": cwd,
        "timestamp": at(filler as i64 + 3),
        "message": { "content": [
            { "type": "tool_result", "tool_use_id": "tu1", "content": "one\ntwo\nthree\n" }
        ] }
    });
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    (body, format!("{settle}\n"))
}

/// Makes the pane's own shell the harness: a copy of `bash` under the harness's name, `exec`ed
/// in place, so that herdr's `pane.process_info` reports a foreground process named after the
/// agent the pane claims to be running. A node serves no conversation to a pane with no harness
/// in it — herdr detects one by scraping the screen and can be wrong — and `pane.report_agent`
/// alone is exactly that wrongness. A shell rather than a `sleep`, because these tests go on to
/// paint the harness's screen through it. Answers with its pid.
async fn become_harness(session: &Session, local: &str, dir: &Path, agent: &str) -> u32 {
    let binary = dir.join(agent);
    if !binary.exists() {
        std::fs::copy(which("bash").expect("bash on PATH"), &binary).unwrap();
    }
    session
        .call(
            "pane.send_text",
            json!({ "pane_id": local, "text": format!("exec {}\n", binary.display()) }),
        )
        .await;
    for _ in 0..100 {
        let info = session
            .call("pane.process_info", json!({ "pane_id": local }))
            .await;
        let found = info["process_info"]["foreground_processes"]
            .as_array()
            .and_then(|ps| ps.iter().find(|p| p["name"] == agent))
            .and_then(|p| p["pid"].as_u64());
        if let Some(pid) = found {
            return pid as u32;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("herdr never reported a {agent} process in the pane");
}

/// The `timestamp` a record carries, read back out of the transcript that was written. The claim
/// is that the wire passes the harness's own stamp through, so the transcript has to be what the
/// expectation is taken from — a literal repeated in the test only ever agrees with itself.
fn stamp_of(body: &str, uuid: &str) -> String {
    body.lines()
        .map(|l| serde_json::from_str::<Value>(l).unwrap())
        .find(|r| r["uuid"] == uuid)
        .expect("the record")["timestamp"]
        .as_str()
        .unwrap()
        .to_string()
}

/// The conversation path end to end: a real node, a real WebSocket, and the bytes checked against
/// what `client/shared/.../Codec.kt` reads — `pane` / `cursor` / `more` / `turns`, each turn
/// `id` / `role` / `at` / `blocks`, each block tagged `b`.
///
/// Herdr 0.8.2 never populates `pane.agent_session` — it detects a harness by scraping the screen
/// — so the transcript is found from the pane's own working directory, which is the path that runs
/// against a real `claude`.
#[tokio::test(flavor = "multi_thread")]
async fn a_watched_agent_pane_streams_its_conversation() {
    let home = tempfile::tempdir().unwrap();
    let cwd = "/tmp";
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();
    let transcript = project.join("9f1c0b2e-0000-4000-8000-000000000042.jsonl");

    let home_path = home.path().display().to_string();
    let h = harness!("convo", |c: &mut Config| c.journals.home = home_path);
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;

    let hello = until(&mut socket, "hello", 10).await;
    assert_eq!(
        hello["caps"]["conversation"], true,
        "a node with a claude adapter serves conversations: {hello}"
    );
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    // Herdr's own agent report is what makes this an agent pane, exactly as detection would —
    // and a real process named after the harness is what makes the report true. The transcript is
    // written after both, because a harness cannot have written one before it started.
    become_harness(&h._session, &local, home.path(), "claude").await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;
    let (body, settle) = claude_transcript(cwd, 45);
    std::fs::write(&transcript, &body).unwrap();

    let mut announced = false;
    for _ in 0..40 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "the pane never claimed a conversation");

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": false, "conversation": true }),
    )
    .await;

    let convo = until(&mut socket, "convo", 25).await;
    assert_eq!(convo["pane"], pane.as_str());
    assert_eq!(convo["more"], true, "45 filler turns are more than one page");
    let cursor = convo["cursor"].as_str().expect("an opaque cursor").to_string();
    let turns = convo["turns"].as_array().unwrap();
    assert_eq!(turns.len(), 40, "a page is bounded: {}", turns.len());
    assert_eq!(
        turns.first().unwrap()["id"],
        cursor,
        "the cursor is the oldest turn in the page"
    );

    let markdown = turns
        .iter()
        .find(|t| t["id"] == "a-md")
        .expect("the assistant turn is in the newest page");
    assert_eq!(markdown["role"], "assistant");
    assert_eq!(markdown["at"], stamp_of(&body, "a-md"));
    let block = &markdown["blocks"][0];
    assert_eq!(block["b"], "md");
    assert!(
        block["text"].as_str().unwrap().contains("| Key | Accepted |"),
        "markdown is passed through verbatim so a table stays a table: {block}"
    );

    let running = turns.iter().find(|t| t["id"] == "a-tool").unwrap();
    assert_eq!(running["blocks"][0]["b"], "tool");
    assert_eq!(running["blocks"][0]["name"], "Bash");
    assert_eq!(running["blocks"][0]["summary"], "probe key grammar");
    assert_eq!(running["blocks"][0]["state"], "running");
    assert_eq!(running["blocks"][1]["b"], "code");
    assert_eq!(running["blocks"][1]["lang"], "bash");

    // The tool settles. It must come back under the same id, replacing the turn rather than
    // arriving as a second one — appending renders every tool twice.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&transcript)
        .unwrap();
    std::io::Write::write_all(&mut file, settle.as_bytes()).unwrap();
    drop(file);

    let revision = until(&mut socket, "convo.turn", 20).await;
    assert_eq!(revision["pane"], pane.as_str());
    let revised = revision["turns"].as_array().unwrap();
    assert_eq!(revised.len(), 1, "only what changed: {revision}");
    assert_eq!(revised[0]["id"], "a-tool", "matched by id, not appended");
    assert_eq!(revised[0]["blocks"][0]["state"], "done");
    assert_eq!(revised[0]["blocks"][0]["lines"], 3);

    // And the client can page backwards through the opaque cursor it was given.
    send(
        &mut socket,
        json!({ "t": "convo.load", "pane": pane, "before": cursor }),
    )
    .await;
    let older = until(&mut socket, "convo", 15).await;
    let older_turns = older["turns"].as_array().unwrap();
    assert_eq!(older_turns.len(), 7, "the remainder of the transcript: {older}");
    assert_eq!(older["more"], false);
    assert_eq!(older_turns.first().unwrap()["id"], "u0");
}

/// `convo.load` used to answer `unsupported`. It is implemented now, so a pane with no transcript
/// is `not_found` — and a shell pane is never watched with a conversation in the first place.
#[tokio::test(flavor = "multi_thread")]
async fn a_shell_pane_has_no_conversation_to_page() {
    let h = harness!("convo-none");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    send(&mut socket, json!({ "t": "convo.load", "pane": pane })).await;
    let refusal = until_pane(&mut socket, "error", &pane, 15).await;
    assert_eq!(refusal["code"], "not_found", "{refusal}");
}

/// One `managed` ack, matched by op: the node serves every session on the machine, so an ack for
/// somebody else's op is not this op's answer.
async fn managed(socket: &mut Socket, op: &str, seconds: u64) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "managed" && message["op"] == op {
            return message;
        }
        seen.push(format!("{}/{}", message["t"], message["op"]));
    }
    panic!("never saw managed for {op}; saw {seen:?}");
}

async fn ok(socket: &mut Socket, request: Value, seconds: u64) -> Value {
    let op = request["op"].as_str().expect("an op").to_string();
    send(socket, request).await;
    let ack = managed(socket, &op, seconds).await;
    assert_eq!(ack["ok"], true, "{op}: {ack}");
    ack
}

/// Waits for a `herd.patch` that adds a pane under `workspace`, which is what proves the node —
/// not the client — put it on screen.
async fn patch_adding(socket: &mut Socket, workspace: &str, seconds: u64) -> String {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] != "herd.patch" {
            continue;
        }
        for pane in message["added"]["panes"].as_array().unwrap_or(&Vec::new()) {
            if pane["workspace_id"] == workspace {
                return pane["id"].as_str().expect("a pane id").to_string();
            }
        }
    }
    panic!("no herd.patch added a pane under {workspace}");
}

/// Every op the Kotlin client can build, in the JSON it builds, against a real herd. The shapes
/// come from `tests/fixtures/manage-ops.json`, which `client/shared`'s `ManageWireTest` asserts
/// the client emits byte for byte — so neither side gets to agree with itself.
#[tokio::test(flavor = "multi_thread")]
async fn every_client_op_lands_on_a_real_herd() {
    let h = harness!("clientops");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let herd = until(&mut socket, "herd", 10).await;

    // The ids a client addresses its containers by have to be on the wire in the first place.
    let seed = herd["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["node_id"] == node.as_str())
        .expect("a pane of this harness's session")
        .clone();
    assert!(seed["workspace_id"].as_str().is_some(), "{seed}");
    assert!(seed["tab_id"].as_str().is_some(), "{seed}");

    let repo = tempfile::tempdir().expect("a repo");
    let repo_path = repo.path().display().to_string();
    for args in [
        vec!["init", "-q", "-b", "main"],
        vec![
            "-c",
            "user.email=t@t",
            "-c",
            "user.name=t",
            "commit",
            "-q",
            "--allow-empty",
            "-m",
            "root",
        ],
    ] {
        assert!(
            std::process::Command::new("git")
                .args(&args)
                .current_dir(repo.path())
                .status()
                .expect("git")
                .success()
        );
    }

    // workspace.create, with the env map the client's old `Map<String, String?>` could not send.
    let created = ok(
        &mut socket,
        json!({ "t": "manage", "op": "workspace.create", "node": node, "label": "kampr-probe",
                "cwd": repo_path, "env": { "KAMPR_PROBE": "1" } }),
        20,
    )
    .await;
    let workspace = created["id"].as_str().expect("a workspace id").to_string();
    let first_pane = patch_adding(&mut socket, &workspace, 25).await;

    // tab.create takes a workspace, and the node derives one from a pane id too.
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "tab.create", "at": workspace, "label": "tests", "cwd": repo_path }),
        20,
    )
    .await;
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "tab.create", "at": first_pane, "label": "derived" }),
        20,
    )
    .await;

    // pane.split, with the float the old type could not send either.
    let split = ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.split", "at": first_pane, "direction": "right", "ratio": 0.35 }),
        20,
    )
    .await;
    let second_pane = split["id"].as_str().expect("a pane id").to_string();
    assert_ne!(second_pane, first_pane);

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.zoom", "at": first_pane, "mode": "on" }),
        15,
    )
    .await;
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.zoom", "at": first_pane, "mode": "off" }),
        15,
    )
    .await;

    // The one op that reshapes a pane, driven for real: claimed, resized, released. This session
    // is headless, so #219 says the size stays — and the reply says what it *measured* rather than
    // echoing what was asked for, because on an attached pane the desk takes it straight back
    // (#19) and a reply that assumed otherwise would be a plausible-looking success.
    let sized = ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": first_pane, "cols": 100, "rows": 30 }),
        30,
    )
    .await;
    assert_eq!(sized["ok"], json!(true), "{sized}");

    // The floor, against a real node: a size no shell is usable at is refused rather than made
    // permanent. This is the guard the whole feature is shaped around.
    send(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": first_pane, "cols": 40, "rows": 12 }),
    )
    .await;
    let refused = managed(&mut socket, "pane.size", 15).await;
    assert_eq!(refused["ok"], json!(false), "40x12 was allowed: {refused}");
    assert_eq!(refused["code"], json!("bad_request"), "{refused}");

    // Holding and letting go. The hold is what makes a size survive on a pane a desk is attached
    // to; the release is the ordinary end of one, and the node's own deadline is the backstop.
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": first_pane,
                "cols": 100, "rows": 30, "mode": "hold" }),
        30,
    )
    .await;
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "pane.size", "at": first_pane, "mode": "release" }),
        15,
    )
    .await;

    // Only a pane's label is nullable, and null is what clears it.
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": first_pane, "label": "build" }),
        15,
    )
    .await;
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": first_pane, "label": null }),
        15,
    )
    .await;
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": workspace, "label": "kampr-probe-2" }),
        15,
    )
    .await;
    send(
        &mut socket,
        json!({ "t": "manage", "op": "rename", "at": workspace, "label": null }),
    )
    .await;
    let refused = managed(&mut socket, "rename", 15).await;
    assert_eq!(
        refused["ok"], false,
        "a workspace has no label to clear: {refused}"
    );
    assert_eq!(refused["code"], "bad_request");

    for at in [&first_pane, &workspace] {
        ok(&mut socket, json!({ "t": "manage", "op": "focus", "at": at }), 15).await;
    }

    // agent.start, with the array of args the old type could not send.
    let kinds = {
        send(&mut socket, json!({ "t": "caps" })).await;
        let caps = until(&mut socket, "caps", 20).await;
        caps["agent_kinds"]
            .as_array()
            .unwrap()
            .iter()
            .filter_map(|k| k.as_str().map(str::to_string))
            .collect::<Vec<_>>()
    };
    assert!(kinds.contains(&"claude".to_string()), "{kinds:?}");
    // The only assertion in this suite that needs a *second* binary on the host. A runner without
    // `claude` cannot exercise herdr's agent detection at all, and pretending otherwise would be a
    // green tick for something never run.
    let has_claude = std::process::Command::new("claude")
        .arg("--version")
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    if !has_claude {
        eprintln!(
            "!!!! agent.start unexercised: no `claude` on PATH, so herdr's agent detection and the \
             herd's `agent` field are untested on this host !!!!"
        );
    }
    // A pane herdr has only just created is not yet "an available shell", so this is the one op
    // that has to wait for the thing it acts on rather than for its own answer.
    let mut started = json!(null);
    if has_claude {
        for _ in 0..20 {
            send(
                &mut socket,
                json!({ "t": "manage", "op": "agent.start", "at": second_pane, "kind": "claude",
                    "name": "probe", "args": [] }),
            )
            .await;
            started = managed(&mut socket, "agent.start", 30).await;
            if started["ok"] == true {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert_eq!(started["ok"], true, "{started}");
        // The agent is real: the node sees the harness on the pane and says so in the herd.
        let mut agent = None;
        for _ in 0..40 {
            agent = h
                .node
                .herd()
                .panes
                .iter()
                .find(|p| p.id == second_pane)
                .and_then(|p| p.agent.clone());
            if agent.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(500)).await;
        }
        assert_eq!(agent.as_deref(), Some("claude"), "the herd never saw the agent");
    }

    // worktree.create / worktree.open — Herdr's own git support, straight through.
    let worktree = ok(
        &mut socket,
        json!({ "t": "manage", "op": "worktree.create", "node": node, "cwd": repo_path,
                "branch": "kampr/probe", "base": "main", "label": "probe-tree" }),
        30,
    )
    .await;
    assert!(worktree["ok"] == true, "{worktree}");
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "worktree.open", "node": node, "path": repo_path,
                "label": "probe-open" }),
        30,
    )
    .await;

    // A pane can be closed on its own, before the layout round-trip renumbers the tab.
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "close", "at": second_pane }),
        20,
    )
    .await;

    // layout.export round-trips into layout.apply unchanged.
    let tab = h
        .node
        .herd()
        .panes
        .iter()
        .find(|p| p.id == first_pane)
        .and_then(|p| p.tab_id.clone())
        .expect("the pane's tab id");
    let exported = ok(
        &mut socket,
        json!({ "t": "manage", "op": "layout.export", "at": tab }),
        20,
    )
    .await;
    assert!(exported["layout"]["root"]["type"].is_string(), "{exported}");
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "layout.apply", "at": tab, "layout": exported["layout"] }),
        20,
    )
    .await;

    // A named session is a whole separate herdr server. Created here, stopped here, gone here.
    let session = format!("kampr-probe-{}", std::process::id());
    let created = CreatedSession::named(&session);
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;
    for _ in 0..100 {
        if created.dir().join("herdr.sock").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        created.dir().join("herdr.sock").exists(),
        "no socket at {:?}",
        created.dir()
    );
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
        20,
    )
    .await;

    // Closing the workspace takes its panes with it, and the client hears about it as a patch.
    let doomed: Vec<String> = h
        .node
        .herd()
        .panes
        .iter()
        .filter(|p| p.workspace_id.as_deref() == Some(workspace.as_str()))
        .map(|p| p.id.clone())
        .collect();
    assert!(!doomed.is_empty(), "the probe workspace had no panes to lose");
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "close", "at": workspace }),
        20,
    )
    .await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut gone = false;
    while tokio::time::Instant::now() < deadline && !gone {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "herd.patch" {
            gone = message["removed_ids"]
                .as_array()
                .is_some_and(|ids| doomed.iter().any(|d| ids.iter().any(|id| id == &json!(d))));
        }
    }
    assert!(gone, "closing the workspace never arrived as a herd.patch");

    // If the client learns a new op, this test has to have driven it against a real herd before
    // it counts as working.
    let fixture: Value = serde_json::from_str(include_str!("fixtures/manage-ops.json")).unwrap();
    let expected: std::collections::BTreeSet<&str> = fixture
        .as_object()
        .unwrap()
        .values()
        .filter_map(|v| v["op"].as_str())
        .collect();
    let exercised: std::collections::BTreeSet<&str> = [
        "workspace.create",
        "tab.create",
        "pane.split",
        "pane.zoom",
        "pane.size",
        "rename",
        "close",
        "focus",
        "agent.start",
        "worktree.create",
        "worktree.open",
        "layout.export",
        "layout.apply",
        "session.create",
        "session.stop",
    ]
    .into_iter()
    .collect();
    assert!(
        expected.difference(&exercised).next().is_none(),
        "no live coverage for {:?}",
        expected.difference(&exercised).collect::<Vec<_>>(),
    );
}

/// A read-only device is refused every one of them by the same gate, which is why the client
/// hides the affordances rather than offering a tap into a refusal.
#[tokio::test(flavor = "multi_thread")]
async fn a_readonly_device_is_refused_every_manage_op() {
    let h = harness!("nomanage");
    let token = h.token(Role::Readonly).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    for op in [
        json!({ "t": "manage", "op": "workspace.create", "node": h.node.node_id() }),
        json!({ "t": "manage", "op": "pane.split", "at": pane, "direction": "right", "ratio": 0.5 }),
        json!({ "t": "manage", "op": "rename", "at": pane, "label": null }),
        json!({ "t": "manage", "op": "close", "at": pane }),
        // The one that reshapes a pane for everybody, and so the one it matters most to refuse.
        // Both forms: the resize itself, and the hold that would keep a desk's screen wrong.
        json!({ "t": "manage", "op": "pane.size", "at": pane, "cols": 200, "rows": 50 }),
        json!({ "t": "manage", "op": "pane.size", "at": pane, "cols": 200, "rows": 50, "mode": "hold" }),
    ] {
        send(&mut socket, op).await;
        assert_eq!(until(&mut socket, "error", 10).await["code"], "not_writer");
    }
}

/// The New sheet builds a `workspace.create` with **no `env` key at all** whenever the operator
/// typed no environment variables, which is almost always. Herdr 0.8.2 refuses `"env": null`
/// with `invalid type: null, expected a map`, so a node that serialises `Option::None` into the
/// params fails every one of those — the button has never worked.
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_with_no_environment_is_created() {
    let h = harness!("bareenv");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;

    // The three shapes a client can produce: no key (the New sheet's own), an empty map, and a
    // populated one. Only the last was ever exercised.
    for (case, request) in [
        (
            "no env key",
            json!({ "t": "manage", "op": "workspace.create", "node": node, "label": "bare" }),
        ),
        (
            "an empty env map",
            json!({ "t": "manage", "op": "workspace.create", "node": node, "label": "empty",
                    "env": {} }),
        ),
        (
            "a populated env map",
            json!({ "t": "manage", "op": "workspace.create", "node": node, "label": "full",
                    "env": { "KAMPR_LIVE": "1" } }),
        ),
    ] {
        send(&mut socket, request).await;
        let ack = managed(&mut socket, "workspace.create", 25).await;
        assert_eq!(ack["ok"], true, "{case}: {ack}");
        let workspace = ack["id"].as_str().expect("a workspace id").to_string();
        patch_adding(&mut socket, &workspace, 25).await;
        ok(
            &mut socket,
            json!({ "t": "manage", "op": "close", "at": workspace }),
            20,
        )
        .await;
    }
}

/// `docs/04-wire-protocol.md`, first rule: unknown `t` values are ignored, not errors. A node
/// that answers `bad_request` breaks every v1 client the moment a v1.1 client shares a build.
#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_tag_draws_no_answer_at_all() {
    let h = harness!("futuret");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    send(&mut socket, json!({ "t": "future.thing", "whatever": 1 })).await;
    send(&mut socket, json!({ "t": "ping", "n": 11 })).await;
    // The pong is the fence: anything the node had to say about `future.thing` is ahead of it.
    let mut answers = Vec::new();
    loop {
        let message = recv(&mut socket, Duration::from_secs(5))
            .await
            .expect("the node answered the ping");
        if message["t"] == "pong" {
            assert_eq!(message["n"], 11);
            break;
        }
        if message["t"] == "error" {
            answers.push(message);
        }
    }
    assert!(
        answers.is_empty(),
        "an unknown `t` must be ignored, not refused: {answers:?}"
    );

    // Ignoring the unknown must not cost the refusal of a *known* verb sent wrong.
    send(&mut socket, json!({ "t": "watch" })).await;
    assert_eq!(until(&mut socket, "error", 10).await["code"], "bad_request");
}

/// Per-device preferences are the mechanism behind remembered zoom, and neither half worked: a
/// fresh socket was never told what it had stored, and a one-key write replaced the blob.
#[tokio::test(flavor = "multi_thread")]
async fn prefs_are_restored_at_hello_and_merged_on_a_partial_write() {
    let h = harness!("prefsmerge");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let empty = until(&mut socket, "prefs", 10).await;
    assert_eq!(empty["panes"], json!({}), "a device with nothing stored: {empty}");
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "view": "conversation" } }),
    )
    .await;
    until(&mut socket, "prefs", 10).await;
    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "zoom": "2" } }),
    )
    .await;
    let merged = until(&mut socket, "prefs", 10).await;
    assert_eq!(
        merged["panes"][&pane],
        json!({ "view": "conversation", "zoom": "2" }),
        "a one-key write must merge, not replace: {merged}"
    );

    // A null clears one key and leaves the rest, which is the only way a client can ever undo a
    // preference once writes merge.
    send(
        &mut socket,
        json!({ "t": "prefs", "pane": pane, "prefs": { "view": null } }),
    )
    .await;
    let cleared = until(&mut socket, "prefs", 10).await;
    assert_eq!(cleared["panes"][&pane], json!({ "zoom": "2" }), "{cleared}");

    // And a new socket is told, unasked. Nothing restores otherwise: the client has no other
    // source for the zoom it left the pane at.
    let mut second = h.connect(&token).await;
    let restored = until(&mut second, "prefs", 10).await;
    assert_eq!(restored["panes"][&pane], json!({ "zoom": "2" }), "{restored}");
}

/// The protocol says a refused op is acknowledged too, and the client's New sheet clears its
/// in-flight state on the ack alone — so a `manage` that draws only an `error` leaves the sheet
/// spinning forever. `rid` has to survive both refusal paths as well.
#[tokio::test(flavor = "multi_thread")]
async fn a_refused_manage_is_still_acknowledged() {
    let h = harness!("refusedack");
    let readonly = h.token(Role::Readonly).await;
    let mut socket = h.connect(&readonly).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "manage", "op": "close", "at": pane, "rid": "r1" }),
    )
    .await;
    let ack = managed(&mut socket, "close", 10).await;
    assert_eq!(ack["ok"], false, "{ack}");
    assert_eq!(ack["code"], "not_writer", "{ack}");
    assert_eq!(ack["rid"], "r1", "the ack echoes the caller's token: {ack}");
    assert_eq!(until(&mut socket, "error", 10).await["code"], "not_writer");

    // The peer-routing refusal is the other path that built an ack by hand and dropped the `rid`.
    let full = h.token(Role::Full).await;
    let mut writer = h.connect(&full).await;
    until(&mut writer, "hello", 10).await;
    send(
        &mut writer,
        json!({ "t": "manage", "op": "layout.export", "at": "01JNOTANODE/w9:t1", "rid": "r2" }),
    )
    .await;
    let refused = managed(&mut writer, "layout.export", 10).await;
    assert_eq!(refused["ok"], false, "{refused}");
    assert_eq!(refused["code"], "unknown_pane", "{refused}");
    assert_eq!(refused["rid"], "r2", "{refused}");
}

/// Field 5 of `/proc/<pid>/stat`, read past the comm field so a process named `a) b` cannot
/// shift the count.
#[cfg(target_os = "linux")]
fn process_group(pid: u32) -> Option<i32> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let after_comm = stat.rsplit_once(") ")?.1;
    after_comm.split_whitespace().nth(2)?.parse().ok()
}

#[cfg(target_os = "linux")]
fn herdr_pid_for(session: &str) -> Option<u32> {
    let wanted = format!("--session\0{session}\0");
    std::fs::read_dir("/proc").ok()?.flatten().find_map(|entry| {
        let pid: u32 = entry.file_name().to_str()?.parse().ok()?;
        let cmdline = std::fs::read_to_string(entry.path().join("cmdline")).ok()?;
        cmdline.contains(&wanted).then_some(pid)
    })
}

/// A herdr session the node created has to outlive the node. It does not while it is a member of
/// the node's process group: `systemctl restart` and a Ctrl-C on a foreground `kampr serve` both
/// signal the group, and take every agent in every node-created session with them.
#[cfg(target_os = "linux")]
#[tokio::test(flavor = "multi_thread")]
async fn a_created_session_leaves_the_nodes_process_group() {
    let h = harness!("detach");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let session = format!("kampr-detach-{}", std::process::id());
    let created = CreatedSession::named(&session);
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;
    for _ in 0..100 {
        if created.dir().join("herdr.sock").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    let spawned = herdr_pid_for(&session).expect("the created herdr is running");
    let ours = process_group(std::process::id()).expect("our own process group");
    let theirs = process_group(spawned).expect("the created herdr's process group");

    let stopped = ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
        20,
    )
    .await;
    // N5: a session op answers with a session, so its `id` must not be dressed up as a pane id a
    // client would then try to watch.
    assert_eq!(stopped["id"], session.as_str(), "{stopped}");

    assert_ne!(
        theirs, ours,
        "a created session shares the node's process group, so one signal kills both"
    );
    assert_eq!(theirs, spawned as i32, "a detached session leads its own group");
}

/// Reported from a phone, twice: "creating a new session — session doesn't open when done" and
/// "closing a session — session doesn't close when done". Both ops did exactly what they said.
/// What was wrong was the answer to the question the client asks next: the New sheet's session
/// list is `caps.sessions`, and `caps` was cached for ten seconds *and* acked before the host
/// agreed with it — `server.stop` answers in under a millisecond while `session list` goes on
/// reporting `running: true` for up to 300 ms (#241). So the one refresh that could have shown
/// the operator anything was handed the session set from before the op, by two mechanisms at once.
#[tokio::test(flavor = "multi_thread")]
async fn caps_answers_a_session_op_with_the_session_set_that_op_produced() {
    let h = harness!("capsfresh");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let session = format!("kampr-capsfresh-{}", std::process::id());
    let _created = CreatedSession::named(&session);

    // Warmed the way a client warms it — on `hello`, long before the operator types a name — so
    // what follows is the answer they would really have been given.
    send(&mut socket, json!({ "t": "caps" })).await;
    let before = until(&mut socket, "caps", 10).await;
    assert_eq!(session_state(&before, &session), None, "{before}");

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;
    send(&mut socket, json!({ "t": "caps" })).await;
    let created = until(&mut socket, "caps", 10).await;
    assert_eq!(
        session_state(&created, &session),
        Some(true),
        "the session the operator just made is missing from the refresh their client sends on its ack: {created}"
    );

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
        20,
    )
    .await;
    send(&mut socket, json!({ "t": "caps" })).await;
    let stopped = until(&mut socket, "caps", 10).await;
    assert_eq!(
        session_state(&stopped, &session),
        Some(false),
        "the session is still advertised as running after its own stop was acknowledged: {stopped}"
    );
}

/// The settle that makes a session op's ack honest is a wait of up to five seconds, and it used
/// to run on the dispatch loop. Dispatch is sequential, so a `server.stop` that goes on being
/// listed as running for another 300 ms (#241) was 300 ms in which this socket answered nothing
/// at all — no input to any other pane, no watch, no caps — because somebody stopped a session.
#[tokio::test(flavor = "multi_thread")]
async fn a_settling_session_op_does_not_stop_the_socket_answering_anything_else() {
    let h = harness!("settling");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let session = format!("kampr-settling-{}", std::process::id());
    let _created = CreatedSession::named(&session);
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;

    send(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
    )
    .await;
    send(&mut socket, json!({ "t": "ping", "n": 41 })).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut answered = false;
    let ack = loop {
        assert!(
            tokio::time::Instant::now() < deadline,
            "never saw the stop acknowledged"
        );
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "pong" && message["n"] == 41 {
            answered = true;
        }
        if message["t"] == "managed" && message["op"] == "session.stop" {
            break message;
        }
    };
    assert!(
        answered,
        "the socket answered nothing until the stop had settled: {ack}"
    );
    assert_eq!(ack["ok"], true, "{ack}");
    assert_eq!(ack["id"], session.as_str(), "{ack}");
}

/// `session.create` used to answer on the spawn alone, which made it a `managed{ok}` for a
/// session that may not exist — the shape of #233, a failure wearing a plausible success. Driven
/// with a herdr that starts, says nothing and leaves: the operator has to be told.
#[cfg(unix)]
#[tokio::test(flavor = "multi_thread")]
async fn a_named_session_that_never_starts_is_refused_rather_than_acknowledged() {
    let bin = tempfile::tempdir().expect("a bin dir");
    let shim = bin.path().join("herdr");
    std::fs::write(&shim, "#!/bin/sh\nexit 0\n").expect("a shim");
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }
    let h = harness!("nosession", |c: &mut Config| {
        c.herdr.binary = shim.display().to_string();
    });
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    send(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": "kampr-never" }),
    )
    .await;
    let ack = managed(&mut socket, "session.create", 30).await;
    assert_eq!(ack["ok"], false, "{ack}");
    assert_eq!(ack["code"], "herdr_unavailable", "{ack}");
    let said = ack["message"].as_str().unwrap_or_default();
    assert!(said.contains("kampr-never"), "the refusal has to name it: {said}");
}

/// `Some(running)` when `caps` lists the session at all — a stopped one stays listed for ever
/// as `running: false` and is only forgotten when its directory goes (#242), so "gone" and
/// "stopped" are two different answers and the sheet draws them differently.
fn session_state(caps: &Value, name: &str) -> Option<bool> {
    caps["sessions"].as_array()?.iter().find(|s| s["name"] == name)?["running"].as_bool()
}

/// `has_conversation` promised a transcript and delivered `not_found`: it was derived from the
/// pane's *harness*, so a `claude` started a minute ago — the pane the New sheet creates, opening
/// on the Conversation view by default — advertised a conversation nothing could load.
#[tokio::test(flavor = "multi_thread")]
async fn a_freshly_started_agent_claims_no_conversation_until_one_exists() {
    let home = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(home.path().join(".claude/projects")).unwrap();
    let home_path = home.path().display().to_string();
    let h = harness!("nojournal", |c: &mut Config| c.journals.home = home_path);
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    let hello = until(&mut socket, "hello", 10).await;
    assert_eq!(hello["caps"]["conversation"], true, "{hello}");

    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();
    become_harness(&h._session, &local, home.path(), "claude").await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    // Give the herd every chance to make the claim before it is disproved.
    let mut claimed = false;
    for _ in 0..24 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(entry) = h.node.herd().pane(&pane) {
            claimed |= serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
    }
    assert!(
        !claimed,
        "the pane advertised a conversation that convo.load answers not_found for"
    );

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    send(&mut socket, json!({ "t": "convo.load", "pane": pane })).await;
    assert_eq!(
        until_pane(&mut socket, "error", &pane, 15).await["code"],
        "not_found"
    );

    // And it flips once the transcript is on disk, which is what makes it a derivation rather
    // than a refusal.
    let project = home.path().join(".claude/projects/-tmp");
    std::fs::create_dir_all(&project).unwrap();
    let (body, _) = claude_transcript("/tmp", 2);
    std::fs::write(project.join("9f1c0b2e-0000-4000-8000-000000000043.jsonl"), body).unwrap();
    let mut announced = false;
    for _ in 0..60 {
        if let Some(entry) = h.node.herd().pane(&pane) {
            announced = serde_json::to_value(entry).unwrap()["has_conversation"] == true;
        }
        if announced {
            break;
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    assert!(announced, "a transcript on disk never reached the herd");
}

/// In a headless session the PTY does not follow the layout rect — a pane whose rect says 47 is
/// really 93 wide — so reporting the rect as `cols` is reporting a number no row was ever
/// wrapped at. The client prints it to the operator in three places.
#[tokio::test(flavor = "multi_thread")]
async fn an_unmeasured_pane_reports_no_width_rather_than_its_rect() {
    let h = harness!("colslie");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    let rect_before = h._session.call("session.snapshot", json!({})).await["snapshot"]["layouts"][0]["panes"]
        [0]["rect"]["width"]
        .as_u64()
        .expect("a layout rect");
    h._session
        .call(
            "pane.split",
            json!({ "target_pane_id": local, "direction": "right", "focus": false }),
        )
        .await;

    let mut narrowed = None;
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        let snapshot = h._session.call("session.snapshot", json!({})).await;
        let rect = snapshot["snapshot"]["layouts"][0]["panes"][0]["rect"]["width"]
            .as_u64()
            .unwrap_or(rect_before);
        if rect < rect_before {
            narrowed = Some(rect);
            break;
        }
    }
    let rect = narrowed.expect("the split never narrowed the rect");

    let mut entry = json!(null);
    for _ in 0..40 {
        tokio::time::sleep(Duration::from_millis(250)).await;
        if let Some(pane) = h.node.herd().pane(&pane) {
            entry = serde_json::to_value(pane).unwrap();
            if entry["rows"].as_u64().is_some() {
                break;
            }
        }
    }
    assert_ne!(
        entry["cols"].as_u64(),
        Some(rect),
        "the herd reported the layout rect as a measured width: {entry}"
    );
    assert!(
        entry["cols"].is_null(),
        "an unmeasured pane has no width to report: {entry}"
    );
    // Rows are knowable without measuring — herdr reports the PTY's own viewport — so they stay.
    let viewport = h._session.call("session.snapshot", json!({})).await["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == local.as_str())
        .and_then(|p| p["scroll"]["viewport_rows"].as_u64())
        .expect("herdr reports the viewport rows");
    assert_eq!(entry["rows"].as_u64(), Some(viewport), "{entry}");
}

/// Probe #112. The pane's measured width moves once — the layout rect is not the PTY width until
/// something wraps and proves it (probes #68/#69) — and the ring restarts on it. What must not
/// happen is the restart repeating: a quiet pane whose width has settled has nothing new to say,
/// and a ring that threw itself away on every read would re-send its whole history for ever.
#[tokio::test(flavor = "multi_thread")]
async fn a_settled_pane_stops_restarting_its_ring() {
    let h = harness!("rewrap");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut socket, "grid.reset", 15).await;
    // Short rows first, so the ring is keyed on the layout rect. Only then a line longer than the
    // pane, which is the one thing that proves the PTY's real width — and the rect is a column
    // wider than the PTY even unsplit (probe #69), so proving it *moves* the ring's width.
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "seq 1 200\n" }),
    )
    .await;
    until_pane(&mut socket, "scrollback", &pane, 25).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "printf '#%.0s' $(seq 1 400); echo\n" }),
    )
    .await;
    let first = until_pane(&mut socket, "scrollback", &pane, 25).await;

    // Let the pane drain and the width prober settle before measuring.
    let mut settled = first["from_top"].as_u64().expect("from_top");
    let settle = tokio::time::Instant::now() + Duration::from_secs(8);
    while tokio::time::Instant::now() < settle {
        if let Some(m) = recv(&mut socket, Duration::from_secs(1)).await
            && m["t"] == "scrollback"
            && m["pane"] == pane.as_str()
            && let Some(from_top) = m["from_top"].as_u64()
        {
            settled = from_top;
        }
    }

    let deadline = tokio::time::Instant::now() + Duration::from_secs(12);
    while tokio::time::Instant::now() < deadline {
        let Some(m) = recv(&mut socket, Duration::from_secs(1)).await else {
            continue;
        };
        if m["t"] != "scrollback" || m["pane"] != pane.as_str() {
            continue;
        }
        assert_eq!(
            m["from_top"].as_u64(),
            Some(settled),
            "the pane is idle, so its ring must not have restarted; it holds {} rows",
            m["total_rows"]
        );
    }
}

/// The count the herd reports for one pane, from a `herd` or a `herd.patch`. `Some(None)` is the
/// field being absent, which is what one viewer looks like on the wire.
fn watchers_in(message: &Value, pane: &str) -> Option<Option<u64>> {
    let lists = [
        &message["panes"],
        &message["changed"]["panes"],
        &message["added"]["panes"],
    ];
    lists
        .iter()
        .filter_map(|l| l.as_array())
        .flatten()
        .find(|p| p["id"] == pane)
        .map(|p| p["watchers"].as_u64())
}

async fn until_watchers(socket: &mut Socket, pane: &str, want: Option<u64>, seconds: u64) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(1)).await else {
            continue;
        };
        if let Some(found) = watchers_in(&message, pane) {
            if found == want {
                return;
            }
            seen.push(found);
        }
    }
    panic!("never saw watchers={want:?} for {pane}; saw {seen:?}");
}

/// Two people can type into one pane and neither could tell. The herd says so now — and it says
/// nothing at all while a pane has one viewer, so the field is a signal rather than noise.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_viewer_is_announced_on_the_pane_they_share() {
    let h = harness!("watchers");
    let token = h.token(Role::Full).await;
    let mut first = h.connect(&token).await;
    until(&mut first, "hello", 10).await;
    let pane = h.pane_id();

    let herd = until(&mut first, "herd", 10).await;
    assert_eq!(
        watchers_in(&herd, &pane),
        Some(None),
        "an unwatched pane says nothing about viewers"
    );

    send(&mut first, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut first, "grid.reset", &pane, 15).await;

    let mut second = h.connect(&token).await;
    until(&mut second, "hello", 10).await;
    send(&mut second, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut second, "grid.reset", &pane, 15).await;
    until_watchers(&mut first, &pane, Some(2), 10).await;

    send(&mut second, json!({ "t": "unwatch", "pane": pane })).await;
    until_watchers(&mut first, &pane, None, 10).await;
}

/// The subscription list, offered to a real herdr.
///
/// All-or-nothing twice over (probes #54, #76): one name herdr will not accept refuses the whole
/// list, and the node then has no events at all and falls back on a sweep measured in tens of
/// seconds with nothing in the log to say why. `pane.output_changed` is exactly such a name —
/// herdr emits it and refuses to subscribe you to it — so no schema check can stand in for this.
#[tokio::test(flavor = "multi_thread")]
async fn herdr_accepts_every_event_the_node_subscribes_to() {
    let Some(session) = Session::start("subs").await else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    session
        .call("workspace.create", json!({ "label": "kampr", "cwd": "/tmp" }))
        .await;
    let snapshot = session.call("session.snapshot", json!({})).await;
    let pane = snapshot["snapshot"]["panes"][0]["pane_id"]
        .as_str()
        .expect("a pane")
        .to_string();

    for agents in [vec![], vec![pane]] {
        let subs = kampr_core::herdr_provider::subscriptions(&agents);
        session
            .herdr()
            .subscribe(&subs)
            .await
            .unwrap_or_else(|e| panic!("herdr refused the {}-entry list: {e}", subs.len()));
    }
}

/// Change-to-client latency, which is the price the slow sweep is not allowed to charge.
///
/// The reconciliation sweep is tens of seconds now, so nothing but the event subscription can
/// carry this: a workspace opened at the desk has to be on the phone in a fraction of a second, or
/// the herd is only as current as its slowest timer.
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_opened_at_the_desk_reaches_a_client_at_once() {
    let h = harness!("evtlatency");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let before: Vec<String> = h.node.herd().panes.iter().map(|p| p.id.clone()).collect();

    let at = tokio::time::Instant::now();
    h._session
        .call("workspace.create", json!({ "label": "second", "cwd": "/tmp" }))
        .await;

    while at.elapsed() < Duration::from_secs(2) {
        let Some(message) = recv(&mut socket, Duration::from_millis(250)).await else {
            continue;
        };
        let fresh = [&message["panes"], &message["added"]["panes"]]
            .iter()
            .filter_map(|l| l.as_array())
            .flatten()
            .filter_map(|p| p["id"].as_str())
            .any(|id| !before.iter().any(|seen| seen == id));
        if fresh {
            eprintln!("a new pane reached the client in {:?}", at.elapsed());
            return;
        }
    }
    panic!("a new workspace took longer than two seconds to reach a connected client");
}

/// A pane whose herdr says `claude`, whose working directory a transcript claims, and whose screen
/// is painted the way Claude 2.1.239 paints one.
///
/// Nothing here is a stub: a real herdr runs the pane, the node's own emulator holds the grid, the
/// journal follows a file on disk, and the frames are read off a websocket.
struct LiveTurn {
    home: tempfile::TempDir,
    work: tempfile::TempDir,
    transcript: PathBuf,
}

impl LiveTurn {
    fn new() -> Self {
        let home = tempfile::tempdir().expect("home");
        let work = tempfile::tempdir().expect("work");
        let cwd = work.path().canonicalize().expect("cwd");
        let slug = cwd.display().to_string().replace('/', "-");
        let project = home.path().join(".claude/projects").join(slug);
        std::fs::create_dir_all(&project).expect("project dir");
        let transcript = project.join("11111111-2222-3333-4444-555555555555.jsonl");
        std::fs::write(&transcript, String::new()).expect("transcript");
        Self {
            home,
            work,
            transcript,
        }
    }

    /// The first record, written **after** the pane exists.
    ///
    /// A harness cannot have written a transcript before the terminal it runs in was opened, and
    /// a node will not serve one that claims to have: the transcripts in a working directory
    /// outnumber the sessions a pane has had, and the ones that predate its harness belong to
    /// somebody else. So the fixture has to happen in the order the real thing happens in.
    fn open(&self) {
        self.append(&json!({
            "type": "user",
            "uuid": "u-1",
            "cwd": self.cwd(),
            "message": { "content": "explain the parser" },
        }));
    }

    fn append(&self, record: &Value) {
        let mut line = record.to_string();
        line.push('\n');
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(&self.transcript)
            .expect("open transcript");
        use std::io::Write;
        file.write_all(line.as_bytes()).expect("append");
    }

    fn cwd(&self) -> String {
        self.work.path().canonicalize().unwrap().display().to_string()
    }
}

/// Paints literal screen fragments, one shell command, with a pause between them — so a block is
/// seen to *grow*, and only one echoed command line ever sits above it.
fn paint(parts: &[&[&str]]) -> String {
    let mut command = String::new();
    for part in parts {
        if !command.is_empty() {
            command.push_str("sleep 1; ");
        }
        let quoted: Vec<String> = part
            .iter()
            .map(|l| format!("'{}'", l.replace('\'', "'\\''")))
            .collect();
        command.push_str(&format!("printf '%s\\n' {}; ", quoted.join(" ")));
    }
    command.push('\n');
    command
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_in_progress_streams_off_the_screen_and_yields_to_the_record() {
    let fixture = LiveTurn::new();
    let home = fixture.home.path().to_path_buf();
    let h = harness!("liveturn", |config: &mut Config| {
        config.journals.home = home.display().to_string();
    });
    // A second workspace, because the harness's own is rooted at /tmp and a transcript is found
    // through the pane's working directory.
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd() }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd()).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();
    fixture.open();

    // Herdr detects a harness by scraping the screen, so a test that wants a `claude` pane at
    // `working` says so through the same API a plugin would — over a pane that really is running
    // a process by that name, because a node serves no conversation to one that is not.
    become_harness(&h._session, &local, fixture.home.path(), "claude").await;
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "source": "kampr-test", "agent": "claude", "state": "working" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 20).await;
    assert_eq!(
        opening["turns"].as_array().map(Vec::len),
        Some(1),
        "the transcript opens with the operator's own turn and nothing else"
    );

    // The message arrives in two paints, because a block earns its preview by growing.
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": paint(&[
            &[
                "● The parser is a state machine over the byte stream, and every escape",
                "  sequence is one walk through it.",
            ],
            &[
                "  Printable text takes the short path and lands in a cell; a control byte",
                "  takes the long one.",
            ],
        ]) }),
    )
    .await;

    let live = until_live(&mut socket, &pane, 20).await.expect("a live turn");
    let text = live["blocks"][0]["text"].as_str().expect("md").to_string();
    assert_eq!(live["id"], "live");
    assert_eq!(live["role"], "assistant");
    assert!(
        text.starts_with("The parser is a state machine over the byte stream"),
        "the marker is layout and the wrap indent is stripped: {text:?}"
    );
    assert!(
        !text.contains('●') && !text.contains("printf"),
        "neither the harness's glyph nor the shell that painted it: {text:?}"
    );

    // The record lands, unwrapped and in markdown, exactly as Claude writes one.
    fixture.append(&json!({
        "type": "assistant",
        "uuid": "a-1",
        "message": { "content": [ { "type": "text", "text":
            "The **parser** is a state machine over the byte stream, and every escape sequence is one walk through it.\n\nPrintable text takes the short path and lands in a cell; a control byte takes the long one." } ] },
    }));

    let mut authoritative = false;
    let mut withdrawn = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    while tokio::time::Instant::now() < deadline && !(authoritative && withdrawn) {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] != "convo.turn" || message["pane"] != pane.as_str() {
            continue;
        }
        for turn in message["turns"].as_array().unwrap_or(&Vec::new()) {
            match turn["id"].as_str() {
                Some("a-1") => authoritative = true,
                Some("live") => {
                    withdrawn = turn["blocks"].as_array().is_none_or(Vec::is_empty);
                    assert!(
                        withdrawn,
                        "once the record is on the wire the preview may only be withdrawn: {turn}"
                    );
                }
                _ => {}
            }
        }
    }
    assert!(authoritative, "the transcript record never arrived");
    assert!(
        withdrawn,
        "the preview was never withdrawn, so the client renders the message twice"
    );

    // And nothing is streamed for a client that did not ask for the conversation.
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut socket, "grid.reset", &pane, 20).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": paint(&[
            &[
                "● A second message nobody asked to see, growing",
                "  across two paints so it would qualify.",
            ],
            &["  And here is the second paint."],
        ]) }),
    )
    .await;
    assert!(
        until_live(&mut socket, &pane, 6).await.is_none(),
        "a pane watched without `conversation` runs no pump and reads no screen"
    );
}

/// The next `convo.turn` carrying the reserved live id, or `None` if none arrives in time.
async fn until_live(socket: &mut Socket, pane: &str, seconds: u64) -> Option<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] != "convo.turn" || message["pane"] != pane {
            continue;
        }
        for turn in message["turns"].as_array().unwrap_or(&Vec::new()) {
            if turn["id"] == "live" && !turn["blocks"].as_array().is_none_or(Vec::is_empty) {
                return Some(turn.clone());
            }
        }
    }
    None
}

/// One `claude`-shaped harness in a pane, and the session file it would have written.
///
/// The process is real: a copy of `sleep` under the name the pane's harness has, run in the pane
/// so that herdr's own `pane.process_info` reports it and the node reads its pid from there. What
/// is written by hand is only the *contents* of `~/.claude/sessions/<pid>.json`, whose shape is
/// checked against a verbatim capture of `claude` 2.1.239 in
/// `kampr-journal/tests/fixtures/identity`.
struct Harnessed {
    home: PathBuf,
    project: PathBuf,
    cwd: String,
    binary: PathBuf,
}

impl Harnessed {
    fn new(home: &Path, work: &Path) -> Self {
        let cwd = work.canonicalize().unwrap().display().to_string();
        let project = home.join(".claude/projects").join(cwd.replace('/', "-"));
        std::fs::create_dir_all(&project).unwrap();
        std::fs::create_dir_all(home.join(".claude/sessions")).unwrap();
        let binary = home.join("claude");
        std::fs::copy(which("sleep").expect("sleep on PATH"), &binary).unwrap();
        Self {
            home: home.to_path_buf(),
            project,
            cwd,
            binary,
        }
    }

    /// Runs the harness in the pane and waits for herdr to report it, answering with its pid.
    async fn start(&self, session: &Session, local: &str) -> u32 {
        session
            .call(
                "pane.send_text",
                json!({ "pane_id": local, "text": format!("{} 600\n", self.binary.display()) }),
            )
            .await;
        for _ in 0..100 {
            let info = session
                .call("pane.process_info", json!({ "pane_id": local }))
                .await;
            let found = info["process_info"]["foreground_processes"]
                .as_array()
                .and_then(|ps| ps.iter().find(|p| p["name"] == "claude"))
                .and_then(|p| p["pid"].as_u64());
            if let Some(pid) = found {
                return pid as u32;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("herdr never reported a claude process in the pane");
    }

    async fn stop(&self, session: &Session, local: &str, pid: u32) {
        std::fs::remove_file(self.session_file(pid)).unwrap();
        session
            .call("pane.send_keys", json!({ "pane_id": local, "keys": ["ctrl+c"] }))
            .await;
        for _ in 0..100 {
            if !Path::new(&format!("/proc/{pid}")).exists() {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }

    fn session_file(&self, pid: u32) -> PathBuf {
        self.home.join(".claude/sessions").join(format!("{pid}.json"))
    }

    /// What the harness publishes about itself, with the pid and `procStart` of the process that
    /// is really running — so a node that checks them against `/proc` is checking real values.
    fn announce(&self, pid: u32, id: &str) {
        let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("/proc stat");
        let start = stat[stat.rfind(") ").unwrap() + 2..]
            .split_whitespace()
            .nth(19)
            .expect("field 22")
            .to_string();
        let record = json!({
            "pid": pid, "sessionId": id, "cwd": self.cwd, "procStart": start,
            "version": "2.1.239", "kind": "interactive", "entrypoint": "cli", "status": "idle",
        });
        std::fs::write(self.session_file(pid), record.to_string()).unwrap();
    }

    /// A further turn on a transcript that is already open, which is what the follow tick sends
    /// as a `convo.turn` rather than as a page.
    fn append(&self, id: &str, uuid: &str, text: &str) {
        let at = time::OffsetDateTime::now_utc()
            .replace_nanosecond(0)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let record = json!({
            "type": "assistant", "uuid": uuid, "cwd": self.cwd, "timestamp": at,
            "message": { "content": [ { "type": "text", "text": text } ] },
        });
        let mut file = std::fs::OpenOptions::new()
            .append(true)
            .open(self.project.join(format!("{id}.jsonl")))
            .unwrap();
        std::io::Write::write_all(&mut file, format!("{record}\n").as_bytes()).unwrap();
    }

    /// A one-turn transcript for a session, stamped `offset` seconds from now — so a test can put
    /// another run's transcript *ahead* of the one the pane is really on.
    fn transcript(&self, id: &str, text: &str, offset: i64) {
        let at = (time::OffsetDateTime::now_utc() + time::Duration::seconds(offset))
            .replace_nanosecond(0)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap();
        let record = json!({
            "type": "assistant", "uuid": format!("{id}-1"), "cwd": self.cwd, "timestamp": at,
            "message": { "content": [ { "type": "text", "text": text } ] },
        });
        std::fs::write(self.project.join(format!("{id}.jsonl")), format!("{record}\n")).unwrap();
    }
}

/// The operator's two reports, end to end: *"i opened claude on a terminal … that had never
/// opened claude and its showing me the most recent session"*, and *"closed claude -> opened
/// again fresh session -> conversation panel showing old and not updating to new at all"*.
///
/// Both are one defect — a transcript resolved from the pane's *directory* — and both are driven
/// here against a real herdr, with a real process in the pane and the pid read out of herdr's own
/// `pane.process_info`. The decoy transcript is deliberately the newest thing in the directory,
/// which is what every previous rule went by.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_shows_the_session_its_own_process_is_on_and_moves_when_it_restarts() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("identity", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let first = fixture.start(&h._session, &local).await;
    fixture.announce(first, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "FIRST SESSION", -60);
    // Somebody else working in the same directory while this pane sat idle, and the newest
    // transcript in it by half a minute — which is what every rule before this one went by.
    fixture.transcript("99999999-9999-4999-8999-999999999999", "SOMEBODY ELSE", -30);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;

    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    let turns = opening["turns"].as_array().expect("turns").clone();
    assert_eq!(
        turns.len(),
        1,
        "the pane's own session, not the directory's newest: {opening}"
    );
    assert_eq!(turns[0]["blocks"][0]["text"], "FIRST SESSION");
    let stale: Vec<String> = turns
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();

    // Quit the agent and start a fresh one in the same pane, in the same directory. Nothing about
    // the pane changes except the process — herdr goes on reporting `claude` throughout, because
    // its detection is a screen scrape and this test is what a stale one looks like.
    fixture.stop(&h._session, &local, first).await;
    // And somebody else works in the directory in the gap — after the pane's *shell* started, so
    // only the harness's own start time excludes it, and there is no harness.
    fixture.transcript("99999999-9999-4999-8999-999999999998", "SOMEBODY ELSE AGAIN", 0);
    assert!(
        !h.claims_a_conversation(&pane).await,
        "a pane with nothing running in it advertised the directory's newest transcript"
    );
    let second = fixture.start(&h._session, &local).await;
    assert_ne!(second, first);
    fixture.announce(second, "22222222-2222-4222-8222-222222222222");
    fixture.transcript("22222222-2222-4222-8222-222222222222", "SECOND SESSION", 0);

    // The new conversation arrives, and the old one is taken off the client first — a page merges
    // by id, so without the withdrawal the new turns land *above* the old ones and the panel
    // reads as though it never updated.
    let stale = moved_to(&mut socket, &pane, &stale, "SECOND SESSION").await;

    // `/clear` is the same move without a new process: claude rewrites `sessionId` in place in
    // `~/.claude/sessions/<pid>.json` and opens a new transcript under the same pid, so nothing
    // about the pane changes at all (#259). `/compact` does neither and never moves a pane.
    fixture.announce(second, "33333333-3333-4333-8333-333333333333");
    fixture.transcript("33333333-3333-4333-8333-333333333333", "AFTER CLEAR", 0);
    moved_to(&mut socket, &pane, &stale, "AFTER CLEAR").await;
}

/// The same defect one screen away: the transcript moves while nobody is watching the pane.
///
/// **The withdrawal lives with the pump and the conversation lives with the client, and the two
/// have different lifetimes.** A pump is created by `watch` and aborted by `unwatch` — which is
/// what leaving a pane's screen does — while the client's turns are kept for the life of the app.
/// So a `/clear` between leaving a pane and coming back is served by a pump that has never shown
/// this client anything, withdraws nothing, and sends a page whose ids match none of what is on
/// the screen: the new conversation lands *above* the old one and the panel reads as though it
/// never updated.
#[tokio::test(flavor = "multi_thread")]
async fn a_conversation_that_moved_while_the_pane_was_unwatched_is_still_taken_off_the_client() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("unwatched", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let pid = fixture.start(&h._session, &local).await;
    fixture.announce(pid, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "FIRST SESSION", -60);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    let turns = opening["turns"].as_array().expect("turns").clone();
    assert_eq!(turns[0]["blocks"][0]["text"], "FIRST SESSION", "{opening}");
    let mut stale: Vec<String> = turns
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();

    // The conversation goes on after its first page, and a turn that arrived as a revision is on
    // the client exactly as firmly as one that arrived in the page.
    fixture.append(
        "11111111-1111-4111-8111-111111111111",
        "first-session-2",
        "STILL THE FIRST SESSION",
    );
    let grown = until_pane(&mut socket, "convo.turn", &pane, 25).await;
    assert_eq!(grown["turns"][0]["id"], "first-session-2", "{grown}");
    stale.push("first-session-2".to_string());

    // The operator leaves the pane's screen. `AppState` unwatches, and the client keeps every
    // turn it was ever sent — `paneStates` is never pruned.
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;
    // And `/clear`s in the terminal while the phone is on some other screen.
    fixture.announce(pid, "22222222-2222-4222-8222-222222222222");
    fixture.transcript("22222222-2222-4222-8222-222222222222", "AFTER CLEAR", 0);

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    moved_to(&mut socket, &pane, &stale, "AFTER CLEAR").await;
}

/// The other half of the same gap, and the one that actually happens: the transcript **does not
/// move** while the pane is unwatched, it just grows.
///
/// A page merges by id, and the merge an installed phone performs files what it does not recognise
/// at the *top* — a rule written for `convo.load`, which pages backwards. Re-opening the
/// transcript the client is already holding pages forwards, so the turns it is missing are the
/// newest ones and every one of them lands above a conversation from hours earlier, on a view
/// pinned to the bottom. That is a message that was never dropped and never seen, and never
/// revisited either, because the node then records it as delivered.
///
/// Phones on older releases cannot be fixed from the client side, so the node does not send them a
/// page it knows they will misfile. Asserted through the *old* merge on purpose: a test that only
/// checks the turn arrived passes with the defect restored.
#[tokio::test(flavor = "multi_thread")]
async fn a_turn_written_while_the_pane_was_unwatched_lands_below_the_conversation_not_above_it() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("regrown", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let pid = fixture.start(&h._session, &local).await;
    fixture.announce(pid, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "WHICH KEYS?", -60);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    let mut screen: Vec<String> = opening["turns"]
        .as_array()
        .expect("turns")
        .iter()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect();
    assert!(!screen.is_empty(), "{opening}");

    // The operator leaves the pane's screen. `AppState` unwatches; the client keeps every turn it
    // was ever sent, because `paneStates` is never pruned.
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;

    // The agent answers while nobody is on that screen. Same transcript, same session, one turn
    // longer — this is the ordinary case, not `/clear`.
    fixture.append(
        "11111111-1111-4111-8111-111111111111",
        "the-answer",
        "HERE IS THE ANSWER",
    );

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["pane"] != pane {
            continue;
        }
        let tag = message["t"].as_str().unwrap_or("");
        if tag != "convo" && tag != "convo.turn" {
            continue;
        }
        merge_as_an_older_client_would(&mut screen, &message);
        if screen.iter().any(|id| id == "the-answer") {
            assert_eq!(
                screen.last().map(String::as_str),
                Some("the-answer"),
                "the answer was filed above the conversation instead of below it: {screen:?} from {message}",
            );
            return;
        }
    }
    panic!("the answer never reached the client at all");
}

/// The merge performed by every client already installed on a phone, from
/// `docs/04-wire-protocol.md`: a page replaces known ids in place and **prepends** the rest unless
/// it says `fresh`, and a `convo.turn` replaces known ids in place and appends the rest.
fn merge_as_an_older_client_would(screen: &mut Vec<String>, message: &Value) {
    let turns = message["turns"].as_array().cloned().unwrap_or_default();
    let ids: Vec<String> = turns
        .iter()
        .filter_map(|t| t["id"].as_str())
        .map(str::to_string)
        .collect();
    if message["t"] == "convo" && message["fresh"] == json!(true) {
        screen.clear();
    }
    let unknown: Vec<String> = ids.iter().filter(|id| !screen.contains(id)).cloned().collect();
    if message["t"] == "convo" {
        screen.splice(0..0, unknown);
    } else {
        screen.extend(unknown);
    }
}

/// A client that **freezes rather than closes** is reaped, and one that is merely quiet is not.
///
/// Every path where the node learns the client is gone already tears down cleanly — `unwatch`, a
/// clean close, an abrupt RST, the pane closing (#284). The hole is the peer whose kernel is alive
/// and ACKing while the application never reads again: a phone frozen in the background, a
/// suspended laptop, a NAT that dropped the flow. Its socket sits in TCP zero-window persist,
/// which resets the probe counter on every probe, so nothing below ever errors, the writer never
/// breaks, and `outbox.close()` never happens. Measured: the watch was still held after 25
/// minutes, costing herdr exactly what a watched pane costs and holding one of 64 socket permits
/// for ever. The mesh link has had this guard all along; the link a phone uses had none.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_stops_answering_is_dropped_and_a_quiet_one_is_not() {
    let h = harness!("keepalive", |c: &mut Config| c.limits.client_keepalive_secs = 1);
    let deadline = Duration::from_secs(3);
    let token = h.token(Role::Full).await;

    // The control, and it has to come first: a client that answers is *quiet* on this wire too —
    // it sends nothing at all once it has greeted — so a reaper keyed on inbound traffic rather
    // than on the pong would take every idle phone with it, and this test would be the only thing
    // between that and a release. Polling the socket is the whole of what an awake client does:
    // `tungstenite` answers the node's pings from inside `next`, and never touching it is what
    // "frozen" means below.
    let mut awake = h.connect(&token).await;
    until(&mut awake, "hello", 10).await;
    assert!(
        !closed_within(&mut awake, deadline * 3).await,
        "an idle client that still answers its pings was dropped"
    );
    send(&mut awake, json!({ "t": "resync" })).await;
    until(&mut awake, "herd", 10).await;

    // And now one that goes away without saying so. Nothing is read from this socket at all, so
    // every ping the node sends goes unanswered.
    let mut frozen = h.connect(&token).await;
    until(&mut frozen, "hello", 10).await;
    tokio::time::sleep(deadline * 3).await;
    assert!(
        closed_within(&mut frozen, Duration::from_secs(20)).await,
        "a client that stopped answering was still being served past its deadline"
    );
}

/// The expensive case end to end: a frozen client **holding a watch on a producing pane**, which
/// is the one that costs herdr what a live watcher costs (#284) — a frozen client on a quiet pane
/// costs nothing but a socket permit.
///
/// **What this does not prove.** The intended guard here is the deadline on `out.send`: once the
/// socket's buffers fill, the send pends inside the `select!` and the keepalive arm is never polled
/// again, so the ping cannot fire at all. Disabling the ping shows this test still failing, so the
/// send never stalls for a whole deadline at this volume — `pump_pane` purges a congested pane's
/// frames rather than queueing them, which keeps the writer from feeding the socket fast enough to
/// fill it. The state is real regardless: #284 captured `Send-Q 2636553 notsent rwnd_limited:100%`
/// on a socket in exactly this condition. So the send deadline stands on that measurement and on a
/// reading of the `select!`, and **not** on this test, which the ping is what passes.
#[tokio::test(flavor = "multi_thread")]
async fn a_frozen_client_watching_a_producing_pane_is_dropped() {
    let h = harness!("keepalive-write", |c: &mut Config| c
        .limits
        .client_keepalive_secs = 1);
    let token = h.token(Role::Full).await;
    let pane = h.pane_id();

    let mut frozen = h.connect(&token).await;
    until(&mut frozen, "hello", 10).await;
    send(
        &mut frozen,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until_pane(&mut frozen, "grid.reset", &pane, 25).await;

    // From here the socket is never read again, and the pane is told to fill it. The wait has to
    // be a bare sleep: `closed_within` *reads*, and a read is `tungstenite` answering the node's
    // pings from inside `next` — which is the client waking up, not the client being frozen.
    send(
        &mut frozen,
        json!({ "t": "input", "pane": pane, "text": "seq 1 200000\n" }),
    )
    .await;
    tokio::time::sleep(Duration::from_secs(15)).await;

    assert!(
        closed_within(&mut frozen, Duration::from_secs(30)).await,
        "a client that stopped reading was written to for ever instead of being dropped"
    );
}

/// Whether the node closed this socket, as distinct from it having nothing to say. [`recv`] folds
/// a timeout, a close and an undecodable frame into the same `None`, and the difference is the
/// whole claim here.
async fn closed_within(socket: &mut Socket, within: Duration) -> bool {
    let deadline = tokio::time::Instant::now() + within;
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(Duration::from_millis(500), socket.next()).await {
            Err(_) => continue,
            Ok(None) | Ok(Some(Err(_))) => return true,
            Ok(Some(Ok(tungstenite::Message::Close(_)))) => return true,
            Ok(Some(Ok(_))) => continue,
        }
    }
    false
}

/// The half a withdrawal cannot reach: a client that comes back on a **different socket**.
///
/// The node's record of what a client is holding lives with that client's session, and a
/// reconnecting phone gets a new one — `KamprConnection` re-watches every pane it holds and
/// `paneStates` survives the drop, so the turns are still on the screen and the node has no way to
/// name them. Server-side memory cannot close this either: the node itself restarts, and a client
/// that reconnects to a restarted node is in exactly the same position. So the page says so, and
/// the client drops what it holds for the pane before applying it. Additive: a build that ignores
/// the field behaves as it does today.
///
/// The negative is the other half of the claim. `convo.load` answers with older slices of the
/// *same* transcript and those must merge, or paging backwards would throw away the page above.
#[tokio::test(flavor = "multi_thread")]
async fn a_page_a_reconnecting_client_could_not_have_been_told_about_replaces_what_it_holds() {
    let home = tempfile::tempdir().unwrap();
    let work = tempfile::tempdir().unwrap();
    let fixture = Harnessed::new(home.path(), work.path());
    let home_path = home.path().display().to_string();
    let h = harness!("reconnect", |c: &mut Config| c.journals.home = home_path);
    h._session
        .call(
            "workspace.create",
            json!({ "label": "convo", "cwd": fixture.cwd }),
        )
        .await;
    let pane = h.pane_with_cwd(&fixture.cwd).await.expect("the convo pane");
    let local = pane.rsplit('/').next().unwrap().to_string();

    let pid = fixture.start(&h._session, &local).await;
    fixture.announce(pid, "11111111-1111-4111-8111-111111111111");
    fixture.transcript("11111111-1111-4111-8111-111111111111", "FIRST SESSION", -60);
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let opening = until_pane(&mut socket, "convo", &pane, 25).await;
    assert_eq!(opening["turns"][0]["blocks"][0]["text"], "FIRST SESSION");
    assert_eq!(
        opening["fresh"], true,
        "a first page is a conversation starting, and the node cannot know what this socket \
         arrived holding: {opening}"
    );

    let cursor = opening["cursor"].as_str().expect("a cursor").to_string();
    send(
        &mut socket,
        json!({ "t": "convo.load", "pane": pane, "before": cursor }),
    )
    .await;
    let older = until_pane(&mut socket, "convo", &pane, 15).await;
    assert!(
        older["fresh"].is_null(),
        "paging backwards through one transcript must merge, or the page above it goes: {older}"
    );

    // The phone goes to sleep, `/clear` happens in the terminal, and the phone comes back on a
    // socket this node has never seen.
    drop(socket);
    fixture.announce(pid, "22222222-2222-4222-8222-222222222222");
    fixture.transcript("22222222-2222-4222-8222-222222222222", "AFTER CLEAR", 0);

    let mut back = h.connect(&token).await;
    until(&mut back, "hello", 10).await;
    send(
        &mut back,
        json!({ "t": "watch", "pane": pane, "conversation": true }),
    )
    .await;
    let after = until_pane(&mut back, "convo", &pane, 25).await;
    assert_eq!(after["turns"][0]["blocks"][0]["text"], "AFTER CLEAR", "{after}");
    assert_eq!(
        after["fresh"], true,
        "the old conversation is still on this client and nothing has told it to let go: {after}"
    );
}

/// Waits for the pane to move to the conversation whose only turn reads `text`, and answers with
/// the ids the client is left holding. Fails unless the previous conversation was withdrawn on
/// the way, because a page that merges leaves the old one underneath the new.
async fn moved_to(socket: &mut Socket, pane: &str, stale: &[String], text: &str) -> Vec<String> {
    let mut withdrawn = false;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(40);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["pane"] != pane {
            continue;
        }
        let turns = message["turns"].as_array().cloned().unwrap_or_default();
        if message["t"] == "convo.turn" {
            let retired: Vec<&str> = turns
                .iter()
                .filter(|t| t["blocks"].as_array().is_none_or(Vec::is_empty))
                .filter_map(|t| t["id"].as_str())
                .collect();
            withdrawn |= stale.iter().all(|id| retired.contains(&id.as_str()));
        }
        if message["t"] == "convo" {
            let texts: Vec<&str> = turns
                .iter()
                .filter_map(|t| t["blocks"][0]["text"].as_str())
                .collect();
            assert_eq!(texts, [text], "{message}");
            assert!(
                withdrawn,
                "the previous conversation was never taken off the client"
            );
            return turns
                .iter()
                .map(|t| t["id"].as_str().unwrap().to_string())
                .collect();
        }
    }
    panic!("the pane never moved to the conversation reading {text:?}");
}

/// A node with nothing to do rebuilds its herd because something changed, not because it can.
///
/// Release discovery is declined by dropping the sender, and a `watch` whose sender is gone
/// answers `changed()` immediately and for ever — so the rebuild loop selected on a channel that
/// was always ready and went round as fast as it could, pinging every herdr it serves on every
/// pass. Nothing in the model moved, so nothing on the wire ever showed it; what it cost was a
/// core, and everything else on the runtime being starved of one.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_with_release_discovery_off_rebuilds_its_herd_only_when_something_changes() {
    let h = harness!("quiet");
    let mut herd = h.node.subscribe_herd();
    herd.borrow_and_update();
    // Nothing is watched, nothing is connected and nothing moves in this window, and the
    // reconcile sweep behind it is thirty seconds — so the honest number of rebuilds is none.
    let mut rebuilds = 0u32;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while tokio::time::timeout_at(deadline, herd.changed()).await.is_ok() {
        rebuilds += 1;
    }
    assert!(
        rebuilds <= 8,
        "the herd was rebuilt {rebuilds} times in two seconds with nothing to rebuild it for"
    );
}

/// Reported from a phone: a workspace made from the New sheet comes up with nothing on it, typing
/// into it puts nothing on the screen, and there is no shell anywhere in sight.
///
/// Everything `a_paired_device_drives_a_pane_end_to_end` proves, against a pane that did not exist
/// when the herd was first sent — which is the only difference, and the whole report.
#[tokio::test(flavor = "multi_thread")]
async fn a_workspace_created_from_the_client_is_a_pane_you_can_type_into() {
    let h = harness!("newws");
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;

    send(
        &mut socket,
        json!({ "t": "manage", "op": "workspace.create", "node": node,
                "label": "typed", "cwd": "/tmp" }),
    )
    .await;
    let ack = managed(&mut socket, "workspace.create", 25).await;
    assert_eq!(ack["ok"], true, "{ack}");
    let workspace = ack["id"].as_str().expect("a workspace id").to_string();
    let pane = patch_adding(&mut socket, &workspace, 30).await;

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let reset = until_pane(&mut socket, "grid.reset", &pane, 25).await;
    assert!(
        reset["rows"].as_u64().unwrap_or(0) > 0,
        "the new pane arrived with no grid at all: {reset}"
    );
    // A shell that has printed its prompt and nothing else puts every non-blank row at the very
    // top of a 40-row grid, with the caret on row 0 — the shape the phone reported as blank.
    assert_eq!(reset["cursor"]["row"], 0, "{reset}");

    let marker = "kampr-new-workspace-marker";
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": format!("echo {marker}\n") }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
    let mut echoed = false;
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline && !echoed {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] == "error" {
            seen.push(message.to_string());
        }
        if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
            continue;
        }
        echoed = message["pane"] == pane.as_str() && message.to_string().contains(marker);
    }
    assert!(
        echoed,
        "a workspace the client made never echoed what was typed into it; errors: {seen:?}"
    );
}

/// Reported from a phone: a pane that produced a burst of output showed a few lines from the top
/// of the screen and never the bottom, and reopening it showed the same truncated screen.
///
/// The layout rect's height is not the PTY's — probe #205: a `down` split halves the rect and
/// leaves the PTY at its old size — and `observe --rows` *crops* to the rows it is handed rather
/// than scrolling to the bottom (probe #206). Sizing the stream from the rect therefore serves
/// every client the top of the pane and nothing else, permanently: there is no later frame that
/// repairs it, because the bottom of the pane was never in the stream to begin with.
#[tokio::test(flavor = "multi_thread")]
async fn a_split_pane_is_observed_at_the_pty_height_not_the_rect() {
    let h = harness!("ptyheight");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    h._session
        .call(
            "pane.split",
            json!({ "target_pane_id": local, "direction": "down" }),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(3)).await;

    let rect = rect_height(&h._session, &local).await;
    let pty = viewport_rows(&h._session, &local).await;
    assert!(
        rect < pty,
        "the split must have halved the rect: rect {rect}, PTY {pty}"
    );

    // Content past the rect, so the bottom of the pane is only reachable at the PTY's height.
    let marker = "kampr-bottom-marker";
    h._session
        .call(
            "pane.send_text",
            json!({ "pane_id": local, "text":
                format!("clear; for i in $(seq 1 {}); do echo filler-$i; done; echo {marker}\n", pty - 5) }),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(45);
    let mut heights = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] != "grid.reset" || message["pane"] != pane.as_str() {
            continue;
        }
        heights.push(message["rows"].as_u64().unwrap_or(0));
        if message["rows"].as_u64() == Some(pty as u64) && message.to_string().contains(marker) {
            return;
        }
    }
    panic!(
        "the pane was never streamed at its PTY's {pty} rows with the bottom of the screen on it; \
         the grid came back at {heights:?} rows, and the rect claims {rect}"
    );
}

async fn rect_height(session: &Session, pane: &str) -> u16 {
    let layout = session.call("pane.layout", json!({ "pane_id": pane })).await;
    layout["layout"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == pane)
        .and_then(|p| p["rect"]["height"].as_u64())
        .unwrap() as u16
}

/// Herdr's own `scroll.viewport_rows` — the PTY's height, which is what the program in the pane
/// is actually writing to.
async fn viewport_rows(session: &Session, pane: &str) -> u16 {
    let snapshot = session.call("session.snapshot", json!({})).await;
    snapshot["snapshot"]["panes"]
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["pane_id"] == pane)
        .and_then(|p| p["scroll"]["viewport_rows"].as_u64())
        .unwrap() as u16
}

/// The operator's own suggested repair — "on opening a session we could ask for the freshest
/// data". A second `watch` must repaint from the pane's *current* screen rather than attaching
/// silently to a stream already in flight, or a pane that moved while nobody was looking stays
/// wrong until it happens to move again.
///
/// A second viewer holds the pane open throughout, so the registry entry survives the `unwatch`
/// and the reopening client is answered from the emulator every viewer shares — the case where
/// there is no fresh `observe` to repaint it by accident.
#[tokio::test(flavor = "multi_thread")]
async fn reopening_a_pane_repaints_it_from_the_screen_as_it_is_now() {
    let h = harness!("reopen");
    let mut held = h.connect(&h.token(Role::Full).await).await;
    let mut socket = h.connect(&h.token(Role::Full).await).await;
    for s in [&mut held, &mut socket] {
        until(s, "hello", 10).await;
        until(s, "herd", 10).await;
    }
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    send(&mut held, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut held, "grid.reset", &pane, 20).await;
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut socket, "grid.reset", &pane, 20).await;
    send(&mut socket, json!({ "t": "unwatch", "pane": pane })).await;

    // Everything from here happens while this client is not looking.
    let marker = "kampr-while-away-marker";
    h._session
        .call(
            "pane.send_text",
            json!({ "pane_id": local, "text": format!("echo {marker}\n") }),
        )
        .await;
    tokio::time::sleep(Duration::from_secs(2)).await;

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut resets = 0;
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] != "grid.reset" || message["pane"] != pane.as_str() {
            continue;
        }
        resets += 1;
        if message.to_string().contains(marker) {
            return;
        }
    }
    panic!(
        "reopening the pane never repainted it as it is now: {resets} grid.reset frames, none of \
         them carrying what the pane printed while this client was away"
    );
}

/// Herdr's cell boundary is UAX #29's and its column count is not `unicode-width`'s: a conjoining
/// jamo block is **one** cell of two columns however many lead jamo it stacks, and an unpaired
/// regional indicator gets the two columns a whole flag gets. Both are only visible through
/// scrollback — `pane.read` hands the node the raw code points and the node lays the row out
/// itself, where the live grid gets herdr's own cursor addressing to lean on.
#[tokio::test(flavor = "multi_thread")]
async fn a_jamo_block_and_a_lone_flag_half_arrive_in_the_columns_herdr_spends() {
    let h = harness!("jamo");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut socket, "grid.reset", 15).await;
    // Only scrollback shows the node laying a row out on its own, and a pane has no scrollback
    // until it has scrolled — so the three lines are pushed off the screen on purpose.
    let script =
        "printf '%b\\n' 'AB\\u1100\\u1100CD' 'AB\\U0001F1EBCD' 'AB\\u1100\\u1161\\u11a8CD'; seq 1 200\n";
    send(&mut socket, json!({ "t": "input", "pane": pane, "text": script })).await;

    let wanted = [
        ("two lead jamo", "\u{1100}\u{1100}"),
        ("an unpaired regional indicator", "\u{1F1EB}"),
        ("a jamo syllable block", "\u{1100}\u{1161}\u{11A8}"),
    ];
    let mut found: Vec<Option<Vec<Option<String>>>> = vec![None; wanted.len()];
    let deadline = std::time::Instant::now() + Duration::from_secs(30);
    while std::time::Instant::now() < deadline && found.iter().any(Option::is_none) {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if message["t"] != "scrollback" {
            continue;
        }
        for row in message["rows"].as_array().cloned().unwrap_or_default() {
            let cols = clusters(&row["runs"]);
            let text = cluster_text(&cols);
            for (i, (_, cluster)) in wanted.iter().enumerate() {
                if text == format!("AB{cluster}CD") {
                    found[i] = Some(cols.clone());
                }
            }
        }
    }

    for (i, (name, cluster)) in wanted.iter().enumerate() {
        let cols = found[i]
            .clone()
            .unwrap_or_else(|| panic!("no row came back reading AB{cluster}CD ({name})"));
        assert_eq!(cols[1].as_deref(), Some("B"), "{name}: the base row is intact");
        assert_eq!(
            cols[2].as_deref(),
            Some(*cluster),
            "{name} is one cell carrying every code point herdr kept"
        );
        assert_eq!(cols[3], None, "{name}: column 3 is the cluster's other half");
        assert_eq!(
            cols[4].as_deref(),
            Some("C"),
            "{name}: C is in column 4, two columns past the cluster"
        );
    }
}

/// Probe #231. The interlock used to refuse history to any pane with a detected agent, on an
/// inherited hazard that was never measured: that reading above the viewport there harvests
/// through the agent's own mouse-scroll interface and moves the operator's screen. It does not —
/// a live `codex` and a live `claude`, both herdr-detected and both holding a ring, answer
/// `lines: 5000` in 1 ms with the whole ring and the viewport where it was. So the pane that is
/// the entire point of the product gets its history like any other.
#[tokio::test(flavor = "multi_thread")]
async fn a_detected_agent_pane_delivers_its_history_too() {
    let h = harness!("agenthistory");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();
    let local = pane.split_once('/').unwrap().1.to_string();

    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

    send(
        &mut socket,
        json!({ "t": "watch", "pane": pane, "scrollback": true }),
    )
    .await;
    until(&mut socket, "grid.reset", 15).await;
    send(
        &mut socket,
        json!({ "t": "input", "pane": pane, "text": "seq 1 400\n" }),
    )
    .await;

    let history = until(&mut socket, "scrollback", 25).await;
    let text: String = history["rows"]
        .as_array()
        .cloned()
        .unwrap_or_default()
        .iter()
        .flat_map(|r| r["runs"].as_array().cloned().unwrap_or_default())
        .filter_map(|run| run["x"].as_str().map(str::to_string))
        .collect();
    assert!(text.contains("100"), "an agent pane's ring is a ring: {history}");
}

/// The reported defect, and the half of it that made it survive for months: a node whose PATH has
/// no herdr serves a correct herd, accepts input and answers every health check, while every pane
/// shows a blank grid and a flashing cursor for ever. The grid was a promise the node made before
/// it knew it could keep it, and the failure to keep it went to a journal that nobody holding a
/// phone can read.
///
/// This is the same machine `transfer.rs` builds, minus the part that hid it: that harness points
/// the socket at nothing too, so its supervisor parks waiting for geometry and never reaches the
/// spawn at all. A node has two ways to reach herdr and can have exactly one of them working.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_that_cannot_run_herdr_says_so_instead_of_promising_a_grid() {
    let nowhere = std::env::temp_dir().join(format!("kampr-no-herdr-{}", std::process::id()));
    let h = harness!("noobserve", |c: &mut Config| {
        c.herdr.binary = nowhere.display().to_string();
    });
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let herd = until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    assert_eq!(
        herd["nodes"][0]["online"], true,
        "the socket is fine and the herd is right — that is what made this invisible: {herd}"
    );

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    let mut refusal: Option<Value> = None;
    let mut detail: Option<String> = None;
    let mut promised = false;
    while tokio::time::Instant::now() < deadline && (refusal.is_none() || detail.is_none()) {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        match message["t"].as_str() {
            Some("grid.reset") if message["pane"] == pane.as_str() => promised = true,
            Some("error") if message["pane"] == pane.as_str() => refusal = Some(message),
            Some("herd") | Some("herd.patch") => detail = detail.or(pane_detail(&message, &pane)),
            _ => {}
        }
    }

    let refusal = refusal.expect("a node that cannot stream a pane has to say so to the client");
    assert_eq!(refusal["code"], "stream_unavailable", "{refusal}");
    let said = refusal["message"].as_str().unwrap_or_default();
    eprintln!("what the operator reads on the phone:\n{said}");
    assert!(
        said.contains("herdr"),
        "the message has to name what is wrong: {said}"
    );
    assert!(
        said.contains("PATH") || said.contains("herdr.binary"),
        "the operator is holding a phone and cannot read a journal, so it has to name the fix: {said}"
    );
    assert!(
        !promised,
        "the node sent a grid it had no stream for; the geometry is a promise it cannot keep"
    );
    let detail = detail.expect("the herd has to carry the state, not just announce the event");
    assert!(detail.contains("herdr"), "{detail}");
}

/// Retrying for ever is right; saying nothing while retrying for ever is not — and neither is
/// leaving the notice up once the pane can paint again. A binary appearing is the recovery this
/// supervisor already retries for, so the whole story has to clear itself.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_recovers_on_its_own_once_herdr_can_be_run() {
    let Some(real) = which("herdr") else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    let bin = tempfile::tempdir().expect("a bin dir");
    let shim = bin.path().join("herdr");
    let h = harness!("recover", |c: &mut Config| {
        c.herdr.binary = shim.display().to_string();
    });
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    let refusal = until_pane(&mut socket, "error", &pane, 30).await;
    assert_eq!(refusal["code"], "stream_unavailable", "{refusal}");

    std::fs::write(&shim, format!("#!/bin/sh\nexec {} \"$@\"\n", real.display())).expect("a shim");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&shim, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    }

    // Both halves in one pass: the herd clears at the spawn and the grid arrives at the first
    // frame after it, so waiting for one and then the other misses whichever came first.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(60);
    let mut painted = false;
    let mut cleared = false;
    while tokio::time::Instant::now() < deadline && !(painted && cleared) {
        let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
            continue;
        };
        match message["t"].as_str() {
            Some("grid.reset") if message["pane"] == pane.as_str() => {
                assert!(message["rows"].as_u64().unwrap_or(0) > 0, "{message}");
                painted = true;
            }
            Some("herd") | Some("herd.patch") if names(&message, &pane) => {
                cleared = pane_detail(&message, &pane).is_none();
            }
            _ => {}
        }
    }
    assert!(painted, "the pane never painted once herdr could be run");
    assert!(cleared, "the notice outlived the fault it was about");
}

/// Whether a herd message says anything about this pane at all, so "no detail" is read off an
/// entry that is present rather than off one that simply is not in the patch.
fn names(message: &Value, pane: &str) -> bool {
    pane_entries(message).any(|p| p["id"] == pane)
}

fn pane_detail(message: &Value, pane: &str) -> Option<String> {
    pane_entries(message)
        .find(|p| p["id"] == pane)
        .and_then(|p| p["detail"].as_str().map(str::to_string))
}

fn pane_entries(message: &Value) -> impl Iterator<Item = &Value> {
    ["panes", "added", "changed"]
        .into_iter()
        .flat_map(move |key| match message[key].as_array() {
            Some(panes) => panes.iter().collect::<Vec<_>>(),
            None => message[key]["panes"]
                .as_array()
                .map_or_else(Vec::new, |p| p.iter().collect()),
        })
}

/// A named session joins the herd as its own node, and it did so only on the next discovery poll
/// — up to fifteen seconds after the operator watched their own create succeed, with the herd
/// screen showing nothing in between (#240). The op is the one thing that *knows* the session set
/// changed, so it reconciles rather than leaving the loop to find out for itself.
///
/// Four seconds is not an arbitrary bound. `discover` polls on a `tokio::time::interval`, whose
/// first tick fires the moment the node starts, so its ticks land at node start + 0, +15, +30.
/// Both ops here land two to four seconds in, which is the middle of that window — without the
/// reconcile there is no tick anywhere near this deadline, and with it the herd moves in
/// milliseconds.
#[tokio::test(flavor = "multi_thread")]
async fn a_created_session_is_in_the_herd_by_the_time_its_ack_arrives() {
    let session = format!("kampr-herdjoin-{}", std::process::id());
    // The harness serves only its own session, for the reason its comment gives. This one has to
    // serve the session it is about to make as well — and nothing else, so a throwaway herd
    // belonging to another test running beside it is still not this test's herd (#97).
    let allowed = session.clone();
    let h = harness!("herdjoin", |c: &mut Config| {
        c.herdr.sessions = Some(vec![allowed.clone()]);
    });
    let node = h.node.node_id().to_string();
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;

    let joined = format!("{node}.{session}");
    let _created = CreatedSession::named(&session);
    let serving = async |want: bool| {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(4);
        loop {
            if h.node.herd().nodes.iter().any(|n| n.id == joined) == want {
                return true;
            }
            if tokio::time::Instant::now() >= deadline {
                return false;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    };
    assert!(
        serving(false).await,
        "the session is in the herd before it was made"
    );

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;
    assert!(
        serving(true).await,
        "the session was acknowledged and the herd waited for the poll to hear about it"
    );

    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
        20,
    )
    .await;
    assert!(
        serving(false).await,
        "the session was stopped and is still an online node"
    );
}

/// Probe #272: keypress-to-glyph, socket in to socket out, against a real herdr.
///
/// The hop this measures is everything the node owns — websocket read, `pane.send_text` over the
/// herdr socket, the PTY, herdr's own render and its `observe` frame, the `vte` emulator, the diff
/// and the wire encode — but not the browser's frame-drained input queue and not the LAN. It is
/// the number that did not exist: #22 measured herdr alone through `session control`, and #257
/// measured a frame's round trip under an attachment's load, but nothing measured a keystroke
/// going all the way through a node and coming back as a glyph.
///
/// One character at a time and no newline: a shell that runs a command answers on its own
/// schedule, and this is asking what the echo costs.
///
/// **It is `bash`'s echo, and on a machine with `ble.sh` that is nearly all of the number.** The
/// same keystrokes into a pane running `cat` come back in **1.2 ms** and into `bash --norc` in
/// 1.2 ms, against 26.5 ms into an interactive `bash` with the operator's rc (#273) — so what this
/// records is a ceiling on the whole path, not a reading of the node's share, and the node's share
/// is the part that is genuinely below the instrument.
#[tokio::test(flavor = "multi_thread")]
async fn a_keystroke_comes_back_as_a_glyph_and_the_round_trip_is_recorded() {
    let h = harness!("latency");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    let pane = h.pane_id();

    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until_pane(&mut socket, "grid.reset", &pane, 15).await;

    // A prompt that has not finished painting keeps sending frames of its own, and they would be
    // counted as the answer to the first keystroke.
    drain(&mut socket, Duration::from_millis(600)).await;

    let mut readings = Vec::new();
    for round in 0..48u32 {
        // Every keystroke is a character the screen does not already hold, so the frame that
        // carries it cannot be a repaint of something older.
        let ch = char::from(b'a' + (round % 26) as u8);
        let at = std::time::Instant::now();
        send(
            &mut socket,
            json!({ "t": "input", "pane": pane, "text": ch.to_string() }),
        )
        .await;

        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        let mut took = None;
        while tokio::time::Instant::now() < deadline && took.is_none() {
            let Some(message) = recv(&mut socket, Duration::from_secs(2)).await else {
                continue;
            };
            if message["pane"] != pane.as_str() {
                continue;
            }
            if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
                continue;
            }
            if message.to_string().contains(ch) {
                took = Some(at.elapsed());
            }
        }
        let took = took.unwrap_or_else(|| panic!("round {round}: {ch} never came back as a glyph"));
        readings.push(took.as_secs_f64() * 1000.0);

        // Take the line back so the next keystroke is landing on a clean prompt, and wait an
        // uneven amount before the next one. A fixed cadence puts every keystroke at the same
        // phase of whatever else in the path is periodic and the readings all land together: the
        // first run of this used a flat 120 ms and measured a p50 of 98 ms, against 31 ms with the
        // jitter (#272). That was read as a ~100 ms tick in herdr and it is not one — herdr's only
        // periodicity is a 16 ms floor between frames and it delivers a write in ~1 ms (#274) —
        // so the aliasing is in the pane's own shell. The jitter stays because the number moves
        // by a factor of three without it.
        send(
            &mut socket,
            json!({ "t": "input", "pane": pane, "keys": ["BSpace"] }),
        )
        .await;
        let jitter = 50 + (round as u64 * 37) % 190;
        drain(&mut socket, Duration::from_millis(jitter)).await;
    }

    readings.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let at = |q: f64| readings[((readings.len() as f64 - 1.0) * q).round() as usize];
    eprintln!(
        "keypress-to-glyph over {} readings: min {:.1} ms  p50 {:.1} ms  p90 {:.1} ms  max {:.1} ms",
        readings.len(),
        readings[0],
        at(0.50),
        at(0.90),
        readings[readings.len() - 1],
    );

    // Load-sensitive, so this is a ceiling that says "the path is not broken", not the measurement.
    // The measurement is the line above; #272 is where it is written down and #273 is what it
    // turned out to be a measurement of.
    assert!(
        at(0.50) < 250.0,
        "the median keystroke took {:.1} ms to come back as a glyph",
        at(0.50)
    );
}

async fn drain(socket: &mut Socket, quiet: Duration) {
    while recv(socket, quiet).await.is_some() {}
}

/// Every parameter in `agent.view.set` is a guess unless a real herdr accepts it, and this is the
/// one call in the naming path with **no read-back at all** — there is no `agent.view.get`, and
/// `agent.list` is untouched by the view (probe #296), so a schema that drifted would go on
/// answering an error nothing looks at. The reply is the whole of what herdr will say back:
/// `active`, the `source`, and the `label` that replaced the sort-mode word in its header.
#[tokio::test(flavor = "multi_thread")]
async fn herdr_accepts_the_agents_view_the_node_sets_and_gives_the_desk_back_on_clear() {
    let Some(session) = Session::start("agentview").await else {
        eprintln!("skipping: no herdr on PATH");
        return;
    };
    let view = kampr_core::agent_view::View::by_name();
    let set = session
        .herdr()
        .set_agent_view(&view.source, &view.token, view.order, &view.label)
        .await
        .unwrap_or_else(|e| panic!("herdr refused the view the node sets: {e}"));
    assert_eq!(
        set,
        kampr_herdr::AgentView {
            active: true,
            source: Some(view.source.clone()),
            label: Some(view.label.clone()),
        }
    );

    let cleared = session
        .herdr()
        .clear_agent_view()
        .await
        .expect("herdr clears the view");
    assert!(!cleared.active, "the desk has its own order back");
}

/// **A paste is bytes that become a path.** An agent reached over ssh reads a local file
/// perfectly well and chokes on a terminal's image-paste protocol, so nothing here speaks one:
/// the node writes what it is given beside its own state and types in where it put it.
///
/// The pane is a real herdr pane and the text really is typed, because the half that could
/// silently not happen is the typing — a file written into a directory nobody looks at is exactly
/// the shape of a feature that reports success and does nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_paste_lands_on_the_node_and_its_path_is_typed_into_the_pane() {
    let h = harness!("paste");
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    until(&mut socket, "hello", 10).await;
    until(&mut socket, "herd", 10).await;
    let pane = h.pane_id();
    send(&mut socket, json!({ "t": "watch", "pane": pane })).await;
    until(&mut socket, "grid.reset", 15).await;

    // A PNG's magic bytes, so the extension is decided by the body and can be seen to have been.
    let body = b"\x89PNG\r\n\x1a\n\x00kampr-paste-body";
    send(
        &mut socket,
        json!({
            "t": "paste",
            "pane": pane,
            "b64": base64::engine::general_purpose::STANDARD.encode(body),
            "name": "shot",
        }),
    )
    .await;

    let deadline = tokio::time::Instant::now() + Duration::from_secs(20);
    let mut typed: Option<String> = None;
    while tokio::time::Instant::now() < deadline && typed.is_none() {
        let Some(message) = recv(&mut socket, Duration::from_secs(3)).await else {
            continue;
        };
        if !matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset")) {
            continue;
        }
        let painted = message.to_string();
        if let Some(at) = painted.find("shot-") {
            typed = Some(painted[at..].chars().take(64).collect());
        }
    }

    let typed = typed.expect("the path was never typed into the pane");
    assert!(
        typed.contains(".png"),
        "the extension came from somewhere other than the bytes: {typed}"
    );

    let dir = h.node.state_dir.join("pastes");
    let written: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
        .unwrap_or_else(|e| panic!("no paste directory at {dir:?}: {e}"))
        .flatten()
        .map(|e| e.path())
        .collect();
    assert_eq!(written.len(), 1, "expected one pasted file in {dir:?}");
    assert_eq!(
        std::fs::read(&written[0]).expect("the pasted file"),
        body,
        "the bytes on disk are not the bytes that were sent"
    );
}

/// **A conversation must not be torn down because the agent went from busy to idle.**
///
/// The pump decides a conversation has moved by comparing what identifies the pane's session, and
/// answers a difference by taking the open transcript off the client and paging a fresh one. So
/// anything that merely *describes* a session and changes while it runs — the harness's own
/// `status`, flipping every turn — must stay out of that comparison, or every turn ends by
/// replacing the reader's conversation with its newest page. That is #314's defect wearing a
/// different hat, and it was briefly reintroduced by carrying the session marker in the wrong
/// struct.
///
/// **What this does and does not prove.** The pane here runs a shell, so the marker resolves to
/// nothing and a status field could not move even if one were back in [`Identity`] — this pins the
/// invariant and would catch a field that churns on *any* pane, but only a real agent pane could
/// catch one that churns solely on an agent's. The comment on `Identity` is what carries the rest.
#[tokio::test(flavor = "multi_thread")]
async fn what_the_pump_compares_holds_still_while_the_agent_works() {
    let h = harness!("identity");
    let pane = h.pane_id();
    let (_, local) = h.node.resolve(&pane).expect("a local pane");
    let (session, _) = h.node.resolve(&pane).expect("a local pane");
    let journals = h.node.journals();

    let first = kampr_node::convo::identity(&journals, &session.provider, &local);
    for _ in 0..8 {
        tokio::time::sleep(Duration::from_millis(60)).await;
        assert_eq!(
            kampr_node::convo::identity(&journals, &session.provider, &local),
            first,
            "what the pump compares moved on its own, which re-pages the conversation"
        );
    }
}
