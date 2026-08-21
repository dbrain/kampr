//! The provider against something that speaks herdr's socket protocol and counts what it is
//! asked.
//!
//! Nothing here mocks the provider's own view of itself. The fake answers `session.snapshot`,
//! holds an `events.subscribe` stream open and can push events down it, which is the whole of the
//! surface the topology loop uses — so the numbers below are calls a real herdr would have served.

use kampr_core::registry::PaneRegistry;
use kampr_core::{HerdrConfig, HerdrProvider};
use kampr_herdr::Herdr;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{UnixListener, UnixStream};
use tokio::sync::mpsc;

struct FakeHerdr {
    calls: Mutex<HashMap<String, usize>>,
    subscribed: Mutex<Vec<String>>,
    events: Mutex<Vec<mpsc::UnboundedSender<Value>>>,
    subscribes: AtomicUsize,
    _dir: tempfile::TempDir,
    socket: std::path::PathBuf,
}

impl FakeHerdr {
    fn start() -> Arc<Self> {
        let dir = tempfile::tempdir().expect("tempdir");
        let socket = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        let fake = Arc::new(Self {
            calls: Mutex::default(),
            subscribed: Mutex::default(),
            events: Mutex::default(),
            subscribes: AtomicUsize::new(0),
            _dir: dir,
            socket,
        });
        tokio::spawn({
            let fake = fake.clone();
            async move {
                while let Ok((stream, _)) = listener.accept().await {
                    tokio::spawn(fake.clone().serve(stream));
                }
            }
        });
        fake
    }

    fn herdr(&self) -> Herdr {
        Herdr::new(&self.socket)
    }

    fn count(&self, method: &str) -> usize {
        self.calls.lock().unwrap().get(method).copied().unwrap_or(0)
    }

    /// Pushes one event to every live subscriber, the way a structural change would.
    fn emit(&self, kind: &str) {
        let event = json!({ "data": { "type": kind } });
        self.events
            .lock()
            .unwrap()
            .retain(|tx| tx.send(event.clone()).is_ok());
    }

    fn subscribers(&self) -> usize {
        self.events.lock().unwrap().len()
    }

    async fn serve(self: Arc<Self>, stream: UnixStream) {
        let mut reader = BufReader::new(stream);
        let mut line = String::new();
        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
            return;
        }
        let Ok(request) = serde_json::from_str::<Value>(&line) else {
            return;
        };
        let method = request["method"].as_str().unwrap_or_default().to_string();
        *self.calls.lock().unwrap().entry(method.clone()).or_default() += 1;
        let mut stream = reader.into_inner();

        if method == "events.subscribe" {
            self.subscribes.fetch_add(1, Ordering::SeqCst);
            let kinds: Vec<String> = request["params"]["subscriptions"]
                .as_array()
                .map(|subs| {
                    subs.iter()
                        .map(|s| s["type"].as_str().unwrap_or_default().to_string())
                        .collect()
                })
                .unwrap_or_default();
            *self.subscribed.lock().unwrap() = kinds;
            let (tx, mut rx) = mpsc::unbounded_channel();
            self.events.lock().unwrap().push(tx);
            let ack = json!({ "id": "kampr-events", "result": { "type": "subscription_started" } });
            if write_line(&mut stream, &ack).await.is_err() {
                return;
            }
            while let Some(event) = rx.recv().await {
                if write_line(&mut stream, &event).await.is_err() {
                    return;
                }
            }
            return;
        }

        let result = match method.as_str() {
            "session.snapshot" => json!({ "snapshot": snapshot() }),
            "pane.read" => json!({ "read": { "text": "", "truncated": false } }),
            other => json!({ "ok": other }),
        };
        let _ = write_line(&mut stream, &json!({ "id": "kampr", "result": result })).await;
    }
}

async fn write_line(stream: &mut UnixStream, value: &Value) -> std::io::Result<()> {
    stream.write_all(value.to_string().as_bytes()).await?;
    stream.write_all(b"\n").await?;
    stream.flush().await
}

fn snapshot() -> Value {
    json!({
        "version": "0.8.2",
        "protocol": 20,
        "focused_pane_id": "w1:p1",
        "workspaces": [{ "workspace_id": "w1", "number": 1, "label": "kampr" }],
        "tabs": [{ "tab_id": "w1:t1", "workspace_id": "w1", "label": "1" }],
        "panes": [{
            "pane_id": "w1:p1",
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "cwd": "/tmp",
            "label": null,
            "agent": null,
            "agent_status": "unknown",
            "agent_session": null,
            "scroll": { "offset_from_bottom": 0, "max_offset_from_bottom": 12, "viewport_rows": 40 },
        }],
        "layouts": [{
            "tab_id": "w1:t1",
            "area": { "x": 0, "y": 0, "width": 94, "height": 40 },
            "panes": [{ "pane_id": "w1:p1", "rect": { "x": 0, "y": 0, "width": 94, "height": 40 } }],
        }],
    })
}

fn config() -> HerdrConfig {
    HerdrConfig {
        sweep: Duration::from_millis(600),
        sweep_watched: Duration::from_millis(60),
        settle: Duration::from_millis(20),
        width_poll: Duration::from_secs(60),
        ..HerdrConfig::default()
    }
}

