//! `GET /api/attachment/{pane}/{id}` — who may ask, and what the node refuses to hand over.
//!
//! No herd is needed. The route is two halves: an authorised caller, and an id resolved against
//! the transcript *the node* says the pane is on. The first half is driven over a real socket
//! against a node whose herdr socket deliberately does not exist; the second is driven against a
//! real transcript on disk, because that is the half a hostile id attacks.

use axum::body::to_bytes;
use axum::http::StatusCode;
use axum::response::Response;
use kampr_auth::Role;
use kampr_journal::attach::{self, Locator};
use kampr_journal::{Block, JournalAdapter, Registry, TranscriptRoot};
use kampr_node::{Config, Node, attach as node_attach, http};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::net::TcpStream;

const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn pasted(mime: &str, data: &str) -> Value {
    untyped(json!({ "type": "base64", "media_type": mime, "data": data }))
}

/// A source with no `media_type` at all — the field the wire's `mime` is absent for.
fn untyped(source: Value) -> Value {
    json!({
        "type": "user",
        "uuid": "549c13ed-c2b4-4013-b072-f26304a5bb6c",
        "timestamp": "2026-08-20T02:56:27.681Z",
        "message": { "role": "user", "content": [
            { "type": "text", "text": "look" },
            { "type": "image", "source": source }
        ] }
    })
}

/// A transcript with one image in it, inside a root of its own, plus the id the wire would carry.
struct Fixture {
    home: tempfile::TempDir,
    transcript: PathBuf,
    journals: Registry,
    id: String,
}

impl Fixture {
    fn new(record: Value) -> Self {
        let home = tempfile::tempdir().expect("a home");
        let root = home.path().join("claude");
        let dir = root.join("projects/-home-u-demo");
        std::fs::create_dir_all(&dir).expect("a project directory");
        let transcript = dir.join("session.jsonl");
        std::fs::write(&transcript, record.to_string() + "\n").expect("a transcript");

        let adapter = Arc::new(kampr_journal::ClaudeAdapter::new(
            TranscriptRoot::new(&root).expect("a root"),
        ));
        let mut journal = adapter.open_path(transcript.clone());
        let turns = journal.poll().expect("poll");
        let id = turns
            .iter()
            .flat_map(|t| &t.blocks)
            .find_map(|b| match b {
                Block::Md { att: Some(att), .. } => Some(att.id.clone()),
                _ => None,
            })
            .expect("an attachment header on the wire");
        let mut journals = Registry::new();
        journals.register(adapter);
        Self {
            home,
            transcript,
            journals,
            id,
        }
    }

    fn png() -> Self {
        Self::new(pasted("image/png", PNG))
    }

    fn root(&self) -> PathBuf {
        self.home.path().join("claude")
    }

    fn serve(&self, id: &str) -> Response {
        node_attach::serve(&self.journals, &self.transcript, id)
    }

    fn locator(&self) -> Locator {
        Locator::decode(&self.id).expect("our own id decodes")
    }

    fn serve_from(&self, path: &str) -> Response {
        let mut locator = self.locator();
        locator.path = path.to_string();
        self.serve(&locator.encode())
    }
}

fn header(response: &Response, name: &str) -> String {
    response
        .headers()
        .get(name)
        .map(|v| v.to_str().expect("a printable header").to_string())
        .unwrap_or_default()
}

async fn bytes(response: Response) -> Vec<u8> {
    to_bytes(response.into_body(), 16 * 1024 * 1024)
        .await
        .expect("a body")
        .to_vec()
}

#[tokio::test]
async fn an_image_comes_back_inline_with_its_own_type_and_length() {
    let fixture = Fixture::png();
    let response = fixture.serve(&fixture.id);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "image/png");
    assert_eq!(header(&response, "content-length"), "70");
    assert_eq!(header(&response, "content-disposition"), "inline");
    assert_eq!(bytes(response).await.len(), 70);
}

/// The recorded media type is a string an agent wrote into a file, so it decides what the node
/// *shows*, never what the node *is*. A transcript claiming `text/html` would otherwise be a
/// document served from this origin.
#[tokio::test]
async fn a_recorded_media_type_the_node_will_not_render_is_a_download() {
    for hostile in ["text/html", "image/svg+xml", "application/javascript"] {
        let fixture = Fixture::new(pasted(hostile, PNG));
        let response = fixture.serve(&fixture.id);

        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            header(&response, "content-type"),
            "application/octet-stream",
            "{hostile} must not be echoed back as the type of a response from this origin"
        );
        assert!(
            header(&response, "content-disposition").starts_with("attachment; filename="),
            "{hostile}"
        );
    }
}

