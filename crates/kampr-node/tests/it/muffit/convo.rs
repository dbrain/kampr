//! The conversation wire vocabulary against muffit (docs/21, Phase 2a): the daemon's own data
//! arrives on the wire as the new `convo.*` frames, and the pane's new verbs reach the daemon.

use serde_json::{Value, json};

use super::{Client, Harness, pane};
use crate::muffit::fake::{self, session, text};
use std::time::Duration;

/// The daemon's REST is the ground truth here, and the node's write to a client that has stopped
/// reading can hold the answer in its queue, so the wait keeps the socket drained.
async fn until_posted(client: &mut Client, fake: &fake::Fake, what: &str, want: impl Fn(&[Value]) -> bool) {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let posts = fake.posts("/sessions/s1/permission");
        if want(&posts) {
            return;
        }
        assert!(tokio::time::Instant::now() < deadline, "{what} never happened");
        client.pump(Duration::from_millis(20)).await;
    }
}

fn sets(list: &Value, id: &str) -> bool {
    list.as_array()
        .is_some_and(|set| set.iter().any(|x| x.as_str() == Some(id)))
}

fn control<'a>(controls: &'a Value, id: &str) -> &'a Value {
    controls
        .as_array()
        .unwrap()
        .iter()
        .find(|c| c["id"] == id)
        .unwrap_or_else(|| panic!("no control {id}: {controls}"))
}

#[tokio::test]
async fn the_daemons_facts_ride_the_new_frames() {
    let fake = fake::Fake::start(vec![session("s1", "running", Some("install ripgrep"))]).await;
    fake.set_transcript("s1", vec![text(1, "user", "install ripgrep")]);
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    super::eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;

    fake.status(
        Value::Null,
        json!([
            { "call_id": "call_7", "id": "s1.s1", "liveness": "Alive", "role": "TaskChimp",
              "state": "running", "task_id": "t-1", "title": "Make the file" }
        ]),
    );

    let abilities = client
        .frame("convo.abilities", |f| {
            f["t"] == "convo.abilities" && f["pane"] == id
        })
        .await;
    let steer = &abilities["abilities"]["steer"];
    for s in ["interrupt", "continue", "pause", "resume", "stop", "compact"] {
        assert!(sets(steer, s), "{abilities}");
    }
    let facts = &abilities["abilities"]["facts"];
    for f in ["context", "cost", "trouble"] {
        assert!(sets(facts, f), "{abilities}");
    }
    assert!(
        sets(&abilities["abilities"]["streams"], "taskoutput"),
        "{abilities}"
    );

    let state = client
        .frame("convo.state", |f| f["t"] == "convo.state" && f["pane"] == id)
        .await;
    assert_eq!(state["state"], "running", "{state}");

    let tasks = client
        .frame("convo.tasks", |f| f["t"] == "convo.tasks" && f["pane"] == id)
        .await;
    assert_eq!(tasks["tasks"].as_array().unwrap().len(), 1, "{tasks}");
    assert_eq!(tasks["tasks"][0]["id"], "s1.s1", "{tasks}");
    assert_eq!(tasks["tasks"][0]["call"], "call_7", "{tasks}");
    assert_eq!(tasks["tasks"][0]["title"], "Make the file", "{tasks}");
    assert_eq!(tasks["tasks"][0]["state"], "running", "{tasks}");
    assert_eq!(tasks["tasks"][0]["backgrounded"], true, "{tasks}");

    let controls = client
        .frame("convo.controls with the model's options", |f| {
            f["t"] == "convo.controls"
                && f["pane"] == id
                && control(&f["controls"], "model")["options"]
                    .as_array()
                    .is_some_and(|o| o.len() == 2 && o[0]["value"] == "mock" && o[1]["value"] == "mock2")
        })
        .await;
    assert_eq!(
        control(&controls["controls"], "model")["value"],
        "mock",
        "{controls}"
    );
    assert_eq!(
        control(&controls["controls"], "thinking")["value"],
        "low",
        "{controls}"
    );
    assert_eq!(
        control(&controls["controls"], "mode")["value"],
        true,
        "{controls}"
    );
    assert_eq!(
        control(&controls["controls"], "permissions")["value"],
        "auto",
        "{controls}"
    );

    let actions = client
        .frame("convo.actions", |f| f["t"] == "convo.actions" && f["pane"] == id)
        .await;
    let ids: Vec<&str> = actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"compact"), "{actions}");
    assert!(ids.contains(&"cost"), "{actions}");

    let session_facts = client
        .frame("convo.session", |f| f["t"] == "convo.session" && f["pane"] == id)
        .await;
    assert_eq!(session_facts["facts"]["id"], "s1", "{session_facts}");
    assert_eq!(session_facts["facts"]["cwd"], "/srv/work", "{session_facts}");
    assert_eq!(session_facts["facts"]["model"], "mock", "{session_facts}");
    assert_eq!(session_facts["facts"]["mode"], "plan", "{session_facts}");

    let context = client
        .frame("convo.context", |f| f["t"] == "convo.context" && f["pane"] == id)
        .await;
    assert_eq!(context["context"]["used"], 1200, "{context}");

    fake.push(json!({ "type": "error", "message": "the model said no" }));
    let trouble = client
        .frame("convo.trouble", |f| f["t"] == "convo.trouble" && f["pane"] == id)
        .await;
    assert_eq!(trouble["trouble"]["text"], "the model said no", "{trouble}");
    super::asked(&fake);
}

