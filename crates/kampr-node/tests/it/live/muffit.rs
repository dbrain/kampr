//! A TUI started by hand in a herdr pane, the way an operator runs `muffit`, against a real herdr.

use crate::muffit::eventually;
use crate::muffit::fake::{Fake, TOKEN, session};
use serde_json::json;

#[tokio::test]
async fn a_herdr_pane_running_the_tui_is_the_session_s_pane() {
    let fake = Fake::start(vec![]).await;
    let h = super::Harness::start_with("muffit-tui", |config| {
        config.muffit.enabled = Some(true);
        config.muffit.url = fake.url.clone();
        config.muffit.token = TOKEN.to_string();
    })
    .await;
    let node = h.node.node_id().to_string();
    let pane = eventually("the herdr pane", || {
        h.node
            .herd()
            .panes
            .iter()
            .find(|p| p.node_id == node)
            .map(|p| p.id.clone())
    })
    .await;
    let local = pane.split_once('/').unwrap().1.to_string();
    let dir = tempfile::tempdir().unwrap();
    let script = format!(
        "sh -c 'echo $$ > {0}/pid; exec cat > /dev/null'\n",
        dir.path().display()
    );
    kampr_herdr::Herdr::new(&h.node.config.herdr.socket)
        .call::<serde_json::Value>("pane.send_text", json!({ "pane_id": local, "text": script }))
        .await
        .expect("typed into the pane");
    let pid: u32 = eventually("the stand-in's pid", || {
        std::fs::read_to_string(dir.path().join("pid"))
            .ok()?
            .trim()
            .parse()
            .ok()
    })
    .await;

    let mut row = session("t1", "waiting_user", Some("from the tui"));
    row["clients"] = json!([{ "kind": "tui", "pid": pid, "host": kampr_muffit::this_host().expect("a hostname"), "observing": false }]);
    fake.insert(row);
    let linked = eventually("the pane taken for the session", || {
        h.node
            .herd()
            .panes
            .iter()
            .find(|p| p.id == pane && p.agent.as_deref() == Some("muffit"))
            .cloned()
    })
    .await;
    assert!(linked.has_conversation, "{linked:?}");
    assert!(
        !h.node.herd().panes.iter().any(|p| p.id.ends_with("/m:t1")),
        "the session has a pane beside its terminal"
    );
}
