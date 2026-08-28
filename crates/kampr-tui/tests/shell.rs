//! The shell, drawn against a node scripted frame by frame.
//!
//! The client is written against `04-wire-protocol.md` and never against herdr, so the honest
//! counterpart for the chrome is a node that says exactly what that document says a node may
//! say — and a fixed-size backend that renders what the operator would have seen.

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

struct Conn {
    to_client: mpsc::UnboundedSender<Message>,
}

impl Conn {
    fn send(&self, frame: Value) {
        self.to_client
            .send(Message::text(frame.to_string()))
            .expect("the scripted node still has a client");
    }

    fn greet(&self, nodes: Value, panes: Value, role: &str) {
        self.send(json!({
            "t": "hello", "protocol": 1, "node_id": "01JNODE", "node_name": "comingclean",
            "build": "0.1.29", "role": role,
            "caps": { "push": false, "scrollback": true, "conversation": true, "manage": true }
        }));
        self.send(json!({ "t": "herd", "nodes": nodes, "panes": panes }));
        self.send(json!({ "t": "prefs", "panes": {} }));
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
                    if conns.send(Conn { to_client }).is_err() {
                        return;
                    }
                    tokio::spawn(async move {
                        while let Some(message) = outbox.recv().await {
                            if sink.send(message).await.is_err() {
                                return;
                            }
                        }
                    });
                    while let Some(Ok(_)) = source.next().await {}
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

#[tokio::test]
async fn a_role_frame_removes_the_write_affordances_mid_connection() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "working")]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    let full = painted(&mut app, 100, 20);
    assert!(
        full.contains("^b c new"),
        "a full device is offered the writes:\n{full}"
    );
    assert!(!full.contains("readonly"));

    // A demotion lands on the connection that is already open and is **not** a second hello. A
    // client that gated on the role it was greeted with keeps every write affordance drawn.
    conn.send(json!({ "t": "role", "role": "readonly" }));
    until(&mut events, |e| matches!(e, Event::Role(_)).then_some(())).await;

    let after = painted(&mut app, 100, 20);
    assert!(
        !after.contains("^b c new"),
        "a read-only device draws no write affordances:\n{after}"
    );
    assert!(after.contains("readonly"), "and says so:\n{after}");
}

#[tokio::test]
async fn a_pane_with_a_detail_and_no_grid_shows_the_reason_rather_than_a_blank_grid() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut entry = pane("01JNODE/w1:p1", "herdr", None, "idle");
    entry["detail"] = json!("herdr is not on this node's PATH");
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([entry]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    // #233: no `grid.reset` is sent for a pane in this state, and a blank grid with a flashing
    // cursor is what that looked like on a phone for months.
    let screen = painted(&mut app, 100, 20);
    assert!(screen.contains("no picture"), "{screen}");
    assert!(
        screen.contains("herdr is not on this node's PATH"),
        "the operator-readable reason is what goes on the screen:\n{screen}"
    );
}

#[tokio::test]
async fn the_sidebar_groups_spaces_by_node_and_keeps_an_offline_nodes_row() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut away = pane("01JLAPTOP/w1:p9", "website", None, "idle");
    away["node_id"] = json!("01JLAPTOP");
    away["updated_at"] = json!("2026-08-20T13:44:02Z");
    conn.greet(
        json!([
            node("01JNODE", "comingclean", true),
            { "id": "01JLAPTOP", "name": "laptop", "kind": "peer", "online": false }
        ]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "blocked"), away,]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    let screen = painted(&mut app, 110, 22);
    assert!(screen.contains("spaces"), "{screen}");
    assert!(screen.contains("comingclean"), "{screen}");
    assert!(screen.contains("agents"), "{screen}");
    // Panes are not dropped for an outage (#70): the row stays, with the count and the last-seen.
    assert!(
        screen.contains("laptop"),
        "an offline node keeps its row:\n{screen}"
    );
    assert!(screen.contains("offline"), "{screen}");
    assert!(screen.contains("1 pane · seen 13:44"), "{screen}");
    // Blocked sorts above idle in the triage list, which is sorted locally (#296).
    let agents = screen.split("agents").nth(1).expect("an agents section");
    assert!(agents.contains("herdr"), "{screen}");
}

#[tokio::test]
async fn a_live_grid_reaches_the_frame() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", None, "idle")]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    conn.send(json!({
        "t": "grid.reset", "pane": "01JNODE/w1:p1", "cols": 12, "rows": 2,
        "rows_data": [
            { "row": 0, "runs": [{ "s": 0, "x": "hello " }, { "s": 0, "x": "日本", "w": 2 }] },
            { "row": 1, "runs": [{ "s": 0, "x": "❯ " }] }
        ],
        "cursor": { "col": 2, "row": 1, "visible": true },
        "links": []
    }));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = app(&client);

    let screen = painted(&mut app, 60, 12);
    assert!(
        screen.contains("hello 日 本"),
        "the tail column is blank, not drawn:\n{screen}"
    );
    assert!(screen.contains('❯'), "{screen}");
}