#[tokio::test]
async fn the_pane_drives_the_session_over_the_new_verbs() {
    let fake = fake::Fake::start(vec![session("s1", "running", Some("install ripgrep"))]).await;
    fake.set_transcript("s1", vec![text(1, "user", "install ripgrep")]);
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    super::eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;

    client
        .send(json!({ "t": "convo.set", "pane": id, "control": "model", "value": "mock2" }))
        .await;
    let swap = super::eventually("the model swap reached the daemon", || {
        fake.attach_frames("control")
            .into_iter()
            .find(|f| f["verb"] == "model.swap")
    })
    .await;
    assert_eq!(swap["args"], json!({ "spec": "mock2" }), "{swap}");
    let acted = client
        .frame("convo.acted for the swap", |f| {
            f["t"] == "convo.acted" && f["pane"] == id && f["action"] == "model.swap"
        })
        .await;
    assert_eq!(acted["ok"], true, "{acted}");

    client
        .send(json!({ "t": "convo.steer", "pane": id, "steer": "interrupt" }))
        .await;
    super::eventually("the interrupt reached the daemon", || {
        fake.attach_frames("interrupt").first().cloned()
    })
    .await;

    client
        .send(json!({ "t": "convo.act", "pane": id, "action": "compact", "args": {} }))
        .await;
    let compact = super::eventually("the compact reached the daemon", || {
        fake.attach_frames("control")
            .into_iter()
            .find(|f| f["verb"] == "context.compact")
    })
    .await;
    assert_eq!(compact["args"], json!({}), "{compact}");
    client
        .frame("convo.acted for the compact", |f| {
            f["t"] == "convo.acted" && f["pane"] == id && f["action"] == "context.compact"
        })
        .await;

    client
        .send(json!({ "t": "convo.follow", "pane": id, "id": "call_7", "from": 0 }))
        .await;
    fake.push(json!({ "type": "content", "blocks": [
        { "type": "tool_output_chunk", "call_id": "call_7", "stream": "stdout", "text": "Worker s1 on mock\n", "seq": 0 }
    ] }));
    let chunk = client
        .frame("convo.task for the followed output", |f| {
            f["t"] == "convo.task" && f["pane"] == id && f["id"] == "call_7" && f["ended"].is_null()
        })
        .await;
    assert_eq!(chunk["at"], 0, "{chunk}");
    assert_eq!(chunk["text"], "Worker s1 on mock\n", "{chunk}");
    fake.push(json!({ "type": "content", "blocks": [
        { "type": "tool_result", "call_id": "call_7", "summary": "The worker completed the task.", "truncated": false }
    ] }));
    let ended = client
        .frame("convo.task for the end", |f| {
            f["t"] == "convo.task" && f["pane"] == id && f["id"] == "call_7" && !f["ended"].is_null()
        })
        .await;
    assert_eq!(ended["ended"]["ok"], true, "{ended}");
    assert_eq!(
        ended["ended"]["summary"], "The worker completed the task.",
        "{ended}"
    );
    super::asked(&fake);
}

#[tokio::test]
async fn the_widened_answer_names_the_reply_and_the_old_shape_still_decodes() {
    let fake = fake::Fake::start(vec![session("s1", "running", Some("install ripgrep"))]).await;
    fake.set_transcript("s1", vec![text(1, "user", "install ripgrep")]);
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    super::eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;

    let prompt = json!({ "action": "run `cargo install ripgrep` with write access outside the project",
                         "id": 1, "tool": "bash" });
    fake.update("s1", |row| row["permission"] = prompt.clone());
    fake.status(prompt.clone(), json!([]));
    let pending = client
        .frame("the prompt", |f| f["t"] == "pending" && !f["question"].is_null())
        .await;
    assert_eq!(pending["id"], "1", "{pending}");

    client
        .send(json!({ "t": "answer", "pane": id, "key": "1", "id": "1",
                      "reply": { "allow": { "input": null, "remember": true } } }))
        .await;
    until_posted(&mut client, &fake, "the answer posted", |posts| !posts.is_empty()).await;
    assert_eq!(
        fake.posts("/sessions/s1/permission"),
        [json!({ "id": 1, "answer": "always" })],
        "the reply's remember is what picks always, over the key"
    );

    let prompt2 = json!({ "action": "run `cargo install ripgrep` with write access outside the project",
                      "id": 2, "tool": "bash" });
    fake.update("s1", |row| row["permission"] = prompt2.clone());
    fake.status(prompt2, json!([]));
    // A key-only answer resolves against the prompt the node holds, and under load that is still
    // the first one until the second's pending frame lands — so the answer waits for it, as the
    // first does.
    client
        .frame("the second prompt", |f| f["t"] == "pending" && f["id"] == "2")
        .await;
    client
        .send(json!({ "t": "answer", "pane": id, "key": "3" }))
        .await;
    until_posted(&mut client, &fake, "the second answer posted", |posts| {
        posts.len() >= 2
    })
    .await;
    assert_eq!(
        fake.posts("/sessions/s1/permission"),
        [
            json!({ "id": 1, "answer": "always" }),
            json!({ "id": 2, "answer": "deny" })
        ],
        "a key-only answer, the old shape, still reaches the daemon"
    );
    tokio::time::sleep(Duration::from_millis(100)).await;
    client.pump(Duration::from_millis(50)).await;
    super::asked(&fake);
}
