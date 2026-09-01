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
use kampr_tui::mouse::Click;
use kampr_tui::sidebar;
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
/// Long enough for a frame the client wrote to reach the scripted node over a loopback socket,
/// and the window a "nothing was sent" assertion waits out.
const SETTLE: Duration = Duration::from_millis(250);

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

    /// Everything the client wrote in the window a loopback socket needs, so "nothing was sent"
    /// is an assertion about frames rather than about timing.
    async fn heard(&mut self) -> Vec<Value> {
        let mut frames = Vec::new();
        let deadline = tokio::time::Instant::now() + SETTLE;
        while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, self.from_client.recv()).await {
            frames.push(frame);
        }
        frames
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

    // The strip's `+`, which is where the write affordance lives now that no hint bar is drawn
    // in the pane keymap (#374).
    let full = painted(&mut app, 100, 20);
    assert!(
        full.lines().next().is_some_and(|strip| strip.contains(" + ")),
        "a full device is offered the writes:\n{full}"
    );
    assert!(!full.contains("readonly"));

    // A demotion lands on the connection that is already open and is **not** a second hello. A
    // client that gated on the role it was greeted with keeps every write affordance drawn.
    conn.send(json!({ "t": "role", "role": "readonly" }));
    until(&mut events, |e| matches!(e, Event::Role(_)).then_some(())).await;

    let after = painted(&mut app, 100, 20);
    assert!(
        after.lines().next().is_some_and(|strip| !strip.contains(" + ")),
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
            .rfind("\u{250c} herdr \u{b7} bash ")
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
    app.key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT));
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
}

/// **The navigator moves the sidebar cursor and leaves the frame alone.** It used to force the
/// herd screen for as long as it was open, so `^b w` read as "open a different screen" — and the
/// one surface it was actually navigating was the one it covered up.
#[tokio::test]
async fn the_navigator_walks_the_sidebar_and_leaves_the_panes_on_screen() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "idle")]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    let screen = painted(&mut app, 120, 20);

    assert!(
        screen.contains("NAVIGATE the sidebar"),
        "the modal footer says what is being navigated:\n{screen}"
    );
    // The pane's own surface, not its border: a lone pane is flush now (#375), so a box is no
    // longer proof that anything was drawn.
    assert!(
        screen.contains("waiting for the first frame"),
        "the pane the operator was reading is still drawn:\n{screen}"
    );
}

/// The pane helper puts everything in one tab; this one names its own, because the mosaic is a
/// tab's panes and a second tab is what makes focusing change what is watched.
fn tabbed(id: &str, tab: &str, agent: Option<&str>, status: &str) -> Value {
    json!({
        "id": id, "node_id": "01JNODE",
        "workspace_id": "01JNODE/w1", "tab_id": format!("01JNODE/w1:{tab}"),
        "workspace": "herdr", "tab": tab, "cwd": "/home/dbrain/dev/kampr",
        "agent": agent, "agent_status": status, "rows": 4, "cols": 12
    })
}

fn watched(frames: &[Value]) -> Vec<String> {
    frames
        .iter()
        .filter(|f| f["t"] == json!("watch"))
        .filter_map(|f| f["pane"].as_str().map(str::to_string))
        .collect()
}

/// **A focus is a subscription change and nothing else was saying so.**
///
/// `mosaic()` is the focused pane's tab, so focusing a pane in another tab changes what this
/// client is watching — but the only things that re-stated the watches were the greeting and a
/// `herd` frame. An agent pane hid it: its `agent_status` churns, the node patches the herd, and
/// the watches are restated within a second. A shell sitting at its prompt produces no herd
/// traffic at all, so it stayed on "waiting for the first frame" for ever — the same pane the
/// operator had just asked for.
#[tokio::test]
async fn a_pane_the_operator_focused_is_watched_without_waiting_for_a_herd_frame() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([
            tabbed("01JNODE/w1:p1", "t1", Some("claude"), "working"),
            tabbed("01JNODE/w1:p2", "t2", None, "unknown")
        ]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;

    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.absorb(&Event::Prefs { greeting: true });
    let opening = watched(&conn.heard().await);
    assert_eq!(
        opening,
        vec!["01JNODE/w1:p1".to_string()],
        "the greeting watches the pane it opened on"
    );

    app.clicked(Click::Focus("01JNODE/w1:p2".into()));
    let frames = conn.heard().await;
    assert_eq!(
        watched(&frames),
        vec!["01JNODE/w1:p2".to_string()],
        "the shell the operator just opened is subscribed to, with no herd frame to prompt it: \
         {frames:?}"
    );
}

