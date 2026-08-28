//! A node scripted frame by frame.
//!
//! The client is written against `04-wire-protocol.md` and never against herdr, so the honest
//! counterpart for it is a server that says exactly what that document says a node may say — in
//! the orders and shapes a real one is hard to provoke into. `node.rs` beside this drives the
//! real node for the handshake and the greeting; this drives the rules.

use futures_util::{SinkExt, StreamExt};
use kampr_client::{Client, Event, Policy, Role, Session, Via};
use kampr_core::Backoff;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::net::TcpListener;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};

const BEAT: Duration = Duration::from_secs(2);

struct Conn {
    to_client: mpsc::UnboundedSender<Message>,
    from_client: mpsc::UnboundedReceiver<String>,
}

impl Conn {
    fn send(&self, frame: Value) {
        self.to_client
            .send(Message::text(frame.to_string()))
            .expect("the scripted node still has a client");
    }

    fn close(&self) {
        let _ = self.to_client.send(Message::Close(None));
    }

    async fn recv(&mut self) -> Value {
        let text = tokio::time::timeout(BEAT, self.from_client.recv())
            .await
            .expect("the client sent nothing")
            .expect("the client hung up");
        serde_json::from_str(&text).expect("the client sent JSON")
    }

    /// The three greeting frames, in the order the protocol names them.
    fn greet(&self, nodes: Value, panes: Value) {
        self.send(json!({
            "t": "hello", "protocol": 1, "node_id": "01JNODE", "node_name": "scripted",
            "build": "0.1.29", "role": "full",
            "caps": { "push": false, "scrollback": true, "conversation": true, "manage": true }
        }));
        self.send(json!({ "t": "herd", "nodes": nodes, "panes": panes }));
        self.send(json!({ "t": "prefs", "panes": {} }));
    }
}

struct Fake {
    origin: String,
    conns: mpsc::UnboundedReceiver<Conn>,
}

impl Fake {
    async fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let origin = format!("http://{}", listener.local_addr().expect("an address"));
        let (conns, incoming) = mpsc::unbounded_channel();
        tokio::spawn(async move {
            while let Ok((stream, _)) = listener.accept().await {
                let conns = conns.clone();
                tokio::spawn(async move {
                    // The node echoes the token subprotocol back verbatim, and a client that is
                    // offered nothing back has no way to tell a node from anything else on the
                    // port. The fake does what the node does.
                    let mut offered = None;
                    let accepted = tokio_tungstenite::accept_hdr_async(
                        stream,
                        #[allow(clippy::result_large_err)]
                        |request: &Request, mut response: Response| {
                            offered = request.headers().get("sec-websocket-protocol").cloned();
                            if let Some(protocol) = offered.clone() {
                                response.headers_mut().insert("sec-websocket-protocol", protocol);
                            }
                            Ok(response)
                        },
                    )
                    .await;
                    let Ok(socket) = accepted else { return };
                    let (mut sink, mut source) = socket.split();
                    let (to_client, mut outbox) = mpsc::unbounded_channel::<Message>();
                    let (inbox, from_client) = mpsc::unbounded_channel::<String>();
                    if conns
                        .send(Conn {
                            to_client,
                            from_client,
                        })
                        .is_err()
                    {
                        return;
                    }
                    tokio::spawn(async move {
                        while let Some(message) = outbox.recv().await {
                            if sink.send(message).await.is_err() {
                                return;
                            }
                        }
                    });
                    while let Some(Ok(message)) = source.next().await {
                        if let Message::Text(text) = message
                            && inbox.send(text.to_string()).is_err()
                        {
                            return;
                        }
                    }
                });
            }
        });
        Self {
            origin,
            conns: incoming,
        }
    }

    async fn accept(&mut self) -> Conn {
        tokio::time::timeout(BEAT, self.conns.recv())
            .await
            .expect("no client dialled")
            .expect("the listener died")
    }

    fn client(&self) -> Client {
        Client::with_policy(
            Session {
                origin: self.origin.clone(),
                token: "scripted-token".into(),
                via: Via::Profile {
                    name: "scripted".into(),
                },
            },
            Policy {
                backoff: Backoff {
                    initial: Duration::from_millis(10),
                    max: Duration::from_millis(50),
                },
                connect_timeout: Duration::from_secs(2),
                manage_timeout: Duration::from_millis(500),
                event_capacity: 256,
            },
        )
    }
}

async fn next(events: &mut tokio::sync::broadcast::Receiver<Event>) -> Event {
    tokio::time::timeout(BEAT, events.recv())
        .await
        .expect("no event arrived")
        .expect("the event stream ended")
}

