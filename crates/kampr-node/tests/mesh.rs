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
use kampr_mesh::{Incoming, Outgoing, Presence, mesh_url};
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
        // Nothing in this suite reaches the internet: the release check is the one thing in a
        // node that would, and a test that phoned GitHub would be one with a rate limit.
        config.update.check = false;
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
        Self::spawn(home, name, session.socket.clone(), true).await
    }

    /// A hub with no herdr behind it at all. Nothing in a node waits on herdr — it binds its port
    /// and serves `/mesh` either way — and the properties below are about links rather than about
    /// panes, so they run on a machine with no herdr installed and leave no session to tear down.
    /// The socket named here is inside the test's own home, so the one node this finds is none.
    async fn hub(home: &Home, name: &str) -> Self {
        Self::spawn(home, name, home.dir.path().join("no-herdr.sock"), false).await
    }

    async fn spawn(home: &Home, name: &str, socket: PathBuf, panes: bool) -> Self {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let mut config = home.config_for(name);
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.config_dir = home.config().display().to_string();
        config.state_dir = home.state().display().to_string();
        config.herdr.socket = socket.display().to_string();
        // One machine is hosting both nodes, so left to itself each would discover the other's
        // herdr session and serve it locally — correct behaviour, and useless here. An empty list
        // is "only the configured session", which is what two real hosts look like.
        config.herdr.sessions = Some(Vec::new());
        // Every node here is either a hub or about to be dialled by one. The door is shut by
        // default, so a mesh test has to open it as an operator would.
        config.mesh.accept = true;
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
        if panes {
            for _ in 0..60 {
                if !node.herd().panes.is_empty() {
                    break;
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
        Self { node, origin, server }
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
    post_as(url, body, None).await
}

async fn post_as(url: &str, body: &Value, token: Option<&str>) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.trim_start_matches("http://");
    let (authority, path) = rest.split_once('/').expect("a path");
    let (host, port) = authority.split_once(':').expect("a port");
    let payload = body.to_string();
    let bearer = token.map_or_else(String::new, |t| format!("Authorization: Bearer {t}\r\n"));
    let request = format!(
        "POST /{path} HTTP/1.1\r\nHost: {authority}\r\nContent-Type: application/json\r\n\
         {bearer}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
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
        "the herd never got there;\n  nodes {:?}\n  panes {:?}\n  links {:?}",
        model
            .nodes
            .iter()
            .map(|n| (n.id.clone(), n.kind.clone(), n.online))
            .collect::<Vec<_>>(),
        model.panes.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
        node.peers
            .links()
            .iter()
            .map(|l| (l.node_id.clone(), l.rtt_ms()))
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
    assert_eq!(
        peer.node.node_id(),
        peer_node_id,
        "the node it joined as is the node it runs as"
    );

    // The link measures its own round trip over the socket the frames use. This is the number a
    // client renders to say how far away a node is, and on loopback it is a fraction of a
    // millisecond — which is exactly what one machine cannot make interesting.
    let measured = tokio::time::timeout(Duration::from_secs(20), async {
        loop {
            if let Some(rtt) = hub.node.peers.links().first().and_then(|link| link.rtt_ms()) {
                return rtt;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the mesh link never measured a round trip");
    assert!(measured >= 0.0);

    // One herd, two hosts.
    let owned_by_peer = |id: &str| id == peer_node_id || id.starts_with(&format!("{peer_node_id}."));
    herd_becomes(&hub.node, 45, |herd| {
        herd.nodes
            .iter()
            .any(|n| n.id == peer_node_id && n.kind == "peer" && n.online && n.rtt_ms.is_some())
            && herd.panes.iter().any(|p| owned_by_peer(&p.node_id))
    })
    .await;
    let peer_pane = hub
        .node
        .herd()
        .panes
        .iter()
        .find(|p| owned_by_peer(&p.node_id))
        .expect("a pane on the peer, listed by the hub")
        .id
        .clone();
    let merged = hub.node.herd();
    let peer_entry = merged.nodes.iter().find(|n| n.id == peer_node_id).unwrap();
    assert_eq!(
        peer_entry.build.as_deref(),
        Some(BUILD),
        "each node names its own build, which is the whole of version skew"
    );
    assert!(
        merged
            .nodes
            .iter()
            .any(|n| n.id == hub.node.node_id() && n.kind == "local"),
        "the hub's own sessions are still local"
    );

    let mut client = hub.connect().await;
    let hello = until(&mut client, "hello", 10).await;
    assert_eq!(hello["caps"]["mesh"], true);
    until(&mut client, "herd", 10).await;

    // A pane on the peer renders through the hub.
    send(
        &mut client,
        json!({ "t": "watch", "pane": peer_pane, "scrollback": true }),
    )
    .await;
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

    // Making something *on* the peer, addressed by node id rather than by a pane that already
    // lives there. Until the New sheet could be aimed at a machine, the only reachable path to
    // this was `at` on a peer's pane — so the relay carried every verb except the one that
    // creates the first thing on a node.
    send(
        &mut client,
        json!({ "t": "manage", "op": "workspace.create", "node": peer_node_id,
                "label": "mesh-made", "cwd": "/tmp" }),
    )
    .await;
    let ack = until(&mut client, "managed", 25).await;
    assert_eq!(ack["op"], "workspace.create");
    assert_eq!(ack["ok"], true, "a manage op aimed at a peer: {ack}");
    let made = ack["id"].as_str().expect("a workspace id").to_string();
    assert!(
        owned_by_peer(made.split('/').next().unwrap_or_default()),
        "the workspace was made on {made}, not on the peer it was addressed to"
    );
    herd_becomes(&hub.node, 30, |herd| {
        herd.panes.iter().any(|p| p.id.starts_with(&format!("{made}:")))
    })
    .await;

    // The hub's own pane is a different node in the same herd, driven over the same socket.
    send(
        &mut client,
        json!({ "t": "watch", "pane": hub_pane, "scrollback": true }),
    )
    .await;
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
    let error = refusal(&mut client, &peer_pane, "node_offline", 15).await;
    assert!(!error["message"].as_str().unwrap_or_default().is_empty());
    send(
        &mut client,
        json!({ "t": "input", "pane": peer_pane, "text": "x" }),
    )
    .await;
    refusal(&mut client, &peer_pane, "node_offline", 15).await;

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
    send(
        &mut client,
        json!({ "t": "watch", "pane": peer_pane, "scrollback": true }),
    )
    .await;
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

/// The next `code` *about this pane*. An error about some other pane is not an answer to a
/// question asked about this one, and taking it would make the assertion a coin toss — and a
/// relayed pane carries the peer's own errors too, so a busy machine can put a `stream_unavailable`
/// in front of the refusal being waited for.
async fn refusal(socket: &mut Socket, pane: &str, code: &str, seconds: u64) -> Value {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    let mut seen = Vec::new();
    while tokio::time::Instant::now() < deadline {
        let Some(message) = recv(socket, Duration::from_secs(2)).await else {
            continue;
        };
        if message["t"] == "error" {
            if message["pane"] == pane && message["code"] == code {
                return message;
            }
            seen.push(message);
        }
    }
    panic!("no {code} about {pane}; saw {seen:?}");
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

/// The handshake binds a key to one node id, and nothing after it does. Two enrolled machines on
/// one hub is the entire point of the mesh, so a second link claiming an id that is already live
/// is one of them reaching for the other's terminals — the hub refuses it rather than letting the
/// order they connected in decide who receives the watches and the keystrokes.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_cannot_take_over_another_peers_node_id() {
    let Some(hub_session) = Session::start("claims").await else {
        eprintln!("skipping: herdr is not on PATH");
        return;
    };
    let hub_home = Home::new();
    let hub = Running::start(&hub_home, &hub_session, "front").await;
    let mesh = hub.node.auth.store().mesh();
    let now = kampr_auth::now();

    let dial = async |home: &Home, node_id: &str, name: &str| {
        let code = mesh.invite(now, now + 600).await.expect("a join code");
        kampr_mesh::dial(
            &Hub {
                url: hub.origin.clone(),
                name: "hub".into(),
                key: None,
                join: Some(code),
            },
            &home.identity(),
            &Presence {
                node_id: node_id.to_string(),
                node_name: name.to_string(),
                build: BUILD.to_string(),
            },
            Duration::from_secs(10),
        )
        .await
    };

    // The node that owns the id, holding its link open exactly as `kampr serve` would.
    let laptop_home = Home::new();
    let (_laptop_hub, _laptop_out, mut laptop_in) = dial(&laptop_home, "01JLAPTOP", "laptop")
        .await
        .expect("an enrolled node is served");
    let laptop_key = laptop_home.identity().public_hex();
    tokio::time::timeout(Duration::from_secs(10), async {
        while hub.node.peers.links().is_empty() {
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    })
    .await
    .expect("the hub never served the laptop");

    // A second enrolled machine, dialling in as the laptop. The handshake itself succeeds: its own
    // key is enrolled here and it signed with it. What it may not have is the laptop's id.
    let impostor_home = Home::new();
    let (_impostor_hub, _impostor_out, mut impostor_in) = dial(&impostor_home, "01JLAPTOP", "not the laptop")
        .await
        .expect("the handshake is about the key, and this key is enrolled");
    assert!(
        tokio::time::timeout(Duration::from_secs(10), async {
            while impostor_in.recv().await.is_some() {}
        })
        .await
        .is_ok(),
        "the hub served two links for one node id",
    );

    let links = hub.node.peers.links();
    assert_eq!(links.len(), 1, "the impostor stayed in the herd");
    assert_eq!(
        links[0].pubkey, laptop_key,
        "the id resolves to the machine that authenticated as it",
    );
    assert_eq!(
        hub.node
            .peers
            .link_for("01JLAPTOP/w1:p1")
            .expect("a live link")
            .pubkey,
        laptop_key,
        "traffic for the laptop's pane was handed to the impostor",
    );
    // And the laptop's own socket is untouched: a refused impostor costs the node it impersonated
    // nothing at all.
    if let Ok(text) = tokio::time::timeout(Duration::from_millis(500), laptop_in.recv()).await {
        assert!(text.is_some(), "the laptop's own link was collateral damage");
    }

    hub.stop();
}

/// A peer that speaks for itself, over the socket and the handshake a real one uses.
///
/// Everything beneath the script is the hub's own path: the dial, the mutual challenge, the
/// websocket framing, the enrolment row on disk and `Peers::serve`. That is what it buys over the
/// in-process harness in `kampr-mesh`, which cannot have the two things the properties below turn
/// on — a socket that can be closed, and a store another writer can reach.
struct Scripted {
    out: kampr_mesh::dial::WsOut,
    incoming: kampr_mesh::dial::WsIn,
}

impl Scripted {
    async fn join(hub: &Running, home: &Home, node_id: &str, name: &str) -> Self {
        let now = kampr_auth::now();
        let code = hub
            .node
            .auth
            .store()
            .mesh()
            .invite(now, now + 600)
            .await
            .expect("a join code");
        Self::dial(hub, home, node_id, name, Some(code)).await
    }

    async fn dial(hub: &Running, home: &Home, node_id: &str, name: &str, code: Option<String>) -> Self {
        let (_, out, incoming) = kampr_mesh::dial(
            &Hub {
                url: hub.origin.clone(),
                name: "hub".into(),
                key: None,
                join: code,
            },
            &home.identity(),
            &Presence {
                node_id: node_id.to_string(),
                node_name: name.to_string(),
                build: BUILD.to_string(),
            },
            Duration::from_secs(10),
        )
        .await
        .expect("the hub served an enrolled node");
        Self { out, incoming }
    }

    async fn say(&mut self, message: Value) {
        assert!(
            self.out.send(message.to_string()).await,
            "the hub had already hung up"
        );
    }

    /// The peer's own words about what it holds, which is the only thing a `herd` message ever is.
    async fn advertise(&mut self, nodes: &[(&str, &str)], panes: &[&str]) {
        let nodes: Vec<Value> = nodes
            .iter()
            .map(|(id, name)| json!({ "id": id, "name": name, "kind": "local", "online": true }))
            .collect();
        let panes: Vec<Value> = panes
            .iter()
            .map(|id| {
                json!({
                    "id": id,
                    "node_id": id.split('/').next().unwrap_or_default(),
                    "rows": 24,
                })
            })
            .collect();
        self.say(json!({ "t": "herd", "nodes": nodes, "panes": panes }))
            .await;
    }

    /// The next request that is not a keepalive, answered on the way past — a peer that leaves
    /// them unanswered is one this hub drops, which is a different property. The deadline covers
    /// the request rather than each frame, or a hub that only ever pinged would hang here instead
    /// of failing.
    async fn next_but_ping(&mut self) -> Value {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let text = tokio::time::timeout_at(deadline, self.incoming.recv())
                .await
                .expect("the hub asked for nothing")
                .expect("the hub closed the link");
            let message: Value = serde_json::from_str(&text).expect("the hub sent JSON");
            let Some(n) = message["n"].as_u64().filter(|_| message["t"] == "ping") else {
                return message;
            };
            self.say(json!({ "t": "pong", "n": n })).await;
        }
    }

    /// Waits out the keepalive that has just been sent, so what follows starts a full interval
    /// away from the next one — the difference between "the hub hung up" and "the hub noticed on
    /// its next tick" is the whole of what an immediate disconnection buys.
    async fn after_a_keepalive(&mut self) {
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        loop {
            let text = tokio::time::timeout_at(deadline, self.incoming.recv())
                .await
                .expect("the hub never pinged")
                .expect("the hub closed the link");
            let message: Value = serde_json::from_str(&text).expect("the hub sent JSON");
            if let Some(n) = message["n"].as_u64().filter(|_| message["t"] == "ping") {
                self.say(json!({ "t": "pong", "n": n })).await;
                return;
            }
        }
    }

    /// Whether the hub hung up. `None` from the read half is the socket closing, which is all a
    /// refused, revoked or silent peer is ever told.
    async fn hung_up(&mut self, seconds: u64) -> bool {
        tokio::time::timeout(Duration::from_secs(seconds), async {
            while self.incoming.recv().await.is_some() {}
        })
        .await
        .is_ok()
    }
}

/// Polls the hub's own mesh state, because most of what a link does to a hub is visible there
/// before it reaches the herd model.
async fn mesh_settles(hub: &Running, seconds: u64, want: impl Fn(&kampr_mesh::Peers) -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(seconds);
    while tokio::time::Instant::now() < deadline {
        if want(&hub.node.peers) {
            return;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!(
        "the hub's mesh never got there;\n  links {:?}\n  panes {:?}",
        hub.node
            .peers
            .links()
            .iter()
            .map(|l| (l.node_id.clone(), l.name.clone()))
            .collect::<Vec<_>>(),
        hub.node
            .peers
            .herd()
            .panes
            .iter()
            .map(|p| p.id.clone())
            .collect::<Vec<_>>(),
    );
}

fn offline_detail(node: &Arc<Node>, node_id: &str) -> String {
    node.herd()
        .nodes
        .iter()
        .find(|n| n.id == node_id && !n.online)
        .unwrap_or_else(|| panic!("{node_id} is not listed as offline"))
        .detail
        .clone()
        .unwrap_or_else(|| panic!("{node_id} went offline without saying why"))
}

/// `kampr mesh revoke`, in a process of its own, writing nothing but SQLite — the operator's
/// actual gesture. Nothing tells the running node, which is the whole reason its keepalive
/// re-reads the enrolment rather than trusting what the handshake found.
async fn revoke_elsewhere(home: &Home, needle: &str) {
    let db = Config::state_db(&home.state());
    match kampr_binary() {
        Some(binary) => {
            let out = tokio::process::Command::new(binary)
                .args([
                    "mesh",
                    "revoke",
                    "--config-dir",
                    &home.config().display().to_string(),
                    "--state-dir",
                    &home.state().display().to_string(),
                    needle,
                ])
                .output()
                .await
                .expect("running kampr mesh revoke");
            assert!(
                out.status.success(),
                "kampr mesh revoke: {}",
                String::from_utf8_lossy(&out.stderr)
            );
        }
        // The binary is built by `cargo test --workspace` and absent from `cargo test -p
        // kampr-node`. The fallback is the same write on a second connection to the same file,
        // which is all that process is once it is past `clap`.
        None => {
            let store = kampr_auth::Store::open(&db).await.expect("a second connection");
            store
                .mesh()
                .revoke(needle, kampr_auth::now())
                .await
                .expect("the revoke")
                .expect("a row to revoke");
        }
    }
    let store = kampr_auth::Store::open(&db).await.expect("the hub's store");
    assert!(
        store
            .mesh()
            .find(needle)
            .await
            .expect("the row")
            .expect("a row")
            .revoked_at
            .is_some(),
        "nothing was revoked, so what follows would be measuring nothing",
    );
}

fn kampr_binary() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let candidate = exe.parent()?.parent()?.join("kampr");
    candidate.is_file().then_some(candidate)
}

/// The link a hub already has is the one revocation has to bite, and the operator who revokes is
/// somewhere else entirely: a second process, holding its own connection to the same file, that
/// tells the running node nothing at all. Everything here crosses a real socket, so a hub that
/// noticed only at the next handshake goes on serving a node it has been told to cut off, and a
/// client goes on reading a terminal on it.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_revoked_by_another_process_loses_a_link_that_crosses_a_real_socket() {
    sessions!(hub_session, peer_session);
    let hub_home = Home::new();
    let peer_home = Home::new();

    let hub = Running::start(&hub_home, &hub_session, "front").await;
    let now = kampr_auth::now();
    let code = hub
        .node
        .auth
        .store()
        .mesh()
        .invite(now, now + 600)
        .await
        .expect("a join code");
    let peer_node_id = peer_home.settle("laptop");
    join(&peer_home, "laptop", &peer_node_id, &hub.origin, &code).await;
    let peer = Running::start(&peer_home, &peer_session, "laptop").await;

    let owned_by_peer = |id: &str| id == peer_node_id || id.starts_with(&format!("{peer_node_id}."));
    herd_becomes(&hub.node, 45, |herd| {
        herd.nodes.iter().any(|n| n.id == peer_node_id && n.online)
            && herd.panes.iter().any(|p| owned_by_peer(&p.node_id))
    })
    .await;
    let peer_pane = hub
        .node
        .herd()
        .panes
        .iter()
        .find(|p| owned_by_peer(&p.node_id))
        .expect("a pane on the peer")
        .id
        .clone();

    // A client is watching that pane at the moment the operator cuts the node off, because a
    // frozen grid is exactly what a revocation nobody was told about looks like.
    let mut client = hub.connect().await;
    until(&mut client, "hello", 10).await;
    send(
        &mut client,
        json!({ "t": "watch", "pane": peer_pane, "scrollback": true }),
    )
    .await;
    until(&mut client, "grid.reset", 20).await;

    revoke_elsewhere(&hub_home, &peer_node_id).await;

    mesh_settles(&hub, 30, |peers| peers.links().is_empty()).await;
    herd_becomes(&hub.node, 30, |herd| {
        herd.nodes.iter().any(|n| n.id == peer_node_id && !n.online)
    })
    .await;
    let detail = offline_detail(&hub.node, &peer_node_id);
    assert!(detail.contains("revoked"), "the herd says {detail:?}");

    // And the client is told, in the same words, rather than left reading a grid that stopped.
    let error = refusal(&mut client, &peer_pane, "node_offline", 20).await;
    assert!(
        error["message"].as_str().unwrap_or_default().contains("revoked"),
        "a revoked node's viewer was told {error}",
    );

    // The peer's supervisor is dialling this whole time. A revocation that only dropped the
    // socket would be undone by the next reconnect.
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(
        hub.node.peers.links().is_empty(),
        "a revoked node dialled back in and was served",
    );

    peer.stop();
    hub.stop();
}

/// A peer that stops answering is not a peer that closed its socket: the connection stays up and
/// readable, and nothing arrives on it. Only the keepalive can tell the difference, and until it
/// does a client reads a frozen grid beside an `rtt_ms` that stopped moving.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_that_stops_answering_keepalives_is_dropped_and_the_herd_says_so() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JSILENT", "silent").await;
    peer.advertise(&[("01JSILENT", "silent")], &["01JSILENT/w1:p1"])
        .await;
    mesh_settles(&hub, 10, |peers| {
        peers.herd().panes.iter().any(|p| p.id == "01JSILENT/w1:p1")
    })
    .await;

    // Nothing is ever sent back: the pings arrive and are read, and no pong follows them.
    assert!(
        peer.hung_up(60).await,
        "the hub kept a link to a node that had stopped answering",
    );
    mesh_settles(&hub, 15, |peers| peers.links().is_empty()).await;
    herd_becomes(&hub.node, 20, |herd| {
        herd.nodes.iter().any(|n| n.id == "01JSILENT" && !n.online)
    })
    .await;
    let detail = offline_detail(&hub.node, "01JSILENT");
    assert!(detail.contains("keepalives"), "the herd says {detail:?}");
    assert!(
        hub.node.herd().panes.iter().any(|p| p.id == "01JSILENT/w1:p1"),
        "its panes vanished out from under the client instead of being listed as unreachable",
    );

    hub.stop();
}

/// A `herd` message is a peer's own words about itself, and nothing under it is evidence: only
/// the id the handshake bound to its key is. An enrolled but hostile machine that names another
/// node's panes would otherwise be handed that node's watches and keystrokes — one host reading
/// another's terminals through the hub that exists to join them.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_may_not_keep_a_node_id_that_another_link_authenticates_as() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;

    let impostor_home = Home::new();
    let mut impostor = Scripted::join(&hub, &impostor_home, "01JIMPOSTOR", "impostor").await;
    impostor
        .advertise(
            &[("01JIMPOSTOR", "impostor"), ("01JOWNER", "laptop")],
            &["01JIMPOSTOR/w1:p1", "01JOWNER/w1:p1"],
        )
        .await;
    // Nobody has authenticated as the laptop yet, so the hub has nothing to check the claim
    // against and it stands — which is what makes taking it back a property worth measuring.
    mesh_settles(&hub, 10, |peers| peers.link_for("01JOWNER/w1:p1").is_some()).await;

    let owner_home = Home::new();
    let mut owner = Scripted::join(&hub, &owner_home, "01JOWNER", "laptop").await;
    mesh_settles(&hub, 10, |peers| peers.links().len() == 2).await;
    assert_eq!(
        hub.node
            .peers
            .link_for("01JOWNER/w1:p1")
            .expect("a live link")
            .pubkey,
        owner_home.identity().public_hex(),
        "the laptop's pane was still routed to the machine that only claimed it",
    );
    mesh_settles(&hub, 10, |peers| {
        !peers.herd().panes.iter().any(|p| p.id == "01JOWNER/w1:p1")
    })
    .await;

    // And saying it again does not get it back.
    impostor
        .advertise(
            &[("01JIMPOSTOR", "impostor"), ("01JOWNER", "laptop")],
            &["01JIMPOSTOR/w2:p2", "01JOWNER/w1:p1"],
        )
        .await;
    mesh_settles(&hub, 10, |peers| {
        peers.herd().panes.iter().any(|p| p.id == "01JIMPOSTOR/w2:p2")
    })
    .await;
    let herd = hub.node.peers.herd();
    assert!(
        !herd.panes.iter().any(|p| p.id == "01JOWNER/w1:p1"),
        "a peer re-advertised a node another link had authenticated as: {:?}",
        herd.panes.iter().map(|p| p.id.clone()).collect::<Vec<_>>(),
    );
    assert!(
        !herd.nodes.iter().any(|n| n.id == "01JOWNER"),
        "the laptop is in this herd on the impostor's word, and the only link that may speak for \
         it has said nothing",
    );

    // A refused claim costs the node it was aimed at nothing: both links are still up.
    assert!(
        !owner.hung_up(1).await,
        "the node that owns the id was the one hung up on",
    );
    assert_eq!(
        hub.node.peers.links().len(),
        2,
        "one of the two enrolled machines lost its link over the other's claim",
    );

    hub.stop();
}

/// A node whose socket died without either end noticing dials again, and the hub is still holding
/// the corpse. Two rows for one peer publish it twice and route to whichever the scan reaches
/// first, which may be the dead one.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_dialling_again_replaces_the_link_it_had() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();

    let mut first = Scripted::join(&hub, &peer_home, "01JTWICE", "laptop").await;
    first
        .advertise(&[("01JTWICE", "laptop")], &["01JTWICE/w1:p1"])
        .await;
    mesh_settles(&hub, 10, |peers| {
        peers.herd().panes.iter().any(|p| p.id == "01JTWICE/w1:p1")
    })
    .await;

    // The same key, so the same node: enrolled already, and dialling with no join code at all.
    let mut second = Scripted::dial(&hub, &peer_home, "01JTWICE", "laptop", None).await;
    assert!(
        first.hung_up(10).await,
        "the hub kept the socket it had already replaced",
    );
    second
        .advertise(&[("01JTWICE", "laptop")], &["01JTWICE/w9:p9"])
        .await;
    mesh_settles(&hub, 10, |peers| {
        peers.herd().panes.iter().any(|p| p.id == "01JTWICE/w9:p9")
    })
    .await;

    assert_eq!(hub.node.peers.links().len(), 1, "one node, two links");
    let herd = hub.node.peers.herd();
    assert_eq!(
        herd.nodes.iter().filter(|n| n.id == "01JTWICE").count(),
        1,
        "the node it replaced is listed beside it, offline for ever: {:?}",
        herd.nodes
            .iter()
            .map(|n| (n.id.clone(), n.online))
            .collect::<Vec<_>>(),
    );
    assert!(
        herd.nodes.iter().all(|n| n.online),
        "the replaced link was remembered as an outage that never happened",
    );

    hub.stop();
}

/// Style ids are minted append-only by one encoder per link, so a batch that starts past the end
/// of the table this hub holds is not a gap — there is no honest way to make one — and the number
/// is unbounded, so resizing to it is an allocation a forty-byte message can ask for.
#[tokio::test(flavor = "multi_thread")]
async fn a_styles_frame_that_skips_past_the_table_it_appends_to_ends_the_link() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JSTYLE", "styles").await;
    peer.advertise(&[("01JSTYLE", "styles")], &["01JSTYLE/w1:p1"])
        .await;
    mesh_settles(&hub, 10, |peers| !peers.links().is_empty()).await;

    peer.say(json!({ "t": "styles", "from": u32::MAX, "styles": [] }))
        .await;

    assert!(
        peer.hung_up(10).await,
        "the hub went on serving a link that had lied about its own style table",
    );
    mesh_settles(&hub, 10, |peers| peers.links().is_empty()).await;
    herd_becomes(&hub.node, 15, |herd| {
        herd.nodes.iter().any(|n| n.id == "01JSTYLE" && !n.online)
    })
    .await;
    let detail = offline_detail(&hub.node, "01JSTYLE");
    assert!(detail.contains("styles"), "the herd says {detail:?}");

    hub.stop();
}

/// One `watch` per pane per link is what keeps the WAN hop carrying one copy, so the first viewer
/// decides what the peer sends. A later one asking for the transcript has to say so, or it lands
/// on the agent pane's default surface with nothing in it.
#[tokio::test(flavor = "multi_thread")]
async fn a_second_viewer_asking_for_the_conversation_gets_an_upgrade_watch() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JCONVO", "agentic").await;
    peer.advertise(&[("01JCONVO", "agentic")], &["01JCONVO/w1:p1"])
        .await;
    mesh_settles(&hub, 10, |peers| {
        peers.herd().panes.iter().any(|p| p.id == "01JCONVO/w1:p1")
    })
    .await;

    let mut watcher = hub.connect().await;
    until(&mut watcher, "hello", 10).await;
    send(
        &mut watcher,
        json!({ "t": "watch", "pane": "01JCONVO/w1:p1", "scrollback": true }),
    )
    .await;
    let asked = peer.next_but_ping().await;
    assert_eq!(asked["t"], "watch");
    assert_eq!(asked["pane"], "01JCONVO/w1:p1");
    assert_eq!(asked["conversation"], false, "{asked}");

    let mut reader = hub.connect().await;
    until(&mut reader, "hello", 10).await;
    send(
        &mut reader,
        json!({ "t": "watch", "pane": "01JCONVO/w1:p1", "scrollback": true, "conversation": true }),
    )
    .await;
    let upgraded = peer.next_but_ping().await;
    assert_eq!(upgraded["t"], "watch", "{upgraded}");
    assert_eq!(upgraded["pane"], "01JCONVO/w1:p1");
    assert_eq!(
        upgraded["conversation"], true,
        "the second viewer was attached to a stream that carries no transcript: {upgraded}",
    );

    hub.stop();
}

/// The same revocation from the other end of the herd: an operator with a phone rather than a
/// shell, spending a device token on the hub's own API. This one must not wait for a keepalive at
/// all — the handler ends the link itself — so the test is armed just after a tick, where a hub
/// that only marked the row would have four more seconds of serving a node it was told to cut off.
#[tokio::test(flavor = "multi_thread")]
async fn a_peer_revoked_by_a_client_of_the_hub_is_cut_off_without_waiting_for_a_keepalive() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JCUTOFF", "laptop").await;
    peer.advertise(&[("01JCUTOFF", "laptop")], &["01JCUTOFF/w1:p1"])
        .await;
    mesh_settles(&hub, 10, |peers| !peers.links().is_empty()).await;

    let token = hub.token().await;
    peer.after_a_keepalive().await;
    let answer = post_as(
        &format!("{}/api/mesh/01JCUTOFF/revoke", hub.origin),
        &json!({}),
        Some(&token),
    )
    .await;
    assert_eq!(answer["revoked"], "01JCUTOFF", "{answer}");
    assert!(
        peer.hung_up(2).await,
        "the row was marked and the socket left open, which is a revoked node still typing",
    );

    mesh_settles(&hub, 10, |peers| peers.links().is_empty()).await;
    herd_becomes(&hub.node, 15, |herd| {
        herd.nodes.iter().any(|n| n.id == "01JCUTOFF" && !n.online)
    })
    .await;
    let detail = offline_detail(&hub.node, "01JCUTOFF");
    assert!(detail.contains("revoked"), "the herd says {detail:?}");

    hub.stop();
}

/// Watches one relayed pane and waits until the hub has served its first grid, so what follows is
/// measured against a pane the hub actually holds a shadow of.
async fn watch_relayed(client: &mut Socket, peer: &mut Scripted, pane: &str) {
    send(client, json!({ "t": "watch", "pane": pane, "scrollback": true })).await;
    let asked = peer.next_but_ping().await;
    assert_eq!(asked["t"], "watch", "{asked}");
    assert_eq!(asked["pane"], pane, "{asked}");
    peer.say(json!({
        "t": "grid.reset", "pane": pane, "cols": 8, "rows": 2,
        "rows_data": [{ "row": 0, "runs": [{ "s": 0, "x": "hello" }] }],
        "cursor": { "col": 0, "row": 0, "visible": true }, "links": [],
    }))
    .await;
    until(client, "grid.reset", 10).await;
}

/// The mesh twin of #252, and a *scheduler race* rather than a certainty: `JoinHandle::abort` is
/// not synchronous, so the aborted pump still holds the pane when its replacement watches — and
/// tokio's LIFO slot normally polls that replacement first. One pane therefore survives a resync.
/// Two or more do not: `resync` spawns a pump per pane, so the second spawn evicts the first from
/// that slot, the aborted pump is reaped first, and the pane it was the last watcher of goes with
/// it — the hub's shadow, the stitched history, and one `unwatch`+`watch` across the WAN each.
#[tokio::test(flavor = "multi_thread")]
async fn resyncing_several_peer_panes_asks_the_peer_for_none_of_them_again() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JHOLD", "laptop").await;
    let panes = ["01JHOLD/w1:p1", "01JHOLD/w1:p2", "01JHOLD/w1:p3"];
    peer.advertise(&[("01JHOLD", "laptop")], &panes).await;
    mesh_settles(&hub, 10, |peers| {
        peers.herd().panes.iter().any(|p| p.id == "01JHOLD/w1:p3")
    })
    .await;

    let mut client = hub.connect().await;
    until(&mut client, "hello", 10).await;
    for pane in panes {
        watch_relayed(&mut client, &mut peer, pane).await;
    }

    send(&mut client, json!({ "t": "resync" })).await;
    // `input` is a fence: it is the next request the hub has any reason to make, so a re-watch
    // that the resync sent would arrive ahead of it.
    send(
        &mut client,
        json!({ "t": "input", "pane": panes[0], "text": "ls\r" }),
    )
    .await;
    let asked = peer.next_but_ping().await;
    assert_eq!(
        asked["t"], "input",
        "a resync re-crossed the WAN for a relayed pane the hub already held: {asked}",
    );

    // And the client was re-served out of the hub's shadow, with no round trip at all.
    for _ in panes {
        until(&mut client, "grid.reset", 10).await;
    }

    hub.stop();
}

/// The other half of holding a relayed pane across a handover: a hold that never releases is a hub
/// that keeps a shadow and a stitched history for every pane it has ever shown, and a peer that
/// keeps streaming panes nobody is looking at. A pane whose viewers have genuinely gone must still
/// be unwatched upstream — including one that has been through a handover.
#[tokio::test(flavor = "multi_thread")]
async fn relayed_panes_the_last_client_unwatches_are_still_unwatched_upstream() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JFREE", "laptop").await;
    let panes = ["01JFREE/w1:p1", "01JFREE/w1:p2"];
    peer.advertise(&[("01JFREE", "laptop")], &panes).await;
    mesh_settles(&hub, 10, |peers| {
        peers.herd().panes.iter().any(|p| p.id == "01JFREE/w1:p2")
    })
    .await;

    let mut client = hub.connect().await;
    until(&mut client, "hello", 10).await;
    for pane in panes {
        watch_relayed(&mut client, &mut peer, pane).await;
    }
    send(&mut client, json!({ "t": "resync" })).await;
    for _ in panes {
        until(&mut client, "grid.reset", 10).await;
    }

    for pane in panes {
        send(&mut client, json!({ "t": "unwatch", "pane": pane })).await;
    }

    let mut released = Vec::new();
    while released.len() < panes.len() {
        let asked = peer.next_but_ping().await;
        if asked["t"] == "unwatch" {
            released.push(asked["pane"].as_str().unwrap_or_default().to_string());
        }
    }
    released.sort();
    assert_eq!(
        released, panes,
        "a relayed pane nobody is watching was left streaming across the WAN",
    );

    hub.stop();
}

