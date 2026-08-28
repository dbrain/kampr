//! W4 — the management surface, against a node scripted frame by frame.
//!
//! The harness is `shell.rs`'s, with one thing added: this suite has to read what the client
//! *sent*, because the whole workstream is about addressing an op correctly. A `tab.rename` that
//! reaches the node with a pane id in its `at` is refused by a real herdr and looks, from the
//! outside, exactly like one that worked and did nothing.
//!
//! **No real herdr is driven here, and no `manage` op is ever aimed at a live node.** Creating or
//! closing panes in somebody's session is precisely the side effect this project has rules about.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use futures_util::{SinkExt, StreamExt};
use kampr_client::{Client, Event, Policy, Session, Via};
use kampr_core::Backoff;
use kampr_tui::app::{App, Options};
use kampr_tui::image::Images;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

const BEAT: Duration = Duration::from_secs(2);
/// Long enough that a frame the client meant to send has been written and read; short enough that
/// a test asserting silence does not cost a second.
const HUSH: Duration = Duration::from_millis(250);

struct Conn {
    to_client: mpsc::UnboundedSender<Message>,
    from_client: mpsc::UnboundedReceiver<Value>,
}

impl Conn {
    fn send(&self, frame: Value) {
        self.to_client
            .send(Message::text(frame.to_string()))
            .expect("the scripted node still has a client");
    }

    fn greet(&self, nodes: Value, panes: Value, role: &str, manage: bool) {
        self.send(json!({
            "t": "hello", "protocol": 1, "node_id": "01JNODE", "node_name": "comingclean",
            "build": "0.1.29", "role": role,
            "caps": { "push": false, "scrollback": true, "conversation": true, "manage": manage }
        }));
        self.send(json!({ "t": "herd", "nodes": nodes, "panes": panes }));
        self.send(json!({ "t": "prefs", "panes": {} }));
    }

    /// The next frame of this kind the client wrote.
    async fn frame(&mut self, kind: &str) -> Value {
        for _ in 0..64 {
            let frame = tokio::time::timeout(BEAT, self.from_client.recv())
                .await
                .unwrap_or_else(|_| panic!("the client never sent a {kind}"))
                .expect("the scripted node lost its client");
            if frame["t"] == kind {
                return frame;
            }
        }
        panic!("the client never sent a {kind}");
    }

    /// The next `manage` op the client wrote. Watches and pings go past.
    async fn op(&mut self) -> Value {
        for _ in 0..64 {
            let frame = tokio::time::timeout(BEAT, self.from_client.recv())
                .await
                .expect("the client sent no manage op")
                .expect("the scripted node lost its client");
            if frame["t"] == "manage" {
                return frame;
            }
        }
        panic!("the client sent no manage op");
    }

    /// Nothing structural left this client. A confirmation that is not a confirmation is an op
    /// already in flight before the operator was told what it does.
    async fn sent_nothing(&mut self) {
        let until = tokio::time::Instant::now() + HUSH;
        while let Ok(Some(frame)) = tokio::time::timeout_at(until, self.from_client.recv()).await {
            assert_ne!(
                frame["t"], "manage",
                "an op went out with no confirmation: {frame}"
            );
        }
    }
}

struct Fake {
    origin: String,
    conns: mpsc::UnboundedReceiver<Conn>,
}