/// Waits for the event `want` matches, failing rather than hanging if something else arrives.
async fn until<T>(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    want: impl Fn(Event) -> Option<T>,
) -> T {
    for _ in 0..64 {
        if let Some(found) = want(next(events).await) {
            return found;
        }
    }
    panic!("the event never arrived");
}

fn node(id: &str, name: &str) -> Value {
    json!({ "id": id, "name": name, "kind": "local", "online": true })
}

fn pane(id: &str, node_id: &str) -> Value {
    json!({ "id": id, "node_id": node_id, "rows": 4, "agent_status": "idle" })
}

fn run(style: u32, text: &str, link: Option<u32>) -> Value {
    match link {
        Some(l) => json!({ "s": style, "x": text, "l": l }),
        None => json!({ "s": style, "x": text }),
    }
}

#[tokio::test]
async fn the_greeting_is_three_frames_and_the_first_prefs_is_not_a_write_ack() {
    let mut fake = Fake::start().await;
    let client = fake.client();
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(json!([node("01JNODE", "scripted")]), json!([]));

    assert!(matches!(next(&mut events).await, Event::Connected(_)));
    assert!(matches!(next(&mut events).await, Event::Herd));
    assert!(
        matches!(next(&mut events).await, Event::Prefs { greeting: true }),
        "the third greeting frame arrives unasked and is not the answer to a write"
    );

    assert!(client.write_prefs("01JNODE/w1:p1", json!({ "zoom": 1.6 })));
    assert_eq!(conn.recv().await["t"], "prefs");
    conn.send(json!({ "t": "prefs", "panes": { "01JNODE/w1:p1": { "zoom": 1.6 } } }));
    assert!(
        matches!(next(&mut events).await, Event::Prefs { greeting: false }),
        "a prefs frame after the greeting is the answer to a write"
    );
    assert_eq!(client.state().prefs["01JNODE/w1:p1"]["zoom"], json!(1.6));
}

#[tokio::test]
async fn an_unknown_t_is_ignored_rather_than_an_error() {
    let mut fake = Fake::start().await;
    let client = fake.client();
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([node("01JNODE", "scripted")]), json!([]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;

    conn.send(json!({ "t": "weather.report", "pane": "01JNODE/w1:p1", "sky": "clear" }));
    conn.send(json!({ "t": "herd.patch", "added": { "panes": [pane("01JNODE/w1:p1", "01JNODE")] } }));

    match next(&mut events).await {
        Event::Herd => {}
        other => panic!("an unknown `t` produced {other:?} instead of being ignored"),
    }
    assert_eq!(client.state().herd.panes.len(), 1, "the socket carried on");
}

#[tokio::test]
async fn an_unrecognised_error_code_renders_its_message() {
    let mut fake = Fake::start().await;
    let client = fake.client();
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(json!([node("01JNODE", "scripted")]), json!([]));

    // A hub forwards a peer's codes verbatim, so a newer peer's code reaches a client that has
    // never heard of it — and the message beside it is the whole diagnosis.
    conn.send(json!({
        "t": "error", "code": "the_flux_capacitor_is_unavailable",
        "message": "this pane needs 1.21 gigawatts", "pane": "01JNODE/w1:p1"
    }));
    let failure = until(&mut events, |e| match e {
        Event::Error(f) => Some(f),
        _ => None,
    })
    .await;
    assert_eq!(failure.code, "the_flux_capacitor_is_unavailable");
    assert_eq!(failure.message, "this pane needs 1.21 gigawatts");
    assert_eq!(failure.pane.as_deref(), Some("01JNODE/w1:p1"));

    // And the connection is unharmed by a code it could not name.
    conn.send(json!({ "t": "pong", "n": 9 }));
    assert!(matches!(next(&mut events).await, Event::Pong { n: 9 }));
}

#[tokio::test]
async fn role_changes_the_role_without_throwing_the_herd_away() {
    let mut fake = Fake::start().await;
    let client = fake.client();
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "scripted"), node("01JPEER", "laptop")]),
        json!([pane("01JNODE/w1:p1", "01JNODE"), pane("01JPEER/w1:p1", "01JPEER")]),
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;
    assert_eq!(client.state().role, Role::Full);

    conn.send(json!({ "t": "role", "role": "readonly" }));
    let role = until(&mut events, |e| match e {
        Event::Role(role) => Some(role),
        _ => None,
    })
    .await;

    assert_eq!(role, Role::Readonly);
    let state = client.state();
    assert_eq!(state.role, Role::Readonly);
    assert!(!state.role.writes(), "write affordances gate on this");
    assert_eq!(
        state.herd.nodes.len(),
        2,
        "a permission change is not a second greeting and must not cost the herd"
    );
    assert_eq!(state.herd.panes.len(), 2);
    assert!(state.hello.is_some(), "nor the greeting it arrived with");
}