/// A whole `GET`: the status, the headers as they came, and the bytes underneath them. A relayed
/// attachment is served with a `Content-Type` this hub decided and a body it never held at once,
/// so all three are the answer rather than only the first.
struct Got {
    status: u16,
    headers: String,
    body: Vec<u8>,
}

async fn get(url: &str, token: &str) -> Got {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = url.trim_start_matches("http://");
    let (authority, path) = rest.split_once('/').expect("a path");
    let (host, port) = authority.split_once(':').expect("a port");
    let request = format!(
        "GET /{path} HTTP/1.1\r\nHost: {authority}\r\nAuthorization: Bearer {token}\r\n\
         Connection: close\r\n\r\n"
    );
    let mut stream = TcpStream::connect((host, port.parse::<u16>().unwrap()))
        .await
        .expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let split = response
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .expect("a header block");
    let headers = String::from_utf8_lossy(&response[..split]).to_string();
    let status = headers
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .expect("a status line");
    Got {
        status,
        body: response[split + 4..].to_vec(),
        headers,
    }
}

async fn get_status(url: &str, token: &str) -> u16 {
    get(url, token).await.status
}

/// A hub serves a relayed attachment by asking the peer for it, so the promise it relays is only
/// as good as the peer at the other end of that ask. **This peer never said it answers
/// `att.fetch`** — the shape of every build from before that verb existed — so the id has nothing
/// behind it here and comes back as the one 404 every refusal wears, which would reach the
/// operator as "the node no longer has this attachment" on a picture that is perfectly intact one
/// hop away. A client handed an `att` renders a button for it, so relaying one here is offering a
/// control that cannot work. Both halves are measured: the 404 first, so the promise is known to
/// be unkeepable, then the absence of the promise.
#[tokio::test(flavor = "multi_thread")]
async fn a_relayed_pane_promises_no_attachment_this_hub_could_not_serve() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JATTACH", "laptop").await;
    let pane = "01JATTACH/w1:p1";
    peer.advertise(&[("01JATTACH", "laptop")], &[pane]).await;
    mesh_settles(&hub, 10, |peers| peers.herd().panes.iter().any(|p| p.id == pane)).await;

    let id = kampr_journal::attach::Locator {
        agent: "claude".into(),
        path: "projects/-home-u-demo/session.jsonl".into(),
        offset: 0,
        index: 0,
        bytes: 68,
    }
    .encode();

    let token = hub.token().await;
    assert_eq!(
        get_status(&format!("{}/api/attachment/{pane}/{id}", hub.origin), &token).await,
        404,
        "this hub can serve a peer's attachment after all, and the promise below is keepable",
    );
    assert!(
        !hub.node.peers.can_serve_attachments(pane),
        "a peer that has never claimed the verb was taken at a word it did not give",
    );

    let mut client = hub.connect().await;
    until(&mut client, "hello", 10).await;
    send(
        &mut client,
        json!({ "t": "watch", "pane": pane, "scrollback": true, "conversation": true }),
    )
    .await;
    let asked = peer.next_but_ping().await;
    assert_eq!(asked["t"], "watch", "{asked}");

    peer.say(json!({
        "t": "convo.turn", "pane": pane, "turns": [{
            "id": "549c13ed-c2b4-4013-b072-f26304a5bb6c",
            "role": "user",
            "blocks": [
                { "b": "md", "text": "look" },
                { "b": "md", "text": "[image · png]",
                  "att": { "id": id, "kind": "image", "mime": "image/png", "bytes": 68 } }
            ]
        }]
    }))
    .await;

    let relayed = until(&mut client, "convo.turn", 10).await;
    let blocks = relayed["turns"][0]["blocks"].as_array().expect("blocks");
    assert_eq!(blocks[1]["text"], "[image · png]", "{relayed}");
    assert!(
        blocks[1].get("att").is_none(),
        "the hub relayed a button for bytes it answers 404 for: {relayed}",
    );

    // The paged answer is a different `t` off the same relay, and it is the one an operator
    // scrolling back through yesterday's screenshots lands on.
    peer.say(json!({
        "t": "convo", "pane": pane, "more": false, "turns": [{
            "id": "6f1b0f77-3c21-4a5e-9d0c-3b6b1f7c2ea1",
            "role": "user",
            "blocks": [
                { "b": "md", "text": "[image · png]",
                  "att": { "id": id, "kind": "image", "mime": "image/png", "bytes": 68 } }
            ]
        }]
    }))
    .await;
    let paged = until(&mut client, "convo", 10).await;
    assert!(
        paged["turns"][0]["blocks"][0].get("att").is_none(),
        "a relayed page carried the promise the live turn did not: {paged}",
    );

    hub.stop();
}