impl Fake {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let origin = format!("http://{}", listener.local_addr().expect("an address"));
        let (conns, incoming) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let conns = conns.clone();
                tokio::spawn(async move {
                    let accepted = tokio_tungstenite::accept_hdr_async(
                        stream,
                        #[allow(clippy::result_large_err)]
                        |request: &Request, mut response: Response| {
                            if let Some(protocol) = request.headers().get("sec-websocket-protocol") {
                                response
                                    .headers_mut()
                                    .insert("sec-websocket-protocol", protocol.clone());
                            }
                            Ok(response)
                        },
                    )
                    .await;
                    let Ok(socket) = accepted else { return };
                    let (mut sink, mut source) = socket.split();
                    let (to_client, mut outbox) = mpsc::unbounded_channel::<Message>();
                    let (heard, from_client) = mpsc::unbounded_channel::<Value>();
                    if conns
                        .send(Conn {
                            to_client,
                            from_client,
                        })
                        .is_err()
                    {
                        return;
                    }
                    tokio::spawn(async move {
                        while let Some(message) = outbox.recv().await {
                            if sink.send(message).await.is_err() {
                                return;
                            }
                        }
                    });
                    while let Some(Ok(message)) = source.next().await {
                        if let Message::Text(text) = message
                            && let Ok(frame) = serde_json::from_str::<Value>(&text)
                            && heard.send(frame).is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        Self {
            origin,
            conns: incoming,
        }
    }

    async fn accept(&mut self) -> Conn {
        tokio::time::timeout(BEAT, self.conns.recv())
            .await
            .expect("no client dialled")
            .expect("the listener died")
    }

    fn client(&self) -> Client {
        Client::with_policy(
            Session {
                origin: self.origin.clone(),
                token: "scripted-token".into(),
                via: Via::Profile {
                    name: "scripted".into(),
                },
            },
            Policy {
                backoff: Backoff {
                    initial: Duration::from_millis(10),
                    max: Duration::from_millis(50),
                },
                connect_timeout: Duration::from_secs(2),
                manage_timeout: Duration::from_millis(500),
                event_capacity: 256,
            },
        )
    }
}

async fn until<T>(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    want: impl Fn(Event) -> Option<T>,
) -> T {
    for _ in 0..64 {
        let event = tokio::time::timeout(BEAT, events.recv())
            .await
            .expect("no event arrived")
            .expect("the event stream ended");
        if let Some(found) = want(event) {
            return found;
        }
    }
    panic!("the event never arrived");
}

/// Drains the event stream through [`absorb`] until `want` has been seen.
async fn pump(
    app: &mut App,
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    want: impl Fn(&Event) -> bool,
) {
    for _ in 0..64 {
        let event = tokio::time::timeout(BEAT, events.recv())
            .await
            .expect("no event arrived")
            .expect("the event stream ended");
        let done = want(&event);
        app.absorb(&event);
        if done {
            return;
        }
    }
    panic!("the event never arrived");
}

fn node(id: &str, name: &str, online: bool) -> Value {
    json!({ "id": id, "name": name, "kind": "local", "online": online })
}

fn pane(id: &str, workspace: &str, agent: Option<&str>, status: &str) -> Value {
    json!({
        "id": id, "node_id": id.split('/').next().unwrap(),
        "workspace_id": "01JNODE/w1", "tab_id": "01JNODE/w1:t1",
        "workspace": workspace, "tab": "1", "cwd": "/home/dbrain/dev/kampr",
        "agent": agent, "agent_status": status, "rows": 4, "cols": 12
    })
}

fn painted(app: &mut App, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
    terminal.draw(|frame| app.draw(frame)).expect("a frame");
    let buffer = terminal.backend().buffer().clone();
    (0..buffer.area.height)
        .map(|y| {
            (0..buffer.area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn app(client: &Arc<Client>) -> App {
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.refocus();
    app
}

const PREFIX: KeyEvent = KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL);

fn tap(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn ch(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

/// `prefix` then a shifted letter — herdr's own grammar for the write binds (#289).
fn shifted(app: &mut App, c: char) {
    app.key(PREFIX);
    app.key(KeyEvent::new(
        KeyCode::Char(c.to_ascii_uppercase()),
        KeyModifiers::SHIFT,
    ));
}

fn typed(app: &mut App, text: &str) {
    for c in text.chars() {
        ch(app, c);
    }
}

/// One node, one pane, `manage` claimed, a writer — and **the sidebar closed**, so a word found
/// on the screen is one the prompt put there rather than one the `agents` header always draws.
async fn desk(fake: &mut Fake) -> (Arc<Client>, tokio::sync::broadcast::Receiver<Event>, Conn, App) {
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", None, "idle")]),
        "full",
        true,
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    bare(&mut app);
    (client, events, conn, app)
}

fn bare(app: &mut App) {
    app.key(PREFIX);
    ch(app, 'b');
}

#[tokio::test]
async fn an_op_the_node_refuses_is_not_a_success_and_the_in_flight_state_clears() {
    let mut fake = Fake::start().await;
    let (_client, mut events, mut conn, mut app) = desk(&mut fake).await;

    shifted(&mut app, 'p');
    typed(&mut app, "build");
    tap(&mut app, KeyCode::Enter);

    let sent = conn.op().await;
    assert_eq!(sent["op"], "rename");
    let waiting = painted(&mut app, 100, 20);
    assert!(
        waiting.contains("waiting for the node"),
        "an op in flight says so and claims nothing:\n{waiting}"
    );

    // The wire's own sequence for a refusal: the ack, carrying the `rid` it was sent with, and
    // then the ordinary `error` frame. The ack goes to the op's own waiter; the error is the half
    // this surface ever sees, and it is what has to take the in-flight state down.
    conn.send(json!({
        "t": "managed", "rid": sent["rid"], "op": "rename", "ok": false,
        "code": "not_writer", "message": "this device is read-only"
    }));
    conn.send(json!({
        "t": "error", "code": "not_writer", "message": "this device is read-only", "pane": null
    }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Error(_))).await;

    let after = painted(&mut app, 100, 20);
    assert!(
        after.contains("rename was refused"),
        "a refusal is read, not waited through:\n{after}"
    );
    assert!(
        !after.contains("waiting for the node"),
        "and the in-flight state goes with it:\n{after}"
    );
}

#[tokio::test]
async fn an_unsolicited_refusal_is_read_off_ok_and_never_off_its_arrival() {
    let mut fake = Fake::start().await;
    let (_client, mut events, conn, mut app) = desk(&mut fake).await;

    // No `rid`, so nothing is waiting on it — a hub's relay, or an ack that crossed a timeout.
    // `quota_exhausted` is not in the v1 code list on purpose: the vocabulary is open and an
    // unrecognised code must render its `message` rather than fail.
    conn.send(json!({
        "t": "managed", "op": "pane.split", "ok": false,
        "code": "quota_exhausted", "message": "too many panes"
    }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Managed(_))).await;

    let screen = painted(&mut app, 100, 20);
    assert!(
        screen.contains("pane.split was refused"),
        "arrival is not success:\n{screen}"
    );
    assert!(
        screen.contains("too many panes"),
        "an unknown code still shows its message:\n{screen}"
    );
}

#[tokio::test]
async fn a_node_that_does_not_claim_manage_offers_no_manage_affordance_at_all() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "working")]),
        "full",
        false,
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    bare(&mut app);

    shifted(&mut app, 'p');
    let screen = painted(&mut app, 100, 20);
    // Absent, not disabled (findings §3.7): there is no prompt, no greyed row and no `+`.
    assert!(
        !screen.contains("rename pane"),
        "a node that does not claim manage draws no prompt:\n{screen}"
    );
    assert!(
        screen.contains("does not claim manage"),
        "and says why, once, in the status line:\n{screen}"
    );
    assert!(!screen.contains("^b c new"), "{screen}");
    assert!(!screen.contains(" + "), "{screen}");
}

#[tokio::test]
async fn a_demotion_to_readonly_mid_connection_takes_the_prompts_away() {
    let mut fake = Fake::start().await;
    let (_client, mut events, conn, mut app) = desk(&mut fake).await;

    shifted(&mut app, 'p');
    let before = painted(&mut app, 100, 20);
    assert!(before.contains("rename pane"), "{before}");
    tap(&mut app, KeyCode::Esc);

    // Not a second greeting: a client gated on the role it was *greeted* with keeps every write
    // affordance drawn after this frame.
    conn.send(json!({ "t": "role", "role": "readonly" }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Role(_))).await;

    shifted(&mut app, 'p');
    let after = painted(&mut app, 100, 20);
    assert!(
        !after.contains("rename pane"),
        "a read-only device is refused every op with not_writer, so it is offered none:\n{after}"
    );
    assert!(after.contains("read-only"), "{after}");
}

#[tokio::test]
async fn renaming_a_tab_addresses_the_tab_id_and_never_the_pane_id() {
    let mut fake = Fake::start().await;
    let (_client, _events, mut conn, mut app) = desk(&mut fake).await;

    shifted(&mut app, 't');
    typed(&mut app, "tests");
    tap(&mut app, KeyCode::Enter);

    let sent = conn.op().await;
    assert_eq!(sent["op"], "rename");
    // A pane id carries its workspace (`w3:p2` -> `w3`) and **never its tab**, so this can only
    // come off the pane entry's own `tab_id`.
    assert_eq!(
        sent["at"], "01JNODE/w1:t1",
        "tab.rename addresses the tab: {sent}"
    );
    assert_eq!(sent["label"], "tests");
    assert!(sent["rid"].is_u64(), "several ops are in flight at once: {sent}");
}

#[tokio::test]
async fn a_pane_entry_with_no_tab_id_offers_no_tab_op_rather_than_a_broken_one() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut entry = pane("01JNODE/w1:p1", "herdr", None, "idle");
    entry["tab_id"] = Value::Null;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([entry]),
        "full",
        true,
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    bare(&mut app);

    shifted(&mut app, 't');
    let screen = painted(&mut app, 100, 20);
    assert!(
        !screen.contains("What is this tab called"),
        "without a tab_id a client cannot address the tab at all:\n{screen}"
    );
}

#[tokio::test]
async fn a_split_says_what_it_does_to_everyone_else_before_it_does_it() {
    let mut fake = Fake::start().await;
    let (_client, _events, mut conn, mut app) = desk(&mut fake).await;

    app.key(PREFIX);
    ch(&mut app, 'v');
    let asked = painted(&mut app, 110, 24);
    assert!(asked.contains("Split 01JNODE/w1:p1 right?"), "{asked}");
    // Single words, because the copy is wrapped at the popup's width and a phrase may not be.
    assert!(
        asked.contains("re-lays") && asked.contains("size") && asked.contains("desk"),
        "the copy names the consequence, not just the act:\n{asked}"
    );
    assert!(asked.contains("#298"), "and cites what measured it:\n{asked}");
    conn.sent_nothing().await;

    tap(&mut app, KeyCode::Enter);
    let sent = conn.op().await;
    assert_eq!(sent["op"], "pane.split");
    assert_eq!(sent["at"], "01JNODE/w1:p1");
    // herdr's split grammar is exactly two directions (#46/#47).
    assert_eq!(sent["direction"], "right");
}

#[tokio::test]
async fn the_herd_does_not_move_until_the_patch_says_so() {
    let mut fake = Fake::start().await;
    let (client, mut events, mut conn, mut app) = desk(&mut fake).await;

    shifted(&mut app, 'p');
    typed(&mut app, "build");
    tap(&mut app, KeyCode::Enter);
    let sent = conn.op().await;
    assert_eq!(sent["label"], "build");

    let waiting = painted(&mut app, 100, 20);
    assert_eq!(
        client
            .state()
            .herd
            .pane("01JNODE/w1:p1")
            .and_then(|p| p.label.clone()),
        None,
        "the node is authoritative: nothing is renamed here first"
    );
    assert!(
        !waiting.contains("build"),
        "and the screen does not claim it either:\n{waiting}"
    );
    assert!(waiting.contains("waiting for the node"), "{waiting}");

    let mut renamed = pane("01JNODE/w1:p1", "herdr", Some("claude"), "working");
    renamed["label"] = json!("build");
    conn.send(json!({
        "t": "managed", "rid": sent["rid"], "op": "rename", "ok": true
    }));
    conn.send(json!({ "t": "herd.patch", "changed": { "panes": [renamed] }, "removed_ids": [] }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Herd)).await;

    let after = painted(&mut app, 100, 20);
    assert!(
        after.contains("build"),
        "and it lands when the patch does:\n{after}"
    );
}

#[tokio::test]
async fn agent_start_offers_only_the_kinds_the_node_named() {
    let mut fake = Fake::start().await;
    let (_client, _events, mut conn, mut app) = desk(&mut fake).await;

    // No `caps` answer yet: a client that compiled a list of harnesses in would offer one here.
    shifted(&mut app, 'n');
    let cold = painted(&mut app, 110, 24);
    assert!(cold.contains("workspace"), "the menu is open:\n{cold}");
    assert!(
        !cold.contains("this node named"),
        "kinds come from the node, so before it has answered there is nothing to offer:\n{cold}"
    );
    tap(&mut app, KeyCode::Esc);

    app.manage.observe(&Event::Caps(
        serde_json::from_value(json!({
            "node": "01JNODE",
            "agent_kinds": ["claude", "codex"],
            "sessions": [{ "name": "default", "running": true, "served": true }]
        }))
        .expect("node caps"),
    ));

    shifted(&mut app, 'n');
    ch(&mut app, 'a');
    let kinds = painted(&mut app, 110, 24);
    assert!(kinds.contains("claude") && kinds.contains("codex"), "{kinds}");
    assert!(
        !kinds.contains("gemini") && !kinds.contains("aider"),
        "only what the node named:\n{kinds}"
    );

    ch(&mut app, '1');
    let sent = conn.op().await;
    assert_eq!(sent["op"], "agent.start");
    assert_eq!(sent["at"], "01JNODE/w1:p1");
    assert_eq!(sent["kind"], "claude");
}

#[tokio::test]
async fn a_session_that_is_running_and_unserved_is_never_offered_as_somewhere_to_put_a_pane() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([
            node("01JNODE", "comingclean", true),
            { "id": "01JWORKBOX", "name": "workbox", "kind": "peer", "online": true }
        ]),
        json!([pane("01JNODE/w1:p1", "herdr", None, "idle")]),
        "full",
        true,
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    bare(&mut app);

    app.manage.observe(&Event::Caps(
        serde_json::from_value(json!({
            "node": "01JNODE",
            "agent_kinds": ["claude"],
            "sessions": [
                { "name": "default", "running": true,  "served": true },
                { "name": "agents",  "running": true,  "served": false }
            ]
        }))
        .expect("node caps"),
    ));

    // A `node`-scoped op names a machine, and the machines are the **herd's** — a session that is
    // running and not served never joins this herd, so a workspace made in one would be a
    // workspace nothing here could ever reach.
    shifted(&mut app, 'n');
    ch(&mut app, 'w');
    let machines = painted(&mut app, 110, 24);
    assert!(
        machines.contains("comingclean") && machines.contains("workbox"),
        "{machines}"
    );
    assert!(
        !machines.contains("agents"),
        "an unserved session is not a machine to make anything on:\n{machines}"
    );
    tap(&mut app, KeyCode::Esc);

    // It is still listed where a session belongs, said plainly, and it can still be stopped.
    shifted(&mut app, 'n');
    ch(&mut app, 'n');
    let sessions = painted(&mut app, 110, 24);
    assert!(sessions.contains("agents"), "{sessions}");
    assert!(
        sessions.contains("not served"),
        "and why it is not somewhere to work:\n{sessions}"
    );
}

#[tokio::test]
async fn a_session_ack_carries_a_bare_name_and_is_never_dressed_up_as_a_pane_id() {
    let mut fake = Fake::start().await;
    let (_client, mut events, conn, mut app) = desk(&mut fake).await;

    conn.send(json!({ "t": "managed", "op": "tab.create", "ok": true, "id": "01JNODE/w1:t2" }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Managed(_))).await;
    let container = painted(&mut app, 110, 20);
    assert!(container.contains("01JNODE/w1:t2"), "{container}");
    assert!(
        container.contains("herd patch"),
        "a container id is something the herd will bring:\n{container}"
    );

    conn.send(json!({ "t": "managed", "op": "session.create", "ok": true, "id": "agents" }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Managed(_))).await;
    let session = painted(&mut app, 110, 20);
    assert!(
        session.contains("the session agents"),
        "a session's id is its bare name:\n{session}"
    );
    assert!(
        !session.contains("herd patch"),
        "and it is a node joining the herd, not a pane arriving in it:\n{session}"
    );
}

#[tokio::test]
async fn layout_export_holds_the_tree_and_only_then_is_there_one_to_apply() {
    let mut fake = Fake::start().await;
    let (_client, mut events, mut conn, mut app) = desk(&mut fake).await;

    shifted(&mut app, 'n');
    ch(&mut app, 'l');
    let cold = painted(&mut app, 110, 24);
    assert!(cold.contains("export"), "{cold}");
    assert!(
        !cold.contains("apply"),
        "there is nothing to apply until a tree has been held:\n{cold}"
    );
    ch(&mut app, 'e');
    let sent = conn.op().await;
    assert_eq!(sent["op"], "layout.export");
    // layout.export needs a **tab**, which a pane id does not carry.
    assert_eq!(sent["at"], "01JNODE/w1:t1");

    // The tree rides on the ack, and it is opaque here: held exactly as it arrived.
    conn.send(json!({
        "t": "managed", "op": "layout.export", "ok": true,
        "layout": { "workspace_id": "w1", "tab_id": "w1:t1",
                    "root": { "type": "split", "direction": "right", "ratio": 0.5 } }
    }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Managed(_))).await;

    shifted(&mut app, 'n');
    ch(&mut app, 'l');
    let warm = painted(&mut app, 110, 24);
    assert!(warm.contains("apply"), "{warm}");
    ch(&mut app, 'a');
    tap(&mut app, KeyCode::Enter);
    let applied = conn.op().await;
    assert_eq!(applied["op"], "layout.apply");
    assert_eq!(applied["at"], "01JNODE/w1:t1");
    assert_eq!(applied["layout"]["root"]["direction"], "right");
}

#[tokio::test]
async fn a_new_tab_lands_with_no_prompt_at_the_workspace_the_pane_is_in() {
    let mut fake = Fake::start().await;
    let (_client, _events, mut conn, mut app) = desk(&mut fake).await;

    app.key(PREFIX);
    ch(&mut app, 'c');

    let sent = conn.op().await;
    assert_eq!(sent["op"], "tab.create");
    // `at` for tab.create is a WORKSPACE id — nodes take a tab id and derive it, but a client
    // that has the workspace in hand names it.
    assert_eq!(sent["at"], "01JNODE/w1");
}

#[tokio::test]
async fn a_pane_rename_clears_with_nothing_typed_and_a_tab_rename_refuses_to() {
    let mut fake = Fake::start().await;
    let (_client, _events, mut conn, mut app) = desk(&mut fake).await;

    // Only a pane's label is nullable: herdr's tab and workspace renames take a required string,
    // so there is nothing to clear them to and an empty answer must not become a round trip.
    shifted(&mut app, 't');
    tap(&mut app, KeyCode::Enter);
    conn.sent_nothing().await;
    tap(&mut app, KeyCode::Esc);

    shifted(&mut app, 'p');
    tap(&mut app, KeyCode::Enter);
    let sent = conn.op().await;
    assert_eq!(sent["op"], "rename");
    assert_eq!(sent["at"], "01JNODE/w1:p1");
    assert!(sent["label"].is_null(), "null clears a pane's label: {sent}");
}

/// The rest of the surface, as a table: every op the wire carries, the key that reaches it, and
/// the target it must be addressed at. `rename`, `close` and `focus` are **one verb each** that
/// route by the kind of id in `at`, so the whole risk is which id a client puts there.
#[tokio::test]
async fn every_bound_op_reaches_the_node_addressed_at_the_right_kind_of_id() {
    let mut fake = Fake::start().await;
    let (_client, _events, mut conn, mut app) = desk(&mut fake).await;
    app.manage.observe(&Event::Caps(
        serde_json::from_value(json!({
            "node": "01JNODE",
            "agent_kinds": ["claude"],
            "sessions": [{ "name": "default", "running": true, "served": true }]
        }))
        .expect("node caps"),
    ));

    shifted(&mut app, 'x');
    tap(&mut app, KeyCode::Enter);
    let close_tab = conn.op().await;
    assert_eq!(
        (&close_tab["op"], &close_tab["at"]),
        (&json!("close"), &json!("01JNODE/w1:t1"))
    );

    shifted(&mut app, 'd');
    tap(&mut app, KeyCode::Enter);
    let close_ws = conn.op().await;
    assert_eq!(
        (&close_ws["op"], &close_ws["at"]),
        (&json!("close"), &json!("01JNODE/w1"))
    );

    shifted(&mut app, 'w');
    typed(&mut app, "kampr");
    tap(&mut app, KeyCode::Enter);
    let rename_ws = conn.op().await;
    assert_eq!(rename_ws["at"], "01JNODE/w1");
    assert_eq!(rename_ws["label"], "kampr");

    app.key(PREFIX);
    ch(&mut app, 'x');
    tap(&mut app, KeyCode::Enter);
    let close_pane = conn.op().await;
    assert_eq!(
        (&close_pane["op"], &close_pane["at"]),
        (&json!("close"), &json!("01JNODE/w1:p1"))
    );

    app.key(PREFIX);
    ch(&mut app, '-');
    tap(&mut app, KeyCode::Enter);
    let down = conn.op().await;
    assert_eq!(
        down["direction"], "down",
        "herdr splits right or down and nothing else"
    );

    shifted(&mut app, 'n');
    ch(&mut app, 'f');
    let focus = conn.op().await;
    assert_eq!(
        (&focus["op"], &focus["at"]),
        (&json!("focus"), &json!("01JNODE/w1:p1"))
    );

    shifted(&mut app, 'n');
    ch(&mut app, 'z');
    tap(&mut app, KeyCode::Enter);
    let zoom = conn.op().await;
    assert_eq!(
        (&zoom["op"], &zoom["mode"]),
        (&json!("pane.zoom"), &json!("toggle"))
    );

    // `workspace.create`, `worktree.*` and `session.*` name a **node**, not a container.
    shifted(&mut app, 'n');
    ch(&mut app, 'w');
    typed(&mut app, "spike");
    tap(&mut app, KeyCode::Enter);
    let workspace = conn.op().await;
    assert_eq!(workspace["op"], "workspace.create");
    assert_eq!(workspace["node"], "01JNODE");
    assert_eq!(workspace["label"], "spike");
    assert!(workspace["at"].is_null(), "a create names a machine: {workspace}");

    shifted(&mut app, 'n');
    ch(&mut app, 'g');
    ch(&mut app, 'c');
    typed(&mut app, "feat/x");
    tap(&mut app, KeyCode::Enter);
    let worktree = conn.op().await;
    assert_eq!(worktree["op"], "worktree.create");
    assert_eq!(worktree["node"], "01JNODE");
    assert_eq!(worktree["branch"], "feat/x");

    shifted(&mut app, 'n');
    ch(&mut app, 'g');
    ch(&mut app, 'o');
    typed(&mut app, "~/dev/kampr-feat-x");
    tap(&mut app, KeyCode::Enter);
    let opened = conn.op().await;
    assert_eq!(opened["op"], "worktree.open");
    assert_eq!(opened["path"], "~/dev/kampr-feat-x");

    shifted(&mut app, 'n');
    ch(&mut app, 'n');
    ch(&mut app, '+');
    typed(&mut app, "agents");
    tap(&mut app, KeyCode::Enter);
    let created = conn.op().await;
    assert_eq!(created["op"], "session.create");
    assert_eq!(created["node"], "01JNODE");
    assert_eq!(created["name"], "agents");

    // A running one is stopped, and it is asked first: a named session is its own herdr server
    // and every pane in it goes with it.
    shifted(&mut app, 'n');
    ch(&mut app, 'n');
    ch(&mut app, '1');
    let asked = painted(&mut app, 110, 24);
    assert!(asked.contains("Stop the named session default?"), "{asked}");
    conn.sent_nothing().await;
    tap(&mut app, KeyCode::Enter);
    let stopped = conn.op().await;
    assert_eq!(stopped["op"], "session.stop");
    assert_eq!(stopped["name"], "default");
}

#[tokio::test]
async fn the_client_asks_a_node_what_it_can_be_told_to_make_and_routes_the_answer() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    // The app exists before the greeting, so the whole greeting goes through the router the
    // shipped binary uses rather than past it.
    let mut app = app(&client);
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", None, "idle")]),
        "full",
        true,
    );
    pump(&mut app, &mut events, |e| matches!(e, Event::Prefs { .. })).await;
    bare(&mut app);

    // Nothing else on the wire carries the agent kinds or the named sessions, so a client that
    // never asks hides those rows for the wrong reason.
    let asked = conn.frame("caps").await;
    assert_eq!(asked["t"], "caps");

    conn.send(json!({
        "t": "caps", "node": "01JNODE", "agent_kinds": ["claude", "codex"],
        "sessions": [{ "name": "default", "running": true, "served": true }]
    }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Caps(_))).await;

    shifted(&mut app, 'n');
    ch(&mut app, 'a');
    let kinds = painted(&mut app, 110, 24);
    assert!(
        kinds.contains("claude") && kinds.contains("codex"),
        "the answer reaches the menu:\n{kinds}"
    );
}

