//! The peer's half of a relayed attachment: what a hub asks for, and what it costs the panes on
//! the same link.
//!
//! A real session, a real outbox and a real transcript on disk. No herdr is needed — the pane is
//! published into the herd model directly, which is all `transcript_of` reads — and no socket,
//! because the transport is a trait and the interesting property is about *when* bytes are
//! written rather than about who writes them.

use base64::Engine;
use kampr_auth::{Device, Role};
use kampr_core::provider::PaneInfo;
use kampr_core::wire::PaneEntry;
use kampr_journal::{Block, JournalAdapter, TranscriptRoot};
use kampr_mesh::{ATT_CHUNK_BYTES, ATT_WINDOW, Incoming, Outgoing};
use kampr_node::herd::HerdModel;
use kampr_node::session::{self, Caller};
use kampr_node::{Config, Node};
use serde_json::{Value, json};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// A link that takes as long to write a frame as a link of `bytes_per_second` would.
///
/// The whole of what a head-of-line measurement needs: an in-process channel writes instantly, so
/// nothing ever queues behind anything and there is no property left to measure.
struct Throttled {
    to_hub: mpsc::Sender<(Instant, String)>,
    bytes_per_second: usize,
}

impl Outgoing for Throttled {
    async fn send(&mut self, text: String) -> bool {
        let micros = (text.len() as u64 * 1_000_000) / self.bytes_per_second as u64;
        if micros > 0 {
            tokio::time::sleep(Duration::from_micros(micros)).await;
        }
        self.to_hub.send((Instant::now(), text)).await.is_ok()
    }

    async fn close(&mut self) {}
}

struct FromHub(mpsc::Receiver<String>);

impl Incoming for FromHub {
    async fn recv(&mut self) -> Option<String> {
        self.0.recv().await
    }
}

/// One image, in a transcript a node can find from the pane's own working directory.
struct Fixture {
    _home: tempfile::TempDir,
    _work: tempfile::TempDir,
    cwd: String,
    home: PathBuf,
    id: String,
    bytes: Vec<u8>,
}

impl Fixture {
    /// `size` bytes of a deterministic pattern, wrapped in the PNG magic so the sniffing path has
    /// something honest to find.
    fn new(size: usize) -> Self {
        let home = tempfile::tempdir().expect("a home");
        let work = tempfile::tempdir().expect("a work dir");
        let cwd = work.path().canonicalize().expect("a cwd");
        let project = home
            .path()
            .join(".claude/projects")
            .join(cwd.display().to_string().replace('/', "-"));
        std::fs::create_dir_all(&project).expect("a project directory");
        let transcript = project.join("11111111-2222-3333-4444-555555555555.jsonl");

        let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
        bytes.extend((0..size.saturating_sub(8)).map(|n| (n % 251) as u8));
        // The directory a transcript belongs to is read off its *head*, and only the first 256 KiB
        // of it — so an image record larger than that has to sit behind a record that is not.
        let opening = json!({
            "type": "user",
            "uuid": "0e1d2c3b-4a59-4687-9012-345678901234",
            "timestamp": "2026-08-20T02:56:26.000Z",
            "cwd": cwd.display().to_string(),
            "message": { "role": "user", "content": [{ "type": "text", "text": "hello" }] }
        });
        let record = json!({
            "type": "user",
            "uuid": "549c13ed-c2b4-4013-b072-f26304a5bb6c",
            "timestamp": "2026-08-20T02:56:27.681Z",
            "cwd": cwd.display().to_string(),
            "message": { "role": "user", "content": [
                { "type": "text", "text": "look" },
                { "type": "image", "source": {
                    "type": "base64", "media_type": "image/png",
                    "data": base64::engine::general_purpose::STANDARD.encode(&bytes),
                } }
            ] }
        });
        std::fs::write(&transcript, format!("{opening}\n{record}\n")).expect("a transcript");

        // The id the wire would carry, minted by the very parser the node will resolve it with.
        let adapter = Arc::new(kampr_journal::ClaudeAdapter::new(
            TranscriptRoot::new(home.path().join(".claude")).expect("a root"),
        ));
        let mut journal = adapter.open_path(transcript.clone());
        let id = journal
            .poll()
            .expect("poll")
            .iter()
            .flat_map(|turn| &turn.blocks)
            .find_map(|block| match block {
                Block::Md { att: Some(att), .. } => Some(att.id.clone()),
                _ => None,
            })
            .expect("an attachment header on the wire");

        Self {
            home: home.path().to_path_buf(),
            cwd: cwd.display().to_string(),
            _home: home,
            _work: work,
            id,
            bytes,
        }
    }
}