/// The other half of the promise: a peer that **does** answer `att.fetch` keeps its `att`, and the
/// hub serves the bytes behind it.
///
/// The hub has no inbound path to the peer (ADR 0007), so it cannot fetch over HTTP: it asks on
/// the link the peer dialled and streams what comes back, a chunk at a time. What is measured here
/// is the whole hop — the promise surviving the relay, the ask crossing the link, and a body that
/// is byte for byte what the peer sent under a `Content-Type` **this hub** decided.
#[tokio::test(flavor = "multi_thread")]
async fn a_hub_serves_the_attachment_its_peer_promised_it_could() {
    use base64::Engine;

    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JSERVE", "laptop").await;
    let pane = "01JSERVE/w1:p1";
    peer.say(json!({ "t": "hello", "node_id": "01JSERVE", "caps": { "attachments": true } }))
        .await;
    peer.advertise(&[("01JSERVE", "laptop")], &[pane]).await;
    mesh_settles(&hub, 10, |peers| {
        peers.herd().panes.iter().any(|p| p.id == pane) && peers.can_serve_attachments(pane)
    })
    .await;

    let id = kampr_journal::attach::Locator {
        agent: "claude".into(),
        path: "projects/-home-u-demo/session.jsonl".into(),
        offset: 0,
        index: 0,
        bytes: 68,
    }
    .encode();

    let mut client = hub.connect().await;
    until(&mut client, "hello", 10).await;
    send(
        &mut client,
        json!({ "t": "watch", "pane": pane, "scrollback": true, "conversation": true }),
    )
    .await;
    assert_eq!(peer.next_but_ping().await["t"], "watch");
    peer.say(json!({
        "t": "convo.turn", "pane": pane, "turns": [{
            "id": "549c13ed-c2b4-4013-b072-f26304a5bb6c",
            "role": "user",
            "blocks": [
                { "b": "md", "text": "[image · png]",
                  "att": { "id": id, "kind": "image", "mime": "image/png", "bytes": 68 } }
            ]
        }]
    }))
    .await;
    let relayed = until(&mut client, "convo.turn", 10).await;
    assert_eq!(
        relayed["turns"][0]["blocks"][0]["att"]["id"], id,
        "the hub dropped a promise it can keep: {relayed}",
    );

    // A PNG the peer will hand over in one chunk, and the sniffing path's only honest input.
    let png = b"\x89PNG\r\n\x1a\n and then some bytes".to_vec();
    let token = hub.token().await;
    let asked = {
        let url = format!("{}/api/attachment/{pane}/{id}", hub.origin);
        tokio::spawn(async move { get(&url, &token).await })
    };

    let fetch = peer.next_but_ping().await;
    assert_eq!(fetch["t"], "att.fetch", "{fetch}");
    assert_eq!(fetch["pane"], pane);
    assert_eq!(fetch["id"], id);
    let rid = fetch["rid"].as_u64().expect("an rid");
    peer.say(json!({
        "t": "att.open", "rid": rid, "bytes": png.len(), "kind": "image", "mime": "image/png"
    }))
    .await;
    peer.say(json!({
        "t": "att.chunk", "rid": rid, "seq": 0,
        "b64": base64::engine::general_purpose::STANDARD.encode(&png)
    }))
    .await;
    peer.say(json!({ "t": "att.end", "rid": rid })).await;

    let got = asked.await.expect("the request task");
    assert_eq!(got.status, 200, "{}", got.headers);
    assert_eq!(got.body, png, "the hub served different bytes: {}", got.headers);
    assert!(
        got.headers
            .to_ascii_lowercase()
            .contains("content-type: image/png"),
        "{}",
        got.headers,
    );
    hub.stop();
}

