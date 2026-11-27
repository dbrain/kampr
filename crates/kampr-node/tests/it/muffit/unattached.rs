//! A pane whose attach socket is gone reaches muffit over REST only where the daemon has a route
//! (`muffit-server/src/lib.rs` `routes`); a verb with none is refused aloud, never posted to a 404.

use serde_json::{Value, json};
use std::time::Duration;

use super::{Client, Harness, TOKEN, eventually, pane};
use crate::muffit::fake::{self, Daemon, session, text};

async fn hung_up(hang_up: fn(&fake::Fake)) -> (Daemon, Harness, Client, String) {
    let fake = fake::Fake::start(vec![session("s1", "running", Some("install ripgrep"))]).await;
    fake.set_transcript(
        "s1",
        vec![
            text(1, "user", "install ripgrep"),
            text(2, "assistant", "installed"),
        ],
    );
    let h = Harness::start(&fake).await;
    let mut client = h.client().await;
    let id = pane("s1");
    client.watch(&id).await;
    client.convo(&id).await;
    client
        .frame("the 3.9 greeting", |f| {
            f["t"] == "convo.abilities"
                && f["pane"] == id
                && f["abilities"]["steer"]
                    .as_array()
                    .is_some_and(|s| s.iter().any(|x| x == "fork"))
        })
        .await;
    hang_up(&fake.fake);
    eventually("the attach closes", || (fake.attached() == 0).then_some(())).await;
    client.pump(Duration::from_millis(300)).await;
    (fake, h, client, id)
}

async fn acted(client: &mut Client, fake: &fake::Fake, id: &str, steer: Value, action: &str) -> Value {
    let mut frame = json!({ "t": "convo.steer", "pane": id });
    for (k, v) in steer.as_object().unwrap() {
        frame[k] = v.clone();
    }
    client.send(frame).await;
    let acted = client
        .frame(&format!("{action} acted"), |f| {
            f["t"] == "convo.acted" && f["action"] == action
        })
        .await;
    assert!(
        fake.attach_frames("control").iter().all(|f| f["verb"] != action),
        "{action} went over a socket"
    );
    acted
}

#[tokio::test]
async fn without_a_socket_rename_and_fork_take_the_daemons_rest_routes() {
    let (fake, _h, mut client, id) = hung_up(fake::Fake::hang_up).await;

    let renamed = acted(
        &mut client,
        &fake,
        &id,
        json!({ "steer": "rename", "text": "a better title" }),
        "rename",
    )
    .await;
    assert_eq!(renamed["ok"], true, "{renamed}");
    assert_eq!(
        fake.posts("/sessions/s1/title"),
        vec![json!({ "title": "a better title" })]
    );

    let forked = acted(&mut client, &fake, &id, json!({ "steer": "fork" }), "fork").await;
    assert_eq!(forked["ok"], true, "{forked}");
    assert_eq!(fake.posts("/sessions/s1/fork"), vec![json!({ "from_seq": 2 })]);
}

#[tokio::test]
async fn without_a_socket_a_verb_muffit_has_no_route_for_is_refused_not_posted() {
    let (fake, _h, mut client, id) = hung_up(fake::Fake::hang_up).await;
    for (steer, verb) in [
        ("compact", "context.compact"),
        ("clear", "clear"),
        ("stoptask", "job.stop"),
    ] {
        let refused = acted(
            &mut client,
            &fake,
            &id,
            json!({ "steer": steer, "text": "job1" }),
            verb,
        )
        .await;
        assert_eq!(refused["ok"], false, "{refused}");
        assert!(
            refused["text"]
                .as_str()
                .is_some_and(|t| t.contains("not attached")),
            "{refused}"
        );
        assert!(
            fake.seen()
                .iter()
                .all(|s| s.path != format!("/sessions/s1/{verb}")),
            "{verb} was posted to a route muffit does not have"
        );
    }
}

#[tokio::test]
async fn a_verb_on_a_session_nobody_may_watch_attaches_and_restores_it() {
    let (fake, _h, mut client, id) = hung_up(fake::Fake::hang_up_on_watchers).await;
    for (steer, verb) in [("compact", "context.compact"), ("fork", "fork")] {
        client
            .send(json!({ "t": "convo.steer", "pane": id, "steer": steer }))
            .await;
        eventually(&format!("{verb} on a held socket"), || {
            fake.attach_frames("control")
                .into_iter()
                .find(|f| f["verb"] == verb)
        })
        .await;
        assert!(
            fake.seen()
                .iter()
                .all(|s| s.path != format!("/sessions/s1/{verb}")),
            "{verb} was posted"
        );
    }
    let held = fake
        .seen()
        .into_iter()
        .filter(|s| s.path == "/sessions/s1/attach" && s.query.as_deref() != Some("observe=true"))
        .count();
    assert_eq!(
        held,
        1,
        "one attach holds the session, the rest watch: {:#?}",
        fake.seen()
    );
}

#[tokio::test]
async fn the_fake_refuses_what_muffit_refuses() {
    let fake = fake::Fake::start(vec![session("s1", "running", None)]).await;
    let http = reqwest::Client::new();
    let post = |path: &str, body: Value| {
        http.post(format!("{}{path}", fake.url))
            .bearer_auth(TOKEN)
            .json(&body)
            .send()
    };
    let status = |r: reqwest::Response| r.status().as_u16();
    assert_eq!(
        status(post("/sessions/s1/context.compact", json!({})).await.unwrap()),
        404
    );
    assert_eq!(
        status(post("/sessions/s1/fork", json!({ "to_seq": 2 })).await.unwrap()),
        422
    );
    assert_eq!(status(post("/sessions/s1/fork", json!({})).await.unwrap()), 400);
    assert_eq!(
        status(
            post("/sessions/s1/permission", json!({ "id": 1, "answer": "maybe" }))
                .await
                .unwrap()
        ),
        422
    );
    assert_eq!(
        status(
            post(
                "/sessions/s1/permission",
                json!({ "id": 1, "answer": "allow", "why": "x" })
            )
            .await
            .unwrap()
        ),
        422
    );
    assert_eq!(
        status(
            post("/sessions/s1/question", json!({ "id": 1, "answer": [0] }))
                .await
                .unwrap()
        ),
        422
    );
    assert_eq!(
        status(
            post("/sessions/s1/input", json!({ "text": "hi", "cursor": 3 }))
                .await
                .unwrap()
        ),
        422
    );
}
