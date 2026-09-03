//! The fleet book over the wire: what the node remembers about commands, and where it keeps it.
//!
//! Nothing here needs a herd. A fleet run is a pty the node forked rather than a pane it asked
//! herdr for (`docs/13-fleet-runs.md`), so the herdr socket in this file deliberately does not
//! exist — which also means the operator's own session is never reachable from here.
//!
//! The load-bearing test is [`a_book_written_before_a_restart_is_there_after_it`]. "Persisted on
//! the server" is the whole of what was asked for, and an in-memory list that died with the
//! process would satisfy every other test in this file.

use futures_util::{SinkExt, StreamExt};
use kampr_auth::Role;
use kampr_node::{Config, Node, http};
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite};

type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// A node that can be stopped and started again on the same state directory, which is the only
/// way to ask the question this feature exists to answer.
struct Harness {
    home: tempfile::TempDir,
    node: Arc<Node>,
    origin: String,
    port: u16,
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
        Self::on(home, listener, port).await
    }

    /// The same state directory, a new process's worth of node. The listener is rebound to the
    /// port the first one had so a caller keeps one origin across the restart.
    async fn restart(mut self) -> Self {
        let home = std::mem::replace(&mut self.home, tempfile::tempdir().expect("a spare"));
        let port = self.port;
        drop(self);
        // The old server's socket is closed in `Drop`; the kernel may take a moment to say so.
        let listener = loop {
            match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(listener) => break listener,
                Err(_) => tokio::time::sleep(Duration::from_millis(50)).await,
            }
        };
        Self::on(home, listener, port).await
    }

    async fn on(home: tempfile::TempDir, listener: tokio::net::TcpListener, port: u16) -> Self {
        let config_dir = home.path().join("config");
        let state_dir = home.path().join("state");
        std::fs::create_dir_all(&state_dir).expect("a state dir");

        let mut config = Config::bootstrap("book");
        config.update.check = false;
        config.config_dir = config_dir.display().to_string();
        config.state_dir = state_dir.display().to_string();
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        // A socket and a binary that do not exist: this node has no herd and must not find the
        // operator's (#97).
        config.herdr.socket = home.path().join("herdr.sock").display().to_string();
        config.herdr.binary = home.path().join("no-such-herdr").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        // Never read the operator's login shell for a test's `PATH`: `$SHELL -lc` is a profile
        // this suite has no business paying for, on every run.
        config.fleet.path = "/usr/bin:/bin".into();
        config.auth.audit = true;
        // The node id is minted per `bootstrap`, and a restart that changed it would leave every
        // `fleet.run` in this file addressed at a node that no longer exists.
        config.node_id = "01JBOOKNODE".into();
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
            home,
            node,
            origin,
            port,
            server,
        }
    }

    fn audit(&self) -> String {
        std::fs::read_to_string(self.home.path().join("state").join("audit.jsonl")).unwrap_or_default()
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
        let body = json!({ "code": pairing.code, "device_name": "book" }).to_string();
        let response = post(&self.origin, "/auth/pair", &body).await;
        response["token"].as_str().expect("a token").to_string()
    }

    async fn connect(&self, token: &str) -> Client {
        let url = self.origin.replacen("http", "ws", 1) + "/ws";
        let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("kampr.token.{token}").parse().unwrap(),
        );
        let (socket, _) = tokio_tungstenite::connect_async(request)
            .await
            .expect("a websocket");
        Client { socket }
    }
}

struct Client {
    socket: Socket,
}

impl Client {
    async fn send(&mut self, value: Value) {
        self.socket
            .send(tungstenite::Message::Text(value.to_string().into()))
            .await
            .expect("a send");
    }

    /// The next frame with this `t`, or a panic naming what did arrive. Every read in this file is
    /// bounded: a book frame that never comes must fail the test rather than hang the suite.
    async fn expect(&mut self, tag: &str) -> Value {
        let mut seen = Vec::new();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(10);
        while let Ok(Some(Ok(message))) = tokio::time::timeout_at(deadline, self.socket.next()).await {
            let tungstenite::Message::Text(text) = message else {
                continue;
            };
            let value: Value = serde_json::from_str(&text).expect("a frame");
            let t = value["t"].as_str().unwrap_or_default().to_string();
            if t == tag {
                return value;
            }
            seen.push(t);
        }
        panic!("no `{tag}` frame arrived; saw {seen:?}");
    }

    /// The greeting's book, which is the one a client actually reads: it is pushed unasked, so a
    /// client never has to ask for the memory it is about to render.
    async fn greeting_book(&mut self) -> Value {
        self.expect("fleet.book").await
    }

