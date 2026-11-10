//! Questions answered with words rather than a key, questions answered only at the terminal, and a
//! question whose context is a whole plan.

use super::*;

fn asked(multi: bool, options: Value) -> Value {
    json!({
        "t": "pending", "pane": PANE, "question": "Which fruit?", "header": "Fruit",
        "source": "hook", "id": "q1", "multi": multi, "options": options
    })
}

fn fruit() -> Value {
    asked(
        false,
        json!([
            { "key": "1", "label": "Apple" },
            { "key": "2", "label": "Pear" },
            { "key": "3", "label": "Type something", "text": true }
        ]),
    )
}

async fn on_a_blocked_claude(conn: &mut Conn, events: &mut Events, client: &Arc<Client>) -> App {
    conn.greet(json!([entry(PANE, Some("claude"), "blocked", true)]));
    pump_until_prefs(events).await;
    showing_conversation(client)
}

type Events = tokio::sync::broadcast::Receiver<Event>;

async fn pump_until_prefs(events: &mut Events) {
    until(events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
}

fn press(app: &mut App, c: char) {
    app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
}

fn typed(app: &mut App, words: &str) {
    words.chars().for_each(|c| press(app, c));
}

fn hit(app: &mut App, code: KeyCode) {
    app.key(KeyEvent::new(code, KeyModifiers::NONE));
}

fn answers(frames: &[Value]) -> Vec<&Value> {
    frames.iter().filter(|f| f["t"] == json!("answer")).collect()
}

#[tokio::test]
async fn a_text_option_opens_a_line_for_the_words_and_sends_them_with_its_key() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = on_a_blocked_claude(&mut conn, &mut events, &client).await;
    blocked_on(&mut conn, &mut app, &mut events, fruit()).await;

    press(&mut app, '3');
    let screen = painted(&mut app, 110, 24);
    assert!(screen.contains("esc cancels"), "{screen}");
    hit(&mut app, KeyCode::Enter);
    typed(&mut app, "2 ripe mangos");
    let frames = conn.heard().await;
    assert!(
        answers(&frames).is_empty(),
        "nothing is sent while typing: {frames:?}"
    );
    assert!(
        !frames.iter().any(|f| f["t"] == json!("input")),
        "and nothing reaches the pane: {frames:?}"
    );
    let screen = painted(&mut app, 110, 24);
    assert!(screen.contains("2 ripe mangos"), "{screen}");

    hit(&mut app, KeyCode::Enter);
    let frames = conn.heard().await;
    assert_eq!(
        answers(&frames),
        [&json!({ "t": "answer", "pane": PANE, "key": "3", "id": "q1", "text": "2 ripe mangos" })],
        "{frames:?}"
    );
    assert!(!frames.iter().any(|f| f["t"] == json!("input")), "{frames:?}");
    let screen = painted(&mut app, 110, 24);
    assert!(
        !screen.contains("esc cancels"),
        "the line closes once sent:\n{screen}"
    );

    for cancelled in ["", "half a thought"] {
        press(&mut app, '3');
        typed(&mut app, cancelled);
        hit(&mut app, KeyCode::Esc);
        assert!(answers(&conn.heard().await).is_empty(), "{cancelled:?}");
        press(&mut app, '1');
        assert_eq!(
            answers(&conn.heard().await),
            [&json!({ "t": "answer", "pane": PANE, "key": "1", "id": "q1" })],
            "esc after {cancelled:?} gives the keys back to the question"
        );
    }

    painted(&mut app, 110, 24);
    let chip = app.layout.chips[0].rects[2];
    let click = app.mouse.hit(down(chip.x, chip.y), &app.layout, Role::Full);
    app.clicked(click);
    assert!(
        answers(&conn.heard().await).is_empty(),
        "a click opens the line too"
    );
    let screen = painted(&mut app, 110, 24);
    assert!(screen.contains("esc cancels"), "{screen}");
    typed(&mut app, "kiwi");
    hit(&mut app, KeyCode::Enter);
    assert_eq!(answers(&conn.heard().await)[0]["text"], json!("kiwi"));
}

#[tokio::test]
async fn the_line_for_words_closes_when_its_question_goes() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = on_a_blocked_claude(&mut conn, &mut events, &client).await;
    blocked_on(&mut conn, &mut app, &mut events, fruit()).await;

    press(&mut app, '3');
    typed(&mut app, "mango");
    conn.send(json!({ "t": "pending", "pane": PANE, "question": null, "options": [], "source": "hook" }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Pending(_))).await;
    let screen = painted(&mut app, 110, 24);
    assert!(!screen.contains("esc cancels"), "{screen}");
    hit(&mut app, KeyCode::Enter);
    let frames = conn.heard().await;
    assert!(
        answers(&frames).is_empty(),
        "an answer to a question nobody is asking is never sent: {frames:?}"
    );
}

