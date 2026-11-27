//! What muffit's conversation sources widening (wire 3.4 to 3.9, kampr.md) reaches a client as:
//! each capability is asserted on the frame the node sends, so one the node never consumes fails.

use serde_json::{Value, json};
use std::time::Duration;

use super::{Client, Harness, eventually, pane};
use crate::muffit::fake::{self, Daemon, session, text};

async fn watched(fake: &Daemon) -> (Harness, Client, String) {
    fake.set_transcript("s1", vec![text(1, "user", "install ripgrep")]);
    let h = Harness::start(fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    eventually("the node attaches", || (fake.attached() > 0).then_some(())).await;
    (h, client, id)
}

async fn daemon() -> Daemon {
    fake::Fake::start(vec![session("s1", "running", Some("install ripgrep"))]).await
}

fn has(list: &Value, word: &str) -> bool {
    list.as_array().is_some_and(|l| l.iter().any(|x| x == word))
}

async fn abilities(client: &mut Client, id: &str, set: &str, word: &str) -> Value {
    client
        .frame(&format!("abilities with {set} {word}"), |f| {
            f["t"] == "convo.abilities" && f["pane"] == id && has(&f["abilities"][set], word)
        })
        .await
}

fn snapshot(extra: Value) -> Value {
    let mut snapshot = json!({ "session_id": "s1", "state": "running", "title": "install ripgrep",
                               "subagents": [], "model": "mock", "cwd": "/srv/work",
                               "prompt_tokens": 10, "completion_tokens": 2 });
    for (k, v) in extra.as_object().unwrap() {
        snapshot[k] = v.clone();
    }
    snapshot
}

fn control_frame(fake: &fake::Fake, verb: &str) -> Option<Value> {
    fake.attach_frames("control")
        .into_iter()
        .find(|f| f["verb"] == verb)
}

async fn until_posted(client: &mut Client, fake: &fake::Fake, path: &str, count: usize) -> Vec<Value> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        let posts = fake.posts(path);
        if posts.len() >= count {
            return posts;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "{count} posts to {path} never happened"
        );
        client.pump(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn the_models_question_is_an_ask_answered_by_option_or_in_words() {
    let fake = daemon().await;
    let (_h, mut client, id) = watched(&fake).await;
    abilities(&mut client, &id, "asks", "question").await;

    let asked =
        json!({ "id": 1, "text": "Which approach?", "options": ["Refactor", "Patch"], "free_text": true });
    fake.update("s1", |row| row["question"] = asked.clone());
    fake.snapshot(snapshot(json!({ "question": asked })));
    let pending = client
        .frame("the question", |f| {
            f["t"] == "pending" && f["question"] == "Which approach?"
        })
        .await;
    assert_eq!(pending["id"], "q1", "{pending}");
    assert_eq!(pending["kind"], "question", "{pending}");
    let options = pending["options"].as_array().unwrap();
    assert_eq!(options.len(), 3, "{pending}");
    assert_eq!(
        (&options[0]["key"], &options[0]["label"]),
        (&json!("1"), &json!("Refactor"))
    );
    assert_eq!(
        (&options[1]["key"], &options[1]["label"]),
        (&json!("2"), &json!("Patch"))
    );
    assert_eq!(options[2]["text"], true, "{pending}");

    // What every client sends today: the key, and a permission-shaped reply beside it.
    client
        .send(json!({ "t": "answer", "pane": id, "key": "2", "id": "q1",
                      "reply": { "allow": { "input": null, "remember": false } } }))
        .await;
    let posts = until_posted(&mut client, &fake, "/sessions/s1/question", 1).await;
    assert_eq!(posts, [json!({ "id": 1, "answer": 1 })]);

    let asked =
        json!({ "id": 2, "text": "Which approach?", "options": ["Refactor", "Patch"], "free_text": true });
    fake.update("s1", |row| row["question"] = asked.clone());
    fake.snapshot(snapshot(json!({ "question": asked })));
    client
        .frame("the second question", |f| f["t"] == "pending" && f["id"] == "q2")
        .await;
    client
        .send(json!({ "t": "answer", "pane": id, "key": "3", "id": "q2", "text": "neither, revert it" }))
        .await;
    let posts = until_posted(&mut client, &fake, "/sessions/s1/question", 2).await;
    assert_eq!(posts[1], json!({ "id": 2, "answer": "neither, revert it" }));
    assert!(
        fake.posts("/sessions/s1/permission").is_empty(),
        "a question is not a permission"
    );
}

#[tokio::test]
async fn shell_tasks_are_tasks_and_stop_through_job_stop() {
    let fake = daemon().await;
    let (_h, mut client, id) = watched(&fake).await;
    abilities(&mut client, &id, "steer", "stoptask").await;
    fake.snapshot(snapshot(
        json!({ "shell_tasks": [{ "id": "job1", "command": "cargo build", "state": "running" }] }),
    ));
    let tasks = client
        .frame("the shell task", |f| {
            f["t"] == "convo.tasks"
                && f["tasks"]
                    .as_array()
                    .is_some_and(|t| t.iter().any(|t| t["id"] == "job1"))
        })
        .await;
    let task = tasks["tasks"]
        .as_array()
        .unwrap()
        .iter()
        .find(|t| t["id"] == "job1")
        .unwrap()
        .clone();
    assert_eq!(task["kind"], "shell", "{task}");
    assert_eq!(task["title"], "cargo build", "{task}");
    assert_eq!(task["state"], "running", "{task}");

    client
        .send(json!({ "t": "convo.steer", "pane": id, "steer": "stoptask", "text": "job1" }))
        .await;
    let stop = eventually("job.stop reached the daemon", || control_frame(&fake, "job.stop")).await;
    assert_eq!(stop["args"], json!({ "id": "job1" }), "{stop}");
    client
        .frame("the stop acted", |f| {
            f["t"] == "convo.acted" && f["action"] == "job.stop" && f["ok"] == true
        })
        .await;
}

#[tokio::test]
async fn tool_argument_deltas_build_the_call_being_typed() {
    let fake = daemon().await;
    let (_h, mut client, id) = watched(&fake).await;
    abilities(&mut client, &id, "streams", "toolargs").await;
    fake.push(json!({ "type": "content", "blocks": [
        { "type": "tool_call_delta", "call_id": "c1", "tool": "bash", "delta": "{\"command\":", "seq": 0 } ] }));
    fake.push(json!({ "type": "content", "blocks": [
        { "type": "tool_call_delta", "call_id": "c1", "tool": "bash", "delta": "\"ls\"}", "seq": 1 } ] }));
    let live = client
        .frame("the call's arguments", |f| {
            f["t"] == "convo.live" && f["live"]["tools"][0]["args"] == "{\"command\":\"ls\"}"
        })
        .await;
    assert_eq!(live["live"]["tools"][0]["id"], "c1", "{live}");
    assert_eq!(live["live"]["tools"][0]["tool"], "bash", "{live}");
    assert_eq!(live["live"]["tools"][0]["state"], "running", "{live}");
}

fn action_ids(frame: &Value) -> Vec<String> {
    frame["actions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|a| a["id"].as_str().unwrap().to_string())
        .collect()
}

#[tokio::test]
async fn the_command_catalog_is_the_actions_and_the_slash_completion() {
    let fake = daemon().await;
    let (_h, mut client, id) = watched(&fake).await;
    abilities(&mut client, &id, "input", "slash").await;
    let actions = client
        .frame("the catalog's actions", |f| {
            f["t"] == "convo.actions" && action_ids(f).iter().any(|a| a == "rename")
        })
        .await;
    let ids = action_ids(&actions);
    for want in [
        "compact",
        "cost",
        "rename",
        "fork",
        "clear",
        "job.stop",
        "mcp.status",
        "btw",
    ] {
        assert!(ids.iter().any(|a| a == want), "{want} missing: {actions}");
    }
    for control in [
        "model.swap",
        "thinking.set",
        "permission.answer",
        "question.answer",
        "commands.list",
    ] {
        assert!(
            !ids.iter().any(|a| a == control),
            "{control} is not an action: {actions}"
        );
    }
    let rename = actions["actions"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["id"] == "rename")
        .unwrap();
    assert_eq!(rename["args"][0]["name"], "title", "{rename}");

    client
        .send(json!({ "t": "convo.suggest", "pane": id, "kind": "commands", "query": "re" }))
        .await;
    let suggested = client
        .frame("the completion", |f| {
            f["t"] == "convo.suggested" && f["query"] == "re"
        })
        .await;
    let items: Vec<&str> = suggested["items"]
        .as_array()
        .unwrap()
        .iter()
        .map(|i| i["id"].as_str().unwrap())
        .collect();
    for want in ["rename", "revert", "rewind"] {
        assert!(items.contains(&want), "{want}: {suggested}");
    }
    assert!(!items.contains(&"fork"), "{suggested}");

    client.input(&id, "/rename a better title").await;
    client.input(&id, "\r").await;
    let rename = eventually("the slash command ran its verb", || {
        control_frame(&fake, "rename")
    })
    .await;
    assert_eq!(rename["args"], json!({ "title": "a better title" }), "{rename}");
    tokio::time::sleep(Duration::from_millis(200)).await;
    assert!(
        fake.attach_frames("input").is_empty(),
        "a command is not a message to the model"
    );
}

#[tokio::test]
async fn billing_rate_limit_and_the_window_reach_their_frames() {
    let fake = daemon().await;
    let (_h, mut client, id) = watched(&fake).await;
    abilities(&mut client, &id, "facts", "billing").await;
    fake.snapshot(snapshot(json!({
        "billed_to": "openai-main: gpt-4",
        "rate_limit": { "limited": true, "retry_after_ms": 30000 },
        "context": { "measured_prompt_tokens": 1200, "pressure": { "estimated_tokens": 1000, "budget": 200000 } }
    })));
    let session = client
        .frame("the billed session", |f| {
            f["t"] == "convo.session" && !f["facts"]["billing"]["provider"].is_null()
        })
        .await;
    assert_eq!(
        session["facts"]["billing"]["provider"], "openai-main",
        "{session}"
    );
    assert_eq!(session["facts"]["billing"]["rate_limited"], true, "{session}");
    let limits = client.frame("the limits", |f| f["t"] == "convo.limits").await;
    assert_eq!(limits["limits"]["usage"], "rate-limited", "{limits}");
    assert_eq!(limits["limits"]["resets_at"], "in 30 s", "{limits}");
    let context = client.frame("the context", |f| f["t"] == "convo.context").await;
    assert_eq!(
        context["context"],
        json!({ "used": 1200, "total": 200000 }),
        "{context}"
    );
}

#[tokio::test]
async fn prompt_events_raise_and_settle_the_pending_ask() {
    let fake = daemon().await;
    let (_h, mut client, id) = watched(&fake).await;
    let hello = &fake.attach_frames("hello")[0];
    assert!(has(&hello["capabilities"], "events"), "{hello}");
    let prompt = json!({ "id": 5, "tool": "bash", "action": "run rm -rf target" });
    fake.push(
        json!({ "type": "event", "id": 9, "time_unix_ms": 1, "event_type": "PromptRaised",
                      "event": { "PromptRaised": prompt } }),
    );
    let pending = client
        .frame("the raised prompt", |f| {
            f["t"] == "pending" && f["pane"] == id && f["id"] == "5"
        })
        .await;
    assert_eq!(pending["question"], "run rm -rf target", "{pending}");
    fake.push(
        json!({ "type": "event", "id": 10, "time_unix_ms": 2, "event_type": "PromptSettled",
                      "event": { "PromptSettled": { "prompt": prompt, "outcome": "denied" } } }),
    );
    client
        .frame("the settled prompt", |f| {
            f["t"] == "pending" && f["pane"] == id && f["question"].is_null()
        })
        .await;
}

#[tokio::test]
async fn pause_resume_and_stop_go_over_rest() {
    let fake = daemon().await;
    let (_h, mut client, id) = watched(&fake).await;
    for steer in ["pause", "resume", "stop"] {
        client
            .send(json!({ "t": "convo.steer", "pane": id, "steer": steer }))
            .await;
        until_posted(&mut client, &fake, &format!("/sessions/s1/{steer}"), 1).await;
        client
            .frame(&format!("{steer} acted"), |f| {
                f["t"] == "convo.acted" && f["action"] == steer
            })
            .await;
        assert!(
            control_frame(&fake, steer).is_none(),
            "{steer} is not a socket verb"
        );
    }
}

#[tokio::test]
async fn a_worker_is_watched_over_its_own_observe_socket() {
    let fake = daemon().await;
    fake.set_transcript("s1.s1", vec![text(1, "user", "Make the file")]);
    let (_h, mut client, id) = watched(&fake).await;
    abilities(&mut client, &id, "streams", "sublive").await;
    client
        .send(json!({ "t": "convo.sub", "pane": id, "id": "s1.s1" }))
        .await;
    client
        .frame("the worker's page", |f| f["t"] == "convo" && f["sub"] == "s1.s1")
        .await;
    eventually("the worker is observed", || {
        fake.children().contains(&"s1.s1".to_string()).then_some(())
    })
    .await;
    fake.append("s1.s1", vec![text(2, "assistant", "Created made.txt.")]);
    fake.push_child(
        json!({ "type": "event", "id": 16, "time_unix_ms": 1, "event_type": "ToolCalled",
                            "event": { "ToolCalled": {} } }),
    );
    client.turn("the worker's new turn", |t| t["id"] == "m2").await;
}

#[tokio::test]
async fn an_older_daemon_gets_exactly_the_pane_it_got_before() {
    let fake = daemon().await;
    fake.speak(303);
    let (_h, mut client, id) = watched(&fake).await;
    fake.snapshot(snapshot(json!({})));
    client.frame("the state", |f| f["t"] == "convo.state").await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    for _ in 0..5 {
        client.pump(Duration::from_millis(20)).await;
    }
    let declared: Vec<Value> = client
        .frames
        .iter()
        .filter(|f| f["t"] == "convo.abilities" && f["pane"] == id)
        .cloned()
        .collect();
    assert!(!declared.is_empty(), "no abilities at all");
    for abilities in &declared {
        let a = &abilities["abilities"];
        for (set, word) in [
            ("asks", "question"),
            ("input", "slash"),
            ("streams", "toolargs"),
            ("steer", "stoptask"),
            ("facts", "billing"),
            ("streams", "sublive"),
        ] {
            assert!(
                !has(&a[set], word),
                "a 3.3 daemon declared {set} {word}: {abilities}"
            );
        }
        assert!(has(&a["steer"], "compact"), "{abilities}");
    }
    assert!(
        control_frame(&fake, "commands.list").is_none(),
        "a 3.3 daemon has no catalog to ask"
    );
    let actions = client
        .frame("today's actions", |f| f["t"] == "convo.actions")
        .await;
    assert!(action_ids(&actions).iter().any(|a| a == "compact"), "{actions}");
}

#[tokio::test]
async fn a_question_nobody_is_watching_is_the_warm_reads_pending() {
    let fake = daemon().await;
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    let asked =
        json!({ "id": 3, "text": "Which approach?", "options": ["Refactor", "Patch"], "free_text": false });
    fake.update("s1", |row| row["question"] = asked.clone());
    client.pane(&id, |p| p["agent_status"] == "blocked").await;
    let warm = super::request(
        &h.origin,
        "GET",
        &format!("/api/warm?pane={}", id.replace('/', "%2F")),
        Some(&client.token),
        "",
    )
    .await;
    let pending = &warm["pending"];
    assert_eq!(pending["question"], "Which approach?", "{warm}");
    assert_eq!(pending["id"], "q3", "{warm}");
    assert_eq!(pending["options"][1]["label"], "Patch", "{warm}");
}
