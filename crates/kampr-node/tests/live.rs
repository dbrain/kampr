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
        std::thread::sleep(Duration::from_millis(300));
        if let Some(dir) = self.socket.parent() {
            let _ = std::fs::remove_dir_all(dir);
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
    async fn start(tag: &str) -> Option<Self> {
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
        config.auth.audit = true;
        config.limits.client_queue = 32;
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
            if !node.herd().panes.is_empty() {
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
        let code = self.node.auth.create_pairing(role).await.unwrap();
        let body = json!({ "code": code, "device_name": "integration" });
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

    fn pane_id(&self) -> String {
        self.node.herd().panes.first().expect("a pane").id.clone()
    }
}

async fn post(url: &str, body: &Value) -> Value {
    post_as(url, body, None).await.1
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

async fn send(socket: &mut Socket, value: Value) {
    socket
        .send(tungstenite::Message::text(value.to_string()))
        .await
        .expect("send");
}

macro_rules! harness {
    ($tag:expr) => {
        match Harness::start($tag).await {
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
    assert!(!patch["added"].as_array().unwrap().is_empty(), "{patch}");
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
    assert!(body["security"]["unlocks"].as_str().unwrap().contains("hostname"));

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

#[tokio::test(flavor = "multi_thread")]
async fn a_revoked_device_loses_its_socket_and_a_wrong_code_never_gets_one() {
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

    let pending = until(&mut socket, "pending", 25).await;
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
    let cleared = until(&mut socket, "pending", 25).await;
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
