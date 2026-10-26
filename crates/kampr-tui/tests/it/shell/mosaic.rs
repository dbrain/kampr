//! Kampr's own mosaic: how a tab's panes are tiled, and the boxes resized by key and by drag.
//! Every change here is the client's arrangement of `observe` streams, so none of it may put a
//! frame on the wire (rule 3, ADR 0015).

use super::{Fake, app, node, painted, pane, until};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kampr_client::Event;
use kampr_tui::app::App;
use ratatui::layout::Rect;
use serde_json::{Value, json};

const WIDTH: u16 = 130;
const HEIGHT: u16 = 23;

type Boxes = &'static [(u16, u16, u16, u16)];
type Cell = (u16, u16);

async fn tab_of(count: usize) -> (App, super::Conn) {
    let mut fake = Fake::start().await;
    let client = std::sync::Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let panes: Vec<Value> = (1..=count)
        .map(|n| pane(&format!("01JNODE/w1:p{n}"), "herdr", None, "idle"))
        .collect();
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!(panes),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    painted(&mut app, WIDTH, HEIGHT);
    conn.heard().await;
    (app, conn)
}

fn boxes(app: &mut App) -> Vec<(u16, u16, u16, u16)> {
    painted(app, WIDTH, HEIGHT);
    app.layout
        .panes
        .iter()
        .map(|p| (p.rect.x, p.rect.y, p.rect.width, p.rect.height))
        .collect()
}

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn prefixed(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    press(app, KeyCode::Char(c));
}

fn resize(app: &mut App, keys: &str) {
    prefixed(app, 'r');
    for c in keys.chars() {
        press(app, KeyCode::Char(c));
    }
    press(app, KeyCode::Esc);
}

fn mouse(app: &mut App, kind: MouseEventKind, at: (u16, u16)) {
    let event = MouseEvent {
        kind,
        column: at.0,
        row: at.1,
        modifiers: KeyModifiers::NONE,
    };
    let click = app.mouse.hit(event, &app.layout, kampr_client::Role::Full);
    app.clicked(click);
}

fn drag(app: &mut App, from: (u16, u16), to: (u16, u16)) {
    painted(app, WIDTH, HEIGHT);
    mouse(app, MouseEventKind::Down(MouseButton::Left), from);
    mouse(app, MouseEventKind::Drag(MouseButton::Left), to);
    mouse(app, MouseEventKind::Up(MouseButton::Left), to);
}

/// No op, no resize, no claim: the boxes are the client's own. A `watch` is the only thing a
/// focus may say, and nothing in these tests moves the focus off the tab.
async fn quiet(conn: &mut super::Conn) {
    let heard = conn.heard().await;
    assert!(
        heard.iter().all(|frame| frame["t"] != "manage"),
        "a local resize reached the node: {heard:?}"
    );
}

#[tokio::test]
async fn a_tabs_panes_tile_by_count_and_stack_on_request() {
    // The sidebar takes 30 columns and the tab strip a row, which leaves a 100x22 body at (30, 1).
    let cases: &[(usize, &str, Boxes)] = &[
        (1, "", &[(30, 1, 100, 22)]),
        (2, "", &[(30, 1, 50, 22), (80, 1, 50, 22)]),
        (3, "", &[(30, 1, 50, 22), (80, 1, 50, 11), (80, 12, 50, 11)]),
        (
            4,
            "",
            &[
                (30, 1, 50, 11),
                (80, 1, 50, 11),
                (30, 12, 50, 11),
                (80, 12, 50, 11),
            ],
        ),
        (2, " ", &[(30, 1, 50, 22), (80, 1, 50, 22)]),
        (2, "  ", &[(30, 1, 100, 11), (30, 12, 100, 11)]),
        (3, "  ", &[(30, 1, 100, 7), (30, 8, 100, 8), (30, 16, 100, 7)]),
        (2, "   ", &[(30, 1, 50, 22), (80, 1, 50, 22)]),
    ];
    for (count, cycle, want) in cases {
        let (mut app, mut conn) = tab_of(*count).await;
        resize(&mut app, cycle);
        assert_eq!(
            boxes(&mut app),
            *want,
            "{count} panes after {} presses of space",
            cycle.len()
        );
        quiet(&mut conn).await;
    }
}

#[tokio::test]
async fn focus_moves_to_the_pane_on_that_side() {
    let (mut app, _conn) = tab_of(4).await;
    let walk = [
        ('l', "01JNODE/w1:p2"),
        ('j', "01JNODE/w1:p4"),
        ('h', "01JNODE/w1:p3"),
        ('k', "01JNODE/w1:p1"),
        ('j', "01JNODE/w1:p3"),
        ('j', "01JNODE/w1:p3"),
        ('h', "01JNODE/w1:p3"),
    ];
    for (key, want) in walk {
        prefixed(&mut app, key);
        assert_eq!(app.focused(), Some(want), "after prefix {key}");
    }
}