/// **Two different questions, and the sidebar used to answer only one.** The cursor mark belongs
/// to the navigator and is drawn only while it is open, so outside it nothing said which of the
/// rows was the pane the frame was showing.
#[tokio::test]
async fn the_sidebar_says_which_pane_the_frame_is_showing() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([
            tabbed("01JNODE/w1:p1", "t1", Some("claude"), "idle"),
            tabbed("01JNODE/w1:p2", "t2", None, "unknown")
        ]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.absorb(&Event::Prefs { greeting: true });

    let marked = |screen: &str| -> Vec<String> {
        screen
            .lines()
            .filter(|l| l.contains('\u{258c}'))
            .map(|l| l[..sidebar::WIDTH as usize].trim().to_string())
            .collect()
    };

    // A pane sits in `spaces` and, when it has an agent, in `agents` too — so the mark is on
    // every row that names it rather than on one row.
    let opened = marked(&painted(&mut app, 120, 20));
    assert!(
        !opened.is_empty() && opened.iter().all(|row| row.contains("claude")),
        "the marked rows are the agent pane the client opened on: {opened:?}"
    );

    app.clicked(Click::Focus("01JNODE/w1:p2".into()));
    let moved = marked(&painted(&mut app, 120, 20));
    assert!(
        !moved.is_empty() && moved.iter().all(|row| !row.contains("claude")),
        "the mark follows the focus rather than staying where it was: {moved:?}"
    );
}

/// The panel is the one screen a person opens when they do not know what this client is, and on a
/// terminal shorter than it the tail used to be cut with no way to reach it.
#[tokio::test]
async fn the_help_panel_reaches_its_last_row_on_a_terminal_too_short_to_hold_it() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "idle")]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
    let opened = painted(&mut app, 100, 20);
    assert!(
        opened.contains("the prefix is ctrl+b"),
        "the two facts every row assumes are said once, at the top:\n{opened}"
    );
    assert!(
        opened.contains("walk the sidebar"),
        "the first section is the one a newcomer needs:\n{opened}"
    );
    assert!(
        !opened.contains("kampr connect"),
        "this terminal cannot hold the whole panel, which is the point of the test:\n{opened}"
    );

    // Any key at all used to dismiss it, so it could not be paged even once it had pages.
    app.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    let bottom = painted(&mut app, 100, 20);
    assert!(
        bottom.contains("kampr connect"),
        "and the tail is reachable, which is where the answer to \"how do I pair\" lives:\n{bottom}"
    );

    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert!(
        !painted(&mut app, 100, 20).contains("the prefix is ctrl+b"),
        "esc closes it"
    );
}

/// A pane's directory is most of what says which pane it is, and the status line is the only
/// surface wide enough to hold one. `~` because the node's own `$HOME` is not on the wire.
#[tokio::test]
async fn the_status_line_says_where_the_pane_is_without_spelling_out_a_home_directory() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "idle")]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    let screen = painted(&mut app, 120, 20);
    assert!(screen.contains("~/dev/kampr"), "{screen}");
    assert!(
        !screen.contains("/home/dbrain"),
        "the home directory is the half a reader already knows:\n{screen}"
    );
}

/// **The navigator has to have something to navigate.** It used to force the herd screen for as
/// long as it was open, which was always drawn; moving it onto the sidebar meant it could be
/// entered against a sidebar that is hidden — `^b b` — or one too narrow to draw, and then the
/// cursor moved and the arrows were swallowed with nothing on screen to show for it.
#[tokio::test]
async fn the_navigator_brings_the_sidebar_back_rather_than_walking_one_nobody_can_see() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "idle")]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::NONE));
    let hidden = painted(&mut app, 120, 20);
    assert!(!hidden.contains("spaces"), "the sidebar is away:\n{hidden}");

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    let opened = painted(&mut app, 120, 20);
    assert!(
        opened.contains("spaces"),
        "asking to walk the sidebar brings it back:\n{opened}"
    );
}

