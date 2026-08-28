//! W5 — hit testing, selection, and the pane passthrough toggle.
//!
//! herdr's observe frames carry no mouse mode and no other surface on its socket does either
//! (#292), so nothing here asks a node whether a click means anything: the chrome is the
//! client's own, and a pane is driven only when the operator armed it.

use crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use futures_util::{SinkExt, StreamExt};
use kampr_client::{Client, Event, PendingOption, Policy, Role, Session, Via};
use kampr_core::Backoff;
use kampr_term::{Cell, CellAttrs, Color};
use kampr_tui::app::{App, Layout, Options, Placed};
use kampr_tui::image::Images;
use kampr_tui::mouse::{Click, Link, Mouse};
use kampr_tui::render::fit::{Pan, Placement};
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::layout::Rect;
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

fn cell(ch: char) -> Cell {
    Cell {
        ch,
        fg: Color::Default,
        bg: Color::Default,
        attrs: CellAttrs::default(),
        link: None,
        marks: None,
    }
}

fn linked(ch: char, id: u32) -> Cell {
    Cell {
        link: Some(id),
        ..cell(ch)
    }
}

fn row(text: &str) -> Vec<Cell> {
    text.chars().map(cell).collect()
}

fn padded(text: &str, cols: usize) -> Vec<Cell> {
    let mut row = row(text);
    while row.len() < cols {
        row.push(cell(' '));
    }
    row
}

/// One pane with an empty ring, drawn unpanned and unscrolled — the placement a bordered box of
/// this size produces when the whole grid fits inside it, which is what the body-relative
/// coordinates these tests were written against mean.
fn one_pane(rect: Rect) -> Layout {
    let body = Rect::new(rect.x + 1, rect.y + 1, rect.width - 2, rect.height - 2);
    Layout {
        panes: vec![Placed {
            pane: "01JNODE/w1:p1".to_string(),
            rect,
            ring: 0,
            placement: Some(Placement {
                history: Rect::new(body.x, body.y, body.width, 0),
                grid: body,
                skip_history: 0,
                skip_grid: 0,
                pan: Pan::default(),
                scroll: 0,
            }),
        }],
        ..Layout::default()
    }
}

fn at(kind: MouseEventKind, x: u16, y: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    }
}

fn down(x: u16, y: u16) -> MouseEvent {
    at(MouseEventKind::Down(MouseButton::Left), x, y)
}

fn drag(x: u16, y: u16) -> MouseEvent {
    at(MouseEventKind::Drag(MouseButton::Left), x, y)
}

fn up(x: u16, y: u16) -> MouseEvent {
    at(MouseEventKind::Up(MouseButton::Left), x, y)
}

/// The pane is focused and the click that arrives on it is spent focusing it, so every test
/// about what a *second* click does starts here.
fn arrived(mouse: &mut Mouse, layout: &Layout, role: Role) {
    assert!(matches!(mouse.hit(down(5, 5), layout, role), Click::Focus(_)));
    let _ = mouse.hit(up(5, 5), layout, role);
}

fn dragged(mouse: &mut Mouse, layout: &Layout, from: (u16, u16), to: (u16, u16)) {
    mouse.hit(down(from.0, from.1), layout, Role::Full);
    mouse.hit(drag(to.0, to.1), layout, Role::Full);
    mouse.hit(up(to.0, to.1), layout, Role::Full);
}

#[test]
fn a_click_on_a_pane_body_focuses_it_and_puts_nothing_into_it() {
    let layout = one_pane(Rect::new(0, 0, 20, 10));
    let mut mouse = Mouse::new();
    assert_eq!(
        mouse.hit(down(5, 5), &layout, Role::Full),
        Click::Focus("01JNODE/w1:p1".into())
    );
    // The pane was never armed, so no further click ever becomes a report either.
    for event in [down(6, 6), drag(7, 6), up(7, 6)] {
        assert!(!matches!(
            mouse.hit(event, &layout, Role::Full),
            Click::Passthrough { .. }
        ));
    }
}

