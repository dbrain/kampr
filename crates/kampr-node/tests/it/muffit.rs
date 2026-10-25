//! Muffit sessions over the wire (docs/18): a node beside a muffit daemon, here a fake one that
//! speaks the daemon's own frames, with no herdr anywhere.

use futures_util::{SinkExt, StreamExt};
use kampr_auth::Role;
use kampr_node::{Config, Node, http};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream, tungstenite};

mod fake;
mod live;

use fake::{Daemon, Fake, TOKEN, session, stored, text};

const NODE: &str = "01JMUFFITNODE";
const SOON: Duration = Duration::from_secs(10);

fn pane(id: &str) -> String {
    format!("{NODE}/m:{id}")
}

struct Harness {
    _home: tempfile::TempDir,
    node: Arc<Node>,
    origin: String,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.node.shutdown();
        self.server.abort();
    }
}

impl Harness {
    async fn beside(url: &str, token: &str) -> Self {
        let home = tempfile::Builder::new().prefix("km").tempdir_in("/tmp").unwrap();
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let config_dir = home.path().join("config");
        let state_dir = home.path().join("state");
        std::fs::create_dir_all(&state_dir).unwrap();
        let mut config = Config::bootstrap("muffit");
        config.update.check = false;
        config.config_dir = config_dir.display().to_string();
        config.state_dir = state_dir.display().to_string();
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.herdr.socket = home.path().join("herdr.sock").display().to_string();
        config.herdr.binary = home.path().join("no-such-herdr").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        config.native.root = home.path().join("native").display().to_string();
        config.journals.home = home.path().display().to_string();
        config.muffit.url = url.to_string();
        config.muffit.token = token.to_string();
        config.node_id = NODE.into();
        config.save(&config_dir).unwrap();
        let origin = config.origin();
        let node = Node::start(config, &state_dir).await.expect("a node");
        let server = tokio::spawn({
            let app = http::router(node.clone());
            async move {
                let _ = http::serve_on(listener, app).await;
            }
        });
        Self {
            _home: home,
            node,
            origin,
            server,
        }
    }

    async fn start(fake: &Daemon) -> Self {
        Self::beside(&fake.url, TOKEN).await
    }

    async fn token(&self) -> String {
        let pairing = self
            .node
            .auth
            .create_pairing(Role::Full, kampr_auth::Delivery::Console)
            .await
            .unwrap();
        if !pairing.armed {
            assert!(self.node.auth.arm_pairing(&pairing.code).await.unwrap());
        }
        let body = json!({ "code": pairing.code, "device_name": "muffit" }).to_string();
        request(&self.origin, "POST", "/auth/pair", None, &body).await["token"]
            .as_str()
            .expect("a token")
            .to_string()
    }

    async fn client(&self) -> Client {
        let token = self.token().await;
        let url = self.origin.replacen("http", "ws", 1) + "/ws";
        let mut request = tungstenite::client::IntoClientRequest::into_client_request(url).unwrap();
        request.headers_mut().insert(
            "sec-websocket-protocol",
            format!("kampr.token.{token}").parse().unwrap(),
        );
        let (socket, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        Client {
            socket,
            token,
            nodes: HashMap::new(),
            panes: HashMap::new(),
            frames: Vec::new(),
            log: Vec::new(),
        }
    }
}

struct Client {
    socket: WebSocketStream<MaybeTlsStream<TcpStream>>,
    token: String,
    nodes: HashMap<String, Value>,
    panes: HashMap<String, Value>,
    frames: Vec<Value>,
    log: Vec<Value>,
}

impl Client {
    async fn send(&mut self, value: Value) {
        self.socket
            .send(tungstenite::Message::Text(value.to_string().into()))
            .await
            .unwrap();
    }

