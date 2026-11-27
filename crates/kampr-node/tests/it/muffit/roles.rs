//! A worker muffit gives a custom role (`AgentRole::Other`, `{"Other": "<key>"}` on the wire) is
//! one more task, not a status frame the node cannot read; one it really cannot read is said aloud.

use serde_json::{Value, json};

use super::warnings::Warnings;
use super::{Client, Harness, eventually, pane};
use crate::muffit::fake::{self, Daemon, session, text};

async fn watched() -> (Daemon, Harness, Client, String) {
    let fake = fake::Fake::start(vec![session("s1", "running", Some("install ripgrep"))]).await;
    fake.set_transcript("s1", vec![text(1, "user", "install ripgrep")]);
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;
    (fake, h, client, id)
}

fn worker(title: &str, role: Value) -> Value {
    json!({ "id": "s1.s1", "liveness": "Alive", "role": role, "state": "running", "title": title })
}

async fn task_titled(client: &mut Client, id: &str, title: &str) -> Value {
    client
        .frame(&format!("the task {title}"), |f| {
            f["t"] == "convo.tasks" && f["pane"] == id && f["tasks"][0]["title"] == title
        })
        .await
}

#[tokio::test]
async fn a_worker_with_a_custom_role_is_a_task() {
    let (fake, _h, mut client, id) = watched().await;
    fake.status(
        Value::Null,
        json!([worker("Scout the repo", json!({ "Other": "scout" }))]),
    );
    let tasks = task_titled(&mut client, &id, "Scout the repo").await;
    assert_eq!(tasks["tasks"][0]["state"], "running", "{tasks}");
}

#[tokio::test]
async fn a_status_the_node_cannot_read_is_warned_once_per_change() {
    let (warnings, _guard) = Warnings::capture();
    let (fake, _h, mut client, id) = watched().await;
    let unreadable = |state: u32| json!({ "type": "status", "cursor": 2, "snapshot": { "session_id": "s1", "state": state } });
    fake.push(unreadable(4242));
    fake.push(unreadable(4242));
    fake.status(Value::Null, json!([worker("first", json!("TaskChimp"))]));
    task_titled(&mut client, &id, "first").await;
    assert_eq!(
        warnings.naming("4242").len(),
        1,
        "{:#?}",
        warnings.naming("status")
    );

    fake.push(unreadable(4343));
    fake.status(Value::Null, json!([worker("second", json!("Review"))]));
    task_titled(&mut client, &id, "second").await;
    assert_eq!(
        warnings.naming("4343").len(),
        1,
        "{:#?}",
        warnings.naming("status")
    );
}
