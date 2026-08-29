//! Push, proved against a stub push service rather than a mock sender.
//!
//! The stub is a real HTTP server that speaks RFC 8030: it takes the POST, keeps the headers and
//! the encrypted body, and answers 201 — or 410 when the test wants an endpoint that has gone
//! away. What it cannot do is decrypt, so the assertions here are about *which* device gets *how
//! many* notifications and what the VAPID envelope looks like. That the ciphertext decrypts to the
//! right JSON is a browser's job, and it is proved by hand against a real browser.

use axum::Router;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::any;
use kampr_auth::{PushRule, Role, Store};
use kampr_node::push::Push;
use kampr_push::{Blocked, Change, Reach, Vapid};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
struct Received {
    path: String,
    authorization: String,
    content_encoding: String,
    authorizations: usize,
    ttl: String,
    urgency: String,
    body_len: usize,
}

#[derive(Default)]
struct Service {
    received: Mutex<Vec<Received>>,
    /// Paths that answer 410 Gone, as a push service does for a subscription the browser dropped.
    gone: Mutex<Vec<String>>,
    /// Where this service answers 302 to, as an attacker-owned endpoint does when it wants the
    /// node to make the *next* request from inside its own network.
    redirect_to: Mutex<Option<String>>,
}

async fn accept(
    State(service): State<Arc<Service>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    let path = format!("/push/{id}");
    if service.gone.lock().unwrap().contains(&path) {
        return StatusCode::GONE.into_response();
    }
    let header = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_string()
    };
    service.received.lock().unwrap().push(Received {
        path,
        authorization: header("authorization"),
        authorizations: headers.get_all("authorization").iter().count(),
        content_encoding: header("content-encoding"),
        ttl: header("ttl"),
        urgency: header("urgency"),
        body_len: body.len(),
    });
    if let Some(target) = service.redirect_to.lock().unwrap().clone() {
        return (StatusCode::FOUND, [("location", target)]).into_response();
    }
    StatusCode::CREATED.into_response()
}

struct Stub {
    service: Arc<Service>,
    base: String,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Stub {
    fn drop(&mut self) {
        self.server.abort();
    }
}

impl Stub {
    async fn start() -> Self {
        let service = Arc::new(Service::default());
        // `any`, not `post`: a 302 turns the node's POST into a GET, so a stub that only routes
        // POST answers 405 and records nothing — and a test asserting "nothing arrived" would
        // pass with the redirect policy removed.
        let app = Router::new()
            .route("/push/{id}", any(accept))
            .with_state(service.clone());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Self {
            service,
            // Plain HTTP because the stub is not a TLS server. The `https` rule lives on the
            // subscribe endpoint, where a client supplies the URL; the sender posts to whatever
            // the store already accepted.
            base: format!("http://127.0.0.1:{port}"),
            server,
        }
    }

