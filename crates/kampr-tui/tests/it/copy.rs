//! Copy mode's keyboard cursor: herdr's `h/j/k/l w/b/e { } move · v/space select · y/enter copy`
//! (#290), plus the vi motions herdr's footer does not list, over one surface — the ring, then the
//! live grid — and one selection the mouse shares.

use super::shell::{Fake, absorb_until, app, history, node, painted, pane, until};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kampr_client::{Event, Role};
use kampr_tui::app::App;
use kampr_tui::keymap::Mode;
use ratatui::Terminal;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::style::Modifier;
use serde_json::{Value, json};

const PANE: &str = "01JNODE/w1:p1";

fn grid(lines: &[&str]) -> Value {
    json!({
        "t": "grid.reset", "pane": PANE, "cols": 20, "rows": lines.len(),
        "rows_data": lines.iter().enumerate()
            .map(|(r, text)| json!({ "row": r, "runs": [{ "s": 0, "x": text }] }))
            .collect::<Vec<_>>(),
        "cursor": { "col": 0, "row": 0, "visible": false },
        "links": []
    })
}

async fn opened(lines: &[&str], ring: u16) -> (App, super::shell::Conn) {
    let mut fake = Fake::start().await;
    let client = std::sync::Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane(PANE, "herdr", None, "idle")]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    conn.send(grid(lines));
    absorb_until(&mut app, &mut events, |e| matches!(e, Event::Grid { .. })).await;
    if ring > 0 {
        conn.send(history(PANE, ring));
        absorb_until(&mut app, &mut events, |e| matches!(e, Event::Scrollback { .. })).await;
    }
    (app, conn)
}

fn press(app: &mut App, keys: &str) {
    for word in keys.split_whitespace() {
        let key = match word {
            "Esc" => KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE),
            "Enter" => KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE),
            "Up" => KeyEvent::new(KeyCode::Up, KeyModifiers::NONE),
            "Down" => KeyEvent::new(KeyCode::Down, KeyModifiers::NONE),
            "Left" => KeyEvent::new(KeyCode::Left, KeyModifiers::NONE),
            "Right" => KeyEvent::new(KeyCode::Right, KeyModifiers::NONE),
            "C-u" => KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL),
            "C-d" => KeyEvent::new(KeyCode::Char('d'), KeyModifiers::CONTROL),
            "C-b" => KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL),
            "space" => KeyEvent::new(KeyCode::Char(' '), KeyModifiers::NONE),
            one => {
                let mut chars = one.chars();
                let c = chars.next().expect("a key");
                assert!(chars.next().is_none(), "one key per word: {one}");
                KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
            }
        };
        app.key(key);
    }
}

fn frame(app: &mut App, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).expect("a test terminal");
    terminal.draw(|frame| app.draw(frame)).expect("a frame");
    terminal.backend().buffer().clone()
}

fn reversed(buffer: &Buffer) -> Vec<(u16, u16)> {
    let area = buffer.area;
    (0..area.height)
        .flat_map(|y| (0..area.width).map(move |x| (x, y)))
        .filter(|at| buffer[*at].modifier.contains(Modifier::REVERSED))
        .collect()
}

const TEXT: [&str; 4] = ["alpha beta.gamma", "  indented line", "", "last words here"];

/// The cursor starts on the bottom-left of what is on screen, which here is the start of `last`:
/// the surface is `hist0..hist2`, then the four rows of [`TEXT`].
#[tokio::test]
async fn copy_mode_motions_select_and_y_copies_exactly_what_they_covered() {
    let cases: &[(&str, &str)] = &[
        ("v l l y", "las"),
        ("v h y", "l"),
        ("v $ y", "last words here"),
        ("$ v 0 y", "last words here"),
        ("v w y", "last w"),
        ("w v e y", "words"),
        ("$ v b y", "here"),
        ("k k ^ v $ y", "indented line"),
        ("k k k v w w y", "alpha beta."),
        ("k k k w w v e y", ".gamma"),
        ("V k k y", "  indented line\n\nlast words here"),
        ("k k k V y", "alpha beta.gamma"),
        ("$ V k k y", "  indented line\n\nlast words here"),
        ("g v l y", "hi"),
        ("g G v l y", "la"),
        ("k k k k j v l y", "al"),
        ("Up Up v Right Right Right y", "  in"),
        ("Up Up Up Up v Down Left y", "hist2\na"),
        ("{ { v l y", "hi"),
        ("k k k v l Enter", "al"),
        ("space l l y", "las"),
    ];
    for (keys, want) in cases {
        let (mut app, _conn) = opened(&TEXT, 3).await;
        painted(&mut app, 60, 12);
        press(&mut app, "C-b [");
        assert_eq!(app.router.mode(), Mode::Copy);
        press(&mut app, keys);
        assert_eq!(app.copied(), Some(*want), "{keys}");
        assert_eq!(app.router.mode(), Mode::Pane, "{keys}: a copy leaves copy mode");
        let lines = want.lines().count().max(1);
        let note = match lines {
            1 => format!("copied {} characters", want.chars().count()),
            n => format!("copied {n} lines"),
        };
        let screen = painted(&mut app, 60, 12);
        assert!(screen.contains(&note), "{keys}: {note}\n{screen}");
    }
}