#[tokio::test]
async fn words_typed_on_a_question_taking_several_answers_go_with_the_ticks() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = on_a_blocked_claude(&mut conn, &mut events, &client).await;
    let several = asked(
        true,
        json!([
            { "key": "1", "label": "unit", "chosen": true },
            { "key": "2", "label": "live" },
            { "key": "3", "label": "Type something", "text": true }
        ]),
    );
    blocked_on(&mut conn, &mut app, &mut events, several).await;

    press(&mut app, '3');
    let screen = painted(&mut app, 110, 24);
    assert!(screen.contains("with the ticked"), "{screen}");
    typed(&mut app, "and e2e");
    hit(&mut app, KeyCode::Enter);
    let frames = conn.heard().await;
    assert_eq!(
        answers(&frames),
        [&json!({ "t": "answer", "pane": PANE, "key": "3", "id": "q1", "text": "and e2e" })],
        "{frames:?}"
    );
    assert_eq!(
        submits(&frames),
        0,
        "the words are the submit; a second commit would answer the next question: {frames:?}"
    );
}

#[tokio::test]
async fn a_hooked_question_with_nothing_to_press_sends_the_operator_to_the_terminal() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = on_a_blocked_claude(&mut conn, &mut events, &client).await;
    blocked_on(
        &mut conn,
        &mut app,
        &mut events,
        json!({
            "t": "pending", "pane": PANE, "question": "Sign in to Linear", "header": "linear",
            "source": "hook", "id": "q2", "context": "Email address", "options": []
        }),
    )
    .await;

    let screen = painted(&mut app, 110, 24);
    assert!(screen.contains("linear · Sign in to Linear"), "{screen}");
    assert!(screen.contains("Email address"), "{screen}");
    assert!(screen.contains("answer at the terminal"), "{screen}");
    assert!(screen.contains("prefix shift+v"), "{screen}");
    assert!(!screen.contains("no keys were offered"), "{screen}");

    press(&mut app, '1');
    hit(&mut app, KeyCode::Enter);
    let frames = conn.heard().await;
    assert!(answers(&frames).is_empty(), "{frames:?}");
    assert_eq!(submits(&frames), 0, "{frames:?}");
}

fn plan(steps: usize) -> Value {
    let body: String = (1..=steps).map(|n| format!("{n}. step number {n}\n")).collect();
    json!({
        "t": "pending", "pane": PANE, "question": "Approve Claude's plan?", "header": "Plan",
        "source": "hook", "id": "q3", "context": format!("# Split the router\n\n{body}"),
        "options": [
            { "key": "1", "label": "Yes, auto-accept edits" },
            { "key": "2", "label": "Yes, manually approve edits" },
            { "key": "3", "label": "No, tell Claude what to change", "text": true }
        ]
    })
}

#[tokio::test]
async fn a_long_plan_is_one_line_in_the_strip_and_opens_whole_above_it() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    let mut app = on_a_blocked_claude(&mut conn, &mut events, &client).await;
    blocked_on(&mut conn, &mut app, &mut events, plan(60)).await;

    let shut = painted(&mut app, 110, 24);
    assert!(shut.contains("Split the router"), "{shut}");
    assert!(shut.contains("alt+o"), "the rest is offered:\n{shut}");
    assert!(
        !shut.contains("step number 1"),
        "and the strip does not swamp the transcript:\n{shut}"
    );

    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::ALT));
    let open = painted(&mut app, 110, 24);
    assert!(open.contains("Split the router"), "{open}");
    assert!(open.contains("step number 1 "), "it opens at the top:\n{open}");
    assert!(!open.contains("step number 60"), "{open}");
    assert!(
        open.contains(" 3  No, tell"),
        "the question stays answerable while it is read:\n{open}"
    );
    for _ in 0..80 {
        hit(&mut app, KeyCode::Down);
    }
    let read = painted(&mut app, 110, 24);
    assert!(read.contains("step number 60"), "and it scrolls:\n{read}");

    hit(&mut app, KeyCode::Esc);
    let back = painted(&mut app, 110, 24);
    assert!(!back.contains("step number 60"), "esc comes back:\n{back}");
    assert!(back.contains("alt+o"), "{back}");

    app.key(KeyEvent::new(KeyCode::Char('o'), KeyModifiers::ALT));
    conn.send(json!({ "t": "pending", "pane": PANE, "question": null, "options": [], "source": "hook" }));
    pump(&mut app, &mut events, |e| matches!(e, Event::Pending(_))).await;
    let gone = painted(&mut app, 110, 24);
    assert!(
        !gone.contains("step number"),
        "a plan nobody is asking about any more is not left open:\n{gone}"
    );
    let frames = conn.heard().await;
    assert!(answers(&frames).is_empty(), "{frames:?}");
}
