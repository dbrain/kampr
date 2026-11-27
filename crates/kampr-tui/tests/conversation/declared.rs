//! The vocabulary a conversation's source declares beyond its turns (docs/21 Phase 2, docs/27): a
//! piece is drawn and a key answers only where the source declared it.

use super::*;

type Events = tokio::sync::broadcast::Receiver<Event>;

async fn opened(conn: &mut Conn, events: &mut Events, client: &Arc<Client>) -> App {
    conn.greet(json!([entry(PANE, Some("claude"), "working", true)]));
    until(events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let app = showing_conversation(client);
    let _ = conn.heard().await;
    app
}

/// Sends the frames and absorbs everything up to the `pong` sent after them, so the app has seen
/// every one of them before the test looks.
async fn told(conn: &Conn, app: &mut App, events: &mut Events, frames: Vec<Value>) {
    static N: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1000);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    for frame in frames {
        conn.send(frame);
    }
    conn.send(json!({ "t": "pong", "n": n }));
    pump(app, events, |e| matches!(e, Event::Pong { n: got } if *got == n)).await;
}

fn abilities(abilities: Value) -> Value {
    json!({ "t": "convo.abilities", "pane": PANE, "abilities": abilities })
}

fn state(state: &str) -> Value {
    json!({ "t": "convo.state", "pane": PANE, "state": state })
}

fn alt(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::ALT));
}

fn typed(app: &mut App, words: &str) {
    words
        .chars()
        .for_each(|c| app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)));
}

fn hit(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn of<'a>(frames: &'a [Value], t: &str) -> Vec<&'a Value> {
    frames.iter().filter(|f| f["t"] == json!(t)).collect()
}

#[tokio::test]
async fn the_facts_row_and_the_stop_key_are_there_only_once_the_source_declares_them() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = opened(&mut conn, &mut events, &client).await;
    let facts = vec![
        state("running"),
        json!({ "t": "convo.context", "pane": PANE, "context": { "used": 12000, "total": 200000 } }),
        json!({ "t": "convo.session", "pane": PANE, "facts": { "id": "s1", "model": "opus-9",
            "billing": { "provider": "plan", "rate_limited": false } } }),
    ];
    told(&conn, &mut app, &mut events, facts).await;

    let screen = painted(&mut app, 110, 24);
    assert!(
        !screen.contains("context 12k/200k"),
        "undeclared facts are drawn:\n{screen}"
    );
    assert!(!screen.contains("alt+k stop"), "{screen}");
    alt(&mut app, 'k');
    assert!(
        of(&conn.heard().await, "convo.steer").is_empty(),
        "an undeclared steer was sent"
    );

    told(
        &conn,
        &mut app,
        &mut events,
        vec![abilities(
            json!({ "facts": ["context", "billing"], "steer": ["interrupt"] }),
        )],
    )
    .await;
    let screen = painted(&mut app, 110, 24);
    assert!(
        screen.contains("opus-9 · context 12k/200k · billed by plan"),
        "{screen}"
    );
    assert!(screen.contains("alt+k stop"), "{screen}");
    alt(&mut app, 'k');
    let frames = conn.heard().await;
    assert_eq!(
        of(&frames, "convo.steer"),
        [&json!({ "t": "convo.steer", "pane": PANE, "steer": "interrupt" })],
        "{frames:?}"
    );

    told(&conn, &mut app, &mut events, vec![state("idle")]).await;
    assert!(!painted(&mut app, 110, 24).contains("alt+k stop"));
    alt(&mut app, 'k');
    assert!(
        of(&conn.heard().await, "convo.steer").is_empty(),
        "nothing runs to stop"
    );
}