#[tokio::test]
async fn resize_mode_j_and_k_move_the_focused_panes_height() {
    // Three panes: p1 on the left, p2 over p3 on the right. p2 is focused for every case.
    let cases: &[(&str, Boxes)] = &[
        ("k", &[(30, 1, 50, 22), (80, 1, 50, 12), (80, 13, 50, 10)]),
        ("kk", &[(30, 1, 50, 22), (80, 1, 50, 13), (80, 14, 50, 9)]),
        ("j", &[(30, 1, 50, 22), (80, 1, 50, 10), (80, 11, 50, 12)]),
        ("h", &[(30, 1, 55, 22), (85, 1, 45, 11), (85, 12, 45, 11)]),
    ];
    for (keys, want) in cases {
        let (mut app, mut conn) = tab_of(3).await;
        prefixed(&mut app, 'l');
        assert_eq!(app.focused(), Some("01JNODE/w1:p2"));
        conn.heard().await;
        resize(&mut app, keys);
        assert_eq!(boxes(&mut app), *want, "resize mode {keys}");
        quiet(&mut conn).await;
    }
}

#[tokio::test]
async fn a_pane_with_nothing_above_or_below_says_so_rather_than_moving() {
    let (mut app, mut conn) = tab_of(2).await;
    let before = boxes(&mut app);
    resize(&mut app, "k");
    assert_eq!(boxes(&mut app), before);
    let screen = painted(&mut app, WIDTH, HEIGHT);
    assert!(
        screen.contains("nothing above or below"),
        "the operator is told why:\n{screen}"
    );
    quiet(&mut conn).await;
}

#[tokio::test]
async fn dragging_a_divider_moves_it() {
    // (layout, from, to, the boxes after). The grip is the two border cells either side of the
    // boundary: the left box's right edge and the right box's left edge.
    let cases: &[(usize, Cell, Cell, Boxes)] = &[
        (2, (79, 10), (89, 10), &[(30, 1, 60, 22), (90, 1, 40, 22)]),
        (2, (80, 10), (69, 10), &[(30, 1, 39, 22), (69, 1, 61, 22)]),
        (
            3,
            (100, 11),
            (100, 6),
            &[(30, 1, 50, 22), (80, 1, 50, 6), (80, 7, 50, 16)],
        ),
        (2, (79, 10), (31, 10), &[(30, 1, 10, 22), (40, 1, 90, 22)]),
    ];
    for (count, from, to, want) in cases {
        let (mut app, mut conn) = tab_of(*count).await;
        drag(&mut app, *from, *to);
        assert_eq!(boxes(&mut app), *want, "{count} panes, {from:?} to {to:?}");
        assert!(app.mouse.selection().is_none(), "a divider drag selects nothing");
        quiet(&mut conn).await;
    }
}

#[tokio::test]
async fn dragging_the_sidebars_edge_resizes_it_within_bounds() {
    let cases = [(39, 40), (2, 16), (120, 65)];
    for (to, width) in cases {
        let (mut app, mut conn) = tab_of(2).await;
        drag(&mut app, (29, 10), (to, 10));
        painted(&mut app, WIDTH, HEIGHT);
        assert_eq!(app.layout.sidebar.width, width, "edge dragged to {to}");
        let first = app.layout.panes.first().map(|p| p.rect).expect("a pane");
        assert_eq!(first.x, width, "the mosaic starts where the sidebar ends");
        quiet(&mut conn).await;
    }
}

#[tokio::test]
async fn a_drag_inside_a_grid_still_selects_and_resizes_nothing() {
    let (mut app, mut conn) = tab_of(2).await;
    conn.send(json!({
        "t": "grid.reset", "pane": "01JNODE/w1:p1", "cols": 12, "rows": 4,
        "rows_data": [ { "row": 0, "runs": [{ "s": 0, "x": "hello world" }] } ],
        "cursor": { "col": 0, "row": 1, "visible": true },
        "links": []
    }));
    let before = boxes(&mut app);
    let grid = loop {
        painted(&mut app, WIDTH, HEIGHT);
        if let Some(grid) = app
            .layout
            .pane("01JNODE/w1:p1")
            .and_then(|p| p.placement)
            .map(|p| p.grid)
        {
            break grid;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    };
    let Rect { x, y, .. } = grid;
    drag(&mut app, (x, y), (x + 4, y));
    let selected = app.mouse.selection().expect("a selection");
    assert_eq!(selected.pane, "01JNODE/w1:p1");
    assert_eq!((selected.from, selected.to), ((0, 0), (4, 0)));
    assert_eq!(boxes(&mut app), before);
    quiet(&mut conn).await;
}