/// A node with that transcript, that pane, and a session on the far end of a link the test drives.
struct Peer {
    node: Arc<Node>,
    pane: String,
    device: String,
    state: PathBuf,
    to_peer: mpsc::Sender<String>,
    from_peer: mpsc::Receiver<(Instant, String)>,
    session: tokio::task::JoinHandle<()>,
    _state: tempfile::TempDir,
}

impl Peer {
    async fn start(fixture: &Fixture, caller: Caller, bytes_per_second: usize) -> Self {
        Self::start_as(fixture, caller, bytes_per_second, Role::Full).await
    }

    async fn start_as(fixture: &Fixture, caller: Caller, bytes_per_second: usize, role: Role) -> Self {
        let state = tempfile::tempdir().expect("a state dir");
        let mut config = Config::bootstrap("laptop");
        config.update.check = false;
        config.journals.home = fixture.home.display().to_string();
        // Nothing here reaches herdr: the pane is published rather than discovered, which is all
        // the route that resolves a transcript ever reads.
        config.herdr.socket = state.path().join("no-herdr.sock").display().to_string();
        config.herdr.sessions = Some(Vec::new());
        let node = Node::start(config, state.path()).await.expect("a node");

        let pane = format!("{}/w1:p1", node.node_id());
        node.publish_herd(HerdModel {
            nodes: Vec::new(),
            panes: vec![PaneEntry::new(
                node.node_id(),
                &PaneInfo {
                    pane_id: "w1:p1".into(),
                    agent: Some("claude".into()),
                    cwd: Some(fixture.cwd.clone()),
                    rows: 24,
                    ..PaneInfo::default()
                },
                true,
            )],
        });

        let device: Device = node
            .auth
            .store()
            .create_device("hub laptop", role, kampr_auth::now(), None, None, None)
            .await
            .expect("a device");

        let device_id = device.id.clone();
        let (to_peer, from_hub) = mpsc::channel(256);
        let (to_hub, from_peer) = mpsc::channel(4096);
        let session = tokio::spawn(session::run_on(
            Throttled {
                to_hub,
                bytes_per_second,
            },
            FromHub(from_hub),
            node.clone(),
            device,
            "mesh:test".into(),
            caller,
        ));
        Self {
            node,
            pane,
            device: device_id,
            state: state.path().to_path_buf(),
            to_peer,
            from_peer,
            session,
            _state: state,
        }
    }

    async fn say(&self, message: Value) {
        self.to_peer
            .send(message.to_string())
            .await
            .expect("the peer is gone");
    }

    async fn next(&mut self) -> Option<(Instant, Value)> {
        let (at, text) = tokio::time::timeout(Duration::from_secs(10), self.from_peer.recv())
            .await
            .ok()??;
        Some((at, serde_json::from_str(&text).expect("the peer sent JSON")))
    }

    async fn until(&mut self, tag: &str) -> Value {
        let mut seen = Vec::new();
        while let Some((_, message)) = self.next().await {
            if message["t"] == tag {
                return message;
            }
            seen.push(message["t"].as_str().unwrap_or("?").to_string());
        }
        panic!("never saw {tag}; saw {seen:?}");
    }

    fn stop(self) {
        self.session.abort();
        self.node.shutdown();
    }
}

/// The whole of what a hub does: ask, then grant one chunk back for each one it takes.
async fn pull(peer: &mut Peer, rid: u64) -> (Value, Vec<u8>, usize) {
    let open = peer.until("att.open").await;
    let mut body = Vec::new();
    let mut largest = 0usize;
    loop {
        let (_, message) = peer.next().await.expect("the peer stopped mid-attachment");
        match message["t"].as_str().unwrap_or_default() {
            "att.chunk" => {
                let chunk = base64::engine::general_purpose::STANDARD
                    .decode(message["b64"].as_str().expect("a chunk"))
                    .expect("base64");
                largest = largest.max(chunk.len());
                body.extend_from_slice(&chunk);
                peer.say(json!({ "t": "att.more", "rid": rid, "n": 1 })).await;
            }
            "att.end" => return (open, body, largest),
            _ => {}
        }
    }
}

const UNTHROTTLED: usize = 1 << 30;

