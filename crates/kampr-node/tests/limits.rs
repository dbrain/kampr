//! What a stranger can make this node do before it knows who they are, and how much of it.
//!
//! Nothing here needs a herd: every test drives the HTTP and WebSocket surface directly, against
//! a node whose herdr socket deliberately does not exist. The operator's own session is never
//! reachable from here.

use futures_util::{SinkExt, StreamExt};
use kampr_auth::{NodeIdentity, Role};
use kampr_mesh::Presence;
use kampr_mesh::dial::Hub;
use kampr_node::{BUILD, Config, Node, http};
use serde_json::{Value, json};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

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
        let home = tempfile::tempdir().expect("a home");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let config_dir = home.path().join("config");
        let state_dir = home.path().join("state");
        std::fs::create_dir_all(&state_dir).expect("a state dir");

        let mut config = Config::bootstrap("limits");
        // Nothing in this suite reaches the internet: the release check is the one thing in a
        // node that would, and a test that phoned GitHub would be one with a rate limit.
        config.update.check = false;
        config.config_dir = config_dir.display().to_string();
        config.state_dir = state_dir.display().to_string();
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        // A socket that does not exist and a binary that does not exist: this node has no herd,
        // and must not find the operator's.
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

    fn keys(&self) -> &Path {
        self.home.path()
    }

    async fn token(&self, role: Role) -> String {
        let pairing = self
            .node
            .auth
            .create_pairing(role, kampr_auth::Delivery::Console)
            .await
            .expect("a pairing");
        if !pairing.armed {
            assert!(self.node.auth.arm_pairing(&pairing.code).await.expect("armed"));
        }
        let body = json!({ "code": pairing.code, "device_name": "limits" });
        let (_, body) = request(&self.origin, "POST", "/auth/pair", &[], Some(&body.to_string())).await;
        body["token"].as_str().expect("a token").to_string()
    }

    async fn try_connect(&self, token: &str) -> Result<Socket, String> {
        let url = self.origin.replacen("http", "ws", 1) + "/ws";
        let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("kampr.token.{token}").parse().unwrap(),
        );
        tokio_tungstenite::connect_async(request)
            .await
            .map(|(socket, _)| socket)
            .map_err(|e| e.to_string())
    }

    async fn connect(&self, token: &str) -> Socket {
        self.try_connect(token).await.expect("a websocket")
    }

    async fn open_mesh(&self) -> Result<Socket, String> {
        let url = self.origin.replacen("http", "ws", 1) + "/mesh";
        tokio_tungstenite::connect_async(url)
            .await
            .map(|(socket, _)| socket)
            .map_err(|e| e.to_string())
    }

    /// A node dialling in with its own freshly generated key — which is all an anonymous caller
    /// needs to reach the join-code check.
    async fn dial(&self, tag: &str, code: &str) -> Result<(), String> {
        let key =
            NodeIdentity::load_or_create(&self.keys().join(format!("{tag}.key"))).expect("a stranger's key");
        let hub = Hub {
            url: self.origin.clone(),
            name: "hub".into(),
            key: None,
            join: Some(code.to_string()),
        };
        let me = Presence {
            node_id: format!("01J{tag}"),
            node_name: tag.to_string(),
            build: BUILD.to_string(),
        };
        kampr_mesh::dial(&hub, &key, &me, Duration::from_secs(10))
            .await
            .map(|_| ())
            .map_err(|e| e.to_string())
    }
}

async fn request(
    origin: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> (String, Value) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let (host, port) = split(origin);
    let extra: String = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let payload = body.unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
         {extra}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = TcpStream::connect((host.as_str(), port)).await.expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text.lines().next().unwrap_or_default().to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or_default();
    (status, serde_json::from_str(body.trim()).unwrap_or(Value::Null))
}

fn split(origin: &str) -> (String, u16) {
    let rest = origin.trim_start_matches("http://");
    let (host, port) = rest.split_once(':').unwrap_or((rest, "80"));
    (host.to_string(), port.parse().unwrap())
}

/// Drains until the peer closes. `true` means the node hung up inside the window.
async fn closed(socket: &mut Socket, seconds: u64) -> bool {
    tokio::time::timeout(Duration::from_secs(seconds), async {
        while let Some(message) = socket.next().await {
            match message {
                Ok(tungstenite::Message::Close(_)) | Err(_) => return,
                Ok(_) => {}
            }
        }
    })
    .await
    .is_ok()
}