    async fn pump(&mut self, within: Duration) {
        let Ok(Some(Ok(tungstenite::Message::Text(text)))) =
            tokio::time::timeout(within, self.socket.next()).await
        else {
            return;
        };
        let value: Value = serde_json::from_str(&text).unwrap();
        match value["t"].as_str().unwrap_or_default() {
            "herd" => {
                self.panes.clear();
                for n in value["nodes"].as_array().into_iter().flatten() {
                    self.nodes.insert(n["id"].as_str().unwrap().into(), n.clone());
                }
                for p in value["panes"].as_array().into_iter().flatten() {
                    self.panes.insert(p["id"].as_str().unwrap().into(), p.clone());
                }
            }
            "herd.patch" => {
                for list in ["added", "changed"] {
                    for p in value[list]["panes"].as_array().into_iter().flatten() {
                        self.panes.insert(p["id"].as_str().unwrap().into(), p.clone());
                    }
                    for n in value[list]["nodes"].as_array().into_iter().flatten() {
                        self.nodes.insert(n["id"].as_str().unwrap().into(), n.clone());
                    }
                }
                for id in value["removed_ids"].as_array().into_iter().flatten() {
                    self.panes.remove(id.as_str().unwrap());
                }
            }
            _ => {
                self.log.push(value.clone());
                self.frames.push(value);
            }
        }
    }

    async fn until<T>(&mut self, what: &str, mut found: impl FnMut(&mut Self) -> Option<T>) -> T {
        let deadline = tokio::time::Instant::now() + SOON;
        loop {
            if let Some(t) = found(self) {
                return t;
            }
            let left = deadline.saturating_duration_since(tokio::time::Instant::now());
            assert!(
                !left.is_zero(),
                "{what} never happened; herd: {:?}; frames: {:?}",
                self.panes,
                self.frames
            );
            self.pump(left.min(Duration::from_millis(100))).await;
        }
    }

    async fn frame(&mut self, what: &str, want: impl Fn(&Value) -> bool) -> Value {
        self.until(what, |c| {
            let at = c.frames.iter().position(&want)?;
            Some(c.frames.remove(at))
        })
        .await
    }

    async fn pane(&mut self, id: &str, want: impl Fn(&Value) -> bool) -> Value {
        self.until(&format!("{id} as wanted"), |c| {
            c.panes.get(id).filter(|p| want(p)).cloned()
        })
        .await
    }

    async fn watch(&mut self, id: &str) {
        self.pane(id, |_| true).await;
        self.send(json!({ "t": "watch", "pane": id, "scrollback": true, "conversation": true }))
            .await;
    }

    async fn convo(&mut self, id: &str) -> Value {
        self.frame("a convo page", |f| {
            f["t"] == "convo" && f["pane"] == id && f["sub"].is_null()
        })
        .await
    }

    async fn turn(&mut self, what: &str, want: impl Fn(&Value) -> bool) -> Value {
        self.frame(what, |f| {
            f["t"] == "convo.turn" && f["turns"].as_array().into_iter().flatten().any(&want)
        })
        .await
    }

