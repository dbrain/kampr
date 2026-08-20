//! Two nodes, two herdr sessions, one hub — on one machine.
//!
//! This is the mesh gate run honestly at the only scale a single host allows: two real nodes, each
//! against its own throwaway herdr session, one of them dialling the other. It exercises the
//! handshake, enrolment, the relay, input, the herd merge and recovery. The one thing it cannot
//! exercise is real network latency, because both ends are on loopback.
//!
//! Every session here is created and destroyed by the test. `default` is never touched.

use futures_util::{SinkExt, StreamExt};
use kampr_auth::{MeshRole, NodeIdentity, Role};
use kampr_mesh::dial::Hub;
use kampr_mesh::{Presence, mesh_url};
use kampr_node::{BUILD, Config, Node, http};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

struct Session {
    socket: PathBuf,
}

impl Session {
    async fn start(tag: &str) -> Option<Self> {
        which("herdr")?;
        let name = format!("kampr-mesh-{tag}-{}", std::process::id());
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
                let session = Self {
                    socket: socket.clone(),
                };
                session
                    .call("workspace.create", json!({ "label": tag, "cwd": "/tmp" }))
                    .await;
                return Some(session);
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        None
    }

    async fn call(&self, method: &str, params: Value) -> Value {
        kampr_herdr::Herdr::new(&self.socket)
            .call::<Value>(method, params)
            .await
            .unwrap_or_else(|e| panic!("{method}: {e}"))
    }
}

impl Drop for Session {
    fn drop(&mut self) {
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

/// One node's directories, kept across a restart so its identity and its enrolments survive one.
struct Home {
    dir: tempfile::TempDir,
}

impl Home {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("a home"),
        }
    }

    fn config(&self) -> PathBuf {
        self.dir.path().join("config")
    }

    fn state(&self) -> PathBuf {
        self.dir.path().join("state")
    }

    fn identity(&self) -> NodeIdentity {
        std::fs::create_dir_all(self.config()).expect("config dir");
        NodeIdentity::load_or_create(&Config::node_key_path(&self.config())).expect("an identity")
    }

    /// The node id this home will always start with. A restart that changed its id would be a
    /// different node to the hub, which is not what "it came back" means.
    fn settle(&self, name: &str) -> String {
        let config = self.config_for(name);
        config.save(&self.config()).expect("a config");
        std::fs::create_dir_all(self.state()).expect("state dir");
        config.node_id
    }

    fn config_for(&self, name: &str) -> Config {
        let mut config = Config::load(&self.config()).unwrap_or_else(|_| Config::bootstrap(name));
        config.config_dir = self.config().display().to_string();
        config.state_dir = self.state().display().to_string();
        config
    }
}

struct Running {
    node: Arc<Node>,
    origin: String,
    server: tokio::task::JoinHandle<()>,
}

impl Running {
    async fn start(home: &Home, session: &Session, name: &str) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let mut config = home.config_for(name);
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.config_dir = home.config().display().to_string();
        config.state_dir = home.state().display().to_string();
        config.herdr.socket = session.socket.display().to_string();
        config.limits.client_queue = 64;
        config.save(&home.config()).expect("a config");

        let origin = config.origin();
        let node = Node::start(config, &home.state()).await.expect("a node");
        let server = tokio::spawn({
            let app = http::router(node.clone());
            async move {
                let _ = http::serve_on(listener, app).await;
            }
        });
        for _ in 0..60 {
            if !node.herd().panes.is_empty() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Self {
            node,
            origin,
            server,
        }
    }

    /// Everything a `kill -9` would do to this process, minus the process.
    fn stop(self) {
        self.node.shutdown();
        self.server.abort();
        drop(self.node);
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
        let body = json!({ "code": pairing.code, "device_name": "mesh-test" });
        post(&format!("{}/auth/pair", self.origin), &body).await["token"]
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
            .expect("a websocket")
            .0
    }

    fn local_pane(&self) -> String {
        self.node
            .herd()
            .panes
            .iter()
            .find(|p| p.node_id == self.node.node_id())
            .expect("a local pane")
            .id
            .clone()
    }
}

async fn post(url: &str, body: &Value) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.trim_start_matches("http://");
    let (authority, path) = rest.split_once('/').expect("a path");
    let (host, port) = authority.split_once(':').expect("a port");
    let payload = body.to_string();
    let request = format!(
        "POST /{path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = TcpStream::connect((host, port.parse::<u16>().unwrap()))
        .await
        .expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or_default();
    serde_json::from_str(body.trim()).unwrap_or(Value::Null)
}