#[tokio::test(flavor = "multi_thread")]
async fn a_hub_is_handed_a_peers_attachment_in_chunks_and_gets_every_byte_of_it() {
    let fixture = Fixture::new(200 * 1024);
    let mut peer = Peer::start(&fixture, Caller::Hub, UNTHROTTLED).await;

    let hello = peer.until("hello").await;
    assert_eq!(
        hello["caps"]["attachments"], true,
        "a hub cannot tell whether this build answers `att.fetch`: {hello}",
    );

    peer.say(json!({
        "t": "att.fetch", "rid": 7, "pane": peer.pane, "id": fixture.id, "window": ATT_WINDOW
    }))
    .await;
    let (open, body, largest) = pull(&mut peer, 7).await;

    assert_eq!(open["rid"], 7);
    assert_eq!(open["bytes"], json!(fixture.bytes.len()));
    assert_eq!(open["kind"], "image");
    assert_eq!(open["mime"], "image/png");
    assert_eq!(body, fixture.bytes, "the hub was handed different bytes");
    assert!(
        largest <= ATT_CHUNK_BYTES,
        "one message carried {largest} bytes, which is the whole record's problem again",
    );
    assert!(
        body.len() > ATT_CHUNK_BYTES,
        "this fixture is too small to have been chunked at all",
    );
    peer.stop();
}

/// A browser has `GET /api/attachment`, and the reason that route is HTTP is that bytes must not
/// share a queue with terminal frames. So a client asking for them on the socket is a `t` this
/// node has no verb for, and is ignored exactly as any other unknown one is.
#[tokio::test(flavor = "multi_thread")]
async fn a_browser_gets_no_attachment_bytes_down_its_own_socket() {
    let fixture = Fixture::new(1024);
    let mut peer = Peer::start(&fixture, Caller::Client, UNTHROTTLED).await;

    let hello = peer.until("hello").await;
    assert!(
        hello["caps"].get("attachments").is_none(),
        "a browser was promised a verb this node will not answer for it: {hello}",
    );

    peer.say(json!({
        "t": "att.fetch", "rid": 1, "pane": peer.pane, "id": fixture.id
    }))
    .await;
    peer.say(json!({ "t": "ping", "n": 99 })).await;
    let answer = peer.until("pong").await;
    assert_eq!(
        answer["n"], 99,
        "something answered the attachment ask before the ping: {answer}",
    );
    peer.stop();
}

/// The single-refusal rule, one hop out. A stale id, an escape and an id for another pane's
/// transcript are one answer here for the same reason they are one answer over HTTP.
#[tokio::test(flavor = "multi_thread")]
async fn an_id_this_peer_cannot_resolve_comes_back_as_the_one_refusal() {
    let fixture = Fixture::new(1024);
    let mut peer = Peer::start(&fixture, Caller::Hub, UNTHROTTLED).await;
    peer.until("hello").await;

    let mut locator = kampr_journal::attach::Locator::decode(&fixture.id).expect("our own id");
    locator.offset += 1;
    peer.say(json!({
        "t": "att.fetch", "rid": 3, "pane": peer.pane, "id": locator.encode()
    }))
    .await;
    let refusal = peer.until("att.error").await;
    assert_eq!(refusal["rid"], 3);
    assert_eq!(refusal["code"], "not_found", "{refusal}");

    peer.say(json!({
        "t": "att.fetch", "rid": 4, "pane": peer.pane, "id": "not-even-a-locator"
    }))
    .await;
    let garbage = peer.until("att.error").await;
    assert_eq!(
        garbage["code"], "not_found",
        "a malformed id is distinguishable from a stale one: {garbage}",
    );
    peer.stop();
}

