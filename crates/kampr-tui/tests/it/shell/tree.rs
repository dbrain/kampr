//! A tab laid out as its keeper has it (#616): herdr's tree for a herdr tab, the node's for native
//! sessions split from one another — and kampr's own count tiling only where there is no tree.

use super::{Fake, app, node, painted, pane, until};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kampr_client::Event;
use kampr_tui::app::App;
use serde_json::{Value, json};
use std::time::Duration;

const WIDTH: u16 = 130;
const HEIGHT: u16 = 23;

type Boxes = Vec<(u16, u16, u16, u16)>;
type Ask = fn(&mut App);

fn cut(split: &str, ratio: u16, second: bool) -> Value {
    json!({ "split": split, "ratio": ratio, "second": second })
}

/// p1 on the left at `root`, p2 over p3 on the right at 300 — the tab from #616's probe.
fn herdr_tab(root: u16) -> Value {
    let tiled = |n: u16, path: Vec<Value>| {
        let mut p = pane(&format!("01JNODE/w1:p{n}"), "herdr", None, "idle");
        p["tile"] = json!({ "group": "01JNODE/w1:t1", "path": path });
        p
    };
    json!([
        tiled(1, vec![cut("right", root, false)]),
        tiled(2, vec![cut("right", root, true), cut("down", 300, false)]),
        tiled(3, vec![cut("right", root, true), cut("down", 300, true)]),
    ])
}

async fn open(panes: Value) -> (App, super::Conn) {
    let mut fake = Fake::start().await;
    let client = std::sync::Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(json!([node("01JNODE", "comingclean", true)]), panes, "full");
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    painted(&mut app, WIDTH, HEIGHT);
    conn.heard().await;
    (app, conn)
}

fn boxes(app: &mut App) -> Boxes {
    painted(app, WIDTH, HEIGHT);
    app.layout
        .panes
        .iter()
        .map(|p| (p.rect.x, p.rect.y, p.rect.width, p.rect.height))
        .collect()
}

async fn becomes(app: &mut App, want: &Boxes, why: &str) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(2);
    while boxes(app) != *want && tokio::time::Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(boxes(app), *want, "{why}");
}