#[tokio::test]
async fn two_panes_from_two_hosts_stand_side_by_side() {
    // The whole reason to build this: a herdr TUI attaches to exactly one server (ADR 0002), so
    // no herdr at a desk can draw this. It is kampr's own mosaic — two independent `observe`
    // streams — and it needs no protocol support beyond watching several panes at once.
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut away = pane("01JWORKBOX/w7:p2", "data-pipeline", Some("codex"), "working");
    away["node_id"] = json!("01JWORKBOX");
    away["workspace_id"] = json!("01JWORKBOX/w7");
    away["tab_id"] = json!("01JWORKBOX/w7:t1");
    conn.greet(
        json!([
            node("01JNODE", "comingclean", true),
            { "id": "01JWORKBOX", "name": "workbox", "kind": "peer", "online": true, "rtt_ms": 41.0 }
        ]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "blocked"), away]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    // The blocked agent is what a fresh client opens on; the triage list is sorted locally.
    assert_eq!(app.focused(), Some("01JNODE/w1:p1"));

    // prefix+w opens the navigator, which is modal and takes no prefix (#289), and `space` puts
    // the highlighted pane beside the one already on screen.
    let press = |app: &mut App, key: KeyEvent| app.key(key);
    press(&mut app, KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    press(&mut app, KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    // spaces · comingclean · ▸herdr · pane · workbox · ▸data-pipeline — three steps to the peer's
    // workspace row, because a node header carries no pane of its own.
    for _ in 0..3 {
        press(&mut app, KeyEvent::new(KeyCode::Char('j'), KeyModifiers::NONE));
    }
    press(&mut app, KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE));

    assert_eq!(
        app.pinned(),
        ["01JNODE/w1:p1", "01JWORKBOX/w7:p2"],
        "two hosts, one mosaic"
    );
    press(&mut app, KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let screen = painted(&mut app, 140, 20);
    // The border title, not the sidebar row, which carries the same words.
    assert!(screen.contains("┌ herdr · claude"), "{screen}");
    assert!(screen.contains("┌ data-pipeline · codex"), "{screen}");
}

#[tokio::test]
async fn resize_mode_moves_kamprs_own_split_and_never_the_pane() {
    // ADR 0002: kampr never resizes a pane, and there is no `terminal.resize` on this wire. A
    // herdr user's `prefix+r h/l` still has to land somewhere, so it lands on the boundary
    // between two streams this client is arranging.
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut second = pane("01JNODE/w1:p2", "herdr", None, "idle");
    second["cols"] = json!(40);
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "blocked"), second]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    let even = painted(&mut app, 120, 12);
    // The second pane's border, whose title has no agent after it.
    let border = |frame: &str| {
        frame
            .lines()
            .nth(1)
            .expect("a border row")
            .rfind("\u{250c} herdr \u{2500}")
            .expect("a second pane")
    };
    let before = border(&even);

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('r'), KeyModifiers::NONE));
    for _ in 0..4 {
        app.key(KeyEvent::new(KeyCode::Char('l'), KeyModifiers::NONE));
    }
    let wider = painted(&mut app, 120, 12);
    assert!(
        border(&wider) > before,
        "the focused pane took the space:\n{even}\n---\n{wider}"
    );
    // And the pane's own geometry never moved, because nothing on this wire can move it.
    assert_eq!(
        client.state().herd.pane("01JNODE/w1:p1").map(|p| p.cols),
        Some(Some(12))
    );
}

#[tokio::test]
async fn the_herd_view_is_one_binding_away_and_puts_blocked_first() {
    // Every node, every workspace, every pane, one screen. It is the triage screen you cannot
    // get at a desk, because a herdr TUI attaches to exactly one server.
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut away = pane("01JWORKBOX/w7:p2", "data-pipeline", Some("codex"), "blocked");
    away["node_id"] = json!("01JWORKBOX");
    away["workspace_id"] = json!("01JWORKBOX/w7");
    away["tab_id"] = json!("01JWORKBOX/w7:t1");
    conn.greet(
        json!([
            node("01JNODE", "comingclean", true),
            { "id": "01JWORKBOX", "name": "workbox", "kind": "peer", "online": true, "rtt_ms": 41.0 }
        ]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "idle"), away]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    let screen = painted(&mut app, 120, 20);

    let flagged = screen
        .lines()
        .position(|l| l.contains('\u{2691}'))
        .expect("a flag");
    let local = screen
        .lines()
        .position(|l| l.contains("comingclean") && l.contains("herdr"))
        .expect("the local node's own row");
    assert!(
        flagged < local,
        "the blocked agent on the other host sorts to the top:\n{screen}"
    );
    assert!(screen.contains("workbox"), "{screen}");
    assert!(
        screen.contains("NAVIGATE"),
        "the modal footer is drawn:\n{screen}"
    );
}
