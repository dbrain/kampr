//! Text pasted at the terminal, and a reply of more than one line, go to the node as one bracketed
//! paste marked `paste`, so the pane's own node frames it as the program there asked.

use crate::shell::{Fake, app, node, pane, until};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kampr_client::Event;
use serde_json::{Value, json};
use std::sync::Arc;

fn inputs(frames: &[Value]) -> Vec<Value> {
    frames.iter().filter(|f| f["t"] == "input").cloned().collect()
}

#[tokio::test]
async fn a_paste_and_a_reply_of_several_lines_are_marked_as_pastes() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "comingclean", true)]),
        json!([pane("01JNODE/w1:p1", "alpha", Some("claude"), "idle")]),
        "full",
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let mut app = app(&client);
    conn.heard().await;

    app.paste("one\ntwo");
    let heard = inputs(&conn.heard().await);
    assert_eq!(heard.len(), 1, "{heard:?}");
    assert_eq!(heard[0]["text"], "\u{1b}[200~one\ntwo\u{1b}[201~");
    assert_eq!(heard[0]["paste"], true, "{heard:?}");
    assert_eq!(heard[0]["typed"], true, "{heard:?}");

    app.key(KeyEvent::new(KeyCode::Char('b'), KeyModifiers::CONTROL));
    app.key(KeyEvent::new(KeyCode::Char('V'), KeyModifiers::SHIFT));
    for c in "first".chars() {
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::ALT));
    for c in "second".chars() {
        app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }
    app.key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
    let heard = inputs(&conn.heard().await);
    let texts: Vec<&str> = heard.iter().filter_map(|f| f["text"].as_str()).collect();
    assert_eq!(texts, ["\u{1b}[200~first\nsecond\u{1b}[201~", "\r"], "{heard:?}");
    assert_eq!(heard[0]["paste"], true, "{heard:?}");
    assert!(heard[1].get("paste").is_none(), "the Enter is a key: {heard:?}");
}
