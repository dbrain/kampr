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

mod mosaic;
mod tree;

const BEAT: Duration = Duration::from_secs(2);
/// Long enough for a frame the client wrote to reach the scripted node over a loopback socket,
/// and the window a "nothing was sent" assertion waits out.
const SETTLE: Duration = Duration::from_millis(250);

pub(crate) struct Conn {
    to_client: mpsc::UnboundedSender<Message>,
    from_client: mpsc::UnboundedReceiver<Value>,
}

impl Conn {
    pub(crate) fn send(&self, frame: Value) {
        self.to_client
            .send(Message::text(frame.to_string()))
            .expect("the scripted node still has a client");
    }

    /// Everything the client wrote in the window a loopback socket needs, so "nothing was sent"
    /// is an assertion about frames rather than about timing.
    pub(crate) async fn heard(&mut self) -> Vec<Value> {
        let mut frames = Vec::new();
        let deadline = tokio::time::Instant::now() + SETTLE;
        while let Ok(Some(frame)) = tokio::time::timeout_at(deadline, self.from_client.recv()).await {
            frames.push(frame);
        }
        frames
    }

    pub(crate) fn greet(&self, nodes: Value, panes: Value, role: &str) {
        self.send(json!({
            "t": "hello", "protocol": 1, "node_id": "01JNODE", "node_name": "comingclean",
            "build": "0.1.29", "role": role,
            "caps": { "push": false, "scrollback": true, "conversation": true, "manage": true }
        }));
        self.send(json!({ "t": "herd", "nodes": nodes, "panes": panes }));
        self.send(json!({ "t": "prefs", "panes": {} }));
    }
}

pub(crate) struct Fake {
    origin: String,
    conns: mpsc::UnboundedReceiver<Conn>,
}

