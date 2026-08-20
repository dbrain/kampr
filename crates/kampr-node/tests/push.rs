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
use axum::routing::post;
use kampr_auth::{PushRule, Role, Store};
use kampr_node::push::Push;
use kampr_push::{Blocked, Vapid};
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Default)]
struct Received {
    path: String,
    authorization: String,
    content_encoding: String,
    ttl: String,
    body_len: usize,
}

#[derive(Default)]
struct Service {
    received: Mutex<Vec<Received>>,
    /// Paths that answer 410 Gone, as a push service does for a subscription the browser dropped.
    gone: Mutex<Vec<String>>,
}

async fn accept(
    State(service): State<Arc<Service>>,
    axum::extract::Path(id): axum::extract::Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> StatusCode {
    let path = format!("/push/{id}");
    if service.gone.lock().unwrap().contains(&path) {
        return StatusCode::GONE;
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
        content_encoding: header("content-encoding"),
        ttl: header("ttl"),
        body_len: body.len(),
    });
    StatusCode::CREATED
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
        let app = Router::new()
            .route("/push/{id}", post(accept))
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
    (
        key.public_key_b64(),
        URL_SAFE_NO_PAD.encode(rand_bytes()),
    )
}

fn rand_bytes() -> [u8; 16] {
    let mut out = [0u8; 16];
    getrandom::fill(&mut out).expect("entropy");
    out
}

async fn fixture() -> (Store, Arc<Push>, tempfile::TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_memory().await.unwrap();
    let vapid = Arc::new(Vapid::load_or_create(&dir.path().join("vapid.pem"), "mailto:x@y").unwrap());
    (store, Arc::new(Push::new(vapid)), dir)
}

async fn enrol(store: &Store, name: &str, endpoint: &str) -> String {
    let device = store
        .create_device(name, Role::Full, kampr_auth::now(), None, None, None)
        .await
        .unwrap();
    let (p256dh, auth) = browser_keys();
    store
        .save_push_subscription(
            &device.id,
            "webpush",
            endpoint,
            &p256dh,
            &auth,
            kampr_auth::now(),
        )
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
            vec![
                blocked("w1:p1", "claude", "Run the tests?"),
                blocked("w2:p1", "codex", "Apply the patch?"),
            ],
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

    push.deliver(&store, vec![blocked("w1:p1", "claude", "Run the tests?")])
        .await;

    let received = stub.received();
    assert_eq!(received.len(), 1);
    let one = &received[0];
    assert!(
        one.authorization.starts_with("vapid t=") && one.authorization.contains(", k="),
        "{}",
        one.authorization
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
            vec![
                blocked("w1:p1", "claude", "Run the tests?"),
                blocked("w2:p1", "codex", "Apply the patch?"),
            ],
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
    push.deliver(&store, vec![blocked("w2:p1", "codex", "Again?")])
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
        push.deliver(&store, vec![blocked("w1:p1", "claude", "Run the tests?")])
            .await,
        1
    );
    store.revoke_device(&phone, kampr_auth::now()).await.unwrap();
    assert_eq!(
        push.deliver(&store, vec![blocked("w1:p1", "claude", "Run the tests?")])
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
    stub.service
        .gone
        .lock()
        .unwrap()
        .push("/push/phone".to_string());

    assert_eq!(
        push.deliver(&store, vec![blocked("w1:p1", "claude", "Run the tests?")])
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
        push.deliver(&store, vec![blocked("w1:p1", "claude", "Run the tests?")])
            .await,
        0
    );
    assert!(stub.received().is_empty());
}