/// The audit's finding #1, run the way the audit ran it: twenty anonymous handshakes with a wrong
/// code, from one address, against a node holding one outstanding invite. Each miss charges every
/// outstanding code an attempt and ten of them kill it, so with nothing throttling the misses a
/// stranger decides whether the operator's join works.
#[tokio::test(flavor = "multi_thread")]
async fn a_stranger_cannot_burn_the_operators_join_code() {
    let h = Harness::start(|c| c.mesh.accept = true).await;
    let now = kampr_auth::now();
    let code = h
        .node
        .auth
        .store()
        .mesh()
        .invite(now, now + 600)
        .await
        .expect("an invite");

    let mut throttled = 0;
    for n in 0..20 {
        let refusal = h
            .dial(&format!("bogus{n}"), "ZZZZ-ZZZZ")
            .await
            .expect_err("a wrong code enrols nobody");
        if refusal.contains("429") {
            throttled += 1;
        }
    }
    assert!(
        throttled > 0,
        "twenty anonymous argon2id passes and the node throttled none of them"
    );

    // The limiter refuses this address for a while too, which is the price of keying on one — but
    // when it lets the operator through the code has to still be there to spend.
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        match h.dial("operator", &code).await {
            Ok(()) => break,
            Err(e) if e.contains("429") && tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(500)).await;
            }
            Err(e) => panic!("a stranger burned the join code the operator was holding: {e}"),
        }
    }
}

/// `mesh.accept` is the switch that says the door is not there, and a node nobody asked to be a
/// hub should not be answering a second unauthenticated protocol on the public internet.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_is_not_a_hub_until_the_operator_says_so() {
    let h = Harness::start(|_| {}).await;
    assert!(
        !h.node.config.mesh.accept,
        "a node becomes a hub when the operator asks, not by default"
    );
    let refusal = h.dial("knocker", "ZZZZ-ZZZZ").await.expect_err("no door");
    assert!(refusal.contains("404"), "{refusal}");

    // And the operator gets told what to turn on, rather than a code nothing can ever spend.
    let token = h.token(Role::Full).await;
    let (status, body) = request(
        &h.origin,
        "POST",
        "/api/mesh/invite",
        &[
            ("Authorization", &format!("Bearer {token}")),
            ("Origin", &h.origin),
        ],
        Some("{}"),
    )
    .await;
    assert!(status.contains("409"), "{status} {body}");
    assert!(
        body["error"].as_str().unwrap_or_default().contains("accept"),
        "the refusal has to name the switch: {body}"
    );
}

/// Argon2id is 19 MiB a pass, so what bounds the memory is how many handshakes may be in flight
/// at once — a bound a source-address-rotating caller cannot rotate around.
#[tokio::test(flavor = "multi_thread")]
async fn only_so_many_mesh_handshakes_run_at_once() {
    let h = Harness::start(|c| {
        c.mesh.accept = true;
        c.limits.mesh_handshakes = 2;
    })
    .await;
    let _first = h.open_mesh().await.expect("the first handshake");
    let _second = h.open_mesh().await.expect("the second handshake");
    let refusal = h
        .open_mesh()
        .await
        .expect_err("a third handshake must wait outside");
    assert!(refusal.contains("503"), "{refusal}");
}

/// tungstenite will hand a 64 MiB message to whatever is reading, and `dispatch` parses every one
/// into a `serde_json::Value`. The largest legitimate client message is a few kilobytes.
#[tokio::test(flavor = "multi_thread")]
async fn a_message_larger_than_the_protocol_needs_closes_the_socket() {
    let h = Harness::start(|_| {}).await;
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;
    let giant = json!({ "t": "prefs", "pane": "x", "pad": "a".repeat(2 * 1024 * 1024) });
    // The node may reset the connection before the whole 2 MiB has been written, which refuses the
    // message just as surely as reading it and closing does. Under load it usually does.
    let refused_mid_write = socket
        .send(tungstenite::Message::text(giant.to_string()))
        .await
        .is_err();
    assert!(
        refused_mid_write || closed(&mut socket, 10).await,
        "a 2 MiB message reached the JSON parser instead of being refused"
    );
}

/// One token opens as many sockets as the holder likes, and every one of them is a session with
/// its own queue. A publicly reachable node needs a bound of its own.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_serves_a_bounded_number_of_sockets() {
    let h = Harness::start(|c| c.limits.sockets = 2).await;
    let token = h.token(Role::Full).await;
    let _first = h.connect(&token).await;
    let _second = h.connect(&token).await;
    let refusal = h
        .try_connect(&token)
        .await
        .expect_err("a third socket must be refused");
    assert!(refusal.contains("503"), "{refusal}");
}