#[tokio::test]
async fn an_id_naming_a_path_outside_the_root_is_refused() {
    let fixture = Fixture::png();
    let outside = fixture.home.path().join("secret.jsonl");
    std::fs::write(&outside, "{}\n").expect("a file outside the root");
    std::os::unix::fs::symlink(&outside, fixture.root().join("link.jsonl")).expect("a symlink");

    for escape in [
        "/etc/passwd",
        "../secret.jsonl",
        "projects/../../secret.jsonl",
        "link.jsonl",
    ] {
        assert_eq!(
            fixture.serve_from(escape).status(),
            StatusCode::NOT_FOUND,
            "{escape} must not be readable through an attachment id"
        );
    }
}

/// Inside the root, readable, and not this pane's. Containment alone would hand it over, which is
/// why the resolved file also has to be the one the node says this pane is on.
#[tokio::test]
async fn an_id_for_another_panes_transcript_is_refused() {
    let fixture = Fixture::png();
    let theirs = fixture.root().join("projects/-home-u-secret/session.jsonl");
    std::fs::create_dir_all(theirs.parent().expect("a directory")).expect("a directory");
    std::fs::write(&theirs, pasted("image/png", PNG).to_string() + "\n").expect("a transcript");

    assert_eq!(
        fixture
            .serve_from("projects/-home-u-secret/session.jsonl")
            .status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_forged_or_stale_id_is_refused() {
    let fixture = Fixture::png();
    for forged in ["", "..", "%2e%2e", "AAAA", "not an id"] {
        assert_eq!(
            fixture.serve(forged).status(),
            StatusCode::NOT_FOUND,
            "{forged:?}"
        );
    }
    let mut past_the_end = fixture.locator();
    past_the_end.index = 9;
    assert_eq!(
        fixture.serve(&past_the_end.encode()).status(),
        StatusCode::NOT_FOUND
    );
    let mut mid_record = fixture.locator();
    mid_record.offset += 30;
    assert_eq!(
        fixture.serve(&mid_record.encode()).status(),
        StatusCode::NOT_FOUND
    );
}

#[tokio::test]
async fn a_body_past_the_ceiling_is_refused_with_a_status_that_says_so() {
    let over = "A".repeat((attach::MAX_BYTES as usize + 1).div_ceil(3) * 4);
    let fixture = Fixture::new(pasted("image/png", &over));

    assert_eq!(fixture.serve(&fixture.id).status(), StatusCode::PAYLOAD_TOO_LARGE);
}

/// A screenshot pasted with no media type recorded beside it. The `Content-Type` is the only
/// thing left for a client to name the saved file from, so it is answered from the bytes rather
/// than given up on — and sniffing can only ever produce a type the node already renders.
#[tokio::test]
async fn an_image_with_no_recorded_media_type_is_answered_from_its_own_bytes() {
    let fixture = Fixture::new(untyped(json!({ "type": "base64", "data": PNG })));
    let response = fixture.serve(&fixture.id);

    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(header(&response, "content-type"), "image/png");
    assert_eq!(header(&response, "content-disposition"), "inline");
}

/// A client shows an empty `200` to its operator as "the node answered with no bytes at all",
/// which is a true sentence about a broken route. A record with nothing in it is a refusal.
#[tokio::test]
async fn a_record_with_no_bytes_in_it_is_a_refusal_and_never_an_empty_two_hundred() {
    let fixture = Fixture::new(pasted("image/png", ""));

    assert_eq!(fixture.serve(&fixture.id).status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn every_answer_this_route_gives_carries_a_body() {
    let fixture = Fixture::png();
    for id in [fixture.id.clone(), "forged".to_string()] {
        let response = fixture.serve(&id);
        let status = response.status();
        assert!(
            !bytes(response).await.is_empty(),
            "{status} came back with nothing in it"
        );
    }
}

struct Harness {
    node: Arc<Node>,
    origin: String,
    _home: tempfile::TempDir,
    server: tokio::task::JoinHandle<()>,
}

impl Drop for Harness {
    fn drop(&mut self) {
        self.node.shutdown();
        self.server.abort();
    }
}

impl Harness {
    async fn start() -> Self {
        let home = tempfile::tempdir().expect("a home");
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("a port");
        let port = listener.local_addr().expect("an address").port();
        let config_dir = home.path().join("config");
        let state_dir = home.path().join("state");
        std::fs::create_dir_all(&state_dir).expect("a state dir");

        let mut config = Config::bootstrap("attachment");
        config.update.check = false;
        config.config_dir = config_dir.display().to_string();
        config.state_dir = state_dir.display().to_string();
        config.server.bind = format!("127.0.0.1:{port}");
        config.server.origin = format!("http://127.0.0.1:{port}");
        config.herdr.socket = home.path().join("herdr.sock").display().to_string();
        config.herdr.binary = home.path().join("no-such-herdr").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        config.save(&config_dir).expect("a config");

        let origin = config.origin();
        let node = Node::start(config, &state_dir).await.expect("a node");
        let server = tokio::spawn({
            let app = http::router(node.clone());
            async move {
                let _ = http::serve_on(listener, app).await;
            }
        });
        Self {
            node,
            origin,
            _home: home,
            server,
        }
    }

    async fn token(&self) -> String {
        let pairing = self
            .node
            .auth
            .create_pairing(Role::Readonly, kampr_auth::Delivery::Console)
            .await
            .expect("a pairing");
        if !pairing.armed {
            assert!(self.node.auth.arm_pairing(&pairing.code).await.expect("armed"));
        }
        let body = json!({ "code": pairing.code, "device_name": "attachment" });
        let (_, body) = request(&self.origin, "POST", "/auth/pair", &[], Some(&body.to_string())).await;
        serde_json::from_str::<Value>(body.trim()).expect("json")["token"]
            .as_str()
            .expect("a token")
            .to_string()
    }

    async fn get(&self, path: &str, headers: &[(&str, &str)]) -> String {
        request(&self.origin, "GET", path, headers, None).await.0
    }
}

async fn request(
    origin: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: Option<&str>,
) -> (String, String) {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let rest = origin.trim_start_matches("http://");
    let (host, port) = rest.split_once(':').expect("host:port");
    let port: u16 = port.parse().expect("a port");
    let extra: String = headers
        .iter()
        .map(|(name, value)| format!("{name}: {value}\r\n"))
        .collect();
    let payload = body.unwrap_or_default();
    let request = format!(
        "{method} {path} HTTP/1.1\r\nHost: {host}:{port}\r\nContent-Type: application/json\r\n\
         {extra}Content-Length: {}\r\nConnection: close\r\n\r\n{payload}",
        payload.len()
    );
    let mut stream = TcpStream::connect((host, port)).await.expect("connect");
    stream.write_all(request.as_bytes()).await.expect("write");
    let mut response = Vec::new();
    stream.read_to_end(&mut response).await.expect("read");
    let text = String::from_utf8_lossy(&response).to_string();
    let status = text.lines().next().unwrap_or_default().to_string();
    let body = text.split_once("\r\n\r\n").map(|(_, b)| b).unwrap_or_default();
    (status, body.to_string())
}

/// The route is behind the same bearer check as every other `/api/*` surface, and there is no
/// second way in: an attachment is transcript content, and a stranger who could name a pane must
/// not be able to read one.
#[tokio::test(flavor = "multi_thread")]
async fn a_caller_with_no_token_is_refused_before_a_pane_is_even_looked_for() {
    let h = Harness::start().await;

    assert!(
        h.get("/api/attachment/01J/w1:p1/anything", &[])
            .await
            .contains("401"),
        "an attachment must not be reachable without a device token"
    );
    assert!(
        h.get(
            "/api/attachment/01J/w1:p1/anything",
            &[("Authorization", "Bearer not-a-token")]
        )
        .await
        .contains("401")
    );
}

/// The token here is a **read-only** device's, and it must get as far as the pane lookup: looking
/// at a screenshot somebody pasted into an agent session is reading, so a 403 would be wrong.
#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_device_naming_a_pane_this_node_does_not_serve_gets_nothing() {
    let h = Harness::start().await;
    let token = h.token().await;
    let auth = format!("Bearer {token}");

    for path in [
        "/api/attachment/01JNODE/w3:p2/att-7f3",
        "/api/attachment/01JNODE/w3:p2/",
        "/api/attachment/01JNODE/w3:p2/att-7f3/extra",
        "/api/attachment/anything",
    ] {
        let status = h.get(path, &[("Authorization", auth.as_str())]).await;
        assert!(status.contains("404"), "{path} answered {status}");
        assert!(
            status.trim_start_matches("HTTP/1.1 404").trim().len() > 3,
            "a client renders an unrecognised code as `<code> <reason>`, so the reason has to be \
             there: {status:?}"
        );
    }
}