#[test]
fn an_armed_pane_reports_the_cell_the_click_landed_on_as_an_sgr_1006_press() {
    let layout = one_pane(Rect::new(0, 0, 20, 10));
    let mut mouse = Mouse::new();
    mouse.set_passthrough("01JNODE/w1:p1", true);
    arrived(&mut mouse, &layout, Role::Full);
    // The border is not the grid: the body of a 20x10 box at the origin starts at (1, 1), so a
    // click at (5, 4) is the fourth row's fifth column, one-based as SGR counts them.
    assert_eq!(
        mouse.hit(down(5, 4), &layout, Role::Full),
        Click::Passthrough {
            pane: "01JNODE/w1:p1".into(),
            text: "\u{1b}[<0;5;4M".into()
        }
    );
}

#[test]
fn the_first_click_on_an_armed_pane_focuses_it_rather_than_typing_into_it() {
    let layout = one_pane(Rect::new(0, 0, 20, 10));
    let mut mouse = Mouse::new();
    mouse.set_passthrough("01JNODE/w1:p1", true);
    assert_eq!(
        mouse.hit(down(5, 5), &layout, Role::Full),
        Click::Focus("01JNODE/w1:p1".into()),
        "arriving from somewhere else cannot put a byte into a pane"
    );
}

#[test]
fn a_drag_over_an_armed_pane_reports_once_per_cell_and_the_release_closes_it() {
    let layout = one_pane(Rect::new(0, 0, 20, 10));
    let mut mouse = Mouse::new();
    mouse.set_passthrough("01JNODE/w1:p1", true);
    arrived(&mut mouse, &layout, Role::Full);
    mouse.hit(down(5, 4), &layout, Role::Full);
    assert_eq!(
        mouse.hit(drag(5, 4), &layout, Role::Full),
        Click::None,
        "1002, not 1003: motion inside one cell is not a report"
    );
    assert_eq!(
        mouse.hit(drag(6, 4), &layout, Role::Full),
        Click::Passthrough {
            pane: "01JNODE/w1:p1".into(),
            text: "\u{1b}[<32;6;4M".into()
        }
    );
    assert_eq!(
        mouse.hit(up(6, 4), &layout, Role::Full),
        Click::Passthrough {
            pane: "01JNODE/w1:p1".into(),
            text: "\u{1b}[<0;6;4m".into()
        }
    );
}

#[test]
fn the_wheel_over_an_armed_pane_reports_and_over_an_unarmed_one_does_not() {
    let layout = one_pane(Rect::new(0, 0, 20, 10));
    let mut mouse = Mouse::new();
    arrived(&mut mouse, &layout, Role::Full);
    assert_eq!(
        mouse.hit(at(MouseEventKind::ScrollUp, 5, 4), &layout, Role::Full),
        Click::None
    );
    mouse.set_passthrough("01JNODE/w1:p1", true);
    assert_eq!(
        mouse.hit(at(MouseEventKind::ScrollUp, 5, 4), &layout, Role::Full),
        Click::Passthrough {
            pane: "01JNODE/w1:p1".into(),
            text: "\u{1b}[<64;5;4M".into()
        }
    );
    assert_eq!(
        mouse.hit(at(MouseEventKind::ScrollDown, 5, 4), &layout, Role::Full),
        Click::Passthrough {
            pane: "01JNODE/w1:p1".into(),
            text: "\u{1b}[<65;5;4M".into()
        }
    );
}

#[test]
fn a_readonly_device_puts_nothing_into_a_pane_however_the_toggle_is_set() {
    let layout = one_pane(Rect::new(0, 0, 20, 10));
    let mut mouse = Mouse::new();
    mouse.set_passthrough("01JNODE/w1:p1", true);
    assert!(mouse.passes_through("01JNODE/w1:p1"));
    arrived(&mut mouse, &layout, Role::Readonly);
    for event in [
        down(5, 4),
        drag(6, 4),
        up(6, 4),
        at(MouseEventKind::ScrollUp, 5, 4),
    ] {
        assert!(
            !matches!(
                mouse.hit(event, &layout, Role::Readonly),
                Click::Passthrough { .. }
            ),
            "a read-only device sends nothing into a pane"
        );
    }
    // And the drag it did instead is a selection, which costs the pane nothing.
    assert!(mouse.selection().is_some());
}