#[tokio::test]
async fn a_successful_op_resolves_its_notice_rather_than_ageing_out_in_flight() {
    let mut fake = Fake::start().await;
    let (_client, _events, mut conn, mut app) = desk(&mut fake).await;

    shifted(&mut app, 'p');
    typed(&mut app, "build");
    tap(&mut app, KeyCode::Enter);
    let sent = conn.op().await;
    assert_eq!(sent["op"], "rename");
    let waiting = painted(&mut app, 100, 20);
    assert!(waiting.contains("waiting for the node"), "{waiting}");

    // A success is acked and produces **no other frame** — no `error`, and the `herd.patch` names
    // no op — so an ack that is dropped leaves the surface waiting on something that is done.
    conn.send(json!({
        "t": "managed", "rid": sent["rid"], "op": "rename", "ok": true,
        "id": "01JNODE/w1:p1"
    }));
    tokio::time::sleep(HUSH).await;

    let after = painted(&mut app, 100, 20);
    assert!(
        !after.contains("waiting for the node"),
        "the ack resolves it:\n{after}"
    );
    assert!(
        after.contains("rename · 01JNODE/w1:p1"),
        "and says what the node said:\n{after}"
    );
}

#[tokio::test]
async fn a_layout_export_ack_hands_back_a_tree_that_layout_apply_can_be_offered() {
    let mut fake = Fake::start().await;
    let (_client, _events, mut conn, mut app) = desk(&mut fake).await;

    shifted(&mut app, 'n');
    ch(&mut app, 'l');
    ch(&mut app, 'e');
    let sent = conn.op().await;
    assert_eq!(sent["op"], "layout.export");

    conn.send(json!({
        "t": "managed", "rid": sent["rid"], "op": "layout.export", "ok": true,
        "layout": { "root": { "direction": "right", "children": [] } }
    }));
    tokio::time::sleep(HUSH).await;
    painted(&mut app, 110, 24);

    shifted(&mut app, 'n');
    ch(&mut app, 'l');
    let screen = painted(&mut app, 110, 24);
    assert!(
        screen.contains("apply"),
        "the tree is held, so applying it is offered:\n{screen}"
    );
}