async fn online(provider: &HerdrProvider) {
    for _ in 0..200 {
        if provider.health().online {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("the provider never reached the fake herdr");
}

/// A node nobody is looking at re-derives the herd on the slow sweep, not the watched one. This is
/// the whole of the idle cost: an always-on box serving several sessions was paying the fast
/// cadence around the clock for nobody.
#[tokio::test(flavor = "multi_thread")]
async fn an_unwatched_herd_sweeps_slowly() {
    let fake = FakeHerdr::start();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;

    let before = fake.count("session.snapshot");
    tokio::time::sleep(Duration::from_millis(1200)).await;
    let swept = fake.count("session.snapshot") - before;
    assert!(
        (1..=4).contains(&swept),
        "1.2s at a 600ms sweep is about two snapshots, saw {swept}"
    );
}

/// Somebody watching a pane is the one case where the sweep is load-bearing: a desk resize emits
/// no event at all (probe #52), and the only thing that notices one is looking again.
#[tokio::test(flavor = "multi_thread")]
async fn a_watched_pane_puts_the_sweep_back_on_its_fast_cadence() {
    let fake = FakeHerdr::start();
    let provider = Arc::new(HerdrProvider::spawn(fake.herdr(), config()));
    online(&provider).await;
    let registry = PaneRegistry::new(provider.clone());

    // A 600ms window: about ten sweeps at the watched cadence and about one at the idle one, so
    // the bounds below cannot be met by luck at the wrong cadence in either direction.
    let window = Duration::from_millis(600);
    let sweeps = async |fake: &FakeHerdr| {
        let before = fake.count("session.snapshot");
        tokio::time::sleep(window).await;
        fake.count("session.snapshot") - before
    };

    let idle = sweeps(&fake).await;
    assert!(idle <= 3, "an unwatched herd swept {idle} times in {window:?}");

    let watcher = registry.watch("w1:p1").await.expect("watch");
    tokio::time::sleep(Duration::from_millis(100)).await;
    let watched = sweeps(&fake).await;
    assert!(
        watched >= 6,
        "a watched pane swept only {watched} times in {window:?}"
    );

    // And back down again. A count that only ever went up would pin the node at the fast cadence
    // for the rest of its life the first time anybody looked at a pane.
    drop(watcher);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let after = sweeps(&fake).await;
    assert!(
        after <= 3,
        "the last watcher left and the sweep stayed fast: {after} in {window:?}"
    );
}

/// The sweep is a backstop; events are what make the herd current. One structural change has to
/// land in a fraction of a sweep, or the slow cadence would be paid for in staleness.
#[tokio::test(flavor = "multi_thread")]
async fn an_event_re_derives_the_herd_without_waiting_for_the_sweep() {
    let fake = FakeHerdr::start();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;
    for _ in 0..200 {
        if fake.subscribers() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let before = fake.count("session.snapshot");
    fake.emit("pane_created");
    for _ in 0..30 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        if fake.count("session.snapshot") > before {
            return;
        }
    }
    panic!("a pane.created event did not re-derive the herd within 300ms");
}

/// Herdr replays the herd it already has as a burst of `created` events the moment a subscription
/// opens, and a burst is one change as far as the model is concerned. Re-deriving once per event
/// would turn every subscribe into a stampede of snapshots.
#[tokio::test(flavor = "multi_thread")]
async fn a_burst_of_events_collapses_into_one_snapshot() {
    let fake = FakeHerdr::start();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;
    for _ in 0..200 {
        if fake.subscribers() > 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }

    let before = fake.count("session.snapshot");
    for kind in [
        "workspace_created",
        "workspace_focused",
        "tab_created",
        "tab_focused",
        "pane_created",
        "pane_updated",
        "pane_focused",
        "layout_updated",
    ] {
        fake.emit(kind);
    }
    tokio::time::sleep(Duration::from_millis(200)).await;
    let taken = fake.count("session.snapshot") - before;
    assert!(
        taken <= 2,
        "eight events arriving together cost {taken} snapshots"
    );
}

/// A subscription list is all-or-nothing: one name herdr does not know is refused before the
/// stream opens (probe #54), and the node then has no events at all and falls silently back on its
/// sweep. So every name is checked against herdr's own published catalogue rather than against our
/// memory of it.
///
/// This catches a name herdr does not emit. It cannot catch a name herdr emits but refuses to be
/// subscribed to — `pane.output_changed` is one — because the schema publishes no such list. The
/// live suite offers the real list to a real herdr for that.
#[test]
fn every_subscribed_event_exists_in_herdrs_own_schema() {
    let schema: Value = serde_json::from_str(include_str!("../../../research/herdr-api-schema.json"))
        .expect("herdr's API schema");
    let mut known: Vec<String> = Vec::new();
    for def in schema["schemas"]["event"]["$defs"]
        .as_object()
        .expect("event defs")
        .values()
    {
        for variant in def["oneOf"].as_array().unwrap_or(&Vec::new()) {
            if let Some(kind) = variant["properties"]["type"]["const"].as_str() {
                known.push(kind.to_string());
            }
        }
    }
    assert!(known.len() > 20, "the schema did not parse: {known:?}");

    for sub in kampr_core::herdr_provider::subscriptions(&["w1:p1".to_string()]) {
        let kind = sub.kind.replacen('.', "_", 1);
        assert!(
            known.contains(&kind),
            "herdr publishes no event called {kind:?}; subscribing to it refuses the whole list"
        );
    }
}