#[test]
fn a_selection_across_a_soft_wrap_copies_as_one_logical_line() {
    let layout = one_pane(Rect::new(0, 0, 24, 10));
    let mut mouse = Mouse::new();
    let mut first = row("https://herdr.dev/a");
    first.extend(row("bcd"));
    let rows = vec![first, padded("efg", 22)];
    // Body origin (1, 1): the drag runs from the first cell of row 0 to the last of "efg".
    dragged(&mut mouse, &layout, (1, 1), (3, 2));
    assert_eq!(
        mouse.copy(&rows, 22).as_deref(),
        Some("https://herdr.dev/abcdefg"),
        "a URL copied with a newline through the middle of it is worse than not copying"
    );
    assert_eq!(mouse.copy(&rows, 22), None, "a copy is taken once");
}

#[test]
fn a_selection_that_starts_on_the_tail_of_a_wide_glyph_resolves_to_the_lead_column() {
    let layout = one_pane(Rect::new(0, 0, 20, 10));
    let mut mouse = Mouse::new();
    let rows = vec![vec![cell('A'), cell('日'), cell('\0'), cell('B')]];
    // Body origin (1, 1), so column 2 is the glyph's second column — which belongs to the glyph.
    dragged(&mut mouse, &layout, (3, 1), (4, 1));
    assert_eq!(
        mouse.copy(&rows, 4).as_deref(),
        Some("日B"),
        "the column after a wide glyph belongs to that glyph, or the copy is a glyph out"
    );
}

#[test]
fn block_selection_keeps_the_columns_apart_and_is_not_the_default() {
    let layout = one_pane(Rect::new(0, 0, 20, 10));
    let rows = vec![padded("abcdef", 8), padded("ghijkl", 8)];
    let mut linear = Mouse::new();
    dragged(&mut linear, &layout, (3, 1), (3, 2));
    assert_eq!(
        linear.copy(&rows, 8).as_deref(),
        Some("cdef\nghi"),
        "linear by default: it flows across the rows like a paragraph"
    );
    let mut block = Mouse::new();
    block.hit(
        MouseEvent {
            modifiers: KeyModifiers::ALT,
            ..down(3, 1)
        },
        &layout,
        Role::Full,
    );
    block.hit(drag(4, 2), &layout, Role::Full);
    block.hit(up(4, 2), &layout, Role::Full);
    assert_eq!(block.copy(&rows, 8).as_deref(), Some("cd\nij"));
}

#[test]
fn a_declared_link_is_the_harnesss_and_a_bare_url_is_only_ever_detected() {
    let layout = one_pane(Rect::new(0, 0, 30, 10));
    let mut declared = Mouse::new();
    let rows = vec![vec![linked('d', 0), linked('o', 0), linked('c', 0)]];
    declared.hit(down(2, 1), &layout, Role::Full);
    assert_eq!(
        declared.link(&rows, &["https://herdr.dev".to_string()], 3),
        Some(Link::Declared("https://herdr.dev".into())),
        "an OSC 8 URI is a real harness-declared one and survives here (#36/#37)"
    );

    // Detection runs over the *logical* line, so a URL the grid wrapped is still one URL.
    let mut bare = row("see https://herdr.dev/");
    bare.extend(row("do"));
    let wrapped = vec![bare, padded("cs and more", 24)];
    let mut detector = Mouse::new();
    detector.hit(down(11, 1), &layout, Role::Full);
    assert_eq!(
        detector.link(&wrapped, &[], 24),
        Some(Link::Detected("https://herdr.dev/docs".into())),
        "a URL wrapped at the grid edge is not two URLs"
    );

    detector.hit(down(2, 1), &layout, Role::Full);
    assert_eq!(
        detector.link(&wrapped, &[], 24),
        None,
        "\"see\" is not a link, and detected is not declared"
    );
}