async fn send(socket: &mut Socket, message: Value) {
    socket
        .send(tungstenite::Message::text(message.to_string()))
        .await
        .expect("send");
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

/// Waits for a herd message — full or patch — that satisfies `want`, applied to the hub's own
/// model rather than to the wire, so the test is not fooled by a patch arriving in two parts.
async fn herd_becomes(node: &Arc<Node>, seconds: u64, want: impl Fn(&kampr_node::herd::HerdModel) -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut herd = node.subscribe_herd();
    while tokio::time::Instant::now() < deadline {
        if want(&node.herd()) {
            return;
        }
        let _ = tokio::time::timeout(Duration::from_millis(500), herd.changed()).await;
    }
    let model = node.herd();
    panic!(
        "the herd never got there; nodes {:?}",
        model
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.kind.clone(), n.online))
            .collect::<Vec<_>>()
    );
}

/// The join an operator runs on the peer: dial once with the code, confirm the fingerprint, and
/// write the row `kampr serve` dials from afterwards.
async fn join(peer_home: &Home, peer_name: &str, node_id: &str, hub_origin: &str, code: &str) -> String {
    let identity = peer_home.identity();
    let store = kampr_auth::Store::open(&Config::state_db(&peer_home.state()))
        .await
        .expect("the peer store");
    let hub = Hub {
        url: hub_origin.to_string(),
        name: "hub".into(),
        key: None,
        join: Some(code.to_string()),
    };
    let presence = Presence {
        node_id: node_id.to_string(),
        node_name: peer_name.to_string(),
        build: BUILD.to_string(),
    };
    let (hub_identity, _out, _incoming) =
        kampr_mesh::dial(&hub, &identity, &presence, Duration::from_secs(10))
            .await
            .expect("the join dial");
    store
        .mesh()
        .enrol(
            &hub_identity.key,
            &hub_identity.node_id,
            "hub",
            MeshRole::Hub,
            Some(&mesh_url(hub_origin)),
            kampr_auth::now(),
        )
        .await
        .expect("the hub row");
    hub_identity.key
}

macro_rules! sessions {
    ($hub:ident, $peer:ident) => {
        let (Some($hub), Some($peer)) = (Session::start("hub").await, Session::start("peer").await) else {
            eprintln!("skipping: herdr is not on PATH");
            return;
        };
    };
}

