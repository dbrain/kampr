//! End-to-end against a real Herdr.
//!
//! Every test here runs in a throwaway named session created and destroyed by the test itself.
//! `default` is never touched. When `herdr` is not on PATH the suite reports a skip rather than a
//! failure, so it stays honest on a machine that has no herd.

use futures_util::{SinkExt, StreamExt};
use kampr_auth::Role;
use kampr_node::{Config, Node, http};
use serde_json::{Value, json};
use std::path::PathBuf;
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
        // Never leave a herdr behind. `server.stop` first, then a hard kill of anything holding
        // the socket, then the session directory.
        let socket = self.socket.clone();
        std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("teardown runtime");
            runtime.block_on(async {
                let _ = kampr_herdr::Herdr::new(&socket)
                    .call::<Value>("server.stop", json!({}))
                    .await;
            });
        })
        .join()
        .ok();
        // A herdr that has been asked to stop still owns its session directory for a moment, and
        // one removal races it — which is what leaves a throwaway session listed forever. Wait
        // for the socket to go, then keep removing until the directory stays gone.
        for _ in 0..50 {
            if !self.socket.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        if let Some(dir) = self.socket.parent() {
            for _ in 0..25 {
                let _ = std::fs::remove_dir_all(dir);
                std::thread::sleep(Duration::from_millis(200));
                if !dir.exists() {
                    break;
                }
            }
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

async fn post_with_cookie(url: &str, token: &str, origin: Option<&str>) -> String {
    let (host, port, path) = split(url);
    let origin = origin.map_or(String::new(), |o| format!("Origin: {o}\r\n"));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nCookie: kampr_session={token}\r\n\
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
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == tag {
            return message;
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
    assert!(herd["panes"][0]["cols"].as_u64().unwrap() > 0);

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

/// A cookie is the one credential a browser attaches on its own, so a state-changing request
/// carrying one and no `Origin` is the shape CSRF has — the absence must fail closed there. The
/// same-origin gate is the only CSRF defence on this surface, so it does not get to fail open.
#[tokio::test(flavor = "multi_thread")]
async fn a_cookie_credential_without_an_origin_cannot_change_anything() {
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

    let url = format!("{}/api/devices/{victim_id}/revoke", h.origin);
    assert!(
        post_with_cookie(&url, &token, None).await.contains("403"),
        "an ambient credential with no origin must not revoke anything"
    );
    assert!(
        post_with_cookie(&url, &token, Some(&h.origin))
            .await
            .contains("200")
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
            Some("error") => match message["code"].as_str() {
                Some("herdr_unavailable") => saw_unavailable = true,
                Some("node_offline") => saw_node_offline = true,
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
fn claude_transcript(cwd: &str, filler: usize) -> (String, String) {
    let mut lines = Vec::new();
    for n in 0..filler {
        lines.push(json!({
            "type": "user", "uuid": format!("u{n}"), "cwd": cwd,
            "timestamp": format!("2026-08-20T10:{:02}:00Z", n),
            "message": { "content": format!("filler {n}") }
        }));
    }
    lines.push(json!({
        "type": "assistant", "uuid": "a-md", "cwd": cwd,
        "timestamp": "2026-08-20T13:41:55Z",
        "message": { "content": [
            { "type": "text",
              "text": "Six, and they are…\n\n| Key | Accepted |\n|---|---|\n| `Up` | yes |\n" }
        ] }
    }));
    lines.push(json!({
        "type": "assistant", "uuid": "a-tool", "cwd": cwd,
        "timestamp": "2026-08-20T13:42:01Z",
        "message": { "content": [
            { "type": "tool_use", "id": "tu1", "name": "Bash",
              "input": { "command": "herdr pane list --json", "description": "probe key grammar" } }
        ] }
    }));
    let settle = json!({
        "type": "user", "uuid": "u-result", "cwd": cwd,
        "timestamp": "2026-08-20T13:42:48Z",
        "message": { "content": [
            { "type": "tool_result", "tool_use_id": "tu1", "content": "one\ntwo\nthree\n" }
        ] }
    });
    let body: String = lines.iter().map(|l| format!("{l}\n")).collect();
    (body, format!("{settle}\n"))
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
    let (body, settle) = claude_transcript(cwd, 45);
    std::fs::write(&transcript, &body).unwrap();

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

    // Herdr's own agent report is what makes this an agent pane, exactly as detection would.
    h._session
        .call(
            "pane.report_agent",
            json!({ "pane_id": local, "agent": "claude", "source": "kampr-test", "state": "idle" }),
        )
        .await;

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
    assert_eq!(markdown["at"], "2026-08-20T13:41:55Z");
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
    // A pane herdr has only just created is not yet "an available shell", so this is the one op
    // that has to wait for the thing it acts on rather than for its own answer.
    let mut started = json!(null);
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
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.create", "node": node, "name": &session }),
        20,
    )
    .await;
    let session_dir = herdr_home().join("sessions").join(&session);
    for _ in 0..100 {
        if session_dir.join("herdr.sock").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert!(
        session_dir.join("herdr.sock").exists(),
        "no socket at {session_dir:?}"
    );
    ok(
        &mut socket,
        json!({ "t": "manage", "op": "session.stop", "node": node, "name": &session }),
        20,
    )
    .await;
    // Never leave a herdr session behind: wait for the socket to go, then keep removing until the
    // directory stays gone — a shutting-down herdr writes into it after the socket has vanished.
    for _ in 0..100 {
        if !session_dir.join("herdr.sock").exists() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    for _ in 0..20 {
        let _ = std::fs::remove_dir_all(&session_dir);
        tokio::time::sleep(Duration::from_millis(200)).await;
        if !session_dir.exists() {
            break;
        }
    }
    assert!(!session_dir.exists(), "left {session_dir:?} behind");

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
    ] {
        send(&mut socket, op).await;
        assert_eq!(until(&mut socket, "error", 10).await["code"], "not_writer");
    }
}