#[test]
fn a_pending_prompt_sends_only_a_key_it_offered() {
    let mouse = Mouse::new();
    let options = vec![
        PendingOption {
            key: "1".into(),
            label: "Yes".into(),
        },
        PendingOption {
            key: "2".into(),
            label: "No".into(),
        },
    ];
    let rects = vec![Rect::new(0, 0, 8, 1), Rect::new(8, 0, 8, 1)];
    assert_eq!(
        mouse.answer("01JNODE/w1:p1", &options, &rects, (9, 0), Role::Full),
        Click::Answer {
            pane: "01JNODE/w1:p1".into(),
            key: "2".into()
        }
    );
    assert_eq!(
        mouse.answer("01JNODE/w1:p1", &options, &rects, (20, 0), Role::Full),
        Click::None,
        "nothing is offered outside an option, and an Enter is never synthesised (#43)"
    );
    assert_eq!(
        mouse.answer("01JNODE/w1:p1", &options, &rects, (9, 0), Role::Readonly),
        Click::None
    );
}

#[test]
fn the_chrome_is_clickable_whatever_a_pane_is_doing() {
    let mouse = Mouse::new();
    assert!(mouse.capture(), "the tabs are clickable on a fresh client");
    assert_eq!(mouse.footer("01JNODE/w1:p1"), None);
    let mut armed = Mouse::new();
    armed.set_passthrough("01JNODE/w1:p1", true);
    assert_eq!(
        armed.footer("01JNODE/w1:p1").as_deref(),
        Some("mouse → pane"),
        "a mode that changes what a click does is never invisible"
    );
}

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

    fn greet(&self, panes: Value, prefs: Value) {
        self.send(json!({
            "t": "hello", "protocol": 1, "node_id": "01JNODE", "node_name": "comingclean",
            "build": "0.1.29", "role": "full",
            "caps": { "push": false, "scrollback": true, "conversation": true, "manage": true }
        }));
        self.send(json!({
            "t": "herd",
            "nodes": [{ "id": "01JNODE", "name": "comingclean", "kind": "local", "online": true }],
            "panes": panes
        }));
        self.send(json!({ "t": "prefs", "panes": prefs }));
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

fn pane(id: &str, tab: &str) -> Value {
    json!({
        "id": id, "node_id": "01JNODE",
        "workspace_id": "01JNODE/w1", "tab_id": format!("01JNODE/w1:{tab}"),
        "workspace": "herdr", "tab": tab, "agent": null, "agent_status": "idle",
        "rows": 4, "cols": 12
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

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent {
        code,
        modifiers,
        kind: KeyEventKind::Press,
        state: crossterm::event::KeyEventState::NONE,
    }
}

fn inputs(frames: &[Value]) -> Vec<&Value> {
    frames
        .iter()
        .filter(|f| f["t"] == json!("input"))
        .collect::<Vec<_>>()
}

#[tokio::test]
async fn a_click_on_a_pane_body_reaches_the_node_as_nothing_at_all() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(json!([pane("01JNODE/w1:p1", "t1")]), json!({}));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.adopt_prefs();
    app.refocus();
    painted(&mut app, 100, 20);
    let _ = conn.heard().await;

    let body = app.layout.panes[0].rect;
    for event in [
        down(body.x + 4, body.y + 3),
        drag(body.x + 8, body.y + 3),
        up(body.x + 8, body.y + 3),
    ] {
        let role = client.state().role;
        let click = app.mouse.hit(event, &app.layout, role);
        app.clicked(click);
    }
    let frames = conn.heard().await;
    assert!(
        inputs(&frames).is_empty(),
        "a pane that was never armed hears nothing: {frames:?}"
    );
}

#[tokio::test]
async fn arming_a_pane_writes_the_toggle_to_prefs_and_the_status_line_says_so() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(json!([pane("01JNODE/w1:p1", "t1")]), json!({}));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.adopt_prefs();
    app.refocus();
    let _ = conn.heard().await;

    // A report names a cell of the **live grid**, so there has to be one: an armed pane with no
    // picture has no cell to name and reports nothing.
    conn.send(json!({
        "t": "grid.reset", "pane": "01JNODE/w1:p1", "cols": 12, "rows": 4,
        "rows_data": [{ "row": 0, "runs": [{ "s": 0, "x": "hello" }] }],
        "cursor": { "col": 0, "row": 0, "visible": true }, "links": []
    }));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;

    app.key(key(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(key(KeyCode::Char('m'), KeyModifiers::NONE));
    assert!(app.mouse.passes_through("01JNODE/w1:p1"));
    let frames = conn.heard().await;
    // A merge, so arming the mouse does not forget the pane's view (`null` would remove a key).
    assert!(
        frames.iter().any(|f| f["t"] == json!("prefs")
            && f["pane"] == json!("01JNODE/w1:p1")
            && f["prefs"]["mouse"] == json!(true)),
        "the toggle follows the operator between machines: {frames:?}"
    );
    let screen = painted(&mut app, 100, 20);
    assert!(
        screen.contains("mouse → pane"),
        "a mode that changes what a click does is never invisible:\n{screen}"
    );

    let grid = app.layout.panes[0].placement.expect("a live grid").grid;
    for event in [down(grid.x + 4, grid.y + 3), down(grid.x + 4, grid.y + 3)] {
        let role = client.state().role;
        let click = app.mouse.hit(event, &app.layout, role);
        app.clicked(click);
    }
    let frames = conn.heard().await;
    let sent = inputs(&frames);
    assert_eq!(
        sent.len(),
        1,
        "the click that focused is not a report: {frames:?}"
    );
    assert_eq!(sent[0]["text"], json!("\u{1b}[<0;5;4M"), "{frames:?}");
}

#[tokio::test]
async fn the_passthrough_toggle_survives_a_reconnect_because_it_is_in_prefs() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([pane("01JNODE/w1:p1", "t1")]),
        json!({ "01JNODE/w1:p1": { "mouse": true } }),
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;

    // A brand-new app on the same socket: nothing about the toggle is remembered in this
    // process, so if it is armed it can only have come back from the node.
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    assert!(!app.mouse.passes_through("01JNODE/w1:p1"));
    app.adopt_prefs();
    assert!(
        app.mouse.passes_through("01JNODE/w1:p1"),
        "the pane the operator armed is still armed after a reconnect"
    );
}

#[tokio::test]
async fn a_click_on_a_tab_opens_it_and_a_click_in_the_sidebar_opens_the_herd() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([pane("01JNODE/w1:p1", "t1"), pane("01JNODE/w1:p2", "t2")]),
        json!({}),
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.adopt_prefs();
    app.refocus();
    painted(&mut app, 100, 20);

    let (tab, rect) = app.layout.tabs[1].clone();
    assert_eq!(tab, "01JNODE/w1:t2");
    let click = app
        .mouse
        .hit(down(rect.x + rect.width / 2, rect.y), &app.layout, Role::Full);
    assert_eq!(click, Click::Tab(tab));
    app.clicked(click);
    assert_eq!(app.focused(), Some("01JNODE/w1:p2"), "focus follows the click");

    // A sidebar row that names a pane opens that pane; the section header above it names none,
    // and a click there is the herd view.
    let sidebar = app.layout.sidebar;
    let row = app
        .layout
        .rows
        .iter()
        .position(|(pane, _)| pane.as_deref() == Some("01JNODE/w1:p1"))
        .expect("the sidebar carries its panes") as u16;
    let click = app
        .mouse
        .hit(down(sidebar.x + 4, sidebar.y + row), &app.layout, Role::Full);
    assert_eq!(click, Click::Focus("01JNODE/w1:p1".into()));
    app.clicked(click);
    assert_eq!(app.focused(), Some("01JNODE/w1:p1"));

    let click = app
        .mouse
        .hit(down(sidebar.x + 2, sidebar.y), &app.layout, Role::Full);
    assert_eq!(click, Click::OpenHerd);
    app.clicked(click);
    let screen = painted(&mut app, 100, 20);
    assert!(screen.contains("comingclean"), "{screen}");
}

/// A herd entry whose measured width is the one the grid was actually wrapped at. `cols` is
/// absent until something has measured it, and the rect is never it.
fn measured(id: &str, tab: &str, cols: usize) -> Value {
    let mut entry = pane(id, tab);
    entry["cols"] = json!(cols);
    entry
}

/// A pane wide enough that the terminal has to crop it, so a pan is a real one.
fn wide(pane: &str, cols: usize) -> Value {
    json!({
        "t": "grid.reset", "pane": pane, "cols": cols, "rows": 4,
        "rows_data": [
            { "row": 0, "runs": [{ "s": 0, "x": "0123456789".repeat(cols / 10) }] },
            { "row": 3, "runs": [{ "s": 0, "x": "bottom" }] }
        ],
        "cursor": { "col": 0, "row": 0, "visible": true },
        "links": []
    })
}

#[tokio::test]
async fn a_click_in_a_panned_pane_names_the_cell_the_program_drew_there() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(json!([measured("01JNODE/w1:p1", "t1", 60)]), json!({}));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    conn.send(wide("01JNODE/w1:p1", 60));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.refocus();

    app.key(key(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(key(KeyCode::Char('m'), KeyModifiers::NONE));
    // A 46-column terminal cannot show 60 columns, so rung 3 crops and the operator pans.
    painted(&mut app, 46, 12);
    app.key(key(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(key(KeyCode::Right, KeyModifiers::NONE));
    painted(&mut app, 46, 12);
    let _ = conn.heard().await;

    let placement = app.layout.panes[0].placement.expect("a live grid");
    assert_eq!(placement.pan.col, 4, "the pan moved");
    assert!(
        placement.grid.y > placement.grid.y.saturating_sub(1).min(1),
        "the live grid is pinned to the bottom of the box, not the top"
    );

    // The **rect is not the grid**: its origin is the box's, its first column is the pan's, and
    // its first row is wherever a four-row grid landed in a taller box.
    for event in [down(placement.grid.x, placement.grid.y); 2] {
        let role = client.state().role;
        let click = app.mouse.hit(event, &app.layout, role);
        app.clicked(click);
    }
    let frames = conn.heard().await;
    let sent = inputs(&frames);
    assert_eq!(sent.len(), 1, "{frames:?}");
    assert_eq!(
        sent[0]["text"],
        json!("\u{1b}[<0;5;1M"),
        "column 5 is the pan's, row 1 is the grid's own first row: {frames:?}"
    );
}

#[tokio::test]
async fn a_drag_over_the_grid_copies_it_and_paints_what_was_taken() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([measured("01JNODE/w1:p1", "t1", 20)]), json!({}));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    conn.send(wide("01JNODE/w1:p1", 20));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.refocus();
    painted(&mut app, 46, 12);

    let grid = app.layout.panes[0].placement.expect("a live grid").grid;
    for event in [
        down(grid.x, grid.y),
        drag(grid.x + 4, grid.y),
        up(grid.x + 4, grid.y),
    ] {
        let click = app.mouse.hit(event, &app.layout, Role::Full);
        app.clicked(click);
    }
    let screen = painted(&mut app, 46, 12);
    assert!(
        screen.contains("copied 5 characters"),
        "a finished drag is copied without being asked twice:\n{screen}"
    );

    // The highlight is what says what was taken, and it is still there after the copy.
    let mut terminal = Terminal::new(TestBackend::new(46, 12)).expect("a test terminal");
    terminal.draw(|frame| app.draw(frame)).expect("a frame");
    let buffer = terminal.backend().buffer().clone();
    let painted_bg = buffer[(grid.x + 2, grid.y)].bg;
    let plain_bg = buffer[(grid.x + 8, grid.y)].bg;
    assert_ne!(painted_bg, plain_bg, "the selection is drawn");

    // And `prefix [ y` copies the same range rather than the whole screen.
    app.key(key(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(key(KeyCode::Char('['), KeyModifiers::NONE));
    app.key(key(KeyCode::Char('y'), KeyModifiers::NONE));
    let after = painted(&mut app, 46, 12);
    assert!(after.contains("copied 5 characters"), "{after}");
}

#[tokio::test]
async fn a_url_a_pane_printed_is_offered_and_never_followed() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([measured("01JNODE/w1:p1", "t1", 30)]), json!({}));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    conn.send(json!({
        "t": "grid.reset", "pane": "01JNODE/w1:p1", "cols": 30, "rows": 2,
        "rows_data": [{ "row": 0, "runs": [{ "s": 0, "x": "see https://herdr.dev/docs" }] }],
        "cursor": { "col": 0, "row": 0, "visible": false }, "links": []
    }));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.refocus();
    painted(&mut app, 120, 12);

    let grid = app.layout.panes[0].placement.expect("a live grid").grid;
    let click = app.mouse.hit(down(grid.x + 10, grid.y), &app.layout, Role::Full);
    app.clicked(click);
    let _ = app.mouse.hit(up(grid.x + 10, grid.y), &app.layout, Role::Full);
    let screen = painted(&mut app, 120, 12);
    // Pane output is attacker-influenceable, so a detected URL is something the operator opens.
    assert!(
        screen.contains("link https://herdr.dev/docs · ^b o opens it"),
        "offered, not navigated:\n{screen}"
    );

    // "see" is not a link, so a click on it offers nothing new — and nothing was navigated by
    // either click. `prefix o` is the only thing that opens one, and it is the operator's.
    let mut fresh = App::new(client.clone(), Options::default(), Images::default());
    fresh.refocus();
    fresh.key(key(KeyCode::Char('b'), KeyModifiers::CONTROL));
    fresh.key(key(KeyCode::Char('o'), KeyModifiers::NONE));
    let plain = painted(&mut fresh, 120, 12);
    assert!(
        plain.contains("no link has been offered"),
        "nothing is followed that was not offered first:\n{plain}"
    );
}

#[tokio::test]
async fn a_blocked_agents_option_chip_answers_it_and_the_herd_view_opens_a_pane() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut agent = pane("01JNODE/w1:p1", "t1");
    agent["agent"] = json!("claude");
    agent["agent_status"] = json!("blocked");
    conn.greet(json!([agent]), json!({}));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.refocus();
    conn.send(json!({
        "t": "pending", "pane": "01JNODE/w1:p1", "question": "Do you want to make this edit?",
        "options": [{ "key": "1", "label": "Yes" }, { "key": "2", "label": "No" }],
        "source": "screen"
    }));
    let pending = until(&mut events, |e| match e {
        Event::Pending(p) => Some(Event::Pending(p)),
        _ => None,
    })
    .await;
    app.convo.absorb(&pending);
    painted(&mut app, 110, 24);
    let _ = conn.heard().await;

    let chips = &app.layout.chips[0];
    assert_eq!(chips.rects.len(), 2, "both keys were drawn");
    let second = chips.rects[1];
    let click = app.mouse.hit(down(second.x, second.y), &app.layout, Role::Full);
    app.clicked(click);
    let frames = conn.heard().await;
    let answer = frames
        .iter()
        .find(|f| f["t"] == json!("answer"))
        .unwrap_or_else(|| panic!("no answer was sent: {frames:?}"));
    assert_eq!(answer["key"], json!("2"), "only a key that was offered");

    // The herd view is the triage screen a desk cannot draw, and every row on it is a pane.
    app.clicked(Click::OpenHerd);
    painted(&mut app, 110, 24);
    let (id, rect) = app.layout.herd[0].clone();
    assert_eq!(id, "01JNODE/w1:p1");
    let click = app.mouse.hit(down(rect.x + 2, rect.y), &app.layout, Role::Full);
    assert_eq!(click, Click::Focus("01JNODE/w1:p1".into()));
    app.clicked(click);
    let screen = painted(&mut app, 110, 24);
    assert!(
        screen.contains("Do you want to make this"),
        "clicking a row leaves the herd view for that pane:\n{screen}"
    );
}