    async fn run(&mut self, args: &[&str]) -> Value {
        self.send(json!({
            "t": "manage", "op": "fleet.run", "node": "01JBOOKNODE",
            "cohort": ulid::Ulid::generate().to_string(),
            "args": args,
        }))
        .await;
        self.expect("managed").await
    }

    /// A run the way a client sends one now: the line the operator typed, for the host's own shell.
    async fn typed(&mut self, command: &str) -> Value {
        self.send(json!({
            "t": "manage", "op": "fleet.run", "node": "01JBOOKNODE",
            "cohort": ulid::Ulid::generate().to_string(),
            "command": command,
        }))
        .await;
        self.expect("managed").await
    }

    async fn manage(&mut self, op: Value) -> Value {
        self.send(op).await;
        self.expect("managed").await
    }
}

fn commands(book: &Value, list: &str) -> Vec<Vec<String>> {
    book[list]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| {
            entry["args"]
                .as_array()
                .expect("an argv")
                .iter()
                .map(|a| a.as_str().unwrap_or_default().to_string())
                .collect()
        })
        .collect()
}

fn ids(book: &Value, list: &str) -> Vec<String> {
    book[list]
        .as_array()
        .expect("a list")
        .iter()
        .map(|entry| entry["id"].as_str().expect("an id").to_string())
        .collect()
}

async fn post(origin: &str, path: &str, body: &str) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = origin.trim_start_matches("http://");
    let (host, port) = rest.split_once(':').unwrap_or((rest, "80"));
    let request = format!(
        "POST {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
         Content-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
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

/// **The one this feature was asked for.** A list that lived in the process would pass every other
/// test here and fail the request.
#[tokio::test]
async fn a_book_written_before_a_restart_is_there_after_it() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;
    client.run(&["uptime"]).await;
    client
        .manage(
            json!({ "t": "manage", "op": "fleet.save", "args": ["kampr", "update"],
                        "label": "update everything" }),
        )
        .await;
    drop(client);

    let harness = harness.restart().await;
    // A *new* pairing, so this is the second device the operator picked up rather than the same
    // one reconnecting. A book kept per device — which is how every other row in this schema is
    // kept — would be empty here, and that is exactly the defect: the request was for a list that
    // follows the operator between their phone and their desktop.
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    let book = client.greeting_book().await;
    assert_eq!(commands(&book, "recent"), vec![vec!["uptime".to_string()]]);
    assert_eq!(
        commands(&book, "saved"),
        vec![vec!["kampr".to_string(), "update".to_string()]]
    );
    assert_eq!(book["saved"][0]["label"], "update everything");
}

#[tokio::test]
async fn a_fresh_node_greets_a_client_with_an_empty_book_rather_than_nothing() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    let book = client.greeting_book().await;
    assert_eq!(book["recent"], json!([]));
    assert_eq!(book["saved"], json!([]));
}

/// **Both shapes of `fleet.run`, on one node, in one session.**
///
/// `command` is the line the operator typed and `args` is the argv an older client still sends;
/// the field is additive and the older one is not deprecated, because the older clients are
/// installed on real phones. The book holds a typed line as its **single** argument, so a client
/// that renders an entry by joining `args` with spaces — which every client does, including ones
/// that have never heard of `command` — reads it back byte for byte, pipe and quotes included.
#[tokio::test]
async fn a_typed_line_is_remembered_whole_and_an_older_clients_argv_still_runs() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;

    let line = r#"echo "one two" | cat"#;
    let typed = client.typed(line).await;
    assert_eq!(typed["ok"], json!(true), "a typed line was refused: {typed}");
    client.expect("fleet.book").await;

    let older = client.run(&["echo", "three"]).await;
    assert_eq!(
        older["ok"],
        json!(true),
        "an older client's argv was refused: {older}"
    );
    let book = client.expect("fleet.book").await;

    assert_eq!(
        commands(&book, "recent"),
        vec![vec!["echo", "three"], vec![line]],
        "a typed line must be one entry holding the whole line, and an argv must be unchanged",
    );
}

/// A `fleet.run` carrying neither is refused with a sentence naming both, rather than started as
/// an empty command.
#[tokio::test]
async fn a_run_with_nothing_to_run_is_refused_and_says_what_would_have_worked() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;
    let refused = client
        .manage(json!({
            "t": "manage", "op": "fleet.run", "node": "01JBOOKNODE",
            "cohort": ulid::Ulid::generate().to_string(),
        }))
        .await;
    let message = refused["message"].as_str().unwrap_or_default();
    assert!(message.contains("command"), "{refused}");
    assert!(message.contains("args"), "{refused}");
}

