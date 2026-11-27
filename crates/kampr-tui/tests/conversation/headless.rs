//! Claude with no terminal and the programs to resume after a reboot (docs/21 "Starting a
//! conversation without a terminal", docs/25), from the keyboard.

use super::*;
use kampr_tui::manage::Target;

type Events = tokio::sync::broadcast::Receiver<Event>;

const NODE: &str = "01JNODE";

fn greet_node(conn: &Conn, node: Value, panes: Value) {
    conn.send(json!({
        "t": "hello", "protocol": 1, "node_id": NODE, "node_name": "comingclean",
        "build": "0.1.29", "role": "full",
        "caps": { "push": false, "scrollback": true, "conversation": true, "manage": true }
    }));
    conn.send(json!({ "t": "herd", "nodes": [node], "panes": panes }));
    conn.send(json!({ "t": "prefs", "panes": {} }));
}

fn node(extra: Value) -> Value {
    let mut node = json!({ "id": NODE, "name": "comingclean", "kind": "local", "online": true });
    for (k, v) in extra.as_object().expect("fields") {
        node[k] = v.clone();
    }
    node
}

async fn ready(conn: &Conn, events: &mut Events, client: &Arc<Client>, node: Value, panes: Value) -> App {
    greet_node(conn, node, panes);
    until(events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(client);
    app.absorb(&Event::Herd);
    app
}

fn key(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn press(app: &mut App, c: char) {
    key(app, KeyCode::Char(c));
}

fn typed(app: &mut App, words: &str) {
    words.chars().for_each(|c| press(app, c));
}

/// The ack lands on a task; the next key is what settles it into the app.
async fn answered(conn: &Conn, app: &mut App, ack: Value) {
    conn.send(ack);
    tokio::time::sleep(SETTLE).await;
    key(app, KeyCode::F(12));
}

fn host_menu(app: &mut App) {
    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    press(app, 'w');
    press(app, '.');
}

fn manages(frames: &[Value]) -> Vec<Value> {
    frames
        .iter()
        .filter(|f| f["t"] == json!("manage"))
        .map(|f| {
            let mut f = f.clone();
            f.as_object_mut().unwrap().remove("rid");
            f
        })
        .collect()
}

#[tokio::test]
async fn a_claude_with_no_terminal_is_started_or_resumed_from_the_host_menu() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = ready(
        &conn,
        &mut events,
        &client,
        node(json!({ "claude": true })),
        json!([]),
    )
    .await;
    let _ = conn.heard().await;

    host_menu(&mut app);
    assert!(painted(&mut app, 120, 30).contains("Claude without a terminal"));
    press(&mut app, 'c');
    typed(&mut app, "/srv/app");
    key(&mut app, KeyCode::Enter);
    let asked = conn.sent("manage").await;
    assert_eq!(asked["op"], json!("claude.resumable"));
    assert_eq!(asked["node"], json!(NODE));
    assert_eq!(asked["cwd"], json!("/srv/app"));
    answered(
        &conn,
        &mut app,
        json!({ "t": "managed", "rid": asked["rid"], "op": "claude.resumable", "ok": true,
                "resumable": [{ "id": "s-old", "cwd": "/srv/app", "title": "the parser work" }] }),
    )
    .await;
    assert!(painted(&mut app, 120, 30).contains("the parser work"));

    typed(&mut app, "look at the logs");
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        manages(&conn.heard().await),
        [
            json!({ "t": "manage", "op": "claude.create", "node": NODE, "cwd": "/srv/app", "prompt": "look at the logs" })
        ]
    );

    host_menu(&mut app);
    press(&mut app, 'c');
    typed(&mut app, "/srv/app");
    key(&mut app, KeyCode::Enter);
    let asked = conn.sent("manage").await;
    answered(
        &conn,
        &mut app,
        json!({ "t": "managed", "rid": asked["rid"], "op": "claude.resumable", "ok": true,
                "resumable": [{ "id": "s-old", "cwd": "/srv/app", "title": "the parser work" }] }),
    )
    .await;
    key(&mut app, KeyCode::Down);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        manages(&conn.heard().await),
        [json!({ "t": "manage", "op": "claude.resume", "node": NODE, "id": "s-old" })]
    );
}

#[tokio::test]
async fn a_node_without_headless_claude_is_not_offered_one() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let mut app = ready(&conn, &mut events, &client, node(json!({})), json!([])).await;
    host_menu(&mut app);
    assert!(!painted(&mut app, 120, 30).contains("Claude without a terminal"));
}

