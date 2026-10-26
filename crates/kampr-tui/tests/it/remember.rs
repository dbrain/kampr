//! What this desk remembers between launches, and the switcher that leans on it.

use crate::shell::{Fake, node, painted, pane, until};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kampr_client::{Client, Event, Role};
use kampr_tui::app::{App, Options};
use kampr_tui::image::Images;
use kampr_tui::mouse::Click;
use kampr_tui::prefs::Prefs;
use kampr_tui::sidebar::Order;
use serde_json::{Value, json};
use std::sync::Arc;

fn fleet(id: &str) -> Value {
    let mut run = pane(id, "fleet", None, "unknown");
    run["fleet"] = json!({
        "cohort": "01JCOHORT", "command": "cargo test", "state": "exited", "started_unix": 1
    });
    run
}

fn shell(id: &str, cmd: &str) -> Value {
    let mut shell = pane(id, "alpha", None, "unknown");
    shell["cmd"] = json!(cmd);
    shell["argv"] = json!(cmd);
    shell
}

async fn herd(panes: Value) -> (Arc<Client>, crate::shell::Conn) {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([node("01JNODE", "comingclean", true)]), panes, "full");
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    (client, conn)
}

fn launched(client: &Arc<Client>, dir: &std::path::Path) -> App {
    let mut app = App::new(client.clone(), Options::default(), Images::default());
    app.remember_in(dir.to_path_buf());
    app.refocus();
    app
}

fn press(app: &mut App, code: KeyCode, modifiers: KeyModifiers) {
    app.key(KeyEvent::new(code, modifiers));
}

fn goto(app: &mut App) {
    press(app, KeyCode::Char('b'), KeyModifiers::CONTROL);
    press(app, KeyCode::Char('g'), KeyModifiers::NONE);
}

#[tokio::test]
async fn a_relaunch_opens_on_the_pane_last_looked_at_and_the_sidebar_order_last_chosen() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, _conn) = herd(json!([
        fleet("01JNODE/w9:p9"),
        shell("01JNODE/w1:p1", "htop"),
        shell("01JNODE/w1:p2", "vim"),
    ]))
    .await;

    let mut first = launched(&client, dir.path());
    assert_eq!(
        first.focused(),
        Some("01JNODE/w1:p1"),
        "with nothing remembered and no agent to triage, a fleet run's log is never what opens"
    );
    goto(&mut first);
    for c in "vim".chars() {
        press(&mut first, KeyCode::Char(c), KeyModifiers::NONE);
    }
    press(&mut first, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(first.focused(), Some("01JNODE/w1:p2"));
    first.clicked(Click::Order);
    drop(first);

    let kept = Prefs::load(dir.path());
    assert_eq!(kept.order, Order::Priority);
    let again = launched(&client, dir.path());
    assert_eq!(again.focused(), Some("01JNODE/w1:p2"), "{kept:?}");

    let elsewhere = tempfile::tempdir().expect("another desk");
    let fresh = launched(&client, elsewhere.path());
    assert_eq!(
        fresh.focused(),
        Some("01JNODE/w1:p1"),
        "remembered per desk, not on the node"
    );
}

#[tokio::test]
async fn an_empty_switcher_is_the_panes_last_visited_and_a_digit_or_a_click_goes_there() {
    let dir = tempfile::tempdir().expect("a config dir");
    let (client, _conn) = herd(json!([
        shell("01JNODE/w1:p1", "htop"),
        shell("01JNODE/w1:p2", "vim"),
        shell("01JNODE/w1:p3", "btop"),
    ]))
    .await;
    let mut app = launched(&client, dir.path());
    for pane in ["01JNODE/w1:p3", "01JNODE/w1:p2"] {
        app.clicked(Click::Focus(pane.into()));
    }
    assert_eq!(app.focused(), Some("01JNODE/w1:p2"));

    goto(&mut app);
    let screen = painted(&mut app, 100, 24);
    let rows: Vec<&str> = screen
        .lines()
        .filter(|l| l.contains("alpha · comingclean"))
        .collect();
    let at = |cmd: &str| {
        rows.iter()
            .position(|l| l.contains(cmd))
            .unwrap_or_else(|| panic!("no {cmd} row:\n{screen}"))
    };
    assert!(
        at("btop") < at("htop"),
        "the pane visited before this one leads, as tmux's last-session does:\n{screen}"
    );
    assert!(
        rows[at("btop")].contains('1'),
        "each row carries the digit that opens it:\n{screen}"
    );
    press(&mut app, KeyCode::Char('1'), KeyModifiers::NONE);
    assert_eq!(app.focused(), Some("01JNODE/w1:p3"));

    goto(&mut app);
    for c in "top".chars() {
        press(&mut app, KeyCode::Char(c), KeyModifiers::NONE);
    }
    press(&mut app, KeyCode::Char('1'), KeyModifiers::ALT);
    assert_eq!(
        app.focused(),
        Some("01JNODE/w1:p1"),
        "alt+digit picks among the matches once a query is typed, since a bare one is a letter of it"
    );

    goto(&mut app);
    let screen = painted(&mut app, 100, 24);
    let (y, line) = screen
        .lines()
        .enumerate()
        .find(|(_, l)| l.contains("alpha · comingclean") && l.contains("vim"))
        .unwrap_or_else(|| panic!("no vim row:\n{screen}"));
    let x = line[..line.rfind("vim").expect("the name")].chars().count() as u16;
    let click = app.mouse.hit(
        MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: x,
            row: y as u16,
            modifiers: KeyModifiers::NONE,
        },
        &app.layout,
        Role::Full,
    );
    app.clicked(click);
    assert_eq!(
        app.focused(),
        Some("01JNODE/w1:p2"),
        "a click on a row goes there"
    );
    let after = painted(&mut app, 100, 24);
    assert!(!after.contains(" go to "), "and closes it:\n{after}");
}