/// **The head-of-line measurement, which is the whole reason this path is chunked.**
///
/// A `ping` is answered off the same outbox a pane's frames go through, so the round trip is the
/// delay a frame would have taken. Pings run all the way through a transfer of an attachment far
/// larger than the link can write quickly, and the answer that matters is the *worst* one.
///
/// Two mutations this is written to catch: pushing chunks onto the ordinary queue rather than the
/// bulk lane costs a pong the whole window (five chunks, not one), and sending the record as a
/// single message costs it the whole transfer.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_keeps_repainting_while_an_attachment_crosses_the_same_link() {
    const RATE: usize = 1024 * 1024;
    let fixture = Fixture::new(1024 * 1024);
    let mut peer = Peer::start(&fixture, Caller::Hub, RATE).await;
    peer.until("hello").await;

    let sent = Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));
    let pinger = tokio::spawn({
        let to_peer = peer.to_peer.clone();
        let sent = sent.clone();
        async move {
            for n in 1u64.. {
                sent.lock().unwrap().insert(n, Instant::now());
                if to_peer
                    .send(json!({ "t": "ping", "n": n }).to_string())
                    .await
                    .is_err()
                {
                    return;
                }
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
        }
    });

    let started = Instant::now();
    peer.say(json!({
        "t": "att.fetch", "rid": 11, "pane": peer.pane, "id": fixture.id, "window": ATT_WINDOW
    }))
    .await;

    let mut body = Vec::new();
    let mut worst = Duration::ZERO;
    let mut pongs = 0usize;
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        let (at, message) = peer.next().await.expect("the peer stopped mid-attachment");
        match message["t"].as_str().unwrap_or_default() {
            "pong" => {
                let n = message["n"].as_u64().expect("a pong number");
                let asked = sent.lock().unwrap().get(&n).copied().expect("a ping we sent");
                worst = worst.max(at.saturating_duration_since(asked));
                pongs += 1;
            }
            "att.chunk" => {
                body.extend_from_slice(
                    &base64::engine::general_purpose::STANDARD
                        .decode(message["b64"].as_str().expect("a chunk"))
                        .expect("base64"),
                );
                peer.say(json!({ "t": "att.more", "rid": 11, "n": 1 })).await;
            }
            "att.end" => break,
            "att.error" => panic!("the peer refused the attachment: {message}"),
            _ => {}
        }
    }
    let transfer = started.elapsed();
    pinger.abort();

    assert_eq!(body, fixture.bytes);
    // A chunk's own write time is the floor: a frame can always be behind the one already going
    // out. Anything much past it is a frame waiting behind chunks that were merely *queued*.
    let one_chunk = Duration::from_micros((ATT_CHUNK_BYTES as u64 * 4 / 3 * 1_000_000) / RATE as u64);
    println!(
        "transfer {transfer:?} for {} bytes, {pongs} pongs, worst {worst:?}, one chunk {one_chunk:?}",
        body.len(),
    );
    assert!(
        transfer > Duration::from_millis(500),
        "the link was not slow enough for anything to queue behind anything: {transfer:?}",
    );
    assert!(pongs >= 10, "only {pongs} frames crossed during the transfer");
    assert!(
        worst < one_chunk * 5 / 2,
        "a frame waited {worst:?} during a {transfer:?} transfer — more than the {one_chunk:?} \
         one chunk costs, so it was queued behind chunks rather than overtaking them",
    );
    peer.stop();
}

/// A pane the node serves and cannot name a transcript for — which is what a file id has to work
/// against, because there is no transcript in the question at all.
fn paneless(peer: &Peer) {
    peer.node.publish_herd(HerdModel {
        nodes: Vec::new(),
        panes: vec![PaneEntry::new(
            peer.node.node_id(),
            &PaneInfo {
                pane_id: "w1:p1".into(),
                agent: None,
                cwd: None,
                rows: 24,
                ..PaneInfo::default()
            },
            true,
        )],
    });
}

fn a_file_on_the_peer(dir: &std::path::Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("a file on the peer");
    kampr_journal::FileRef::new(path).encode()
}

/// The whole of the mesh claim: a hub asks for a path on the peer's filesystem over the same
/// `att.*` lane, and the peer answers it without a transcript anywhere in the question.
#[tokio::test(flavor = "multi_thread")]
async fn a_hub_is_handed_a_path_off_the_peers_own_filesystem() {
    let fixture = Fixture::new(1024);
    let mut peer = Peer::start(&fixture, Caller::Hub, UNTHROTTLED).await;
    paneless(&peer);
    let scratch = tempfile::tempdir().expect("a directory");
    let id = a_file_on_the_peer(scratch.path(), "plot.png", &fixture.bytes);

    peer.until("hello").await;
    // The record form is the control: on this pane it cannot resolve, so a green below is the
    // file form and not some transcript answering by accident.
    peer.say(json!({
        "t": "att.fetch", "rid": 1, "pane": peer.pane, "id": fixture.id, "window": ATT_WINDOW
    }))
    .await;
    assert_eq!(peer.until("att.error").await["code"], "not_found");

    peer.say(json!({
        "t": "att.fetch", "rid": 2, "pane": peer.pane, "id": id, "window": ATT_WINDOW
    }))
    .await;
    let (open, body, _) = pull(&mut peer, 2).await;

    assert_eq!(open["bytes"], json!(fixture.bytes.len()));
    assert_eq!(open["kind"], "image");
    assert_eq!(open["mime"], "image/png");
    assert_eq!(open["name"], "plot.png");
    assert_eq!(body, fixture.bytes, "the hub was handed different bytes");
    peer.stop();
}