    async fn input(&mut self, id: &str, text: &str) {
        self.send(json!({ "t": "input", "pane": id, "text": text })).await;
    }
}

async fn eventually<T>(what: &str, mut found: impl FnMut() -> Option<T>) -> T {
    let deadline = tokio::time::Instant::now() + SOON;
    loop {
        if let Some(t) = found() {
            return t;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what} never happened");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn request(origin: &str, method: &str, path: &str, token: Option<&str>, body: &str) -> Value {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = origin.trim_start_matches("http://");
    let auth = token
        .map(|t| format!("Authorization: Bearer {t}\r\n"))
        .unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {rest}\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    let mut stream = TcpStream::connect(rest).await.unwrap();
    stream.write_all(request.as_bytes()).await.unwrap();
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.unwrap();
    let text = String::from_utf8_lossy(&response).to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or_default();
    serde_json::from_str(body.trim()).unwrap_or(Value::Null)
}

fn ids(frame: &Value) -> Vec<String> {
    frame["turns"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|t| t["id"].as_str().unwrap().to_string())
        .collect()
}

fn asked(fake: &Fake) {
    let seen = fake.seen();
    assert!(!seen.is_empty(), "the node never reached the daemon");
    for s in &seen {
        assert_eq!(s.origin, None, "{} {} carried an Origin", s.method, s.path);
        assert_eq!(
            s.authorization.as_deref(),
            Some(&*format!("Bearer {TOKEN}")),
            "{} {} without the bearer",
            s.method,
            s.path
        );
    }
}

#[tokio::test]
async fn a_live_session_is_a_conversation_pane_of_the_herd() {
    let fake = Fake::start(vec![
        session("s1", "waiting_user", Some("hello kampr")),
        session("s2", "running", None),
        session("s3", "stopped", Some("over")),
    ])
    .await;
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;

    let s1 = client.pane(&pane("s1"), |_| true).await;
    for (field, want) in [
        ("provider", json!("muffit")),
        ("agent", json!("muffit")),
        ("session_id", json!("s1")),
        ("converses", json!(true)),
        ("has_conversation", json!(true)),
        ("label", json!("hello kampr")),
        ("cwd", json!("/srv/work")),
        ("rows", json!(0)),
        ("agent_status", json!("done")),
    ] {
        assert_eq!(s1[field], want, "{field}: {s1}");
    }
    assert!(s1.get("cols").is_none(), "{s1}");
    let s2 = client.pane(&pane("s2"), |_| true).await;
    assert_eq!(s2["agent_status"], "working", "{s2}");
    assert!(
        !client.panes.contains_key(&pane("s3")),
        "a stopped session is not a pane"
    );

    fake.update("s1", |row| {
        row["permission"] = json!({ "action": "run `cargo install ripgrep`", "id": 1, "tool": "bash" });
    });
    client.pane(&pane("s1"), |p| p["agent_status"] == "blocked").await;
    fake.update("s1", |row| {
        row["permission"] = Value::Null;
        row["state"] = json!("paused");
    });
    client.pane(&pane("s1"), |p| p["agent_status"] == "idle").await;
    fake.update("s2", |row| row["state"] = json!("stopped"));
    client
        .until("a stopped session leaves the herd", |c| {
            (!c.panes.contains_key(&pane("s2"))).then_some(())
        })
        .await;
    asked(&fake);
}

#[tokio::test]
async fn watching_serves_the_transcript_then_the_live_turn_then_what_was_recorded() {
    let fake = Fake::start(vec![session("s1", "waiting_user", Some("hello kampr"))]).await;
    fake.set_transcript(
        "s1",
        vec![
            text(0, "system", "You are muffit"),
            text(1, "user", "hello"),
            text(2, "assistant", "[mock] hello"),
        ],
    );
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;

    let page = client.convo(&id).await;
    assert_eq!(page["fresh"], json!(true), "{page}");
    assert_eq!(ids(&page), ["m1", "m2"], "{page}");
    assert_eq!(page["turns"][0]["role"], "user");
    assert_eq!(
        page["turns"][1]["blocks"][0],
        json!({ "b": "md", "text": "[mock] hello" })
    );
    let facets = client.frame("facets", |f| f["t"] == "convo.facets").await;
    assert_eq!(facets["facets"]["title"]["text"], "hello kampr", "{facets}");
    eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;
    let hello = &fake.attach_frames("hello")[0];
    for capability in ["input", "history", "status", "subscribe", "permissions"] {
        assert!(
            hello["capabilities"]
                .as_array()
                .unwrap()
                .contains(&json!(capability)),
            "{hello}"
        );
    }

    fake.push(json!({ "type": "state", "state": "running" }));
    fake.push(json!({ "type": "content", "blocks": [
        { "type": "user_message", "text": "second message" }, { "type": "recorded", "seq": 3 } ] }));
    fake.append("s1", vec![text(3, "user", "second message")]);
    fake.push(json!({ "type": "content", "blocks": [
        { "type": "tool_call", "id": "c1", "tool": "bash", "args": "{\"command\":\"ls\"}", "status": "started" },
        { "type": "tool_output_chunk", "call_id": "c1", "stream": "stdout", "text": "a.rs\n", "seq": 0 } ] }));
    let live = client
        .turn("the live turn", |t| {
            t["id"] == "live" && !t["blocks"].as_array().unwrap().is_empty()
        })
        .await;
    let blocks = &live["turns"][0]["blocks"];
    assert_eq!(blocks[0]["b"], "tool", "{live}");
    assert_eq!(blocks[0]["state"], "running", "{live}");
    assert_eq!(
        blocks[1],
        json!({ "b": "code", "text": "a.rs", "role": "output" }),
        "{live}"
    );

    fake.append(
        "s1",
        vec![
            json!({ "seq": 4, "role": "assistant", "blocks": [
                { "type": "tool_call", "id": "c1", "tool": "bash", "args": "{\"command\":\"ls\"}", "status": "finished" } ] }),
            json!({ "seq": 5, "role": "tool", "blocks": [
                { "type": "tool_result", "call_id": "c1", "summary": "a.rs", "exit_status": 0, "truncated": false } ] }),
            text(6, "assistant", "One file."),
        ],
    );
    fake.push(json!({ "type": "content", "blocks": [
        { "type": "tool_result", "call_id": "c1", "summary": "a.rs", "truncated": false },
        { "type": "text", "text": "One file." }, { "type": "recorded", "seq": 6 } ] }));
    let recorded = client.turn("the recorded turns", |t| t["id"] == "m6").await;
    fake.push(json!({ "type": "turn", "outcome": "completed", "reason": null }));
    let m4 = recorded["turns"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "m4")
        .unwrap_or_else(|| panic!("m4 beside m6: {recorded}"));
    assert_eq!(m4["blocks"][0]["state"], "done", "{recorded}");
    assert_eq!(m4["blocks"][1]["text"], "a.rs", "{recorded}");
    client
        .turn("the live turn withdrawn", |t| {
            t["id"] == "live" && t["blocks"] == json!([])
        })
        .await;
    let user = client
        .frames
        .iter()
        .chain(std::iter::once(&recorded))
        .any(|f| f["t"] == "convo.turn" && ids(f).contains(&"m3".to_string()));
    assert!(
        user || ids(&recorded).contains(&"m3".to_string()),
        "the prompt reached the client as m3"
    );
    assert!(
        !client.frames.iter().any(|f| f["t"] == "grid.reset"),
        "a muffit pane has no terminal to paint"
    );
    asked(&fake);
}

#[tokio::test]
async fn a_permission_prompt_is_pending_and_answered_over_rest() {
    let fake = Fake::start(vec![session("s1", "running", Some("install ripgrep"))]).await;
    fake.set_transcript("s1", vec![text(1, "user", "install ripgrep")]);
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;

    let prompt = json!({ "action": "run `cargo install ripgrep` with write access outside the project",
                         "id": 1, "tool": "bash" });
    fake.update("s1", |row| row["permission"] = prompt.clone());
    fake.status(prompt.clone(), json!([]));
    let pending = client
        .frame("the prompt", |f| f["t"] == "pending" && !f["question"].is_null())
        .await;
    assert_eq!(pending["source"], "transcript", "{pending}");
    assert_eq!(pending["header"], "bash", "{pending}");
    assert_eq!(pending["question"], prompt["action"], "{pending}");
    assert_eq!(
        pending["options"],
        json!([{ "key": "1", "label": "Allow" }, { "key": "2", "label": "Always allow" },
               { "key": "3", "label": "Deny" }]),
        "{pending}"
    );
    client.pane(&id, |p| p["agent_status"] == "blocked").await;

    let warm = request(
        &h.origin,
        "GET",
        &format!("/api/warm?pane={}", id.replace('/', "%2F")),
        Some(&client.token),
        "",
    )
    .await;
    assert_eq!(warm["pending"]["question"], prompt["action"], "{warm}");

    client.frames.retain(|f| f["t"] != "pending");
    client
        .send(json!({ "t": "answer", "pane": id, "key": "1" }))
        .await;
    eventually("the answer posted", || {
        let posts = fake.posts("/sessions/s1/permission");
        (!posts.is_empty()).then_some(posts)
    })
    .await;
    assert_eq!(
        fake.posts("/sessions/s1/permission"),
        [json!({ "id": 1, "answer": "allow" })]
    );
    fake.status(Value::Null, json!([]));
    client
        .frame("the prompt cleared", |f| {
            f["t"] == "pending" && f["question"].is_null()
        })
        .await;

    tokio::time::sleep(Duration::from_millis(100)).await;
    client.pump(Duration::from_millis(50)).await;
    client.frames.retain(|f| f["t"] != "pending");
    client
        .send(json!({ "t": "answer", "pane": id, "key": "3" }))
        .await;
    client
        .frame("an answer nobody is waiting on clears the strip", |f| {
            f["t"] == "pending" && f["question"].is_null() && f["options"] == json!([])
        })
        .await;
    assert_eq!(
        fake.posts("/sessions/s1/permission").len(),
        2,
        "an answer to a prompt the daemon has let go of is still asked, and its 409 clears the strip"
    );
    assert!(
        fake.attach_frames("input").is_empty(),
        "an answer is never keystrokes"
    );
}

#[tokio::test]
async fn the_composer_line_is_one_input_and_ctrl_c_interrupts() {
    let fake = Fake::start(vec![session("s1", "waiting_user", None)]).await;
    fake.set_transcript("s1", vec![]);
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;

    client.input(&id, "run the tests").await;
    client.input(&id, "\u{1b}[D").await;
    client.input(&id, " please").await;
    client.input(&id, "\r").await;
    let inputs = eventually("one input", || {
        let inputs = fake.attach_frames("input");
        (!inputs.is_empty()).then_some(inputs)
    })
    .await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let inputs = fake
        .attach_frames("input")
        .into_iter()
        .chain(inputs)
        .take(1)
        .collect::<Vec<_>>();
    assert_eq!(fake.attach_frames("input").len(), 1, "{inputs:?}");
    assert_eq!(inputs[0]["text"], "run the tests please", "{inputs:?}");
    assert!(
        inputs[0]["msg_id"].as_str().is_some_and(|m| !m.is_empty()),
        "{inputs:?}"
    );
    assert!(
        fake.posts("/sessions/s1/input").is_empty(),
        "the text went over the socket, with its msg_id"
    );

    client.input(&id, "\u{3}").await;
    eventually("an interrupt", || {
        (!fake.posts("/sessions/s1/interrupt").is_empty()).then_some(())
    })
    .await;
    client
        .send(json!({ "t": "input", "pane": id, "keys": ["Escape"] }))
        .await;
    eventually("escape interrupts too", || {
        (fake.posts("/sessions/s1/interrupt").len() == 2).then_some(())
    })
    .await;
    assert_eq!(fake.attach_frames("input").len(), 1);
}

#[tokio::test]
async fn convo_load_pages_back_and_convo_find_searches_what_was_read() {
    let fake = Fake::start(vec![session("s1", "waiting_user", None)]).await;
    fake.set_transcript(
        "s1",
        (0..150)
            .map(|n| {
                text(
                    n,
                    if n % 2 == 0 { "user" } else { "assistant" },
                    &format!("message {n}"),
                )
            })
            .collect(),
    );
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    let page = client.convo(&id).await;
    assert_eq!(ids(&page).len(), 100, "{page}");
    assert_eq!(ids(&page)[0], "m50");
    assert_eq!(page["more"], json!(true));
    assert_eq!(page["cursor"], "m50");

    client
        .send(json!({ "t": "convo.load", "pane": id, "before": "m50" }))
        .await;
    let older = client.convo(&id).await;
    assert_eq!(ids(&older).len(), 50, "{older}");
    assert_eq!(ids(&older)[0], "m0");
    assert_eq!(older["more"], json!(false), "{older}");
    assert!(older.get("cursor").is_none(), "{older}");
    assert!(older.get("fresh").is_none(), "an older page merges: {older}");

    client
        .send(json!({ "t": "convo.find", "pane": id, "query": "message 7" }))
        .await;
    let found = client.frame("a search", |f| f["t"] == "convo.find").await;
    let turns: Vec<&str> = found["matches"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["turn"].as_str().unwrap())
        .collect();
    assert!(turns.contains(&"m7") && turns.contains(&"m75"), "{found}");
}

#[tokio::test]
async fn a_worker_is_a_subconversation_followed_while_open() {
    let fake = Fake::start(vec![session("s1", "running", None)]).await;
    fake.set_transcript(
        "s1",
        vec![json!({ "seq": 1, "role": "assistant", "blocks": [
            { "type": "tool_call", "id": "call_7", "tool": "delegate",
              "args": "{\"brief\":\"Create made.txt containing hello.\",\"title\":\"Make the file\"}",
              "status": "finished" } ] })],
    );
    fake.set_transcript(
        "s1.s1",
        vec![text(0, "user", "Create made.txt containing hello.")],
    );
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;
    fake.status(
        Value::Null,
        json!([{ "call_id": "call_7", "id": "s1.s1", "liveness": "Alive", "role": "TaskChimp",
                 "state": "running", "task_id": "t-1", "title": "Make the file" }]),
    );
    let revised = client
        .turn("the worker offered", |t| {
            t["id"] == "m1" && t["blocks"].as_array().unwrap().iter().any(|b| b["b"] == "sub")
        })
        .await;
    let sub = revised["turns"][0]["blocks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|b| b["b"] == "sub")
        .unwrap()
        .clone();
    assert_eq!(
        sub,
        json!({ "b": "sub", "id": "s1.s1", "kind": "worker", "title": "Make the file" })
    );
    let facets = client
        .frame("the worker running", |f| {
            f["t"] == "convo.facets" && !f["facets"]["running"].is_null()
        })
        .await;
    assert_eq!(
        facets["facets"]["running"],
        json!([{ "call": "call_7", "kind": "worker", "title": "Make the file" }])
    );

    client
        .send(json!({ "t": "convo.sub", "pane": id, "id": "s1.s1" }))
        .await;
    let page = client
        .frame("the worker's page", |f| f["t"] == "convo" && f["sub"] == "s1.s1")
        .await;
    assert_eq!(page["fresh"], json!(true));
    assert_eq!(ids(&page), ["m0"]);
    fake.append("s1.s1", vec![text(1, "assistant", "Created made.txt.")]);
    let grown = client
        .frame("the worker followed", |f| {
            f["t"] == "convo.turn" && f["sub"] == "s1.s1"
        })
        .await;
    assert_eq!(ids(&grown), ["m1"], "{grown}");

    client
        .send(json!({ "t": "convo.sub", "pane": id, "id": "other.s1" }))
        .await;
    let refused = client.frame("a refusal", |f| f["t"] == "error").await;
    assert_eq!(refused["code"], "not_found", "{refused}");
}

#[tokio::test]
async fn muffit_create_makes_a_pane_and_close_stops_it() {
    let fake = Fake::start(vec![]).await;
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let hello = client.frame("hello", |f| f["t"] == "hello").await;
    assert_eq!(hello["caps"]["muffit"], json!(true), "{hello}");
    let node = client.until("this node", |c| c.nodes.get(NODE).cloned()).await;
    assert_eq!(node["muffit"], json!(true), "{node}");
    let home = h._home.path().display().to_string();
    let cases = [
        (
            json!({ "op": "muffit.create", "cwd": "/srv/work", "prompt": "fix the parser" }),
            json!({ "cwd": "/srv/work", "prompt": "fix the parser" }),
            "close",
        ),
        (
            json!({ "op": "muffit.create", "node": NODE, "prompt": "and the lexer" }),
            json!({ "cwd": home, "prompt": "and the lexer" }),
            "native.close",
        ),
    ];
    for (n, (create, posted, close)) in cases.into_iter().enumerate() {
        let session = format!("new{}", n + 1);
        let mut op = create;
        op["t"] = json!("manage");
        client.send(op).await;
        let ack = client.frame("the ack", |f| f["t"] == "managed").await;
        assert_eq!(ack["ok"], json!(true), "{ack}");
        assert_eq!(ack["op"], "muffit.create");
        assert_eq!(fake.posts("/sessions")[n], posted);
        let id = pane(&session);
        assert_eq!(ack["id"], id, "{ack}");
        assert_eq!(client.pane(&id, |_| true).await["provider"], "muffit");

        client
            .send(json!({ "t": "manage", "op": "rename", "at": id, "label": "x" }))
            .await;
        let refused = client.frame("a refused rename", |f| f["t"] == "managed").await;
        assert_eq!(refused["ok"], json!(false), "{refused}");

        client.send(json!({ "t": "manage", "op": close, "at": id })).await;
        let ack = client.frame("the close ack", |f| f["t"] == "managed").await;
        assert_eq!(ack["ok"], json!(true), "{close}: {ack}");
        assert_eq!(
            fake.posts(&format!("/sessions/{session}/stop")).len(),
            1,
            "{close}"
        );
        client
            .until("the stopped session leaves", |c| {
                (!c.panes.contains_key(&id)).then_some(())
            })
            .await;
    }
    asked(&fake);
}

#[tokio::test]
async fn a_daemon_that_is_down_leaves_the_node_serving_with_no_muffit_panes() {
    let closed = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let url = format!("http://{}", closed.local_addr().unwrap());
    drop(closed);
    let h = Harness::beside(&url, TOKEN).await;
    let mut client = h.client().await;
    client.until("the herd", |c| c.nodes.get(NODE).cloned()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    client.pump(Duration::from_millis(100)).await;
    assert!(
        !client.panes.keys().any(|p| p.contains("/m:")),
        "{:?}",
        client.panes
    );
    client.watch_unknown(&pane("s1")).await;
    let refused = client.frame("a refusal", |f| f["t"] == "error").await;
    assert_eq!(refused["code"], "unknown_pane", "{refused}");
}

impl Client {
    async fn watch_unknown(&mut self, id: &str) {
        self.send(json!({ "t": "watch", "pane": id, "conversation": true }))
            .await;
    }
}

struct ToHub(tokio::sync::mpsc::Sender<String>);

impl kampr_mesh::Outgoing for ToHub {
    async fn send(&mut self, text: String) -> bool {
        self.0.send(text).await.is_ok()
    }

    async fn close(&mut self) {}
}

struct FromHub(tokio::sync::mpsc::Receiver<String>);

impl kampr_mesh::Incoming for FromHub {
    async fn recv(&mut self) -> Option<String> {
        self.0.recv().await
    }
}

#[tokio::test]
async fn two_clients_typing_through_one_hub_each_send_their_own_message() {
    let fake = Fake::start(vec![session("s1", "waiting_user", None)]).await;
    let h = Harness::start(&fake).await;
    let device = h
        .node
        .auth
        .store()
        .create_device("hub", Role::Full, kampr_auth::now(), None, None, None)
        .await
        .unwrap();
    let (to_peer, from_hub) = tokio::sync::mpsc::channel(64);
    let (to_hub, mut from_peer) = tokio::sync::mpsc::channel(4096);
    let link = tokio::spawn(kampr_node::session::run_on(
        ToHub(to_hub),
        FromHub(from_hub),
        h.node.clone(),
        device,
        "mesh:test".into(),
        kampr_node::session::Caller::Hub,
    ));
    let drain = tokio::spawn(async move { while from_peer.recv().await.is_some() {} });
    let id = pane("s1");
    eventually("the pane", || h.node.herd().pane(&id).map(|_| ())).await;
    for (text, from) in [("alpha", "a"), ("beta", "b"), ("\r", "a"), ("\r", "b")] {
        let frame = json!({ "t": "input", "pane": id, "text": text, "from": from });
        to_peer.send(frame.to_string()).await.unwrap();
    }
    let posts = eventually("two messages", || {
        let posts = fake.posts("/sessions/s1/input");
        (posts.len() == 2).then_some(posts)
    })
    .await;
    assert_eq!(posts, [json!({ "text": "alpha" }), json!({ "text": "beta" })]);
    link.abort();
    drain.abort();
}

const HOUR_MS: u64 = 3_600_000;

fn finished_conversations() -> Vec<Value> {
    let mut rows = vec![
        session("s1", "waiting_user", Some("live one")),
        stored("old", "answered a minute ago", HOUR_MS),
        stored("ancient", "last month", 8 * 24 * HOUR_MS),
    ];
    rows.extend((0..20).map(|n| stored(&format!("bulk{n}"), "bulk", 2 * HOUR_MS + n * 60_000)));
    rows
}

#[tokio::test]
async fn a_finished_conversation_is_a_pane_read_without_attaching() {
    let fake = Fake::start(finished_conversations()).await;
    fake.set_transcript(
        "old",
        (0..120)
            .map(|n| {
                text(
                    n,
                    if n % 2 == 0 { "user" } else { "assistant" },
                    &format!("said {n}"),
                )
            })
            .collect(),
    );
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("old");
    let old = client.pane(&id, |_| true).await;
    for (field, want) in [
        ("provider", json!("muffit")),
        ("agent_status", json!("idle")),
        ("has_conversation", json!(true)),
        ("label", json!("answered a minute ago")),
        ("cwd", json!("/srv/work")),
    ] {
        assert_eq!(old[field], want, "{field}: {old}");
    }
    client.pane(&pane("s1"), |p| p["agent_status"] == "done").await;
    client.pane(&pane("bulk18"), |_| true).await;
    assert!(!client.panes.contains_key(&pane("ancient")), "older than a week");
    assert!(
        !client.panes.contains_key(&pane("bulk19")),
        "only the newest twenty stored conversations"
    );

    client.watch(&id).await;
    let page = client.convo(&id).await;
    assert_eq!(page["fresh"], json!(true));
    assert_eq!(ids(&page)[0], "m20", "{page}");
    client
        .send(json!({ "t": "convo.load", "pane": id, "before": page["cursor"] }))
        .await;
    let older = client.convo(&id).await;
    assert_eq!(ids(&older).len(), 20, "{older}");
    client
        .send(json!({ "t": "convo.find", "pane": id, "query": "said 3" }))
        .await;
    client.frame("a search", |f| f["t"] == "convo.find").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(
        fake.attaches("old"),
        0,
        "looking at a conversation must not restore it"
    );
    assert!(
        !client.log.iter().any(|f| f["t"] == "pending"),
        "a stored conversation asks nothing"
    );

    fake.set_live("old", true);
    let listed = std::time::Instant::now();
    eventually("the watch attaches once the session is live", || {
        (fake.attaches("old") == 1).then_some(())
    })
    .await;
    assert!(
        listed.elapsed() < Duration::from_secs(2),
        "the watch moved to the socket when the stream listed it, not at its next retry"
    );
    eventually("attached", || (fake.attached() > 0).then_some(())).await;
    fake.push(json!({ "type": "content", "blocks": [{ "type": "text", "text": "working on it" }] }));
    client
        .turn("the live turn", |t| {
            t["id"] == "live" && !t["blocks"].as_array().unwrap().is_empty()
        })
        .await;
    asked(&fake);
}

#[tokio::test]
async fn typing_into_a_finished_conversation_restores_it_and_sends_over_the_socket() {
    let fake = Fake::start(finished_conversations()).await;
    fake.set_transcript("old", vec![text(1, "user", "hello")]);
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("old");
    client.watch(&id).await;
    client.convo(&id).await;
    client.input(&id, "carry on").await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert_eq!(
        fake.attaches("old"),
        0,
        "the composer's text alone restores nothing"
    );
    client.input(&id, "\r").await;
    let inputs = eventually("the message over the socket", || {
        let inputs = fake.attach_frames("input");
        (!inputs.is_empty()).then_some(inputs)
    })
    .await;
    assert_eq!(inputs[0]["text"], "carry on");
    assert_eq!(fake.attaches("old"), 1);
    assert!(fake.posts("/sessions/old/input").is_empty(), "{:?}", fake.seen());
    client.pane(&id, |p| p["agent_status"] != "idle").await;
}

#[tokio::test]
async fn a_session_that_stops_being_live_stays_a_pane() {
    let fake = Fake::start(finished_conversations()).await;
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.pane(&id, |p| p["agent_status"] == "done").await;
    fake.set_live("s1", false);
    let started = std::time::Instant::now();
    client.pane(&id, |p| p["agent_status"] == "idle").await;
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the store is read again as soon as the stream changes"
    );
    assert!(client.panes.contains_key(&id));
}