/// A peer that refuses is the same 404 a stale id gets from this hub's own transcripts. It has to
/// be: the peer already answers every refusal but the ceiling identically, and a hub that turned
/// one of them into a different status would put back the distinction that was removed there.
#[tokio::test(flavor = "multi_thread")]
async fn an_attachment_a_peer_refuses_is_the_hubs_own_one_refusal() {
    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JREFUSE", "laptop").await;
    let pane = "01JREFUSE/w1:p1";
    peer.say(json!({ "t": "hello", "node_id": "01JREFUSE", "caps": { "attachments": true } }))
        .await;
    peer.advertise(&[("01JREFUSE", "laptop")], &[pane]).await;
    mesh_settles(&hub, 10, |peers| peers.can_serve_attachments(pane)).await;

    let token = hub.token().await;
    let refused = {
        let url = format!("{}/api/attachment/{pane}/an-id", hub.origin);
        let token = token.clone();
        tokio::spawn(async move { get(&url, &token).await })
    };
    let rid = peer.next_but_ping().await["rid"].as_u64().expect("an rid");
    peer.say(json!({ "t": "att.error", "rid": rid, "code": "not_found" }))
        .await;
    let refused = refused.await.expect("the request task");

    // The hub's own answer for an id that resolves to nothing here, asked of its own node id.
    let local = format!("{}/w1:p1", hub.node.node_id());
    let locally = get(&format!("{}/api/attachment/{local}/an-id", hub.origin), &token).await;
    assert_eq!(refused.status, 404);
    assert_eq!(
        refused.body, locally.body,
        "a relayed refusal is distinguishable from a local one",
    );
    hub.stop();
}