/// A terminal too narrow for a sidebar drops it however the operator set the toggle
/// (`chrome.rs`'s `area.width > sidebar::WIDTH * 2`), so there the navigator has to fall back to
/// the one list that is always drawn.
#[tokio::test]
async fn a_terminal_too_narrow_for_a_sidebar_navigates_the_herd_instead() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "herdr", Some("claude"), "idle")]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    painted(&mut app, 50, 20);
    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    let screen = painted(&mut app, 50, 20);
    assert!(screen.contains("NAVIGATE"), "the mode is open:\n{screen}");
    // The herd view is a table; the panes screen draws a bordered box. The status line names the
    // pane on either, so the border is what tells them apart.
    assert!(
        !screen.contains('\u{250c}'),
        "and it fell back to the herd, which is drawn at any width:\n{screen}"
    );
}

/// A grid of `rows` lines, each naming its own index, so an assertion can say which line of the
/// pane landed on which line of the screen.
fn numbered(pane: &str, rows: u16) -> Value {
    json!({
        "t": "grid.reset", "pane": pane, "cols": 12, "rows": rows,
        "rows_data": (0..rows).map(|r| json!({ "row": r, "runs": [{ "s": 0, "x": format!("row{r}") }] }))
            .collect::<Vec<_>>(),
        "cursor": { "col": 0, "row": 0, "visible": false },
        "links": []
    })
}

#[tokio::test]
async fn the_pane_owns_every_row_below_the_strip_because_nothing_is_reserved_at_the_bottom() {
    // herdr reserves no row: at a 100x30 client the pane's own `tput lines` is 29 and the prompt
    // sits on the last one (#373). Kampr spent two of them on a status line and a hint bar that
    // herdr draws in no mode at all (#374), and a third and fourth on a border with nothing on
    // the other side of it (#375).
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
    conn.send(numbered("01JNODE/w1:p1", 11));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = app(&client);

    // 12 rows: one strip, eleven pane. A grid of eleven rows fits exactly.
    let screen = painted(&mut app, 60, 12);
    let lines: Vec<&str> = screen.lines().collect();
    assert_eq!(lines.len(), 12, "{screen}");
    assert!(
        lines[11].starts_with("row10"),
        "the last row of the screen is the last row of the pane:\n{screen}"
    );
    assert!(
        lines[1].starts_with("row0"),
        "and the first row under the strip is the pane's first:\n{screen}"
    );
    assert!(
        !screen.contains("^b ? help"),
        "herdr draws no hint bar in its pane keymap and neither does kampr:\n{screen}"
    );
}

#[tokio::test]
async fn a_lone_pane_has_no_border_and_a_second_pane_brings_one_back() {
    // #375: herdr's box appears only once there are two panes to separate.
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
    conn.send(numbered("01JNODE/w1:p1", 3));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = app(&client);

    let alone = painted(&mut app, 60, 12);
    assert!(
        !alone.contains('┌') && !alone.contains('└'),
        "one pane is flush, with no box around it:\n{alone}"
    );
    // A grid shorter than its box sits at the bottom of it, as a shell's own screen does — so
    // the row that proves the border is gone is the last one, hard against the screen's edge.
    assert!(
        alone.lines().nth(11).is_some_and(|l| l.starts_with("row2")),
        "and its last row is hard against the screen's bottom:\n{alone}"
    );

    // A second pane on the same tab is kampr's mosaic, and now there is something to separate.
    let mut beside = pane("01JNODE/w1:p2", "herdr", None, "idle");
    beside["id"] = json!("01JNODE/w1:p2");
    conn.send(json!({
        "t": "herd",
        "nodes": [node("01JNODE", "comingclean", true)],
        "panes": [pane("01JNODE/w1:p1", "herdr", None, "idle"), beside]
    }));
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;

    let split = painted(&mut app, 60, 12);
    assert!(
        split.contains('┌') && split.contains('└'),
        "two panes are boxed:\n{split}"
    );
}

#[tokio::test]
async fn the_prefix_footer_borrows_the_pane_s_last_row_and_hands_it_straight_back() {
    // #374: herdr's PREFIX/COPY/RESIZE footers paint over live pane content and vanish with the
    // mode. A row that is reserved for them is a row the pane never gets.
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
    conn.send(numbered("01JNODE/w1:p1", 11));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = app(&client);

    let before = painted(&mut app, 60, 12);
    assert!(
        before.lines().nth(11).is_some_and(|l| l.starts_with("row10")),
        "{before}"
    );

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    let held = painted(&mut app, 60, 12);
    assert!(
        held.lines().nth(11).is_some_and(|l| l.contains("PREFIX")),
        "the footer paints over the pane's own last row:\n{held}"
    );

    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    let after = painted(&mut app, 60, 12);
    assert!(
        after.lines().nth(11).is_some_and(|l| l.starts_with("row10")),
        "and the row goes back to the pane:\n{after}"
    );
}

