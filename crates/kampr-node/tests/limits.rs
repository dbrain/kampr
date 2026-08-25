//! What a stranger can make this node do before it knows who they are, and how much of it.
//!
//! Nothing here needs a herd: every test drives the HTTP and WebSocket surface directly, against
//! a node whose herdr socket deliberately does not exist. The operator's own session is never
//! reachable from here.

use futures_util::{SinkExt, StreamExt};
use kampr_auth::{NodeIdentity, Role};
use kampr_mesh::dial::Hub;
use kampr_mesh::{Incoming, Presence};
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

    fn state_db(&self) -> std::path::PathBuf {
        self.home.path().join("state").join("kampr.db")
    }

    /// Makes the device store genuinely unreadable, from a second connection to the same file, so
    /// what the node's own pool hits is a real error rather than an injected one: every device
    /// query answers `no such table` from here on.
    async fn break_the_store(&self, broken: bool) {
        use sqlx::Connection;
        let mut db = sqlx::SqliteConnection::connect(&format!("sqlite://{}", self.state_db().display()))
            .await
            .expect("the state db");
        let sql = match broken {
            true => "ALTER TABLE devices RENAME TO devices_gone",
            false => "ALTER TABLE devices_gone RENAME TO devices",
        };
        sqlx::query(sql).execute(&mut db).await.expect("a rename");
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

/// Revocation is the product's kill switch, and the one path that can turn its failure into a
/// plausible-looking success is the session's own re-read: `refresh` answered "keep the session"
/// for *any* store error, and nothing distinguishes a transient one from a store that has stopped
/// working. A full disk, a corrupt WAL, the file replaced under a restore — every one of them kept
/// every connected device online, including devices the operator had just revoked.
///
/// One failure still has to be survivable; that half of the old comment was right.
#[tokio::test(flavor = "multi_thread")]
async fn a_session_whose_store_stays_unreadable_closes_instead_of_trusting_its_handshake() {
    let h = Harness::start(|_| {}).await;
    let token = h.token(Role::Full).await;
    let mut socket = h.connect(&token).await;

    // Every write verb re-reads the device row first, so the client itself sets the pace.
    let write = json!({ "t": "input", "pane": "01J/w1:p1", "text": "x" }).to_string();

    h.break_the_store(true).await;
    socket
        .send(tungstenite::Message::Text(write.clone().into()))
        .await
        .expect("a write");
    h.break_the_store(false).await;
    socket
        .send(tungstenite::Message::Text(write.clone().into()))
        .await
        .expect("a write");
    assert!(
        !closed(&mut socket, 2).await,
        "one unreadable check is a database hiccup, not a revocation"
    );

    h.break_the_store(true).await;
    for _ in 0..6 {
        let _ = socket
            .send(tungstenite::Message::Text(write.clone().into()))
            .await;
    }
    assert!(
        closed(&mut socket, 5).await,
        "a store that cannot say whether this device is still authorised must not keep it connected"
    );
}

/// The pairing toast's rate limit was enforced by nothing: the only call site built a `Toaster`
/// inside the task it spawned for each pairing, so `last` was always `None` and `MIN_INTERVAL`
/// was unreachable. Anything that can put arbitrary text on someone's desktop as fast as it likes
/// is a denial of service against the person, not the machine.
#[tokio::test(flavor = "multi_thread")]
async fn two_pairings_in_a_row_put_one_toast_on_the_operators_desk() {
    let h = Harness::start(|_| {}).await;
    h.token(Role::Full).await;
    assert!(
        settles(&h, 1).await,
        "the first pairing announces itself: {}",
        h.node.toaster.attempts()
    );
    h.token(Role::Full).await;
    assert!(
        !settles(&h, 2).await,
        "a second pairing inside the window is refused before it reaches herdr"
    );
}

/// The toast is raised on a task of its own, so this waits for it rather than assuming it ran.
async fn settles(h: &Harness, attempts: u64) -> bool {
    tokio::time::timeout(Duration::from_secs(3), async {
        while h.node.toaster.attempts() < attempts {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .is_ok()
}

/// `expires_at` is half of what "currently paired" means and the inventory did not say so: the
/// rows were serialised raw, a client read `revokedAt` alone, and a device whose token ran out
/// weeks ago showed as connected. `Device::active` is the node's own judgement and it is what
/// goes on the wire — additively, so an older client ignores it and reads what it read before.
#[tokio::test(flavor = "multi_thread")]
async fn the_device_inventory_says_an_expired_device_is_not_active() {
    let h = Harness::start(|_| {}).await;
    let token = h.token(Role::Full).await;
    let now = kampr_auth::now();
    let store = h.node.auth.store();
    store
        .create_device("lapsed", Role::Full, now - 86_400, Some(now - 60), None, None)
        .await
        .expect("a lapsed device");

    let (status, body) = request(
        &h.origin,
        "GET",
        "/api/devices",
        &[("authorization", &format!("Bearer {token}"))],
        None,
    )
    .await;
    assert!(status.contains("200"), "{status}");

    let devices = body["devices"].as_array().expect("a device list");
    let lapsed = devices
        .iter()
        .find(|d| d["name"] == "lapsed")
        .expect("the lapsed device");
    assert_eq!(
        lapsed["revoked_at"],
        Value::Null,
        "the field a client used to read on its own says nothing about an expiry"
    );
    assert_eq!(lapsed["active"], json!(false));

    let live = devices
        .iter()
        .find(|d| d["name"] == "limits")
        .expect("this session's own device");
    assert_eq!(live["active"], json!(true));
}

/// The one herd delta a client could permanently miss.
///
/// `greet` read the herd, sent it, and *then* awaited a database round trip; only after that was
/// the update feed subscribed, at whatever version the model had reached by then. A rebuild
/// landing inside that window — and rebuilds are event-driven, so another client merely opening a
/// pane triggers one — left the client holding V while the feed started at V+1: the V→V+1 delta
/// was never sent and every later patch diffed against a model the client had never been given. A
/// pane added exactly then stayed invisible until it changed again.
#[tokio::test]
async fn a_herd_rebuilt_while_the_greeting_is_still_being_written_still_reaches_the_client() {
    let h = Harness::start(|_| {}).await;
    let device = h
        .node
        .auth
        .store()
        .create_device("greeted", Role::Full, kampr_auth::now(), None, None, None)
        .await
        .expect("a device");

    h.node.publish_herd(herd_of(&["w1:p1"]));
    let (node_side, client_side) = kampr_mesh::transport::pair();
    let (out, incoming) = node_side.split();
    let session = tokio::spawn(kampr_node::session::run_on(
        out,
        incoming,
        h.node.clone(),
        device,
        "test".into(),
        kampr_node::session::Caller::Client,
    ));
    // One poll is all it takes to reach the database read: everything before it is synchronous,
    // and the herd the client has just been handed is already on the wire.
    tokio::task::yield_now().await;
    h.node.publish_herd(herd_of(&["w1:p1", "w1:p2"]));

    let (_, mut from_node) = client_side.split();
    let mut seen: Vec<String> = Vec::new();
    let found = tokio::time::timeout(Duration::from_secs(5), async {
        while let Some(text) = from_node.recv().await {
            let msg: Value = serde_json::from_str(&text).expect("a server message");
            seen.push(msg["t"].as_str().unwrap_or_default().to_string());
            if mentions(&msg, "01J/w1:p2") {
                return true;
            }
        }
        false
    })
    .await
    .unwrap_or(false);
    session.abort();

    assert!(
        found,
        "the pane added during the greeting never reached the client; saw {seen:?}"
    );
}

fn herd_of(panes: &[&str]) -> kampr_node::herd::HerdModel {
    kampr_node::herd::HerdModel {
        nodes: Vec::new(),
        panes: panes
            .iter()
            .map(|id| {
                kampr_core::wire::PaneEntry::new(
                    "01J",
                    &kampr_core::provider::PaneInfo {
                        pane_id: (*id).to_string(),
                        ..kampr_core::provider::PaneInfo::default()
                    },
                    false,
                )
            })
            .collect(),
    }
}

/// A pane reaches a client in the greeting's `herd` or in a `herd.patch`, and either is fine —
/// what must not happen is neither.
fn mentions(msg: &Value, pane: &str) -> bool {
    let listed = |at: &Value| {
        at.as_array()
            .is_some_and(|panes| panes.iter().any(|p| p["id"] == pane))
    };
    listed(&msg["panes"]) || listed(&msg["added"]["panes"]) || listed(&msg["changed"]["panes"])
}

/// `enrolled` says whether this node is claimed, and it is the thing a first run branches on. The
/// read that answers it used to fail open — `unwrap_or(0) > 0` — so an unreadable store invited a
/// stranger to claim a node that is already somebody's.
#[tokio::test(flavor = "multi_thread")]
async fn a_node_that_cannot_read_its_devices_does_not_say_nobody_is_enrolled() {
    let h = Harness::start(|_| {}).await;
    h.token(Role::Full).await;
    let (status, body) = request(&h.origin, "GET", "/api/node", &[], None).await;
    assert!(status.contains("200"), "{status}");
    assert_eq!(body["enrolled"], json!(true));

    h.break_the_store(true).await;
    let (status, body) = request(&h.origin, "GET", "/api/node", &[], None).await;
    assert!(status.contains("500"), "{status}");
    assert_ne!(body["enrolled"], json!(false), "a guess is not an answer");
}

/// `/auth/pair` ran its argon2id pass — 19 MiB, memory-hard, tens of milliseconds — inline on
/// the executor, and nothing bounded how many ran at once. The per-peer
/// limiter cannot: source addresses are free, and a rotated one buys a fresh burst of five. So a
/// couple of hundred wrong codes was every tokio worker parked in a key derivation, and a node
/// with nothing else wrong with it stopped answering anything at all.
///
/// Two workers because that is a small VPS, and because a bound that only holds on sixteen cores
/// is not a bound.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_flood_of_wrong_pairing_codes_leaves_the_node_still_answering_everything_else() {
    let h = Harness::start(|c| c.server.trust_proxy = true).await;
    let body = json!({ "code": "ZZZZ-ZZZZ", "device_name": "flood" }).to_string();
    let flood: Vec<_> = (0..256)
        .map(|n| {
            let (origin, body) = (h.origin.clone(), body.clone());
            tokio::spawn(async move {
                let peer = format!("203.0.113.{n}");
                request(
                    &origin,
                    "POST",
                    "/auth/pair",
                    &[("x-forwarded-for", &peer)],
                    Some(&body),
                )
                .await
            })
        })
        .collect();
    tokio::time::sleep(Duration::from_millis(100)).await;

    let at = std::time::Instant::now();
    let (status, _) = request(&h.origin, "GET", "/healthz", &[], None).await;
    let took = at.elapsed();

    assert!(status.contains("200"), "the health check itself: {status}");
    assert!(
        took < Duration::from_millis(500),
        "a stranger's wrong pairing codes wedged the runtime: /healthz took {took:?}"
    );
    let answers: Vec<String> = futures_util::future::join_all(flood)
        .await
        .into_iter()
        .map(|a| a.expect("a request").0)
        .collect();
    assert!(
        answers.iter().all(|a| !a.contains("200")),
        "not one wrong code may pair"
    );
    // Refused, not queued, and this is what says so: without the bound every one of them is a
    // 401 that cost 19 MiB and a key derivation to answer. That the derivation also runs off the
    // worker is `redeeming_a_pairing_code_leaves_the_thread_that_asked_free` in `kampr-auth` —
    // this test passes on either half alone, and neither half is the whole fix.
    assert!(
        answers.iter().any(|a| a.contains("503")),
        "a flood has to hit the bound rather than each get its own 19 MiB: {:?}",
        answers.iter().collect::<std::collections::BTreeSet<_>>()
    );
}