fn press(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn prefixed(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    press(app, KeyCode::Char(c));
}

fn drag(app: &mut App, from: (u16, u16), to: (u16, u16)) {
    painted(app, WIDTH, HEIGHT);
    for (kind, at) in [
        (MouseEventKind::Down(MouseButton::Left), from),
        (MouseEventKind::Drag(MouseButton::Left), to),
        (MouseEventKind::Up(MouseButton::Left), to),
    ] {
        let event = MouseEvent {
            kind,
            column: at.0,
            row: at.1,
            modifiers: KeyModifiers::NONE,
        };
        let click = app.mouse.hit(event, &app.layout, kampr_client::Role::Full);
        app.clicked(click);
    }
}

#[tokio::test]
async fn a_herdr_tab_is_laid_out_as_herdr_has_it_and_follows_it_when_herdr_moves() {
    // The body is 100x22 at (30, 1): 60 columns for p1, and p2 over p3 at three tenths.
    let (mut app, conn) = open(herdr_tab(600)).await;
    let herdrs = vec![(30, 1, 60, 22), (90, 1, 40, 7), (90, 8, 40, 15)];
    assert_eq!(boxes(&mut app), herdrs);

    prefixed(&mut app, 'r');
    press(&mut app, KeyCode::Char(' '));
    press(&mut app, KeyCode::Esc);
    assert_eq!(boxes(&mut app), herdrs, "there is no arrangement to cycle");
    let screen = painted(&mut app, WIDTH, HEIGHT);
    assert!(
        screen.contains("laid out the way"),
        "and the operator is told why:\n{screen}"
    );

    drag(&mut app, (89, 10), (79, 10));
    assert_eq!(
        boxes(&mut app),
        vec![(30, 1, 50, 22), (80, 1, 50, 7), (80, 8, 50, 15)],
        "a divider moves on this screen alone"
    );

    conn.send(
        json!({ "t": "herd", "nodes": [node("01JNODE", "comingclean", true)], "panes": herdr_tab(700) }),
    );
    becomes(
        &mut app,
        &vec![(30, 1, 70, 22), (100, 1, 30, 7), (100, 8, 30, 15)],
        "herdr moving that divider has the last word",
    )
    .await;
}

#[tokio::test]
async fn every_pane_of_a_tree_is_on_screen_however_many() {
    let column = |n: u16, path: Vec<Value>| {
        let mut p = pane(&format!("01JNODE/w1:p{n}"), "herdr", None, "idle");
        p["tile"] = json!({ "group": "01JNODE/w1:t1", "path": path });
        p
    };
    let panes = json!([
        column(1, vec![cut("right", 500, false), cut("down", 500, false)]),
        column(2, vec![cut("right", 500, false), cut("down", 500, true)]),
        column(
            3,
            vec![
                cut("right", 500, true),
                cut("down", 500, false),
                cut("right", 500, false)
            ]
        ),
        column(
            4,
            vec![
                cut("right", 500, true),
                cut("down", 500, false),
                cut("right", 500, true)
            ]
        ),
        column(5, vec![cut("right", 500, true), cut("down", 500, true)]),
    ]);
    let (mut app, _conn) = open(panes).await;
    assert_eq!(
        boxes(&mut app),
        vec![
            (30, 1, 50, 11),
            (30, 12, 50, 11),
            (80, 1, 25, 11),
            (105, 1, 25, 11),
            (80, 12, 50, 11),
        ]
    );
}

fn native(n: u16) -> Value {
    let mut p = pane(&format!("01JNODE/n:0{n}"), "", None, "idle");
    for field in ["workspace_id", "tab_id", "workspace", "tab"] {
        p.as_object_mut().unwrap().remove(field);
    }
    p["provider"] = json!("native");
    p["cwd"] = json!("/srv/work");
    p
}

/// Every way to ask: the key, the manage menu, and the pane's own right-click menu.
#[tokio::test]
async fn a_native_session_splits_beside_itself_without_herdrs_warning_from_every_way_in() {
    let ways: [(&str, Ask); 3] = [
        ("prefix -", |app| prefixed(app, '-')),
        ("manage menu", |app| {
            app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
            app.key(KeyEvent::new(KeyCode::Char('N'), KeyModifiers::SHIFT));
            press(app, KeyCode::Char('s'));
            press(app, KeyCode::Char('d'));
        }),
        ("pane menu", |app| {
            app.clicked(kampr_tui::mouse::Click::Menu(kampr_tui::mouse::Menu::Pane(
                "01JNODE/n:01".into(),
            )));
            press(app, KeyCode::Char('d'));
        }),
    ];
    for (way, ask) in ways {
        let (mut app, mut conn) = open(json!([native(1)])).await;
        ask(&mut app);
        let sent: Vec<Value> = conn
            .heard()
            .await
            .into_iter()
            .filter(|f| f["t"] == "manage")
            .collect();
        assert_eq!(sent.len(), 1, "{way}: no confirmation stands between: {sent:?}");
        assert_eq!(sent[0]["op"], "pane.split", "{way}");
        assert_eq!(sent[0]["at"], "01JNODE/n:01", "{way}");
        assert_eq!(sent[0]["direction"], "down", "{way}");
        assert_eq!(
            sent[0]["cwd"], "/srv/work",
            "{way}: the new session starts where this one is"
        );
    }
}

#[tokio::test]
async fn native_sessions_split_from_one_another_share_the_screen() {
    let (mut app, conn) = open(json!([native(1)])).await;
    assert_eq!(boxes(&mut app), vec![(30, 1, 100, 22)]);
    let tiled = |n: u16, second: bool| {
        let mut p = native(n);
        p["tile"] = json!({ "group": "01JNODE/k:01", "path": [cut("down", 500, second)] });
        p
    };
    conn.send(
        json!({ "t": "herd", "nodes": [node("01JNODE", "comingclean", true)],
                      "panes": [tiled(1, false), tiled(2, true)] }),
    );
    becomes(
        &mut app,
        &vec![(30, 1, 100, 11), (30, 12, 100, 11)],
        "the two sessions are one mosaic",
    )
    .await;
}
