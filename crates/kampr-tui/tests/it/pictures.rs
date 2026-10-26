//! A picture for the pane's agent: from the clipboard on `ctrl+v`, or a file dropped on the
//! terminal, sent as the wire's `paste` so it lands on the pane's own machine.

use crate::shell::{Fake, app, node, pane, until};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kampr_client::Event;
use serde_json::{Value, json};
use std::sync::Arc;

const PNG: &[u8] = b"\x89PNG\r\n\x1a\nnot really a picture";

fn sent(frames: &[Value], t: &str) -> Vec<Value> {
    frames.iter().filter(|f| f["t"] == t).cloned().collect()
}

#[tokio::test]
async fn ctrl_v_sends_the_clipboards_picture_and_is_the_panes_own_key_when_there_is_none() {
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
    let ctrl_v = KeyEvent::new(KeyCode::Char('v'), KeyModifiers::CONTROL);

    app.read_pictures_with(Arc::new(|| Some(PNG.to_vec())));
    app.key(ctrl_v);
    let heard = conn.heard().await;
    let pasted = sent(&heard, "paste");
    assert_eq!(pasted.len(), 1, "{heard:?}");
    assert_eq!(pasted[0]["pane"], "01JNODE/w1:p1");
    use base64::Engine;
    assert_eq!(
        base64::engine::general_purpose::STANDARD
            .decode(pasted[0]["b64"].as_str().expect("bytes"))
            .expect("base64"),
        PNG
    );
    assert!(
        sent(&heard, "input").is_empty(),
        "the key itself is not typed as well"
    );

    app.read_pictures_with(Arc::new(|| None));
    app.key(ctrl_v);
    let heard = conn.heard().await;
    assert!(sent(&heard, "paste").is_empty());
    assert_eq!(
        sent(&heard, "input")
            .iter()
            .map(|f| f["text"].as_str().unwrap_or_default().to_string())
            .collect::<String>(),
        "\u{16}",
        "with no picture it is the pane's ctrl+v: {heard:?}"
    );

    let dir = tempfile::tempdir().expect("a dir");
    let shot = dir.path().join("my shot.png");
    std::fs::write(&shot, PNG).expect("a picture on disk");
    app.paste(&format!("'{}'", shot.display()));
    let heard = conn.heard().await;
    let dropped = sent(&heard, "paste");
    assert_eq!(
        dropped.len(),
        1,
        "a dropped picture file is the picture: {heard:?}"
    );
    assert_eq!(dropped[0]["name"], "my shot");

    app.paste("just some words.png and more");
    let heard = conn.heard().await;
    assert!(sent(&heard, "paste").is_empty());
    assert_eq!(
        sent(&heard, "input").len(),
        1,
        "and text is still text: {heard:?}"
    );
}