/// The hub asks for a chunk only once it has handed the last one downstream, so what it holds is
/// the window and never the record. A peer that runs past the window is one this hub stops reading
/// from — which is the thing that makes "the hub does not buffer an attachment" true rather than
/// merely intended.
#[tokio::test(flavor = "multi_thread")]
async fn a_hub_pulls_a_peers_attachment_a_chunk_at_a_time() {
    use base64::Engine;

    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JCHUNK", "laptop").await;
    let pane = "01JCHUNK/w1:p1";
    peer.say(json!({ "t": "hello", "node_id": "01JCHUNK", "caps": { "attachments": true } }))
        .await;
    peer.advertise(&[("01JCHUNK", "laptop")], &[pane]).await;
    mesh_settles(&hub, 10, |peers| peers.can_serve_attachments(pane)).await;

    let chunks: Vec<Vec<u8>> = (0..6u8).map(|n| vec![n; 4096]).collect();
    let total: usize = chunks.iter().map(Vec::len).sum();
    let token = hub.token().await;
    let asked = {
        let url = format!("{}/api/attachment/{pane}/an-id", hub.origin);
        tokio::spawn(async move { get(&url, &token).await })
    };

    let rid = peer.next_but_ping().await["rid"].as_u64().expect("an rid");
    peer.say(json!({ "t": "att.open", "rid": rid, "bytes": total, "kind": "image", "mime": "image/png" }))
        .await;
    for (seq, chunk) in chunks.iter().enumerate() {
        peer.say(json!({
            "t": "att.chunk", "rid": rid, "seq": seq,
            "b64": base64::engine::general_purpose::STANDARD.encode(chunk)
        }))
        .await;
        let granted = peer.next_but_ping().await;
        assert_eq!(
            granted,
            json!({ "t": "att.more", "rid": rid, "n": 1 }),
            "chunk {seq} was taken without one being granted back, so the window only shrinks",
        );
    }
    peer.say(json!({ "t": "att.end", "rid": rid })).await;

    let got = asked.await.expect("the request task");
    assert_eq!(got.status, 200, "{}", got.headers);
    assert_eq!(got.body, chunks.concat());
    hub.stop();
}