/// One run is one `fleet.run` per reachable host, all carrying the same cohort. The history is a
/// list of commands, not of runs, so a fan-out is one entry however many hosts it reached.
#[tokio::test]
async fn a_command_run_again_moves_to_the_top_instead_of_being_listed_twice() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;
    client.run(&["echo", "one"]).await;
    client.run(&["echo", "two"]).await;
    client.run(&["echo", "one"]).await;
    let book = client.expect("fleet.book").await;
    assert_eq!(
        commands(&book, "recent"),
        vec![vec!["echo", "one"], vec!["echo", "two"]]
    );
}

#[tokio::test]
async fn the_history_stops_at_five() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;
    for n in 0..8 {
        client.run(&["echo", &n.to_string()]).await;
        client.expect("fleet.book").await;
    }
    // Read off a fresh connection's greeting rather than the last frame this one happened to
    // receive: the book is published from a watch, so a socket mid-flight can be a change behind.
    let mut second = harness.connect(&token).await;
    let book = second.greeting_book().await;
    assert_eq!(book["recent"].as_array().expect("a list").len(), 5);
    assert_eq!(commands(&book, "recent")[0], vec!["echo", "7"]);
}

/// The defect: a command in both lists at once, which reads as the book having lost track of
/// itself. Promoting moves the entry, and running a saved command leaves it saved.
#[tokio::test]
async fn a_command_promoted_to_saved_is_not_also_in_the_history() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;
    client.run(&["uptime"]).await;
    let book = client.expect("fleet.book").await;
    let id = ids(&book, "recent").remove(0);

    client
        .manage(json!({ "t": "manage", "op": "fleet.save", "entry": id, "label": "load" }))
        .await;
    let book = client.expect("fleet.book").await;
    assert_eq!(commands(&book, "recent"), Vec::<Vec<String>>::new());
    assert_eq!(commands(&book, "saved"), vec![vec!["uptime".to_string()]]);

    client.run(&["uptime"]).await;
    let book = client.expect("fleet.book").await;
    assert_eq!(commands(&book, "recent"), Vec::<Vec<String>>::new());
    assert_eq!(commands(&book, "saved"), vec![vec!["uptime".to_string()]]);
}

/// The operator must be able to delete anything the node wrote down, because the rule that keeps
/// secrets out of the book is a reduction and not a filter.
#[tokio::test]
async fn every_entry_can_be_deleted() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;
    client.run(&["echo", "hunter2"]).await;
    let book = client.expect("fleet.book").await;
    let id = ids(&book, "recent").remove(0);
    let ack = client
        .manage(json!({ "t": "manage", "op": "fleet.drop", "entry": id }))
        .await;
    assert_eq!(ack["ok"], true);
    let book = client.expect("fleet.book").await;
    assert_eq!(book["recent"], json!([]));
}

/// Nothing pressed anything to ask for the history, so the automatic half declines what it can
/// recognise. An explicit save is the operator saying they mean it, and is allowed — the client
/// warns rather than refuses, and this asserts the node agrees with that division.
#[tokio::test]
async fn a_command_carrying_a_secret_is_not_written_down_by_itself_and_can_still_be_saved() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;
    let ack = client.run(&["env", "TOKEN=hunter2", "./deploy"]).await;
    assert_eq!(ack["ok"], true, "the run itself must not be refused");
    client.run(&["uptime"]).await;
    let book = client.expect("fleet.book").await;
    assert_eq!(
        commands(&book, "recent"),
        vec![vec!["uptime".to_string()]],
        "a secret-shaped command was written to disk without anybody asking"
    );

    client
        .manage(json!({ "t": "manage", "op": "fleet.save",
                        "args": ["env", "TOKEN=hunter2", "./deploy"] }))
        .await;
    let book = client.expect("fleet.book").await;
    assert_eq!(commands(&book, "saved").len(), 1);
}