#[tokio::test]
async fn the_bottom_row_says_what_is_wrong_and_is_otherwise_the_pane_s() {
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
    conn.send(numbered("01JNODE/w1:p1", 11));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = app(&client);

    let quiet = painted(&mut app, 60, 12);
    assert!(
        quiet.lines().nth(11).is_some_and(|l| l.starts_with("row10")),
        "nothing to say, so the row is the pane's:\n{quiet}"
    );

    // A demotion is worth a row: it changes what every key does.
    conn.send(json!({ "t": "role", "role": "readonly" }));
    until(&mut events, |e| matches!(e, Event::Role(_)).then_some(())).await;
    let demoted = painted(&mut app, 60, 12);
    assert!(
        demoted.lines().nth(11).is_some_and(|l| l.contains("readonly")),
        "a read-only device says so on the borrowed row:\n{demoted}"
    );

    // **A note is joined to what is standing, never substituted for it.** Something passing —
    // "sent", "copied 5 characters" — arriving must not be the reason the operator stops being
    // told the socket is down or that this device cannot type.
    app.note("something passing");
    let both = painted(&mut app, 60, 12);
    let row = both.lines().nth(11).unwrap_or_default();
    assert!(
        row.contains("readonly") && row.contains("something passing"),
        "the standing state survives the note:\n{both}"
    );
}

/// **The fit ladder explains itself once, when it climbs.** Its report is long — *"rung 3 · crop
/// and pan · rung 2 was not tried — this terminal did not answer CSI 14t"* — and it used to be a
/// standing tenant of the chrome, which on any pane wider than the window meant it was on screen
/// for ever. The borrowed row is for departures from steady state, and a ladder that settled some
/// minutes ago is not one; the pan window beside it says the pane is cropped in eight characters.
#[tokio::test]
async fn the_fit_ladders_explanation_is_raised_when_it_climbs_and_does_not_camp_on_the_row() {
    struct Headless(u16, u16);
    impl kampr_tui::render::fit::Display for Headless {
        fn cells(&mut self) -> Option<(u16, u16)> {
            Some((self.0, self.1))
        }
        fn host(&mut self) -> Option<String> {
            Some("headless".into())
        }
        fn largest(&mut self) -> Option<(u16, u16)> {
            Some((320, 90))
        }
        fn request(&mut self, _cols: u16, _rows: u16) {}
        fn settle(&mut self, _was: (u16, u16)) -> Option<(u16, u16)> {
            None
        }
    }

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
    conn.send(numbered("01JNODE/w1:p1", 4));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = app(&client);

    // Nothing has climbed yet, and the pane is drawn without a word about ladders.
    let before = painted(&mut app, 60, 12);
    assert!(!before.contains("rung"), "{before}");

    // A pane far wider than the window, against a terminal that refuses to grow (#291).
    app.fit(
        &mut Headless(60, 12),
        kampr_tui::render::fit::Need { cols: 200, rows: 4 },
        kampr_tui::render::fit::Chrome::default(),
    );
    let climbed = painted(&mut app, 60, 12);
    assert!(
        climbed.contains("crop and pan"),
        "the ladder says what it did, when it does it:\n{climbed}"
    );

    // And it is a note rather than a standing fact, so anything else with something to say takes
    // the row from it — which a permanent tenant could never allow.
    app.note("something else entirely");
    let after = painted(&mut app, 60, 12);
    assert!(
        after.contains("something else entirely") && !after.contains("crop and pan"),
        "the ladder is not still camped on the row:\n{after}"
    );

    // Rung 1 is not news. A wide terminal opening on a pane that fits says nothing about ladders,
    // which is most launches.
    let mut fresh = App::new(client.clone(), Options::default(), Images::default());
    fresh.refocus();
    fresh.fit(
        &mut Headless(200, 60),
        kampr_tui::render::fit::Need { cols: 80, rows: 24 },
        kampr_tui::render::fit::Chrome::default(),
    );
    let quiet = painted(&mut fresh, 200, 60);
    assert!(
        !quiet.contains("rung"),
        "the ordinary case announces nothing:\n{}",
        quiet.lines().next_back().unwrap_or_default()
    );
}