#[tokio::test]
async fn a_session_ack_asks_the_node_what_it_can_make_now_rather_than_trusting_the_old_answer() {
    let mut fake = Fake::start().await;
    let (_client, mut events, mut conn, mut app) = desk(&mut fake).await;
    conn.send(json!({
        "t": "caps", "node": "01JNODE", "agent_kinds": [],
        "sessions": [{ "name": "spike", "running": false, "served": false }]
    }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Caps(_))).await;

    shifted(&mut app, 'n');
    ch(&mut app, 'n');
    ch(&mut app, '1');
    let sent = conn.op().await;
    assert_eq!(sent["op"], "session.create");

    conn.send(json!({
        "t": "managed", "rid": sent["rid"], "op": "session.create", "ok": true, "id": "spike"
    }));
    tokio::time::sleep(HUSH).await;
    painted(&mut app, 110, 24);
    // #241: the ack already waited for the host to agree, so this is the moment the cached answer
    // is out of date rather than a guess about when it might be.
    let asked = conn.frame("caps").await;
    assert_eq!(asked["t"], "caps");
}

#[tokio::test]
async fn a_digit_typed_into_a_manage_prompt_is_not_an_answer_to_a_blocked_agent() {
    let mut fake = Fake::start().await;
    let (_client, mut events, mut conn, mut app) = desk(&mut fake).await;
    conn.send(json!({
        "t": "pending", "pane": "01JNODE/w1:p1", "question": "Do you want to make this edit?",
        "options": [{ "key": "1", "label": "Yes" }], "source": "screen"
    }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Pending(_))).await;
    assert!(app.convo.has("01JNODE/w1:p1"), "the pane is on its conversation");

    shifted(&mut app, 'p');
    ch(&mut app, '1');
    let screen = painted(&mut app, 110, 24);
    assert!(
        screen.contains("❯ 1"),
        "the modal has the keyboard while it is open:\n{screen}"
    );

    let until = tokio::time::Instant::now() + HUSH;
    while let Ok(Some(frame)) = tokio::time::timeout_at(until, conn.from_client.recv()).await {
        assert_ne!(
            frame["t"], "answer",
            "a digit meant for a prompt is not an answer to a question: {frame}"
        );
    }
}