#[tokio::test]
async fn a_reply_to_a_source_that_queues_goes_whole_with_its_id_and_its_queue_word() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = opened(&mut conn, &mut events, &client).await;
    let declared = vec![
        abilities(json!({ "input": ["queuenext", "sendnow", "unqueue"], "steer": ["interrupt"] })),
        state("running"),
    ];
    told(&conn, &mut app, &mut events, declared).await;

    typed(&mut app, "run the tests");
    hit(&mut app, KeyCode::Enter);
    let frames = conn.heard().await;
    let inputs = of(&frames, "input");
    assert_eq!(inputs.len(), 1, "one message, not text then a return: {frames:?}");
    assert_eq!(inputs[0]["text"], json!("run the tests"));
    assert_eq!(inputs[0]["when"], json!("next"), "{frames:?}");
    let queued = inputs[0]["id"]
        .as_str()
        .expect("an id the queue can name")
        .to_string();

    typed(&mut app, "stop and look");
    alt(&mut app, 'n');
    let frames = conn.heard().await;
    let now = of(&frames, "input");
    assert_eq!(now.len(), 1, "{frames:?}");
    assert_eq!(now[0]["when"], json!("now"));
    assert_ne!(now[0]["id"], json!(queued), "every message mints its own id");

    told(
        &conn,
        &mut app,
        &mut events,
        vec![json!({ "t": "convo.heard", "pane": PANE, "text": "stop and look" })],
    )
    .await;
    alt(&mut app, 'u');
    let frames = conn.heard().await;
    assert_eq!(
        of(&frames, "input.unqueue"),
        [&json!({ "t": "input.unqueue", "pane": PANE, "id": queued })],
        "the one the harness has not heard yet: {frames:?}"
    );
    assert_eq!(
        app.composer.text(PANE),
        "run the tests",
        "and its words come back to the box"
    );

    told(&conn, &mut app, &mut events, vec![state("idle")]).await;
    hit(&mut app, KeyCode::Enter);
    let idle = of(&conn.heard().await, "input")
        .into_iter()
        .cloned()
        .collect::<Vec<_>>();
    assert_eq!(idle.len(), 1);
    assert_eq!(
        idle[0]["when"],
        Value::Null,
        "an idle conversation takes it at once: {idle:?}"
    );
}

#[tokio::test]
async fn trouble_and_the_turn_being_written_are_drawn_while_they_stand() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = opened(&mut conn, &mut events, &client).await;
    let frames = vec![
        json!({ "t": "convo.trouble", "pane": PANE, "trouble": { "kind": "overloaded",
            "text": "the API is overloaded", "status": "retrying",
            "retry": { "attempt": 2, "of": 10, "in_ms": 4000 }, "fix": { "wait": { "until": "12:40" } } } }),
        json!({ "t": "convo.live", "pane": PANE, "live": { "text": "Reading the file\nnow the parser",
            "thinking": 2400, "tools": [{ "id": "c1", "tool": "Bash", "args": "cargo test", "state": "running" }] } }),
        json!({ "t": "convo.tasks", "pane": PANE, "tasks": [
            { "id": "t1", "kind": "shell", "title": "cargo build", "backgrounded": true, "state": "running" },
            { "id": "t2", "kind": "shell", "title": "old", "backgrounded": false, "state": "ended" }] }),
    ];
    told(&conn, &mut app, &mut events, frames).await;
    let screen = painted(&mut app, 140, 24);
    assert!(
        screen.contains("the API is overloaded · retry 2 of 10 in 4s · held until 12:40"),
        "{screen}"
    );
    assert!(
        screen.contains("thinking ~2k · now the parser · Bash cargo test"),
        "{screen}"
    );
    assert!(
        screen.contains("1 running · cargo build · backgrounded"),
        "{screen}"
    );

    let cleared = vec![
        json!({ "t": "convo.trouble", "pane": PANE, "trouble": null }),
        json!({ "t": "convo.live", "pane": PANE, "live": { "text": "", "tools": [] } }),
    ];
    told(&conn, &mut app, &mut events, cleared).await;
    let screen = painted(&mut app, 140, 24);
    assert!(!screen.contains("overloaded"), "{screen}");
    assert!(!screen.contains("now the parser"), "{screen}");
}