    fn received(&self) -> Vec<Received> {
        self.service.received.lock().unwrap().clone()
    }
}

/// A subscription's keys have to be real or the encryption step refuses them, so they are minted
/// the same way a browser mints them: a fresh P-256 keypair and 16 bytes of auth secret.
fn browser_keys() -> (String, String) {
    use base64::Engine;
    use base64::engine::general_purpose::URL_SAFE_NO_PAD;
    let dir = tempfile::tempdir().unwrap();
    let key = Vapid::load_or_create(&dir.path().join("k.pem"), "mailto:x@y").unwrap();
    (key.public_key_b64(), URL_SAFE_NO_PAD.encode(rand_bytes()))
}

fn rand_bytes() -> [u8; 16] {
    let mut out = [0u8; 16];
    getrandom::fill(&mut out).expect("entropy");
    out
}

/// The stub is a loopback server, which is the one address a real push endpoint may never be —
/// so every test that wants delivery to happen at all asks for [`Reach::Loopback`] explicitly.
async fn fixture() -> (Store, Arc<Push>, tempfile::TempDir) {
    reaching(Reach::Loopback).await
}

async fn reaching(reach: Reach) -> (Store, Arc<Push>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_memory().await.unwrap();
    let vapid = Arc::new(Vapid::load_or_create(&dir.path().join("vapid.pem"), "mailto:x@y").unwrap());
    (store, Arc::new(Push::new(vapid, reach).expect("a sender")), dir)
}

async fn enrol(store: &Store, name: &str, endpoint: &str) -> String {
    let device = store
        .create_device(name, Role::Full, kampr_auth::now(), None, None, None)
        .await
        .unwrap();
    let (p256dh, auth) = browser_keys();
    store
        .save_push_subscription(&device.id, "webpush", endpoint, &p256dh, &auth, kampr_auth::now())
        .await
        .unwrap();
    device.id
}

fn blocked(pane: &str, agent: &str, question: &str) -> Blocked {
    Blocked {
        pane: format!("01J/{pane}"),
        node: "01J".into(),
        agent: Some(agent.into()),
        label: Some("kampr".into()),
        question: Some(question.into()),
    }
}

/// The batching rule, end to end: two panes blocking together are **one** POST per device, not
/// two. Three notifications racing at a phone is how the feature gets turned off.
#[tokio::test]
async fn two_panes_blocking_together_are_one_push_per_device() {
    let stub = Stub::start().await;
    let (store, push, _dir) = fixture().await;
    enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;
    enrol(&store, "laptop", &format!("{}/push/laptop", stub.base)).await;

    let sent = push
        .deliver(
            &store,
            &Change::fresh(vec![
                blocked("w1:p1", "claude", "Run the tests?"),
                blocked("w2:p1", "codex", "Apply the patch?"),
            ]),
        )
        .await;

    assert_eq!(sent, 2, "one notification each, not one per pane");
    let received = stub.received();
    assert_eq!(received.len(), 2);
    let mut paths: Vec<&str> = received.iter().map(|r| r.path.as_str()).collect();
    paths.sort();
    assert_eq!(paths, ["/push/laptop", "/push/phone"]);
}

/// Every POST has to carry the VAPID envelope a push service checks, and the aes128gcm encoding.
/// Getting either wrong is a 400 from the service and a notification that never arrives.
#[tokio::test]
async fn a_push_carries_a_vapid_authorization_and_an_encrypted_body() {
    let stub = Stub::start().await;
    let (store, push, _dir) = fixture().await;
    enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;

    push.deliver(
        &store,
        &Change::fresh(vec![blocked("w1:p1", "claude", "Run the tests?")]),
    )
    .await;

    let received = stub.received();
    assert_eq!(received.len(), 1);
    let one = &received[0];
    assert!(
        one.authorization.starts_with("vapid t=") && one.authorization.contains(", k="),
        "{}",
        one.authorization
    );
    assert_eq!(
        one.authorizations, 1,
        "the VAPID header rides in web-push's own crypto_headers; adding a second one is a bare \
         nginx 400 from the edge in front of the push service, with nothing in it that says why"
    );
    assert_eq!(one.content_encoding, "aes128gcm");
    assert!(!one.ttl.is_empty(), "a push service needs a TTL");
    assert!(
        one.body_len > 100,
        "the body is the encrypted payload, not an empty ping: {}",
        one.body_len
    );
}

/// Mute is per agent and per device. Muting one agent on a phone must leave the phone hearing
/// about the others, and must not touch anybody else.
#[tokio::test]
async fn a_muted_agent_drops_out_of_that_devices_notification_only() {
    let stub = Stub::start().await;
    let (store, push, _dir) = fixture().await;
    let phone = enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;
    enrol(&store, "laptop", &format!("{}/push/laptop", stub.base)).await;
    store
        .set_push_rule(
            &phone,
            &PushRule {
                pane_id: "01J/w1:p1".into(),
                muted: true,
                snooze_until: None,
            },
            kampr_auth::now(),
        )
        .await
        .unwrap();

    let sent = push
        .deliver(
            &store,
            &Change::fresh(vec![
                blocked("w1:p1", "claude", "Run the tests?"),
                blocked("w2:p1", "codex", "Apply the patch?"),
            ]),
        )
        .await;
    assert_eq!(sent, 2, "the phone still hears about the agent it did not mute");
    assert_eq!(stub.received().len(), 2);

    // And a mute that covers the whole batch means no POST at all for that device.
    store
        .set_push_rule(
            &phone,
            &PushRule {
                pane_id: kampr_auth::ALL_PANES.into(),
                muted: true,
                snooze_until: None,
            },
            kampr_auth::now(),
        )
        .await
        .unwrap();
    push.deliver(&store, &Change::fresh(vec![blocked("w2:p1", "codex", "Again?")]))
        .await;
    let paths: Vec<String> = stub.received().into_iter().map(|r| r.path).collect();
    assert_eq!(
        paths.iter().filter(|p| *p == "/push/phone").count(),
        1,
        "a muted device receives nothing further"
    );
}

/// A revoked device must stop being woken, and it must stop at the database rather than at a
/// filter somebody has to remember to apply.
#[tokio::test]
async fn revoking_a_device_stops_its_notifications() {
    let stub = Stub::start().await;
    let (store, push, _dir) = fixture().await;
    let phone = enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;

    assert_eq!(
        push.deliver(
            &store,
            &Change::fresh(vec![blocked("w1:p1", "claude", "Run the tests?")])
        )
        .await,
        1
    );
    store.revoke_device(&phone, kampr_auth::now()).await.unwrap();
    assert_eq!(
        push.deliver(
            &store,
            &Change::fresh(vec![blocked("w1:p1", "claude", "Run the tests?")])
        )
        .await,
        0
    );
    assert_eq!(stub.received().len(), 1);
}

/// 410 Gone is the push service saying the browser dropped this subscription. Keeping the row
/// would mean a POST that fails forever; the row is deleted instead.
#[tokio::test]
async fn a_gone_endpoint_is_deleted_rather_than_retried_forever() {
    let stub = Stub::start().await;
    let (store, push, _dir) = fixture().await;
    let phone = enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;
    stub.service.gone.lock().unwrap().push("/push/phone".to_string());

    assert_eq!(
        push.deliver(
            &store,
            &Change::fresh(vec![blocked("w1:p1", "claude", "Run the tests?")])
        )
        .await,
        0
    );
    assert!(
        store.push_subscriptions_for(&phone).await.unwrap().is_empty(),
        "a gone endpoint must not survive to fail again"
    );
}

/// A node that cannot push must do nothing at all rather than fail somewhere inside delivery —
/// which is the same promise `caps.push` makes to the client.
#[tokio::test]
async fn a_node_with_no_vapid_key_sends_nothing() {
    let stub = Stub::start().await;
    let store = Store::open_memory().await.unwrap();
    enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;
    let push = Push::disabled();
    assert!(!push.available());
    assert_eq!(push.public_key(), None);
    assert_eq!(
        push.deliver(
            &store,
            &Change::fresh(vec![blocked("w1:p1", "claude", "Run the tests?")])
        )
        .await,
        0
    );
    assert!(stub.received().is_empty());
}

/// The `https://` check on `push_subscribe` is a check on a *string*, and a 302 is how an
/// attacker-owned endpoint that passes it aims the node's own POST at loopback or at a link-local
/// metadata address. The push service that answers the redirect is never asked for anything.
#[tokio::test]
async fn a_push_endpoint_that_redirects_never_reaches_what_it_redirects_to() {
    let inside = Stub::start().await;
    let endpoint = Stub::start().await;
    *endpoint.service.redirect_to.lock().unwrap() = Some(format!("{}/push/inside", inside.base));

    let (store, push, _dir) = fixture().await;
    enrol(&store, "phone", &format!("{}/push/phone", endpoint.base)).await;

    let sent = push
        .deliver(
            &store,
            &Change::fresh(vec![blocked("w1:p1", "claude", "Run the tests?")]),
        )
        .await;

    assert_eq!(sent, 0, "a 302 is not a delivery");
    assert_eq!(endpoint.received().len(), 1, "the endpoint itself was asked once");
    assert!(
        inside.received().is_empty(),
        "the node followed a redirect into its own network"
    );
}

/// And the direct case the redirect was a way around: an endpoint that simply names an address
/// inside the node. Every subscribe path accepts one — a read-only device may subscribe, and the
/// only check on the endpoint is that the string starts `https://`.
#[tokio::test]
async fn a_push_endpoint_addressed_inside_this_node_is_never_dialled() {
    let stub = Stub::start().await;
    let (store, push, _dir) = reaching(Reach::Public).await;
    enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;

    let sent = push
        .deliver(
            &store,
            &Change::fresh(vec![blocked("w1:p1", "claude", "Run the tests?")]),
        )
        .await;

    assert_eq!(sent, 0);
    assert!(
        stub.received().is_empty(),
        "a loopback endpoint is not a push service"
    );
}

/// The defect end to end: a prompt answered anywhere else has to leave the phone.
///
/// The stub cannot decrypt, so what is asserted is the *second POST* — without it there is no
/// payload on the wire at all, and nothing the phone could act on.
#[tokio::test]
async fn a_pane_answered_elsewhere_sends_the_device_a_second_push_that_takes_the_prompt_down() {
    let stub = Stub::start().await;
    let (store, push, _dir) = fixture().await;
    enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;

    push.deliver(
        &store,
        &Change::fresh(vec![blocked("w1:p1", "claude", "Run the tests?")]),
    )
    .await;
    let sent = push
        .deliver(&store, &Change::cleared(Vec::new(), ["01J/w1:p1".to_string()]))
        .await;

    assert_eq!(sent, 1, "the device that was told has to be told it is over");
    let received = stub.received();
    assert_eq!(received.len(), 2);
    assert!(
        received[1].body_len > 100,
        "the clear is an encrypted payload, not a bare ping the worker cannot read: {}",
        received[1].body_len
    );
    assert_eq!(
        received[1].urgency, "normal",
        "a phone is not woken from sleep to be told there is less waiting"
    );
}

/// And the clear is addressed by the same rules as everything else. A device that muted the pane
/// never saw the prompt, so there is nothing on its screen to take down.
#[tokio::test]
async fn a_device_that_muted_the_answered_pane_is_not_woken_to_be_told_it_was_answered() {
    let stub = Stub::start().await;
    let (store, push, _dir) = fixture().await;
    let phone = enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;
    enrol(&store, "laptop", &format!("{}/push/laptop", stub.base)).await;
    store
        .set_push_rule(
            &phone,
            &PushRule {
                pane_id: "01J/w1:p1".into(),
                muted: true,
                snooze_until: None,
            },
            kampr_auth::now(),
        )
        .await
        .unwrap();

    let sent = push
        .deliver(&store, &Change::cleared(Vec::new(), ["01J/w1:p1".to_string()]))
        .await;

    assert_eq!(sent, 1);
    let paths: Vec<String> = stub.received().into_iter().map(|r| r.path).collect();
    assert_eq!(paths, ["/push/laptop"]);
}

/// A second agent blocking used to take the first one off the phone: the payload named the edge,
/// and one tag made it replace everything before it. The alert still fires, and the older block is
/// still in the payload.
#[tokio::test]
async fn a_second_agent_blocking_still_alerts_and_still_carries_the_first() {
    let stub = Stub::start().await;
    let (store, push, _dir) = fixture().await;
    enrol(&store, "phone", &format!("{}/push/phone", stub.base)).await;

    push.deliver(
        &store,
        &Change::fresh(vec![blocked("w1:p1", "claude", "Run the tests?")]),
    )
    .await;
    let change = Change {
        outstanding: vec![
            blocked("w1:p1", "claude", "Run the tests?"),
            blocked("w2:p1", "codex", "Apply the patch?"),
        ],
        fresh: ["01J/w2:p1".to_string()].into_iter().collect(),
        cleared: Default::default(),
    };
    assert_eq!(push.deliver(&store, &change).await, 1);

    let received = stub.received();
    assert_eq!(received.len(), 2);
    assert_eq!(
        received[1].urgency, "high",
        "a new agent blocking is still worth a phone waking up for"
    );
    assert!(
        received[1].body_len > received[0].body_len,
        "two panes is a longer payload than one: the first block is still named"
    );
}
