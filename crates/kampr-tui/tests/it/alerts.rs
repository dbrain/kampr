//! herdr's background toast, for the whole herd: an agent off screen that starts needing the
//! operator, or finishes, is said once it has stayed that way for the debounce.

use crate::shell::{Fake, node, pane, until};
use kampr_client::Event;
use kampr_tui::alert::{Delivery, Toast};
use kampr_tui::app::{App, Options};
use kampr_tui::image::Images;
use serde_json::{Value, json};
use std::sync::Arc;
use std::time::Duration;

const DEBOUNCE: Duration = Duration::from_millis(150);

fn agents(here: &str, away: &str, stays: &str) -> Value {
    let on_screen = pane("01JNODE/w1:p1", "alpha", Some("claude"), here);
    let mut off = pane("01JNODE/w2:p2", "beta", Some("codex"), away);
    off["workspace_id"] = json!("01JNODE/w2");
    off["tab_id"] = json!("01JNODE/w2:t1");
    off["title"] = json!("backfill the events");
    let mut steady = pane("01JNODE/w3:p3", "gamma", Some("claude"), stays);
    steady["workspace_id"] = json!("01JNODE/w3");
    steady["tab_id"] = json!("01JNODE/w3:t1");
    let mut also = pane("01JNODE/w4:p4", "delta", Some("claude"), stays);
    also["workspace_id"] = json!("01JNODE/w4");
    also["tab_id"] = json!("01JNODE/w4:t1");
    json!([on_screen, off, steady, also])
}

#[tokio::test]
async fn an_agent_off_screen_that_blocks_is_toasted_once_after_the_debounce_and_nothing_else_is() {
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let nodes = json!([node("01JNODE", "comingclean", true)]);
    conn.greet(nodes.clone(), agents("idle", "working", "blocked"), "full");
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let options = Options {
        toast: Delivery::Terminal,
        toast_delay: DEBOUNCE,
        ..Options::default()
    };
    let mut app = App::new(client.clone(), options, Images::default());
    app.absorb(&Event::Prefs { greeting: true });
    app.clicked(kampr_tui::mouse::Click::Focus("01JNODE/w1:p1".into()));
    let herd = |panes: Value| conn.send(json!({ "t": "herd", "nodes": nodes, "panes": panes }));

    herd(agents("blocked", "blocked", "blocked"));
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;
    app.absorb(&Event::Herd);
    assert_eq!(app.toasts(), Vec::<Toast>::new(), "not before the debounce");
    tokio::time::sleep(DEBOUNCE * 2).await;
    let toasts = app.toasts();
    assert_eq!(
        toasts,
        vec![Toast {
            pane: "01JNODE/w2:p2".into(),
            title: "codex needs attention".into(),
            body: "backfill the events · beta · comingclean".into(),
        }],
        "the pane on screen is not news, and one already blocked at launch was never a change"
    );
    tokio::time::sleep(DEBOUNCE * 2).await;
    assert_eq!(app.toasts(), Vec::<Toast>::new(), "said once");

    herd(agents("blocked", "working", "blocked"));
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;
    app.absorb(&Event::Herd);
    herd(agents("blocked", "done", "blocked"));
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;
    app.absorb(&Event::Herd);
    herd(agents("blocked", "working", "blocked"));
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;
    app.absorb(&Event::Herd);
    tokio::time::sleep(DEBOUNCE * 2).await;
    assert_eq!(
        app.toasts(),
        Vec::<Toast>::new(),
        "a finish that was over before the debounce ran out is not said"
    );
}

#[test]
fn the_terminal_is_asked_the_way_herdr_asks_it() {
    let toast = Toast {
        pane: "p".into(),
        title: "codex needs attention".into(),
        body: "beta \u{1b}]0;x\u{7}".into(),
    };
    assert_eq!(
        kampr_tui::alert::osc99(&toast),
        "\u{1b}]99;i=1:d=0;codex needs attention\u{1b}\\\u{1b}]99;i=1:p=body;beta ]0;x\u{1b}\\",
        "and pane text cannot end the sequence early"
    );
}

/// kampr's own `[toast]` in `tui.toml` is what a desk with no herdr on it — native sessions need
/// none — says, and it outranks herdr's on a desk that has both.
#[tokio::test]
async fn this_desks_own_toast_setting_is_kamprs_not_herdrs() {
    let dir = tempfile::tempdir().expect("a config dir");
    std::fs::write(
        dir.path().join("tui.toml"),
        "[toast]\ndelivery = \"off\"\ndelay_seconds = 0\n",
    )
    .expect("a tui.toml");
    let mut fake = Fake::start().await;
    let client = Arc::new(fake.client());
    let mut events = client.events();
    let conn = fake.accept().await;
    let nodes = json!([node("01JNODE", "comingclean", true)]);
    conn.greet(nodes.clone(), agents("idle", "working", "blocked"), "full");
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    let options = Options {
        toast: Delivery::Terminal,
        toast_delay: DEBOUNCE,
        ..Options::default()
    };
    let mut app = App::new(client.clone(), options, Images::default());
    app.remember_in(dir.path().to_path_buf());
    app.absorb(&Event::Prefs { greeting: true });
    app.clicked(kampr_tui::mouse::Click::Focus("01JNODE/w1:p1".into()));

    conn.send(json!({ "t": "herd", "nodes": nodes, "panes": agents("idle", "blocked", "blocked") }));
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;
    app.absorb(&Event::Herd);
    tokio::time::sleep(DEBOUNCE * 2).await;
    assert_eq!(app.toasts(), Vec::<Toast>::new());
}