#[tokio::test(flavor = "multi_thread")]
async fn a_peers_panes_are_driven_through_the_hub_and_survive_it_dying() {
    sessions!(hub_session, peer_session);
    let hub_home = Home::new();
    let peer_home = Home::new();

    let hub = Running::start(&hub_home, &hub_session, "front").await;
    let hub_pane = hub.local_pane();

    // Enrolment: a single-use join code, minted on the hub and spent by the peer.
    let now = kampr_auth::now();
    let code = hub
        .node
        .auth
        .store()
        .mesh()
        .invite(now, now + 600)
        .await
        .expect("a join code");

    // The peer's identity and node id have to exist before it joins, because both are what the
    // hub enrols.
    let peer_node_id = peer_home.settle("laptop");
    let hub_key = join(&peer_home, "laptop", &peer_node_id, &hub.origin, &code).await;
    assert_eq!(
        hub_key,
        hub.node.identity().expect("hub identity").public_hex(),
        "the peer pinned the key the hub actually holds"
    );

    let mut peer = Running::start(&peer_home, &peer_session, "laptop").await;
    assert_eq!(peer.node.node_id(), peer_node_id, "the node it joined as is the node it runs as");
    let peer_pane = peer.local_pane();

    // One herd, two hosts.
    herd_becomes(&hub.node, 30, |herd| {
        herd.nodes.iter().any(|n| n.id == peer_node_id && n.kind == "peer" && n.online)
            && herd.panes.iter().any(|p| p.id == peer_pane)
    })
    .await;
    let merged = hub.node.herd();
    let peer_entry = merged.nodes.iter().find(|n| n.id == peer_node_id).unwrap();
    assert!(peer_entry.rtt_ms.is_some(), "a peer reports a measured round trip");
    assert_eq!(
        peer_entry.build.as_deref(),
        Some(BUILD),
        "each node names its own build, which is the whole of version skew"
    );
    assert!(
        merged.nodes.iter().any(|n| n.id == hub.node.node_id() && n.kind == "local"),
        "the hub's own sessions are still local"
    );

    let mut client = hub.connect().await;
    let hello = until(&mut client, "hello", 10).await;
    assert_eq!(hello["caps"]["mesh"], true);
    until(&mut client, "herd", 10).await;

    // A pane on the peer renders through the hub.
    send(&mut client, json!({ "t": "watch", "pane": peer_pane, "scrollback": true })).await;
    let reset = until(&mut client, "grid.reset", 20).await;
    assert_eq!(reset["pane"], peer_pane.as_str());
    assert!(reset["cols"].as_u64().unwrap_or_default() > 0);

    // And input reaches it: this marker crosses client → hub → peer → herdr → PTY and all the way
    // back as a frame.
    let marker = format!("kampr-mesh-{}", std::process::id());
    send(
        &mut client,
        json!({ "t": "input", "pane": peer_pane, "text": format!("echo {marker}\n") }),
    )
    .await;
    assert!(
        saw(&mut client, &marker, 25).await,
        "the peer's pane never echoed {marker} back through the hub"
    );

    // The hub's own pane is a different node in the same herd, driven over the same socket.
    send(&mut client, json!({ "t": "watch", "pane": hub_pane, "scrollback": true })).await;
    let local_reset = until(&mut client, "grid.reset", 20).await;
    assert_eq!(local_reset["pane"], hub_pane.as_str());

    // The peer dies.
    peer.stop();
    herd_becomes(&hub.node, 30, |herd| {
        herd.nodes.iter().any(|n| n.id == peer_node_id && !n.online)
    })
    .await;
    let offline = hub.node.herd();
    let entry = offline.nodes.iter().find(|n| n.id == peer_node_id).unwrap();
    assert!(entry.detail.is_some(), "an offline node says why");
    assert!(
        offline.panes.iter().any(|p| p.id == peer_pane),
        "its panes stay listed rather than vanishing out from under the client"
    );

    // Its panes are refused with an honest reason…
    send(&mut client, json!({ "t": "watch", "pane": peer_pane })).await;
    let error = until(&mut client, "error", 15).await;
    assert_eq!(error["code"], "node_offline");
    send(
        &mut client,
        json!({ "t": "input", "pane": peer_pane, "text": "x" }),
    )
    .await;
    assert_eq!(until(&mut client, "error", 15).await["code"], "node_offline");

    // …and the rest of the herd is untouched.
    let marker = format!("{marker}-local");
    send(
        &mut client,
        json!({ "t": "input", "pane": hub_pane, "text": format!("echo {marker}\n") }),
    )
    .await;
    assert!(
        saw(&mut client, &marker, 25).await,
        "a peer dying took the hub's own pane with it"
    );

    // It comes back unaided: nothing here re-enrols, re-pairs or re-types anything.
    peer = Running::start(&peer_home, &peer_session, "laptop").await;
    herd_becomes(&hub.node, 60, |herd| {
        herd.nodes.iter().any(|n| n.id == peer_node_id && n.online)
    })
    .await;
    send(&mut client, json!({ "t": "watch", "pane": peer_pane, "scrollback": true })).await;
    let reset = until(&mut client, "grid.reset", 25).await;
    assert_eq!(reset["pane"], peer_pane.as_str());

    peer.stop();
    hub.stop();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_node_the_hub_never_enrolled_is_refused() {
    let Some(hub_session) = Session::start("closed").await else {
        eprintln!("skipping: herdr is not on PATH");
        return;
    };
    let hub_home = Home::new();
    let hub = Running::start(&hub_home, &hub_session, "front").await;

    let stranger_home = Home::new();
    let identity = stranger_home.identity();
    let presence = Presence {
        node_id: "01JSTRANGER".into(),
        node_name: "stranger".into(),
        build: BUILD.to_string(),
    };

    let refused = kampr_mesh::dial(
        &Hub {
            url: hub.origin.clone(),
            name: "hub".into(),
            key: None,
            join: None,
        },
        &identity,
        &presence,
        Duration::from_secs(10),
    )
    .await
    .err()
    .expect("an unenrolled node must not be served");
    let text = refused.to_string();
    assert!(text.contains("not enrolled"), "{text}");

    // A wrong code enrols nobody either.
    let wrong = kampr_mesh::dial(
        &Hub {
            url: hub.origin.clone(),
            name: "hub".into(),
            key: None,
            join: Some("ZZZZ-ZZZZ".into()),
        },
        &identity,
        &presence,
        Duration::from_secs(10),
    )
    .await;
    assert!(wrong.is_err(), "a guessed join code let a stranger in");
    assert!(
        hub.node
            .auth
            .store()
            .mesh()
            .nodes(MeshRole::Peer)
            .await
            .expect("the peer list")
            .is_empty(),
        "nothing was enrolled"
    );
    assert!(
        hub.node.herd().nodes.iter().all(|n| n.kind == "local"),
        "and no node joined the herd"
    );

    hub.stop();
}

/// Drains until a relayed grid frame carries `marker`.
async fn saw(socket: &mut Socket, marker: &str, seconds: u64) -> bool {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(3)).await else {
            continue;
        };
        if matches!(message["t"].as_str(), Some("grid.patch" | "grid.reset"))
            && message.to_string().contains(marker)
        {
            return true;
        }
    }
    false
}