#[tokio::test]
async fn a_slash_asks_the_source_for_its_commands_and_tab_takes_the_first() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = opened(&mut conn, &mut events, &client).await;

    typed(&mut app, "/co");
    assert!(
        of(&conn.heard().await, "convo.suggest").is_empty(),
        "a source that declares no slash is not asked"
    );
    app.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));

    told(
        &conn,
        &mut app,
        &mut events,
        vec![abilities(json!({ "input": ["slash", "mention"] }))],
    )
    .await;
    typed(&mut app, "/co");
    let frames = conn.heard().await;
    assert_eq!(
        of(&frames, "convo.suggest").last().copied(),
        Some(&json!({ "t": "convo.suggest", "pane": PANE, "kind": "commands", "query": "co" })),
        "{frames:?}"
    );
    let items = json!([{ "id": "compact", "label": "/compact" }, { "id": "cost", "label": "/cost" }]);
    told(
        &conn,
        &mut app,
        &mut events,
        vec![json!({ "t": "convo.suggested", "pane": PANE, "kind": "commands", "query": "co", "items": items })],
    )
    .await;
    let screen = painted(&mut app, 110, 24);
    assert!(screen.contains("/compact  /cost"), "{screen}");
    hit(&mut app, KeyCode::Tab);
    assert_eq!(app.composer.text(PANE), "/compact ");
    assert!(
        !painted(&mut app, 110, 24).contains("/compact  /cost"),
        "taken, the row goes"
    );
}

fn session_frames() -> Vec<Value> {
    vec![
        abilities(json!({ "facts": ["context"], "steer": ["compact", "rename"] })),
        json!({ "t": "convo.controls", "pane": PANE, "controls": [
            { "id": "model", "label": "Model", "kind": "choice", "value": "opus",
              "options": [{ "value": "opus", "label": "Opus" }, { "value": "sonnet", "label": "Sonnet" }],
              "confirm": "Sonnet bills differently." },
            { "id": "think", "label": "Thinking", "kind": "toggle", "value": false, "options": [] }] }),
        json!({ "t": "convo.actions", "pane": PANE, "actions": [
            { "id": "mcp.add", "label": "Add MCP server", "args": [
                { "name": "name", "label": "Name", "required": true, "options": [] }] }] }),
    ]
}