#[tokio::test]
async fn a_busy_hand_over_is_asked_about_and_sent_again_with_interrupt() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let headless = json!({ "id": "01JNODE/c:k1", "node_id": NODE, "agent": "claude",
        "agent_status": "working", "rows": 24, "converses": true });
    let terminal = json!({ "id": "01JNODE/n:k2", "node_id": NODE, "agent": "claude", "provider": "native",
        "agent_status": "idle", "rows": 24 });
    let mut app = ready(
        &conn,
        &mut events,
        &client,
        node(json!({ "claude": true, "native": true })),
        json!([headless, terminal]),
    )
    .await;
    let _ = conn.heard().await;

    let menu = |app: &mut App, pane: &str| {
        let state = client.state();
        let caps = state.caps();
        app.manage
            .context(Target::Pane, &state.herd, pane, &caps, state.role);
    };
    menu(&mut app, "01JNODE/c:k1");
    let screen = painted(&mut app, 120, 30);
    assert!(
        screen.contains("open in terminal") && screen.contains("end"),
        "{screen}"
    );
    press(&mut app, 't');
    let asked = conn.sent("manage").await;
    assert_eq!(asked["op"], json!("claude.to_terminal"));
    assert_eq!(asked["at"], json!("01JNODE/c:k1"));
    assert_eq!(asked["interrupt"], Value::Null, "an idle Claude is just moved");
    answered(
        &conn,
        &mut app,
        json!({ "t": "managed", "rid": asked["rid"], "op": "claude.to_terminal", "ok": false,
                "code": "busy", "message": "Claude is busy: moving it now stops the turn in progress." }),
    )
    .await;
    let screen = painted(&mut app, 120, 30);
    assert!(
        screen.contains("moving it now stops the turn in progress"),
        "{screen}"
    );
    press(&mut app, 'y');
    let again = conn.sent("manage").await;
    assert_eq!(again["op"], json!("claude.to_terminal"));
    assert_eq!(again["interrupt"], json!(true));

    menu(&mut app, "01JNODE/c:k1");
    press(&mut app, 'e');
    assert!(
        manages(&conn.heard().await).is_empty(),
        "ending is confirmed first"
    );
    press(&mut app, 'y');
    assert_eq!(
        manages(&conn.heard().await),
        [json!({ "t": "manage", "op": "claude.end", "at": "01JNODE/c:k1" })]
    );

    menu(&mut app, "01JNODE/n:k2");
    assert!(painted(&mut app, 120, 30).contains("continue without terminal"));
    press(&mut app, 'h');
    assert_eq!(
        manages(&conn.heard().await),
        [json!({ "t": "manage", "op": "claude.from_terminal", "at": "01JNODE/n:k2" })]
    );
}

// A 0.1.153 node keeps sessions and refuses the op, which a direct connection would show and
// audit on every connect.
#[tokio::test]
async fn a_node_that_keeps_sessions_but_cannot_resume_them_is_not_asked() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = ready(
        &conn,
        &mut events,
        &client,
        node(json!({ "native": true })),
        json!([]),
    )
    .await;
    app.absorb(&Event::Herd);
    assert!(manages(&conn.heard().await).is_empty());
}

#[tokio::test]
async fn programs_to_resume_after_a_reboot_are_asked_for_listed_and_resumed_or_dismissed() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = ready(
        &conn,
        &mut events,
        &client,
        node(json!({ "native": true, "resume": true })),
        json!([]),
    )
    .await;
    let asked = conn.sent("manage").await;
    assert_eq!(asked["op"], json!("native.resumable"));
    assert_eq!(asked["node"], json!(NODE));
    answered(
        &conn,
        &mut app,
        json!({ "t": "managed", "rid": asked["rid"], "op": "native.resumable", "ok": true, "resumable": [
            { "id": "01k8/1", "label": "work", "argv": ["nvim", "-S", "/home/u/s.vim"], "cwd": "/home/u" },
            { "id": "01k8/2", "argv": ["htop", "it's"], "cwd": "/home/u" }] }),
    )
    .await;
    app.absorb(&Event::Herd);
    assert!(
        manages(&conn.heard().await).is_empty(),
        "asked once per socket, not on every herd frame"
    );

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('H'), KeyModifiers::SHIFT));
    let herd = painted(&mut app, 120, 30);
    assert!(herd.contains("resume after reboot"), "{herd}");
    assert!(herd.contains("nvim -S /home/u/s.vim"), "{herd}");
    assert!(
        herd.contains("htop 'it'\\''s'"),
        "each argument is shown quoted:\n{herd}"
    );

    host_menu(&mut app);
    let menu = painted(&mut app, 120, 30);
    assert!(menu.contains("nvim -S /home/u/s.vim"), "{menu}");
    key(&mut app, KeyCode::Down);
    press(&mut app, 'x');
    assert_eq!(
        manages(&conn.heard().await),
        [json!({ "t": "manage", "op": "native.resume.dismiss", "node": NODE, "id": "01k8/2" })]
    );
    key(&mut app, KeyCode::Up);
    key(&mut app, KeyCode::Enter);
    assert_eq!(
        manages(&conn.heard().await),
        [json!({ "t": "manage", "op": "native.resume", "node": NODE, "id": "01k8/1" })]
    );
    assert!(!painted(&mut app, 120, 30).contains("nvim -S"), "both are spent");
}