#[tokio::test]
async fn a_demotion_closes_a_modal_that_is_still_collecting_an_op() {
    let mut fake = Fake::start().await;
    let (_client, mut events, conn, mut app) = desk(&mut fake).await;

    shifted(&mut app, 'p');
    assert!(app.manage.active());

    conn.send(json!({ "t": "role", "role": "readonly" }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Role(_))).await;

    assert!(
        !app.manage.active(),
        "an op this device may no longer send is not left half-collected"
    );
    let screen = painted(&mut app, 110, 24);
    assert!(!screen.contains("Call it what?"), "{screen}");
}

/// The one op that reshapes a pane, and the only one whose consequence lands on a person rather
/// than on a layout — so it says what it will do, names both outcomes, and fires nothing until the
/// operator agrees.
#[tokio::test]
async fn a_resize_says_which_way_it_can_go_before_it_claims_anything() {
    let mut fake = Fake::start().await;
    let (_client, _events, mut conn, mut app) = desk(&mut fake).await;

    // `shifted` carries the prefix itself (#289), so this is prefix+shift+n into the manage menu.
    shifted(&mut app, 'n');
    ch(&mut app, 'r');
    let sizes = painted(&mut app, 110, 24);
    assert!(sizes.contains("80x24") && sizes.contains("200x50"), "{sizes}");
    // A menu, not a typed number: a pane keeps the size it is given, so the operator is offered
    // sizes that clear the node's floor rather than a prompt they can put 40 into.
    conn.sent_nothing().await;

    tap(&mut app, KeyCode::Enter);
    let asked = painted(&mut app, 110, 24);
    assert!(asked.contains("Resize 01JNODE/w1:p1 to 80x24?"), "{asked}");
    assert!(
        asked.contains("headless") && asked.contains("desk"),
        "the copy names both outcomes, because they are opposite:\n{asked}"
    );
    assert!(
        asked.contains("#219") && asked.contains("#19"),
        "and cites what measured them:\n{asked}"
    );
    conn.sent_nothing().await;

    tap(&mut app, KeyCode::Enter);
    let sent = conn.op().await;
    assert_eq!(sent["op"], "pane.size");
    assert_eq!(sent["at"], "01JNODE/w1:p1");
    assert_eq!((&sent["cols"], &sent["rows"]), (&json!(80), &json!(24)));
    // The safe mode is the absent one: a plain resize hands the PTY straight back.
    assert!(
        sent.get("mode").is_none(),
        "a resize does not hold unless asked: {sent}"
    );
}