/// The same line the HTTP route holds, held again on the link: a hub whose device has been
/// demoted here may watch panes and may not read the filesystem.
#[tokio::test(flavor = "multi_thread")]
async fn a_hub_this_node_will_not_take_input_from_is_refused_a_path() {
    let fixture = Fixture::new(1024);
    let mut peer = Peer::start_as(&fixture, Caller::Hub, UNTHROTTLED, Role::Readonly).await;
    let scratch = tempfile::tempdir().expect("a directory");
    let id = a_file_on_the_peer(scratch.path(), "plot.png", &fixture.bytes);

    peer.until("hello").await;
    peer.say(json!({
        "t": "att.fetch", "rid": 3, "pane": peer.pane, "id": id, "window": ATT_WINDOW
    }))
    .await;

    assert_eq!(peer.until("att.error").await["code"], "not_found");
    // And the record form on the same link is untouched, so this is the gate and not the link.
    peer.say(json!({
        "t": "att.fetch", "rid": 4, "pane": peer.pane, "id": fixture.id, "window": ATT_WINDOW
    }))
    .await;
    let (_, body, _) = pull(&mut peer, 4).await;
    assert_eq!(body, fixture.bytes);
    peer.stop();
}

/// A file id names any path on this machine, and the whole argument for serving one is that it is
/// equivalent to typing into the terminal — so it has to be gated like typing, which means the
/// device row is re-read *before* the read rather than up to two seconds after it. The operator
/// who demotes a hub is in another process writing SQLite and tells this session nothing.
#[tokio::test(flavor = "multi_thread")]
async fn a_hub_demoted_by_another_process_is_refused_a_path_on_the_socket_it_is_holding() {
    let fixture = Fixture::new(1024);
    let mut peer = Peer::start(&fixture, Caller::Hub, UNTHROTTLED).await;
    let scratch = tempfile::tempdir().expect("a directory");
    let id = a_file_on_the_peer(scratch.path(), "plot.png", &fixture.bytes);

    peer.until("hello").await;
    peer.say(json!({
        "t": "att.fetch", "rid": 1, "pane": peer.pane, "id": id, "window": ATT_WINDOW
    }))
    .await;
    let (_, body, _) = pull(&mut peer, 1).await;
    assert_eq!(body, fixture.bytes, "the full-role hub reads the file");

    // `kampr` in another process, holding its own connection to the same file — the gesture an
    // operator actually makes, and one nothing in this session is told about.
    let elsewhere = kampr_auth::Store::open(&Config::state_db(&peer.state))
        .await
        .expect("a second connection to the same database");
    assert!(
        elsewhere
            .set_role(&peer.device, Role::Readonly)
            .await
            .expect("the demotion"),
        "nothing was demoted, so what follows would be measuring nothing",
    );

    peer.say(json!({
        "t": "att.fetch", "rid": 2, "pane": peer.pane, "id": id, "window": ATT_WINDOW
    }))
    .await;
    assert_eq!(
        peer.until("att.error").await["code"],
        "not_found",
        "a demoted hub read a path off this machine on the socket it was already holding",
    );
    peer.stop();
}

/// The same expansion on the other lane, and against the *peer's* home rather than the hub's — a
/// hub relays the id untouched, so `~` has to mean the home of the machine the file is on.
#[tokio::test(flavor = "multi_thread")]
async fn a_tilde_on_the_link_resolves_against_the_peers_own_home() {
    let fixture = Fixture::new(1024);
    let mut peer = Peer::start(&fixture, Caller::Hub, UNTHROTTLED).await;
    paneless(&peer);
    std::fs::write(fixture.home.join("plot.png"), &fixture.bytes).expect("a file in the peer's home");

    peer.until("hello").await;
    peer.say(json!({
        "t": "att.fetch", "rid": 9, "pane": peer.pane,
        "id": kampr_journal::FileRef::new("~/plot.png").encode(), "window": ATT_WINDOW
    }))
    .await;
    let (open, body, _) = pull(&mut peer, 9).await;

    assert_eq!(open["name"], "plot.png");
    assert_eq!(body, fixture.bytes);
    peer.stop();
}