#[tokio::test]
async fn a_herd_patch_removes_a_node_and_not_only_a_pane() {
    let mut fake = Fake::start().await;
    let client = fake.client();
    let mut events = client.events();
    let conn = fake.accept().await;
    conn.greet(
        json!([node("01JNODE", "scripted"), node("01JPEER", "laptop")]),
        json!([pane("01JNODE/w1:p1", "01JNODE"), pane("01JPEER/w1:p1", "01JPEER")]),
    );
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;

    conn.send(json!({ "t": "herd.patch", "removed_ids": ["01JPEER"] }));
    until(&mut events, |e| matches!(e, Event::Herd).then_some(())).await;

    let state = client.state();
    assert!(
        state.herd.node("01JPEER").is_none(),
        "a patch carries nodes as well as panes, or an outage is invisible"
    );
    assert_eq!(state.herd.nodes.len(), 1);
    assert!(
        state.herd.pane("01JPEER/w1:p1").is_none(),
        "a node that left takes its panes with it"
    );
    assert!(state.herd.pane("01JNODE/w1:p1").is_some());
}

#[tokio::test]
async fn links_replace_on_a_reset_and_append_on_a_patch() {
    let mut fake = Fake::start().await;
    let client = fake.client();
    let mut events = client.events();
    let conn = fake.accept().await;
    let id = "01JNODE/w1:p1";
    conn.greet(json!([node("01JNODE", "scripted")]), json!([pane(id, "01JNODE")]));
    until(&mut events, |e| matches!(e, Event::Prefs { .. }).then_some(())).await;

    conn.send(json!({
        "t": "grid.reset", "pane": id, "cols": 8, "rows": 1,
        "rows_data": [ { "row": 0, "runs": [ run(0, "one", Some(0)) ] } ],
        "cursor": { "col": 0, "row": 0, "visible": true },
        "links": ["https://one.example", "https://two.example"]
    }));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    assert_eq!(
        client.state().pane(id).unwrap().links(),
        [
            "https://one.example".to_string(),
            "https://two.example".to_string()
        ]
    );

    // A patch carries only the entries discovered since the last message.
    conn.send(json!({
        "t": "grid.patch", "pane": id,
        "rows": [ { "row": 0, "runs": [ run(0, "two", Some(2)) ] } ],
        "cursor": { "col": 3, "row": 0, "visible": true },
        "links": ["https://three.example"]
    }));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    {
        let state = client.state();
        let pane = state.pane(id).unwrap();
        assert_eq!(pane.links().len(), 3, "a patch appends to the table");
        assert_eq!(pane.link(2), Some("https://three.example"));
        assert_eq!(pane.link(0), Some("https://one.example"));
    }

    // A reset carries the whole table. Appending it instead puts every later id out by the
    // length of the previous one, which resolves a link to the WRONG URL rather than failing.
    conn.send(json!({
        "t": "grid.reset", "pane": id, "cols": 8, "rows": 1,
        "rows_data": [ { "row": 0, "runs": [ run(0, "new", Some(0)) ] } ],
        "cursor": { "col": 0, "row": 0, "visible": true },
        "links": ["https://after-the-reset.example"]
    }));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let state = client.state();
    let pane = state.pane(id).unwrap();
    assert_eq!(pane.links().len(), 1, "a reset replaces the table");
    assert_eq!(pane.link(0), Some("https://after-the-reset.example"));
}