fn press(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

#[tokio::test]
async fn the_session_sheet_sets_controls_runs_actions_and_steers_as_the_source_lists_them() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = opened(&mut conn, &mut events, &client).await;
    told(&conn, &mut app, &mut events, session_frames()).await;

    alt(&mut app, 's');
    let screen = painted(&mut app, 110, 30);
    for row in ["Model", "Opus", "Thinking", "Add MCP server", "compact", "rename"] {
        assert!(screen.contains(row), "{row} is missing:\n{screen}");
    }

    hit(&mut app, KeyCode::Enter);
    hit(&mut app, KeyCode::Down);
    hit(&mut app, KeyCode::Enter);
    let screen = painted(&mut app, 110, 30);
    assert!(screen.contains("Sonnet bills differently."), "{screen}");
    assert!(
        of(&conn.heard().await, "convo.set").is_empty(),
        "nothing before the yes"
    );
    press(&mut app, 'y');
    assert_eq!(
        of(&conn.heard().await, "convo.set"),
        [&json!({ "t": "convo.set", "pane": PANE, "control": "model", "value": "sonnet" })]
    );

    hit(&mut app, KeyCode::Down);
    hit(&mut app, KeyCode::Enter);
    assert_eq!(
        of(&conn.heard().await, "convo.set"),
        [&json!({ "t": "convo.set", "pane": PANE, "control": "think", "value": true })]
    );

    hit(&mut app, KeyCode::Down);
    hit(&mut app, KeyCode::Enter);
    hit(&mut app, KeyCode::Enter);
    assert!(
        of(&conn.heard().await, "convo.act").is_empty(),
        "a required argument is blank"
    );
    typed(&mut app, "github");
    hit(&mut app, KeyCode::Enter);
    assert_eq!(
        of(&conn.heard().await, "convo.act"),
        [&json!({ "t": "convo.act", "pane": PANE, "action": "mcp.add", "args": { "name": "github" } })]
    );
    told(
        &conn,
        &mut app,
        &mut events,
        vec![json!({ "t": "convo.acted", "pane": PANE, "action": "mcp.add", "ok": true, "text": "github added" })],
    )
    .await;
    assert!(painted(&mut app, 110, 30).contains("github added"));

    hit(&mut app, KeyCode::Down);
    hit(&mut app, KeyCode::Enter);
    assert_eq!(
        of(&conn.heard().await, "convo.steer"),
        [&json!({ "t": "convo.steer", "pane": PANE, "steer": "compact" })]
    );
    hit(&mut app, KeyCode::Down);
    hit(&mut app, KeyCode::Enter);
    typed(&mut app, "parser work");
    hit(&mut app, KeyCode::Enter);
    assert_eq!(
        of(&conn.heard().await, "convo.steer"),
        [&json!({ "t": "convo.steer", "pane": PANE, "steer": "rename", "text": "parser work" })]
    );

    hit(&mut app, KeyCode::Esc);
    assert!(
        !painted(&mut app, 110, 30).contains("Add MCP server"),
        "esc closes the sheet"
    );
    typed(&mut app, "x");
    assert_eq!(
        app.composer.text(PANE),
        "x",
        "and the keyboard is the box's again"
    );
}

#[tokio::test]
async fn the_tasks_sheet_follows_a_shell_opens_an_agent_and_stops_one_where_declared() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = opened(&mut conn, &mut events, &client).await;
    let frames = vec![
        abilities(json!({ "steer": ["stoptask"], "streams": ["taskoutput"] })),
        json!({ "t": "convo.tasks", "pane": PANE, "tasks": [
            { "id": "t1", "kind": "shell", "title": "cargo test", "backgrounded": false, "state": "running" },
            { "id": "a1", "kind": "agent", "title": "explore the parser", "backgrounded": false, "state": "running" }] }),
    ];
    told(&conn, &mut app, &mut events, frames).await;

    alt(&mut app, 't');
    let screen = painted(&mut app, 110, 30);
    assert!(
        screen.contains("cargo test") && screen.contains("explore the parser"),
        "{screen}"
    );
    hit(&mut app, KeyCode::Enter);
    assert_eq!(
        of(&conn.heard().await, "convo.follow"),
        [&json!({ "t": "convo.follow", "pane": PANE, "id": "t1", "from": 0 })]
    );
    told(
        &conn,
        &mut app,
        &mut events,
        vec![json!({ "t": "convo.task", "pane": PANE, "id": "t1", "at": 0, "text": "running 41 tests\n" })],
    )
    .await;
    assert!(painted(&mut app, 110, 30).contains("running 41 tests"));
    hit(&mut app, KeyCode::Esc);
    press(&mut app, 'x');
    assert_eq!(
        of(&conn.heard().await, "convo.steer"),
        [&json!({ "t": "convo.steer", "pane": PANE, "steer": "stoptask", "text": "t1" })]
    );

    hit(&mut app, KeyCode::Down);
    hit(&mut app, KeyCode::Enter);
    let frames = conn.heard().await;
    assert_eq!(
        of(&frames, "convo.sub"),
        [&json!({ "t": "convo.sub", "pane": PANE, "id": "a1", "before": null })],
        "{frames:?}"
    );
    assert_eq!(
        app.convo.reading(PANE),
        Some("a1"),
        "the sheet hands over to the subagent's view"
    );
}
