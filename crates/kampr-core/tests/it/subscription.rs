//! The provider against something that speaks herdr's socket protocol and counts what it is
//! asked.
//!
//! Nothing here mocks the provider's own view of itself. The fake answers `session.snapshot`,
//! holds an `events.subscribe` stream open and can push events down it, which is the whole of the
//! surface the topology loop uses — so the numbers below are calls a real herdr would have served.

use kampr_core::provider::Provider;
use kampr_core::registry::PaneRegistry;
use kampr_core::{HerdrConfig, HerdrProvider};
use kampr_herdr::Herdr;
use kampr_journal::Harness;
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
    /// Whether herdr calls this an agent pane, and which pid `pane.process_info` names as its
    /// `claude`. **Two knobs, because herdr answers them from two different places**: the pane's
    /// agent is a screen scrape that goes stale, and the process list is the truth. A real pid,
    /// so that what the provider reads out of procfs is a real answer.
    agent: Mutex<Option<String>>,
    harness: Mutex<Option<u32>>,
    /// Which panes the herd has. A list rather than the one pane, because every cost this fake
    /// is used to count is per pane.
    panes: Mutex<Vec<String>>,
    /// What `pane.process_info` names in the pane's foreground besides any harness — a job the
    /// operator started, which is the thing a pane's `cmd` is.
    job: Mutex<Option<String>>,
    /// How long `pane.process_info` takes to answer, and the deepest overlap ever seen while it
    /// did. A hold is what makes concurrency observable at all: N calls that each answer
    /// instantly overlap or not by luck, and N that each take a moment overlap only if they were
    /// actually issued together.
    process_hold: Mutex<Duration>,
    /// How long `session.snapshot` takes to answer. #445's slow mode, on demand.
    snapshot_hold: Mutex<Duration>,
    inflight: AtomicUsize,
    peak: AtomicUsize,
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
            agent: Mutex::new(None),
            harness: Mutex::new(None),
            panes: Mutex::new(vec!["w1:p1".to_string()]),
            job: Mutex::new(None),
            process_hold: Mutex::new(Duration::ZERO),
            snapshot_hold: Mutex::new(Duration::ZERO),
            inflight: AtomicUsize::new(0),
            peak: AtomicUsize::new(0),
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

    /// Makes the pane an agent pane running `claude` as `pid`.
    fn runs_claude(&self, pid: u32) {
        *self.agent.lock().unwrap() = Some("claude".into());
        *self.harness.lock().unwrap() = Some(pid);
    }

    /// The harness exits and the scrape does not notice — the state herdr is actually in for as
    /// long as it takes the screen to look like something else, and the one this all exists for.
    fn harness_exited(&self) {
        *self.harness.lock().unwrap() = None;
    }

    /// Takes the agent back off the pane too, the way the scrape does once the prompt is back.
    fn runs_nothing(&self) {
        *self.agent.lock().unwrap() = None;
        *self.harness.lock().unwrap() = None;
    }

    fn snapshot(&self) -> Value {
        let mut snapshot = snapshot(&self.panes.lock().unwrap());
        if let Some(agent) = self.agent.lock().unwrap().as_deref() {
            snapshot["panes"][0]["agent"] = json!(agent);
        }
        snapshot
    }

    /// The herd this fake has, replaced. Every pane after the first is an ordinary shell pane.
    fn has_panes(&self, n: usize) {
        *self.panes.lock().unwrap() = (1..=n).map(|i| format!("w1:p{i}")).collect();
    }

    fn snapshot_takes(&self, how_long: Duration) {
        *self.snapshot_hold.lock().unwrap() = how_long;
    }

    /// Makes each `pane.process_info` take a moment, so overlap between them is a measurement
    /// rather than a coincidence.
    fn process_info_takes(&self, how_long: Duration) {
        *self.process_hold.lock().unwrap() = how_long;
    }

    /// The most `pane.process_info` calls this fake ever had open at once.
    fn peak_concurrency(&self) -> usize {
        self.peak.load(Ordering::SeqCst)
    }

    /// The job in the pane's foreground, started or finished.
    fn runs_job(&self, name: Option<&str>) {
        *self.job.lock().unwrap() = name.map(str::to_string);
    }

    fn process_info(&self) -> Value {
        let mut processes = match *self.harness.lock().unwrap() {
            Some(pid) => vec![json!({ "pid": pid, "name": "claude", "argv": ["claude"] })],
            None => Vec::new(),
        };
        if let Some(job) = self.job.lock().unwrap().as_deref() {
            processes.push(json!({ "pid": 1, "name": job, "argv": [job] }));
        }
        json!({ "process_info": { "foreground_processes": processes } })
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

        if method == "session.snapshot" {
            let hold = *self.snapshot_hold.lock().unwrap();
            if !hold.is_zero() {
                tokio::time::sleep(hold).await;
            }
        }
        if method == "pane.process_info" {
            let open = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
            self.peak.fetch_max(open, Ordering::SeqCst);
            let hold = *self.process_hold.lock().unwrap();
            if !hold.is_zero() {
                tokio::time::sleep(hold).await;
            }
            self.inflight.fetch_sub(1, Ordering::SeqCst);
        }

        let result = match method.as_str() {
            "session.snapshot" => json!({ "snapshot": self.snapshot() }),
            "pane.process_info" => self.process_info(),
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

fn snapshot(panes: &[String]) -> Value {
    json!({
        "version": "0.8.2",
        "protocol": 20,
        "focused_pane_id": "w1:p1",
        "workspaces": [{ "workspace_id": "w1", "number": 1, "label": "kampr" }],
        "tabs": [{ "tab_id": "w1:t1", "workspace_id": "w1", "label": "1" }],
        "panes": panes.iter().map(|pane_id| json!({
            "pane_id": pane_id,
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "cwd": "/tmp",
            "label": null,
            "agent": null,
            "agent_status": "unknown",
            "agent_session": null,
            "scroll": { "offset_from_bottom": 0, "max_offset_from_bottom": 12, "viewport_rows": 40 },
        })).collect::<Vec<_>>(),
        "layouts": [{
            "tab_id": "w1:t1",
            "area": { "x": 0, "y": 0, "width": 94, "height": 40 },
            "panes": panes.iter().map(|pane_id| json!({
                "pane_id": pane_id, "rect": { "x": 0, "y": 0, "width": 94, "height": 40 }
            })).collect::<Vec<_>>(),
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

/// **N sequential round trips are N coin flips.**
///
/// herdr looks at a freshly accepted connection once and, if the request is not whole at that
/// instant, not again for ~100 ms (#445) — so a herd of N panes read one after another took N
/// independent chances of that stall. Probe #450 measured 64 concurrent calls at 11.0 ms p50
/// against 12.7 ms sequential with zero stalls in either arm: herdr's accept path takes it.
///
/// Asserted on the deepest overlap the server ever saw rather than on how long the sweep took,
/// because a duration here is a measurement of this machine's load.
#[tokio::test(flavor = "multi_thread")]
async fn a_sweeps_process_reads_are_issued_together_rather_than_one_after_another() {
    let fake = FakeHerdr::start();
    fake.has_panes(8);
    // Long enough that eight sequential reads could not overlap by accident, and short enough
    // that eight concurrent ones are one of them.
    fake.process_info_takes(Duration::from_millis(50));
    let provider = HerdrProvider::spawn(fake.herdr(), HerdrConfig { ..config() });
    online(&provider).await;
    for _ in 0..300 {
        if fake.count("pane.process_info") >= 8 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        fake.count("pane.process_info"),
        8,
        "the sweep never read all eight panes"
    );
    assert!(
        fake.peak_concurrency() > 1,
        "eight panes were read strictly one at a time; each is an independent chance of #445's \
         100 ms poll"
    );
}

/// **`rtt_ms` is read off a call the node was making anyway, and it is never a `ping`.**
///
/// One `ping` per session per herd rebuild is 2/min quiet and 19/min with four panes busy (#448),
/// for a number the sweep's own `session.snapshot` already establishes.
#[tokio::test(flavor = "multi_thread")]
async fn the_round_trip_a_client_is_shown_costs_no_call_of_its_own() {
    let fake = FakeHerdr::start();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;
    tokio::time::sleep(Duration::from_millis(400)).await;

    let rtt = provider
        .rtt_ms()
        .expect("no round trip after the herd came online");
    assert!(
        (0.0..10_000.0).contains(&rtt),
        "{rtt} is not a millisecond figure"
    );
    assert_eq!(
        fake.count("ping"),
        0,
        "the node pinged herdr for a number its own sweep had already measured"
    );
}

/// And the honesty half. herdr answers a freshly accepted connection either in ~0.2 ms or ~100 ms
/// and nothing in between (#445), so the *latest* reading is a coin flip — operators were shown a
/// 100 ms herd on a few per cent of rebuilds with nothing wrong. The reported figure is the best
/// of a handful, which is the service time; the 100 ms readings are herdr's accept loop.
#[tokio::test(flavor = "multi_thread")]
async fn a_single_slow_answer_does_not_become_the_herds_round_trip() {
    let fake = FakeHerdr::start();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;
    // Several honest readings first, so there is a fast mode to find.
    tokio::time::sleep(Duration::from_millis(400)).await;
    let quick = provider.rtt_ms().expect("a round trip");

    // Then one answer that takes herdr's 100 ms poll, exactly as #445 describes it.
    fake.snapshot_takes(Duration::from_millis(120));
    let swept = fake.count("session.snapshot");
    while fake.count("session.snapshot") < swept + 2 {
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    fake.snapshot_takes(Duration::ZERO);

    let after = provider.rtt_ms().expect("a round trip");
    assert!(
        after < 100.0,
        "one call that met herdr's 100 ms accept poll became the herd's latency: {quick} then \
         {after}"
    );
}

/// **A pane's command is re-read on every sweep, because a job starting has no cue of its own.**
///
/// Nothing in herdr's snapshot says a pane started a build: the fingerprint hashes cwd, label,
/// agent, status and scroll, and `pane.process_info` is the only thing that answers it — herdr's
/// per-pane `revision` does not move for a job starting or finishing ([#449](#)). So the read has
/// to ride the sweep the pane's own output already wakes, and a cadence in front of it drops
/// exactly the pass the event was asking for: with a thirty-second gate, a pane named from its
/// shell's own startup kept `kampr · node` for a whole fifteen-second window in four live runs of
/// ten, and a three-second gate lost the same four (#451).
///
/// The pane here is not an agent pane and has already been read once, so nothing else would ask
/// about it — which is what makes the second reading below evidence rather than a coincidence.
#[tokio::test(flavor = "multi_thread")]
async fn a_panes_command_is_re_read_on_every_sweep_because_a_job_starting_has_no_cue() {
    let fake = FakeHerdr::start();
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;
    assert_eq!(named(&provider).await, None, "a pane at its prompt has no job");

    fake.runs_job(Some("cargo"));
    assert_eq!(
        settles_on(&provider, Some("cargo")).await,
        Some("cargo".to_string()),
        "the job the pane started never reached its name"
    );

    fake.runs_job(None);
    assert_eq!(
        settles_on(&provider, None).await,
        None,
        "the pane kept the finished job's name"
    );
}

/// What the herd would call the pane's command right now.
async fn named(provider: &HerdrProvider) -> Option<String> {
    provider
        .list_panes()
        .await
        .expect("panes")
        .first()
        .and_then(|pane| pane.cmd.clone())
}

/// Three seconds is five of this config's sweeps — a bound on the sweep, not on the machine.
async fn settles_on(provider: &HerdrProvider, want: Option<&str>) -> Option<String> {
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let mut saw = None;
    while tokio::time::Instant::now() < deadline {
        saw = named(provider).await;
        if saw.as_deref() == want {
            return saw;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    saw
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
    let schema: Value = serde_json::from_str(include_str!("../../../../research/herdr-api-schema.json"))
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

/// A real process to stand in for a harness. Its stdio is detached: a test that fails before
/// killing it would otherwise leave the runner's output pipe held open by the orphan.
fn spawn_harness() -> std::process::Child {
    std::process::Command::new("sleep")
        .arg("600")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("a real process to be the pane's harness")
}

/// Waits for procfs to lose the process, so what follows is asked of a pid that is really gone.
async fn reaped(pid: u32) {
    for _ in 0..400 {
        if !std::path::Path::new(&format!("/proc/{pid}")).exists() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{pid} never left procfs");
}

/// A harness that exits is the one change **nothing announces**.
///
/// Herdr reports panes, not the processes inside them, so the node's own record of which process
/// a pane is running is what goes stale — and it was only ever re-checked while asking herdr for
/// a snapshot, which is once every thirty seconds on a box nobody is watching. For all of that
/// the pane went on naming a dead pid as its harness, and everything downstream — the
/// conversation cache keyed on that process, the pane's `has_conversation`, the transcript a
/// watcher was being sent — went on believing it. Whether a pid is still alive is a `stat`, not a
/// socket, so both halves are asserted here: the answer is right the moment it is asked, and the
/// change is published without herdr being spoken to at all.
#[tokio::test(flavor = "multi_thread")]
async fn a_harness_that_exits_stops_being_the_panes_harness_without_asking_herdr() {
    let fake = FakeHerdr::start();
    let mut child = spawn_harness();
    fake.runs_claude(child.id());
    // Far longer than this test runs, in both cadences: whatever notices the exit, it is not the
    // sweep and it is not a watcher arriving.
    let provider = HerdrProvider::spawn(
        fake.herdr(),
        HerdrConfig {
            sweep: Duration::from_secs(300),
            sweep_watched: Duration::from_secs(300),
            ..config()
        },
    );
    online(&provider).await;
    for _ in 0..200 {
        if matches!(provider.agent_harness("w1:p1"), Harness::Running(_)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let Harness::Running(process) = provider.agent_harness("w1:p1") else {
        panic!("the provider never took the pane's harness from herdr");
    };
    assert_eq!(process.pid, child.id());

    let mut topology = provider.topology();
    let asked = (fake.count("session.snapshot"), fake.count("pane.process_info"));
    child.kill().expect("kill the harness");
    child.wait().expect("reap the harness");
    reaped(process.pid).await;

    assert_eq!(
        provider.agent_harness("w1:p1"),
        Harness::Absent,
        "the pane still names a pid that is gone"
    );
    tokio::time::timeout(Duration::from_secs(2), topology.changed())
        .await
        .expect("the harness exiting was never published, so nothing rebuilds the herd")
        .expect("the provider went away");
    assert_eq!(
        (fake.count("session.snapshot"), fake.count("pane.process_info")),
        asked,
        "herdr was asked whether the process died; procfs already knew"
    );
}

/// A pane this node has no record of is a pane it has not looked into, and on a host it *can*
/// look into that is not the same thing as a host that cannot see processes at all.
///
/// [`Harness::Unknown`] means the second, and it is what licenses a search of the working
/// directory — which serves whichever transcript in that directory was written last, somebody
/// else's as often as this pane's. The record is dropped whenever herdr stops calling the pane an
/// agent pane, and herdr calls it one by scraping the screen, so it comes and goes; every gap was
/// a window in which the pane's conversation was resolved from the directory alone. Once this
/// node has read one pane's processes it has proved it can, and "no record" means `Absent`.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_with_no_record_is_absent_on_a_host_whose_processes_this_node_can_read() {
    let fake = FakeHerdr::start();
    let mut child = spawn_harness();
    fake.runs_claude(child.id());
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;
    for _ in 0..200 {
        if matches!(provider.agent_harness("w1:p1"), Harness::Running(_)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        matches!(provider.agent_harness("w1:p1"), Harness::Running(_)),
        "the provider never read the pane's processes, so this proves nothing"
    );

    // The harness goes on running; it is only herdr that stops calling this an agent pane, which
    // is what drops the record. A record that survived would read `Running` here, so the only way
    // to `Absent` is the rule this test is about.
    fake.runs_nothing();
    let swept = fake.count("session.snapshot");
    for _ in 0..400 {
        if fake.count("session.snapshot") > swept + 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        provider.agent_harness("w1:p1"),
        Harness::Absent,
        "a pane this node holds no record of reported that nothing had looked into it"
    );
    child.kill().expect("kill the harness");
    child.wait().expect("reap the harness");
}

/// A fresh agent in a pane that has just lost one is a different conversation, and **nothing in
/// the snapshot says so**: herdr calls the pane an agent pane running `claude` before and after,
/// with the same label, the same directory and the same status. The only thing that moved is the
/// process, which is why the process is what movement is judged by — a pane going from no
/// harness to a harness has to reach the herd, or the operator quits an agent, starts another,
/// and the phone goes on showing the conversation of the run before it.
#[tokio::test(flavor = "multi_thread")]
async fn a_new_harness_in_a_pane_that_had_none_is_published() {
    let fake = FakeHerdr::start();
    let mut first = spawn_harness();
    fake.runs_claude(first.id());
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;
    for _ in 0..200 {
        if matches!(provider.agent_harness("w1:p1"), Harness::Running(_)) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let mut topology = provider.topology();

    first.kill().expect("kill the harness");
    first.wait().expect("reap the harness");
    fake.harness_exited();
    for _ in 0..400 {
        if provider.agent_harness("w1:p1") == Harness::Absent {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(provider.agent_harness("w1:p1"), Harness::Absent);
    // Two clean sweeps over the empty pane, so everything the exit set in motion has landed and
    // the next thing published is the arrival and nothing else.
    let settled = fake.count("session.snapshot");
    for _ in 0..400 {
        if fake.count("session.snapshot") > settled + 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    topology.borrow_and_update();

    let mut second = spawn_harness();
    fake.runs_claude(second.id());
    let arrived = tokio::time::timeout(Duration::from_secs(5), topology.changed()).await;
    let harness = provider.agent_harness("w1:p1");
    second.kill().expect("kill the harness");
    second.wait().expect("reap the harness");
    arrived
        .expect("a harness starting in an empty pane never reached the herd")
        .expect("the provider went away");
    let Harness::Running(process) = harness else {
        panic!("the pane still has no harness");
    };
    assert_eq!(process.pid, second.id());
}

/// **A pid set outlives the read that produced it, so a pid the kernel has handed on must not.**
///
/// The set is what a pid-keyed session marker is intersected with (#311), and being re-walked on
/// the sweep only ever shrank the window without closing it: the set is read on the sweep and
/// used when the herd is rebuilt, and a pid can be reaped and re-issued in between. So each pid
/// is held with the start time it had when it was read, and a look-up whose start no longer
/// matches yields **nothing** rather than a stranger: a pid the kernel re-issued to another
/// pane's harness would otherwise resolve that pane's marker against this one.
///
/// Reuse cannot be forced in a test, but the mechanism can: a process that has gone is the same
/// mismatch, and before the stamp the node went on offering its pid.
#[tokio::test(flavor = "multi_thread")]
async fn a_pid_whose_process_has_gone_is_dropped_from_the_panes_pipeline() {
    let fake = FakeHerdr::start();
    let mut child = spawn_harness();
    let pid = child.id();
    fake.runs_claude(pid);
    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;
    for _ in 0..300 {
        if provider.pane_processes("w1:p1").iter().any(|p| p.pid == pid) {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let held = provider.pane_processes("w1:p1");
    assert!(
        held.iter().any(|p| p.pid == pid),
        "the provider never read the pane's pipeline, so this proves nothing"
    );
    assert!(
        held.iter().all(|p| p.start.is_some()),
        "a pid was held with no start time, so nothing a later look finds can contradict it"
    );

    child.kill().expect("kill it");
    child.wait().expect("reap it");
    reaped(pid).await;

    assert!(
        provider.pane_processes("w1:p1").iter().all(|p| p.pid != pid),
        "the pane still offers a pid whose process is gone; the next thing to hold it is \
         somebody else"
    );
}

/// Herdr looks into the pane, and this node asks it a moment later — so the pid it is handed can
/// already be gone.
///
/// Believing one is worse than believing nothing: a process with no start time is a harness
/// [`Running`] can never disprove, because there is nothing to compare a later look against, and
/// a pane holding one searches its working directory with no lower bound at all. That serves
/// whichever transcript in the directory was written last, which at the moment an agent has just
/// been quit is exactly the wrong one.
#[tokio::test(flavor = "multi_thread")]
async fn a_pid_that_is_already_gone_is_not_a_harness() {
    let fake = FakeHerdr::start();
    let mut ghost = spawn_harness();
    let pid = ghost.id();
    ghost.kill().expect("kill it");
    ghost.wait().expect("reap it");
    reaped(pid).await;
    fake.runs_claude(pid);

    let provider = HerdrProvider::spawn(fake.herdr(), config());
    online(&provider).await;
    for _ in 0..200 {
        if provider.agent_harness("w1:p1") != Harness::Unknown {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        provider.agent_harness("w1:p1"),
        Harness::Absent,
        "a pid procfs does not have was taken for the pane's harness"
    );
}