#[tokio::test]
async fn a_reconnect_keeps_the_cached_grid_marked_stale_and_swaps_on_the_reset() {
    let mut fake = Fake::start().await;
    let client = fake.client();
    let mut events = client.events();
    let id = "01JNODE/w1:p1";
    let mut conn = fake.accept().await;
    conn.greet(json!([node("01JNODE", "scripted")]), json!([pane(id, "01JNODE")]));
    until(&mut events, |e| matches!(e, Event::Connected(_)).then_some(())).await;
    assert!(client.watch(id, false, false));
    assert_eq!(conn.recv().await["t"], "watch");
    conn.send(json!({
        "t": "grid.reset", "pane": id, "cols": 5, "rows": 1,
        "rows_data": [ { "row": 0, "runs": [ run(0, "first", None) ] } ],
        "cursor": { "col": 0, "row": 0, "visible": true }, "links": []
    }));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    assert_eq!(text(&client.state(), id), "first");

    conn.close();
    until(&mut events, |e| {
        matches!(e, Event::Disconnected { .. }).then_some(())
    })
    .await;
    {
        let state = client.state();
        let pane = state.pane(id).unwrap();
        assert!(pane.stale(), "what is on screen is no longer known to be true");
        assert!(state.herd.stale);
        assert_eq!(
            pane.rows()[0].iter().map(|c| c.ch).collect::<String>(),
            "first",
            "the cached grid is kept — there is no spinner and nothing to drain"
        );
    }

    // The watch is re-issued on the new socket without the caller asking again.
    let mut second = fake.accept().await;
    assert_eq!(second.recv().await["t"], "watch");
    second.greet(json!([node("01JNODE", "scripted")]), json!([pane(id, "01JNODE")]));
    second.send(json!({
        "t": "grid.reset", "pane": id, "cols": 6, "rows": 1,
        "rows_data": [ { "row": 0, "runs": [ run(0, "second", None) ] } ],
        "cursor": { "col": 0, "row": 0, "visible": true }, "links": []
    }));
    until(&mut events, |e| matches!(e, Event::Grid { .. }).then_some(())).await;
    let state = client.state();
    assert_eq!(text(&state, id), "second");
    assert!(!state.pane(id).unwrap().stale(), "the reset is the swap");
}

fn text(state: &kampr_client::State, id: &str) -> String {
    state
        .pane(id)
        .map(|p| {
            p.rows()
                .iter()
                .map(|row| row.iter().map(|c| c.ch).collect::<String>())
                .collect::<Vec<_>>()
                .join("\n")
                .trim_end()
                .to_string()
        })
        .unwrap_or_default()
}

#[tokio::test]
async fn a_manage_op_is_answered_on_its_own_rid_and_a_refusal_is_not_a_success() {
    let mut fake = Fake::start().await;
    let client = fake.client();
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(json!([node("01JNODE", "scripted")]), json!([]));
    until(&mut events, |e| matches!(e, Event::Connected(_)).then_some(())).await;

    let client = std::sync::Arc::new(client);
    let one = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .manage(json!({ "op": "tab.create", "at": "01JNODE/w3" }))
                .await
        })
    };
    let first = conn.recv().await;
    assert_eq!(first["t"], "manage");
    let rid = first["rid"].clone();
    assert!(!rid.is_null(), "several ops will be in flight from a keyboard");

    // An ack for something nobody is waiting on must not resolve this one.
    conn.send(json!({ "t": "managed", "op": "tab.create", "ok": true, "rid": 9999 }));
    conn.send(json!({ "t": "managed", "op": "tab.create", "ok": true, "rid": rid, "id": "01JNODE/w3:t2" }));
    let ack = one.await.expect("the task").expect("the ack");
    assert_eq!(ack.id.as_deref(), Some("01JNODE/w3:t2"));

    let two = {
        let client = client.clone();
        tokio::spawn(async move {
            client
                .manage(json!({ "op": "rename", "at": "01JNODE/w1:p1" }))
                .await
        })
    };
    let second = conn.recv().await;
    conn.send(json!({
        "t": "managed", "op": "rename", "ok": false, "rid": second["rid"].clone(),
        "code": "not_writer", "message": "this device is read-only"
    }));
    let refused = two
        .await
        .expect("the task")
        .expect_err("a refusal is not a success");
    assert!(
        matches!(&refused, kampr_client::ManageError::Refused { code, .. } if code == "not_writer"),
        "got {refused:?}",
    );
}

#[tokio::test]
async fn caps_names_the_agent_kinds_the_node_has_rather_than_a_list_a_client_compiled_in() {
    let mut fake = Fake::start().await;
    let client = fake.client();
    let mut events = client.events();
    let mut conn = fake.accept().await;
    conn.greet(json!([node("01JNODE", "scripted")]), json!([]));
    until(&mut events, |e| matches!(e, Event::Connected(_)).then_some(())).await;

    assert!(client.request_caps());
    assert_eq!(conn.recv().await["t"], "caps");
    conn.send(json!({
        "t": "caps", "node": "01JNODE",
        "agent_kinds": ["claude", "codex"],
        "sessions": [ { "name": "default", "running": true, "served": true },
                      { "name": "agents", "running": true, "served": false } ]
    }));
    let caps = until(&mut events, |e| match e {
        Event::Caps(caps) => Some(caps),
        _ => None,
    })
    .await;
    assert_eq!(caps.agent_kinds, ["claude".to_string(), "codex".to_string()]);
    assert_eq!(caps.sessions.len(), 2);
    assert!(
        !caps.sessions[1].served,
        "a session that is running and unserved never appears in the herd, and a client must \
         not offer to open a pane on one"
    );
}