#[tokio::test]
async fn the_cursor_is_drawn_and_a_selection_is_highlighted_and_neither_outlives_copy_mode() {
    let (mut app, _conn) = opened(&TEXT, 0).await;
    let before = frame(&mut app, 60, 12);
    assert!(reversed(&before).is_empty(), "no cursor outside copy mode");

    press(&mut app, "C-b [");
    let buffer = frame(&mut app, 60, 12);
    let grid = app.layout.panes[0].placement.expect("a live grid").grid;
    let bottom = grid.y + grid.height - 1;
    assert_eq!(
        reversed(&buffer),
        vec![(grid.x, bottom)],
        "one reverse-video cell, bottom-left"
    );
    let screen = painted(&mut app, 60, 12);
    let rows: Vec<&str> = screen.lines().collect();
    assert!(
        rows[bottom as usize].starts_with("last words here"),
        "the footer gets out of the cursor's row:\n{screen}"
    );
    assert!(screen.contains("COPY"), "and is still on screen:\n{screen}");

    press(&mut app, "l l");
    let moved = frame(&mut app, 60, 12);
    assert_eq!(reversed(&moved), vec![(grid.x + 2, bottom)]);

    press(&mut app, "v h");
    let selected = frame(&mut app, 60, 12);
    let plain = selected[(grid.x + 6, bottom)].bg;
    assert_ne!(
        selected[(grid.x + 1, bottom)].bg,
        plain,
        "the selection is painted"
    );
    assert_ne!(
        selected[(grid.x + 2, bottom)].bg,
        plain,
        "inclusive of the anchor"
    );
    assert_eq!(selected[(grid.x + 3, bottom)].bg, plain, "and not past it");

    press(&mut app, "Esc");
    assert_eq!(
        app.router.mode(),
        Mode::Copy,
        "the first esc only drops the selection"
    );
    assert!(app.mouse.selection().is_none());
    press(&mut app, "q");
    assert_eq!(app.router.mode(), Mode::Pane);
    let after = frame(&mut app, 60, 12);
    assert!(reversed(&after).is_empty(), "the cursor leaves with the mode");
}

#[tokio::test]
async fn moving_the_cursor_past_the_top_of_the_window_scrolls_the_ring_into_view() {
    let lines: Vec<String> = (0..10).map(|r| format!("row{r}")).collect();
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    let (mut app, _conn) = opened(&lines, 20).await;
    let tail = painted(&mut app, 60, 12);
    assert!(!tail.contains("hist14"), "{tail}");

    press(&mut app, "C-b [");
    press(&mut app, &"k ".repeat(15));
    let up = painted(&mut app, 60, 12);
    assert!(up.contains("hist14"), "the window followed the cursor:\n{up}");

    press(&mut app, "v $ y");
    assert_eq!(app.copied(), Some("hist14"));

    press(&mut app, "C-b [ g");
    let top = painted(&mut app, 60, 12);
    assert!(top.contains("hist0"), "g is the top of the history:\n{top}");
    press(&mut app, "G");
    let bottom = painted(&mut app, 60, 12);
    assert!(bottom.contains("row9"), "G is the bottom:\n{bottom}");

    let half = app.layout.panes[0].inner.height / 2;
    let line = |row: u16| match row {
        0..20 => format!("hist{row}"),
        _ => format!("row{}", row - 20),
    };
    press(&mut app, "C-u v $ y");
    assert_eq!(
        app.copied(),
        Some(line(29 - half).as_str()),
        "ctrl+u is half a window up"
    );
    press(&mut app, "C-b [ g C-d v $ y");
    assert_eq!(
        app.copied(),
        Some(line(half).as_str()),
        "ctrl+d is half a window down"
    );
}

/// **One selection, whichever made it last.** A drag after `v` replaces the keyboard's, and `v`
/// after a drag replaces the drag's.
#[tokio::test]
async fn y_copies_whichever_selection_was_made_last_by_mouse_or_keyboard() {
    let mouse = |kind, x, y| MouseEvent {
        kind,
        column: x,
        row: y,
        modifiers: KeyModifiers::NONE,
    };
    let drag = |app: &mut App| {
        let grid = app.layout.panes[0].placement.expect("a live grid").grid;
        let top = grid.y;
        for event in [
            mouse(MouseEventKind::Down(MouseButton::Left), grid.x, top),
            mouse(MouseEventKind::Drag(MouseButton::Left), grid.x + 4, top),
            mouse(MouseEventKind::Up(MouseButton::Left), grid.x + 4, top),
        ] {
            let click = app.mouse.hit(event, &app.layout, Role::Full);
            app.clicked(click);
        }
        painted(app, 60, 12);
    };

    let (mut app, _conn) = opened(&TEXT, 0).await;
    painted(&mut app, 60, 12);
    press(&mut app, "C-b [ v l");
    drag(&mut app);
    press(&mut app, "l y");
    assert_eq!(
        app.copied(),
        Some("alpha"),
        "the drag came last, and a motion after it does not bring the keyboard's back"
    );

    let (mut app, _conn) = opened(&TEXT, 0).await;
    painted(&mut app, 60, 12);
    drag(&mut app);
    press(&mut app, "C-b [ v l");
    painted(&mut app, 60, 12);
    press(&mut app, "y");
    assert_eq!(app.copied(), Some("la"), "the keyboard came last");
}