/// A peer's recorded media type is a string an agent wrote into a file on *another machine*, and
/// the hub serves the answer from its own origin. Echoing it would put a document out of the
/// origin the bundle's CSP is written for — the same trap the local route's allowlist exists for,
/// one hop further away, and the allowlist has to be applied at the hop that serves the bytes.
#[tokio::test(flavor = "multi_thread")]
async fn a_peers_media_type_is_run_through_this_hubs_own_allowlist() {
    use base64::Engine;

    let hub_home = Home::new();
    let hub = Running::hub(&hub_home, "front").await;
    let peer_home = Home::new();
    let mut peer = Scripted::join(&hub, &peer_home, "01JMIME", "laptop").await;
    let pane = "01JMIME/w1:p1";
    peer.say(json!({ "t": "hello", "node_id": "01JMIME", "caps": { "attachments": true } }))
        .await;
    peer.advertise(&[("01JMIME", "laptop")], &[pane]).await;
    mesh_settles(&hub, 10, |peers| peers.can_serve_attachments(pane)).await;

    let token = hub.token().await;
    let asked = {
        let url = format!("{}/api/attachment/{pane}/an-id", hub.origin);
        tokio::spawn(async move { get(&url, &token).await })
    };
    let rid = peer.next_but_ping().await["rid"].as_u64().expect("an rid");
    let page = b"<script>alert(1)</script>".to_vec();
    peer.say(json!({
        "t": "att.open", "rid": rid, "bytes": page.len(),
        "kind": "image", "mime": "text/html", "name": "../../evil.html"
    }))
    .await;
    peer.say(json!({
        "t": "att.chunk", "rid": rid, "seq": 0,
        "b64": base64::engine::general_purpose::STANDARD.encode(&page)
    }))
    .await;
    peer.say(json!({ "t": "att.end", "rid": rid })).await;

    let got = asked.await.expect("the request task");
    let headers = got.headers.to_ascii_lowercase();
    assert_eq!(got.status, 200, "{}", got.headers);
    assert!(
        headers.contains("content-type: application/octet-stream"),
        "the hub served a peer's `text/html` from its own origin: {}",
        got.headers,
    );
    assert!(
        headers.contains(r#"content-disposition: attachment; filename="evil.html""#),
        "a peer's filename reached a header with its separators intact: {}",
        got.headers,
    );
    hub.stop();
}