impl Fake {
    pub(crate) async fn start() -> Self {
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

    pub(crate) async fn accept(&mut self) -> Conn {
        tokio::time::timeout(BEAT, self.conns.recv())
            .await
            .expect("no client dialled")
            .expect("the listener died")
    }

    pub(crate) fn client(&self) -> Client {
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

pub(crate) async fn until<T>(
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

pub(crate) fn node(id: &str, name: &str, online: bool) -> Value {
    json!({ "id": id, "name": name, "kind": "local", "online": online })
}

pub(crate) fn pane(id: &str, workspace: &str, agent: Option<&str>, status: &str) -> Value {
    json!({
        "id": id, "node_id": id.split('/').next().unwrap(),
        "workspace_id": "01JNODE/w1", "tab_id": "01JNODE/w1:t1",
        "workspace": workspace, "tab": "1", "cwd": "/home/dbrain/dev/kampr",
        "agent": agent, "agent_status": status, "rows": 4, "cols": 12
    })
}

pub(crate) fn painted(app: &mut App, width: u16, height: u16) -> String {
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

pub(crate) fn app(client: &Arc<Client>) -> App {
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
    // Panes are not dropped for an outage (#70): the row stays, with the count and the last-seen.
    assert!(
        screen.contains("laptop"),
        "an offline node keeps its row:\n{screen}"
    );
    assert!(screen.contains("offline"), "{screen}");
    assert!(screen.contains("1 pane · seen 13:44"), "{screen}");
}

/// A machine with no herdr still serves the sessions Kampr keeps, and drawing it offline hid the
/// one thing on it that worked.
#[tokio::test]
async fn a_machine_with_no_herdr_serving_native_sessions_is_not_drawn_offline() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut kept = pane("01JSHED/n:01ja", "kept", Some("claude"), "idle");
    kept["node_id"] = json!("01JSHED");
    kept["provider"] = json!("native");
    conn.greet(
        json!([{
            "id": "01JSHED", "name": "shed", "kind": "local", "online": false,
            "reachable": true, "native": true, "detail": "herdr socket: No such file",
        }]),
        json!([kept]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    let screen = painted(&mut app, 110, 22);
    let spaces = sidebar_of(&screen);
    assert!(!spaces.contains("offline"), "{screen}");
    assert!(spaces.contains("no herdr"), "{screen}");
    assert!(spaces.contains("  kept"), "{screen}");
}

/// A node whose operator turned herdr off, or never installed it, is just a machine serving its
/// sessions: nothing on the sidebar says herdr is missing.
#[tokio::test]
async fn a_machine_with_herdr_off_never_says_herdr() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut kept = pane("01JSHED/n:01ja", "kept", Some("claude"), "idle");
    kept["node_id"] = json!("01JSHED");
    kept["provider"] = json!("native");
    conn.greet(
        json!([{
            "id": "01JSHED", "name": "shed", "kind": "local", "online": false,
            "reachable": true, "native": true, "herdr": false,
        }]),
        json!([kept]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    let screen = painted(&mut app, 110, 22);
    let spaces = sidebar_of(&screen);
    assert!(!spaces.contains("herdr"), "{screen}");
    assert!(!spaces.contains("offline"), "{screen}");
    assert!(spaces.contains("  kept"), "{screen}");
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

    let opened = marked(&painted(&mut app, 120, 20));
    assert_eq!(
        opened.len(),
        1,
        "one row names the pane the client opened on: {opened:?}"
    );
    assert!(opened[0].contains('\u{25cb}'), "the idle agent: {opened:?}");

    app.clicked(Click::Focus("01JNODE/w1:p2".into()));
    let moved = marked(&painted(&mut app, 120, 20));
    assert_eq!(moved.len(), 1, "{moved:?}");
    assert!(
        !moved[0].contains('\u{25cb}'),
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

/// A ring of `rows` history lines, each naming its own index, above the pane's live grid.
pub(crate) fn history(pane: &str, rows: u16) -> Value {
    json!({
        "t": "scrollback", "pane": pane,
        "rows": (0..rows).map(|r| json!({ "row": r, "runs": [{ "s": 0, "x": format!("hist{r}") }] }))
            .collect::<Vec<_>>(),
        "from_top": 0, "total_rows": rows, "complete": true, "capped": false, "era": 0
    })
}

/// The runtime's own loop: every frame the node says goes to the app, and the loop stops at the
/// one it was waiting for.
pub(crate) async fn absorb_until(
    app: &mut App,
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    want: impl Fn(&Event) -> bool,
) {
    for _ in 0..64 {
        let event = tokio::time::timeout(BEAT, events.recv())
            .await
            .expect("no event arrived")
            .expect("the event stream ended");
        app.absorb(&event);
        if want(&event) {
            return;
        }
    }
    panic!("the event never arrived");
}

#[tokio::test]
async fn a_pane_that_grows_while_scrolled_up_keeps_the_row_under_the_eye_fixed() {
    // Scrolling up into scrollback and holding, the window below the viewport used to keep
    // streaming: the surface grew, and a scroll measured from the tail moved with it, so the
    // anchored row drifted as the tail advanced.
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
    let mut app = app(&client);
    conn.send(numbered("01JNODE/w1:p1", 10));
    absorb_until(&mut app, &mut events, |e| matches!(e, Event::Grid { .. })).await;
    conn.send(history("01JNODE/w1:p1", 20));
    absorb_until(&mut app, &mut events, |e| matches!(e, Event::Scrollback { .. })).await;

    // At the tail, the pane's last row sits on the bottom row of the pane.
    let tail = painted(&mut app, 60, 12);
    assert!(
        tail.lines().nth(11).is_some_and(|l| l.starts_with("row9")),
        "{tail}"
    );

    // One page up: the anchored rows are the ones the window sits on. The bottom row is the
    // borrowed status line while a scroll is held, so the anchor is read from the window itself.
    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::PageUp, KeyModifiers::NONE));
    let held = painted(&mut app, 60, 12);
    assert!(
        held.lines().nth(1).is_some_and(|l| l.starts_with("hist10")),
        "{held}"
    );
    assert!(
        held.lines().nth(10).is_some_and(|l| l.starts_with("hist19")),
        "{held}"
    );

    // Four more rows land at the tail. The window must not move.
    conn.send(numbered("01JNODE/w1:p1", 14));
    absorb_until(&mut app, &mut events, |e| matches!(e, Event::Grid { .. })).await;
    let grown = painted(&mut app, 60, 12);
    assert!(
        grown.lines().nth(1).is_some_and(|l| l.starts_with("hist10")),
        "the anchored row drifted as the tail advanced:\n{grown}"
    );
    assert!(
        grown.lines().nth(10).is_some_and(|l| l.starts_with("hist19")),
        "the window below the viewport kept streaming:\n{grown}"
    );

    // Scrolling down pages the rest of the surface, and the tail is reachable.
    for _ in 0..2 {
        app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
        app.key(KeyEvent::new(KeyCode::PageDown, KeyModifiers::NONE));
    }
    let down = painted(&mut app, 60, 12);
    assert!(
        down.lines().nth(11).is_some_and(|l| l.starts_with("row13")),
        "the tail is where the window ends:\n{down}"
    );

    // At the tail, following resumes: a pane that grows puts its new last row on the bottom.
    conn.send(numbered("01JNODE/w1:p1", 18));
    absorb_until(&mut app, &mut events, |e| matches!(e, Event::Grid { .. })).await;
    let following = painted(&mut app, 60, 12);
    assert!(
        following.lines().nth(11).is_some_and(|l| l.starts_with("row17")),
        "the tail is back where the pane's last row is:\n{following}"
    );
}

#[tokio::test]
async fn a_fresh_pane_that_grows_keeps_following_the_tail() {
    // The pin is for a scroll that is held; a pane nobody scrolled never leaves the tail.
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
    conn.send(numbered("01JNODE/w1:p1", 10));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = app(&client);

    let tail = painted(&mut app, 60, 12);
    assert!(
        tail.lines().nth(11).is_some_and(|l| l.starts_with("row9")),
        "{tail}"
    );

    conn.send(numbered("01JNODE/w1:p1", 14));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let grown = painted(&mut app, 60, 12);
    assert!(
        grown.lines().nth(11).is_some_and(|l| l.starts_with("row13")),
        "the unscrolled pane follows the tail:\n{grown}"
    );
}

fn native(id: &str, agent: Option<&str>, cols: u16) -> Value {
    let mut entry = pane(id, "kept", agent, "idle");
    entry["provider"] = json!("native");
    entry["cols"] = json!(cols);
    entry["rows"] = json!(30);
    entry
}

/// The reflow each `watch` this client sent for `pane` asked for, in order; `None` is a watch of
/// the pane's own grid.
fn reflows(frames: &[Value], pane: &str) -> Vec<Option<u64>> {
    frames
        .iter()
        .filter(|f| f["t"] == json!("watch") && f["pane"] == json!(pane))
        .map(|f| f["reflow"]["cols"].as_u64())
        .collect()
}

fn unmatched() -> Options {
    Options {
        match_view: false,
        ..Options::default()
    }
}

async fn greeted(fake: &mut Fake, panes: Value) -> (Arc<Client>, Conn) {
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([node("01JNODE", "comingclean", true)]), panes, "full");
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    (client, conn)
}

/// docs/17 §3, as the phone does it: a native shell wider than the box it is drawn in is re-laid
/// at the box's width by the node, rather than cropped and panned, and a box that changes width
/// asks again. Nothing reaches the PTY: the pane is still 200 columns to everyone else.
#[tokio::test]
async fn a_native_shell_wider_than_its_box_is_re_laid_at_the_boxs_width() {
    let mut fake = Fake::start().await;
    let at = "01JNODE/n:01ja";
    let (client, mut conn) = greeted(&mut fake, json!([native(at, None, 200)])).await;
    let mut app = App::new(client.clone(), unmatched(), Images::default());
    app.absorb(&Event::Prefs { greeting: true });

    painted(&mut app, 120, 30);
    app.reflow_views();
    let wide = app.layout.pane(at).expect("the pane is drawn").inner.width;
    painted(&mut app, 100, 30);
    app.reflow_views();
    let narrow = app.layout.pane(at).expect("the pane is drawn").inner.width;
    assert!(narrow < wide && wide < 200, "{narrow} {wide}");
    let frames = conn.heard().await;
    assert_eq!(
        reflows(&frames, at),
        vec![None, Some(u64::from(wide)), Some(u64::from(narrow))],
        "{frames:?}"
    );
    assert!(
        !frames.iter().any(|f| f["op"] == json!("pane.size")),
        "re-laying is not a resize: {frames:?}"
    );

    painted(&mut app, 100, 30);
    app.reflow_views();
    assert_eq!(
        reflows(&conn.heard().await, at),
        Vec::<Option<u64>>::new(),
        "asked again for the same width"
    );
}

/// Re-laid, the grid the node sends is the box's width, so that is the width the pane has here:
/// measured from the herd's 200 it would pan into nothing, wrap a copy at the wrong column and
/// ask the terminal to grow to a pane it is already showing whole. Any other answer is the
/// pane's own grid, at the pane's own width.
#[tokio::test]
async fn a_re_laid_grid_is_as_wide_as_it_was_sent_and_any_other_answer_is_the_panes_own() {
    let mut fake = Fake::start().await;
    let at = "01JNODE/n:01ja";
    let (client, conn) = greeted(&mut fake, json!([native(at, None, 200)])).await;
    let mut events = client.events();
    let mut app = App::new(client.clone(), unmatched(), Images::default());
    app.absorb(&Event::Prefs { greeting: true });
    for (served, cols, want) in [("on", 60, 60), ("alternate", 200, 200), ("harness", 200, 200)] {
        conn.send(json!({
            "t": "grid.reset", "pane": at, "cols": cols, "rows": 30, "rows_data": [],
            "cursor": { "col": 0, "row": 0, "visible": false }, "links": [], "reflow": served
        }));
        until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
        let need = app.need(at).expect("a painted pane has a size");
        assert_eq!((need.cols, need.rows), (want, 30), "{served}");
    }
}

/// What the phone leaves alone, left alone here: an agent wraps its own lines at the PTY's
/// width (#588), a herdr pane cannot be re-laid, a box as wide as the pane has nothing to gain,
/// and a pane this window holds at its own size is that size already.
#[tokio::test]
async fn an_agent_a_herdr_pane_a_pane_that_fits_and_a_matched_pane_are_never_re_laid() {
    for (entry, options, width) in [
        (native("01JNODE/n:01ja", Some("claude"), 200), unmatched(), 100),
        (
            {
                let mut herdr = pane("01JNODE/w1:p1", "herdr", None, "idle");
                herdr["cols"] = json!(200);
                herdr
            },
            unmatched(),
            100,
        ),
        (native("01JNODE/n:01ja", None, 40), unmatched(), 100),
        (native("01JNODE/n:01ja", None, 200), Options::default(), 120),
    ] {
        let mut fake = Fake::start().await;
        let at = entry["id"].as_str().expect("an id").to_string();
        let (client, mut conn) = greeted(&mut fake, json!([entry])).await;
        let mut app = App::new(client.clone(), options, Images::default());
        app.absorb(&Event::Prefs { greeting: true });
        painted(&mut app, width, 40);
        app.reflow_views();
        let asked = reflows(&conn.heard().await, &at);
        assert_eq!(asked, vec![None], "{at} at {width} with {options:?}");
    }
}

fn sidebar_of(screen: &str) -> String {
    screen
        .lines()
        .map(|l| l.chars().take(sidebar::WIDTH as usize).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

/// **One list, two orders, and the order is a control.** The sidebar drew `spaces` and then an
/// `agents` section that listed every agent pane a second time, under a `priority` label nothing
/// could press. Now each pane is one row; the header's `grouped`/`priority` is the switch, by click
/// or by `o` in the navigator, and priority is herdr's own rollup order.
#[tokio::test]
async fn the_sidebar_lists_each_pane_once_and_its_order_is_a_control() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut quiet = pane("01JNODE/w1:p1", "alpha", Some("claude"), "idle");
    quiet["title"] = json!("tidy the docs");
    let mut stuck = pane("01JNODE/w2:p2", "beta", Some("claude"), "blocked");
    stuck["workspace_id"] = json!("01JNODE/w2");
    stuck["tab_id"] = json!("01JNODE/w2:t1");
    stuck["title"] = json!("fix the wheel");
    let mut shell = pane("01JNODE/w2:p3", "beta", None, "unknown");
    shell["workspace_id"] = json!("01JNODE/w2");
    shell["tab_id"] = json!("01JNODE/w2:t1");
    shell["cmd"] = json!("cargo");
    shell["argv"] = json!("cargo test");
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([quiet, stuck, shell]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    let grouped = sidebar_of(&painted(&mut app, 120, 24));
    assert!(!grouped.contains("agents"), "{grouped}");
    assert_eq!(grouped.matches("fix the wheel").count(), 1, "{grouped}");
    assert!(
        !grouped.contains("· claude"),
        "an agent's row is its title; the glyph says it is an agent:\n{grouped}"
    );
    assert!(
        grouped.contains("cargo test"),
        "a shell's row still says what it is running:\n{grouped}"
    );
    assert!(
        !grouped.contains("beta · cargo"),
        "and not the workspace its header already names:\n{grouped}"
    );
    let line = |screen: &str, text: &str| -> String {
        screen
            .lines()
            .find(|l| l.contains(text))
            .unwrap_or_else(|| panic!("no row with {text}:\n{screen}"))
            .to_string()
    };
    let indent = |row: String| row.chars().take_while(|c| [' ', '\u{258c}'].contains(c)).count();
    let (node_at, space_at, pane_at) = (
        line(&grouped, "comingclean"),
        line(&grouped, "beta"),
        line(&grouped, "fix the wheel"),
    );
    assert!(
        node_at.starts_with(" comingclean"),
        "a node sits flush under the header:\n{grouped}"
    );
    assert!(
        indent(node_at.clone()) < indent(space_at.clone()) && indent(space_at) < indent(pane_at),
        "and each level below it steps in:\n{grouped}"
    );
    let order = |screen: &str| -> (usize, usize) {
        let at = |t: &str| screen.lines().position(|l| l.contains(t)).unwrap_or(usize::MAX);
        (at("tidy the docs"), at("fix the wheel"))
    };
    let (quiet_at, stuck_at) = order(&grouped);
    assert!(quiet_at < stuck_at, "grouped keeps the herd's order:\n{grouped}");

    let toggle = app.layout.order.expect("the order switch is drawn");
    let click = app.mouse.hit(
        crossterm::event::MouseEvent {
            kind: crossterm::event::MouseEventKind::Down(crossterm::event::MouseButton::Left),
            column: toggle.x,
            row: toggle.y,
            modifiers: KeyModifiers::NONE,
        },
        &app.layout,
        kampr_client::Role::Full,
    );
    app.clicked(click);
    let priority = sidebar_of(&painted(&mut app, 120, 24));
    let (quiet_at, stuck_at) = order(&priority);
    assert!(
        stuck_at < quiet_at,
        "priority puts the blocked pane first:\n{priority}"
    );
    assert_eq!(priority.matches("fix the wheel").count(), 1, "{priority}");

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('w'), KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::NONE));
    let back = sidebar_of(&painted(&mut app, 120, 24));
    let (quiet_at, stuck_at) = order(&back);
    assert!(
        quiet_at < stuck_at,
        "`o` in the navigator switches it back:\n{back}"
    );
}

/// **No way to a session without the mouse but walking the sidebar row by row.** `prefix g` is
/// herdr's own `goto`; here it is a filter over every pane on every node — a few letters of the
/// title, the workspace or the machine, enter, and the frame is that pane.
#[tokio::test]
async fn prefix_g_finds_a_pane_on_any_node_by_a_few_letters() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut here = pane("01JNODE/w1:p1", "alpha", Some("claude"), "idle");
    here["title"] = json!("tidy the docs");
    let mut away = pane("01JWORKBOX/w7:p2", "data-pipeline", Some("codex"), "working");
    away["node_id"] = json!("01JWORKBOX");
    away["workspace_id"] = json!("01JWORKBOX/w7");
    away["tab_id"] = json!("01JWORKBOX/w7:t1");
    away["title"] = json!("backfill the events");
    let mut other = pane("01JWORKBOX/w7:p3", "data-pipeline", None, "unknown");
    other["node_id"] = json!("01JWORKBOX");
    other["workspace_id"] = json!("01JWORKBOX/w7");
    other["tab_id"] = json!("01JWORKBOX/w7:t2");
    other["cmd"] = json!("htop");
    conn.greet(
        json!([
            node("01JNODE", "comingclean", true),
            { "id": "01JWORKBOX", "name": "workbox", "kind": "peer", "online": true, "rtt_ms": 41.0 }
        ]),
        json!([here, away, other]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    let start = app.focused().map(str::to_string);
    let typed = |app: &mut App, text: &str| {
        for c in text.chars() {
            app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    };

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    typed(&mut app, "backfl");
    let screen = painted(&mut app, 120, 24);
    assert!(screen.contains("backfl"), "the query is on screen:\n{screen}");
    assert!(screen.contains("backfill the events"), "{screen}");
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(app.focused(), Some("01JWORKBOX/w7:p2"));
    let after = painted(&mut app, 120, 24);
    assert!(!after.contains("backfl▏"), "enter closes it:\n{after}");

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    typed(&mut app, "workbox");
    app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE));
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(
        app.focused(),
        Some("01JWORKBOX/w7:p3"),
        "the machine's name matches too, and down picks the next match"
    );

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    typed(&mut app, "tidy");
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!(app.focused(), Some("01JWORKBOX/w7:p3"), "esc goes nowhere");
    assert_ne!(start.as_deref(), Some("01JWORKBOX/w7:p3"));
}

fn kept_in(id: &str, label: &str, space: &str, tab: (&str, &str), tile: Value) -> Value {
    json!({
        "id": id, "node_id": "01JSHED", "provider": "native", "label": label,
        "workspace": space, "workspace_id": format!("01JSHED/kw:{space}"),
        "tab": tab.1, "tab_id": format!("01JSHED/{}", tab.0), "tile": tile,
        "cwd": "/home/dbrain/dev/kampr", "agent_status": "idle", "rows": 4, "cols": 12
    })
}

fn shed() -> Value {
    json!([{ "id": "01JSHED", "name": "shed", "kind": "local", "online": false,
             "reachable": true, "native": true, "herdr": false }])
}

/// Kampr's own workspaces are drawn as herdr's are, and a workspace with more than one tab says
/// which tab each pane is in.
#[tokio::test]
async fn a_kampr_workspace_groups_its_sessions_and_names_its_tabs() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        shed(),
        json!([
            kept_in("01JSHED/n:a", "server", "api", ("k:a", "serve"), Value::Null),
            kept_in("01JSHED/n:b", "tests", "api", ("k:b", "check"), Value::Null),
        ]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);

    let spaces = sidebar_of(&painted(&mut app, 110, 22));
    assert_eq!(spaces.matches("  api").count(), 1, "{spaces}");
    let at = |word: &str| {
        spaces
            .lines()
            .position(|l| l.contains(word))
            .unwrap_or_else(|| panic!("no {word}:\n{spaces}"))
    };
    assert!(
        at("  api") < at("serve") && at("serve") < at("server"),
        "{spaces}"
    );
    assert!(
        at("server") < at("check") && at("check") < at("tests"),
        "{spaces}"
    );
}

/// A tree too big for the terminal is not drawn as boxes too small to read: the focused session
/// takes the room, and a strip names the rest of the tab.
#[tokio::test]
async fn a_tab_too_big_for_a_small_terminal_is_flattened_to_a_strip() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let cut = |second: bool| json!({ "group": "01JSHED/k:a", "path": [{ "split": "right", "ratio": 500, "second": second }] });
    conn.greet(
        shed(),
        json!([
            kept_in("01JSHED/n:a", "server", "api", ("k:a", "1"), cut(false)),
            kept_in("01JSHED/n:b", "tests", "api", ("k:a", "1"), cut(true)),
        ]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    app.key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('b'),
        crossterm::event::KeyModifiers::CONTROL,
    ));
    app.key(crossterm::event::KeyEvent::new(
        crossterm::event::KeyCode::Char('b'),
        crossterm::event::KeyModifiers::NONE,
    ));

    let wide = painted(&mut app, 120, 20);
    assert!(wide.contains("┐┌"), "two boxes side by side:\n{wide}");

    let narrow = painted(&mut app, 40, 12);
    assert!(!narrow.contains("┐┌"), "boxes too narrow to read:\n{narrow}");
    assert!(
        narrow
            .lines()
            .any(|l| l.contains("server") && l.contains("tests")),
        "a strip names both:\n{narrow}"
    );
}
