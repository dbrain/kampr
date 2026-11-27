//! OSC 7501 (docs/24): a failed program, its progress, and its records, as the herd carries them.

use super::*;
use kampr_tui::manage::Target;

type Events = tokio::sync::broadcast::Receiver<Event>;

async fn greeted(conn: &mut Conn, events: &mut Events, client: &Arc<Client>, panes: Value) -> App {
    conn.greet(panes);
    until(events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let app = app(client);
    let _ = conn.heard().await;
    app
}

fn shell(id: &str, tab: &str, fields: Value) -> Value {
    let mut pane = entry(id, None, "idle", false);
    pane["tab_id"] = json!(format!("01JNODE/w1:{tab}"));
    pane["label"] = json!(tab);
    for (k, v) in fields.as_object().expect("fields") {
        pane[k] = v.clone();
    }
    pane
}

#[tokio::test]
async fn a_failed_program_and_one_making_progress_are_marked_where_the_pane_is_named() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let panes = json!([
        shell(
            "01JNODE/w1:p1",
            "build",
            json!({ "program_error": true, "program_msg": "link step failed" })
        ),
        shell("01JNODE/w1:p2", "fetch", json!({ "program_progress": 42 })),
        shell("01JNODE/w1:p3", "plain", json!({})),
    ]);
    let mut app = greeted(&mut conn, &mut events, &client, panes).await;
    let screen = painted(&mut app, 140, 24);
    let row = |needle: &str| {
        screen
            .lines()
            .find(|l| l.contains(needle) && l.find(needle).is_some_and(|x| x < 30))
            .unwrap_or_else(|| panic!("no sidebar row for {needle}:\n{screen}"))
            .to_string()
    };
    assert!(row("build").contains('✗'), "{screen}");
    assert!(row("fetch").contains("42%"), "{screen}");
    assert!(
        !row("plain").contains('✗') && !row("plain").contains('%'),
        "{screen}"
    );
    assert!(
        screen.contains("failed: link step failed"),
        "the focused pane's title says why:\n{screen}"
    );
}

#[tokio::test]
async fn the_pane_menu_lists_a_programs_records_under_their_parents() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let program = json!([
        { "id": "a1/w2", "state": "blocked", "app": "claude", "title": "worker", "msg": "may I run rm?" },
        { "id": "", "state": "working", "app": "claude", "progress": 30 },
        { "id": "a1", "state": "working", "title": "explore" },
    ]);
    let panes = json!([shell("01JNODE/w1:p1", "build", json!({ "program": program }))]);
    let mut app = greeted(&mut conn, &mut events, &client, panes).await;
    assert!(painted(&mut app, 140, 24).contains("3 programs"));

    {
        let state = client.state();
        let caps = state.caps();
        app.manage
            .context(Target::Pane, &state.herd, "01JNODE/w1:p1", &caps, state.role);
    }
    app.key(KeyEvent::new(KeyCode::Char('g'), KeyModifiers::NONE));
    let screen = painted(&mut app, 140, 30);
    let at = |needle: &str| row_of(&screen, needle);
    assert!(
        at("claude") < at("explore") && at("explore") < at("worker"),
        "{screen}"
    );
    assert!(screen.contains("working · 30%"), "{screen}");
    assert!(screen.contains("blocked · may I run rm?"), "{screen}");
    let indent = |needle: &str| {
        let line = screen.lines().nth(at(needle)).unwrap();
        line[..line.find(needle).unwrap()].chars().count()
    };
    assert!(
        indent("claude") < indent("explore") && indent("explore") < indent("worker"),
        "{screen}"
    );
}