/// A read-only device sees the book — it can already read every command in every pane — and
/// writes nothing to it. The refusal is the ordinary `manage` one, so a client waiting on an ack
/// gets one.
#[tokio::test]
async fn a_read_only_device_reads_the_book_and_cannot_write_it() {
    let harness = Harness::start().await;
    let writer = harness.token(Role::Full).await;
    let mut author = harness.connect(&writer).await;
    author.greeting_book().await;
    author
        .manage(json!({ "t": "manage", "op": "fleet.save", "args": ["uptime"] }))
        .await;

    let watcher = harness.token(Role::Readonly).await;
    let mut client = harness.connect(&watcher).await;
    let book = client.greeting_book().await;
    assert_eq!(commands(&book, "saved"), vec![vec!["uptime".to_string()]]);
    let ack = client
        .manage(json!({ "t": "manage", "op": "fleet.save", "args": ["rm", "-rf", "/"] }))
        .await;
    assert_eq!(ack["ok"], false);
    assert_eq!(ack["code"], "not_writer");
}

/// The book is a node's, not a herd's: `fleet.save` names no host and must never be relayed to
/// one. A book op that took the peer path would be stored on whichever machine the operator's
/// last pane happened to live on.
#[tokio::test]
async fn a_book_op_naming_a_node_that_is_not_this_one_is_still_this_nodes_book() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;
    let ack = client
        .manage(
            json!({ "t": "manage", "op": "fleet.save", "node": "01JSOMEONEELSE",
                        "args": ["uptime"] }),
        )
        .await;
    assert_eq!(ack["ok"], true);
    let book = client.expect("fleet.book").await;
    assert_eq!(commands(&book, "saved"), vec![vec!["uptime".to_string()]]);
}

#[tokio::test]
async fn saving_and_dropping_are_audited_with_the_command_they_were_about() {
    let harness = Harness::start().await;
    let token = harness.token(Role::Full).await;
    let mut client = harness.connect(&token).await;
    client.greeting_book().await;
    client
        .manage(json!({ "t": "manage", "op": "fleet.save", "args": ["kampr", "update"] }))
        .await;
    let book = client.expect("fleet.book").await;
    let id = ids(&book, "saved").remove(0);
    client
        .manage(json!({ "t": "manage", "op": "fleet.drop", "entry": id }))
        .await;
    client.expect("fleet.book").await;

    let audit = harness.audit();
    assert!(
        audit.contains("fleet.save") && audit.contains("\"kampr\""),
        "a save that put a command on this machine's disk left no trail: {audit}"
    );
    assert!(audit.contains("fleet.drop"), "a delete left no trail: {audit}");
}

/// The rule that keeps the automatic half of the book from writing credentials to disk, and the
/// shapes it cannot see. `client/shared/src/commonTest/.../FleetSecretTest.kt` reads the same
/// file: a client that warned about a different set than the node declines would be worse than
/// one that said nothing.
#[test]
fn the_secret_shapes_both_clients_agree_on() {
    let raw = include_str!("../fixtures/secretish.json");
    let fixture: Value = serde_json::from_str(raw).expect("the fixture is JSON");
    let argv = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .expect("an argv")
            .iter()
            .map(|a| a.as_str().expect("a word").to_string())
            .collect()
    };

    // **Both shapes of the same command.** A run is a command line now and a book entry holds it
    // as its single argument, so a rule that only reads argv would decline to write down
    // `TOKEN=abc ./deploy` and cheerfully write down the identical line an operator typed.
    let both_ways = |value: &Value| -> Vec<Vec<String>> {
        let words = argv(value);
        vec![words.clone(), vec![words.join(" ")]]
    };

    let caught = fixture["caught"].as_object().expect("caught cases");
    assert!(
        caught.len() >= 12,
        "the caught set must cover every shape the rule claims"
    );
    for (name, case) in caught {
        let words = argv(&case["args"]);
        assert_eq!(
            kampr_fleet::secretish(&words).as_deref(),
            case["why"].as_str(),
            "{name} is the shape this rule exists for"
        );
        // The joined shape asserts that it still **fires**, not which word said so: read as one
        // line, `curl --oauth2-bearer eyJ` is caught by the bearer marker rather than by the flag,
        // and naming a different word is not the same as missing the credential.
        let line = vec![words.join(" ")];
        assert!(
            kampr_fleet::secretish(&line).is_some(),
            "{name} typed as one line went unnoticed: {line:?}"
        );
    }
    for (name, case) in fixture["missed"].as_object().expect("missed cases") {
        for shape in both_ways(case) {
            assert_eq!(
                kampr_fleet::secretish(&shape),
                None,
                "{name} is a documented blind spot; if it is now caught, say so in the fixture"
            );
        }
    }
    for (name, case) in fixture["clean"].as_object().expect("clean cases") {
        for shape in both_ways(case) {
            assert_eq!(
                kampr_fleet::secretish(&shape),
                None,
                "{name} is not a secret, and a rule that cries wolf is one nobody believes"
            );
        }
    }
}
