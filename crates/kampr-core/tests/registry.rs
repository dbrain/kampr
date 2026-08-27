use anyhow::Result;
use async_trait::async_trait;
use kampr_core::provider::{Input, PaneEvent, PaneInfo, PaneStream, Provider, RawScrollback};
use kampr_core::registry::{HistoryPolicy, PaneRegistry, PaneUpdate, RegistryConfig, RowRate, Watcher};
use kampr_core::wire::Cursor;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;
use tokio::sync::{Mutex, mpsc, watch};

struct Scripted {
    feeds: Mutex<HashMap<String, mpsc::Sender<PaneEvent>>>,
    opens: AtomicUsize,
    writes: Mutex<Vec<(String, Input)>>,
    topology: watch::Sender<u64>,
    reads: Mutex<std::collections::VecDeque<RawScrollback>>,
    last_read: Mutex<Option<RawScrollback>>,
    reads_done: AtomicUsize,
    stall_when_empty: std::sync::atomic::AtomicBool,
    reads_fail: std::sync::atomic::AtomicBool,
    announce: std::sync::Mutex<Option<(u16, u16)>>,
}

impl Default for Scripted {
    fn default() -> Self {
        Self {
            feeds: Mutex::default(),
            opens: AtomicUsize::default(),
            writes: Mutex::default(),
            topology: watch::channel(0).0,
            reads: Mutex::default(),
            last_read: Mutex::default(),
            reads_done: AtomicUsize::default(),
            stall_when_empty: std::sync::atomic::AtomicBool::new(false),
            reads_fail: std::sync::atomic::AtomicBool::new(false),
            announce: std::sync::Mutex::default(),
        }
    }
}

impl Scripted {
    async fn feed(&self, pane_id: &str) -> mpsc::Sender<PaneEvent> {
        self.feeds
            .lock()
            .await
            .get(pane_id)
            .cloned()
            .expect("pane was never opened")
    }
}

#[async_trait]
impl Provider for Scripted {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        Ok(vec![PaneInfo {
            pane_id: "p".into(),
            cols: Some(20),
            rows: 3,
            ..PaneInfo::default()
        }])
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        let (tx, rx) = mpsc::channel(32);
        // With `announce`, every open reports the pane's geometry before any frame — which is
        // what the real provider does, and the window in which a reopened pane is blank.
        if let Some((cols, rows)) = *self.announce.lock().unwrap() {
            tx.try_send(PaneEvent::Reset { cols, rows }).unwrap();
        }
        self.feeds.lock().await.insert(pane_id.to_string(), tx);
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(PaneStream::new(rx))
    }

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()> {
        self.writes.lock().await.push((pane_id.to_string(), input));
        Ok(())
    }

    async fn read_scrollback(&self, _pane_id: &str) -> Result<Option<RawScrollback>> {
        self.reads_done.fetch_add(1, Ordering::SeqCst);
        if self.reads_fail.load(Ordering::SeqCst) {
            anyhow::bail!("pane.read is not answering");
        }
        if let Some(next) = self.reads.lock().await.pop_front() {
            *self.last_read.lock().await = Some(next.clone());
            return Ok(Some(next));
        }
        if self.stall_when_empty.load(Ordering::SeqCst) {
            // Freezes the poller after the script runs out, so a test can read the cadence it
            // settled on rather than the one after it.
            std::future::pending::<()>().await;
        }
        Ok(self.last_read.lock().await.clone())
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.topology.subscribe()
    }
}

fn text_of(u: &PaneUpdate) -> Vec<String> {
    u.rows()
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

async fn next(w: &mut Watcher) -> PaneUpdate {
    tokio::time::timeout(Duration::from_secs(2), w.recv())
        .await
        .expect("watcher timed out")
        .expect("watcher closed")
}

async fn setup() -> (Arc<Scripted>, Arc<PaneRegistry>) {
    let p = Arc::new(Scripted::default());
    let reg = PaneRegistry::new(p.clone());
    (p, reg)
}

#[tokio::test]
async fn two_watchers_share_a_single_emulator() {
    let (p, reg) = setup().await;
    let mut a = reg.watch("p").await.unwrap();
    let mut b = reg.watch("p").await.unwrap();
    assert_eq!(
        p.opens.load(Ordering::SeqCst),
        1,
        "one observer per pane, not per viewer"
    );
    assert_eq!(reg.watcher_count("p"), 2);

    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 3 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hhello".to_vec(),
    })
    .await
    .unwrap();

    for w in [&mut a, &mut b] {
        let u = next(w).await;
        assert!(matches!(u, PaneUpdate::Reset { .. }));
        assert_eq!(text_of(&u)[0], "hello");
    }
}

#[tokio::test]
async fn a_late_watcher_gets_the_current_grid_without_disturbing_the_others() {
    let (p, reg) = setup().await;
    let mut a = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 3 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hhello".to_vec(),
    })
    .await
    .unwrap();
    assert_eq!(text_of(&next(&mut a).await)[0], "hello");

    let mut b = reg.watch("p").await.unwrap();
    let init = b.initial();
    assert!(matches!(init, PaneUpdate::Reset { .. }));
    assert_eq!(
        text_of(init)[0],
        "hello",
        "a joiner is caught up from the shared emulator"
    );
    assert_eq!(
        p.opens.load(Ordering::SeqCst),
        1,
        "joining must not restart the observer"
    );

    feed.send(PaneEvent::Bytes {
        full: false,
        bytes: b"\x1b[2;1Hworld".to_vec(),
    })
    .await
    .unwrap();
    for w in [&mut a, &mut b] {
        let u = next(w).await;
        assert!(
            matches!(u, PaneUpdate::Patch { .. }),
            "an incremental frame is a patch"
        );
        assert_eq!(text_of(&u), ["world"], "only the dirty row travels");
    }
}

#[tokio::test]
async fn the_last_watcher_leaving_tears_the_pane_down() {
    let (p, reg) = setup().await;
    let a = reg.watch("p").await.unwrap();
    let b = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;

    drop(a);
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert_eq!(reg.watcher_count("p"), 1);
    assert!(!feed.is_closed(), "one viewer left, so the emulator stays up");

    drop(b);
    for _ in 0..100 {
        if feed.is_closed() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(
        feed.is_closed(),
        "the provider stream is dropped with the last watcher"
    );
    assert_eq!(reg.watcher_count("p"), 0);

    let _c = reg.watch("p").await.unwrap();
    assert_eq!(
        p.opens.load(Ordering::SeqCst),
        2,
        "watching again re-opens the stream"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_pane_that_had_a_grid_is_never_republished_a_blank_one_on_a_re_watch() {
    let (p, reg) = setup().await;
    *p.announce.lock().unwrap() = Some((20, 3));
    let mut first = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hhello".to_vec(),
    })
    .await
    .unwrap();
    assert_eq!(text_of(&next(&mut first).await)[0], "hello");

    // A pump owns its watcher, and `stop` aborts the task that holds it. Dropping it here is
    // that abort at its worst: landed already, before the new watch is asked for.
    let hold = reg.hold_while("p", || drop(first));

    let second = reg.watch("p").await.unwrap();
    assert_eq!(
        p.opens.load(Ordering::SeqCst),
        1,
        "a re-watch re-attaches to the pane; it does not re-open it"
    );
    assert!(second.is_ready());
    assert_eq!(
        text_of(second.initial())[0],
        "hello",
        "the client was already looking at this grid"
    );
    assert_eq!(reg.watcher_count("p"), 1, "a hold is not a viewer");
    drop(hold);
}

/// The other half of the same rule: a pane nobody was watching has nothing to hand over, and a
/// genuinely new watch still owes the client the geometry it is about to lay out.
#[tokio::test]
async fn a_pane_nobody_was_watching_still_publishes_its_geometry_before_the_first_frame() {
    let (p, reg) = setup().await;
    *p.announce.lock().unwrap() = Some((94, 40));
    let hold = reg.hold_while("p", || {});
    assert!(hold.is_none(), "there was no pane to hold open");

    let first = reg.watch("p").await.unwrap();
    assert!(first.is_ready(), "the flush is what a silent new pane has to say");
    assert_eq!(first.initial().geometry(), Some((94, 40)));
    assert_eq!(text_of(first.initial())[0], "");
}

#[tokio::test]
async fn an_observer_restart_at_a_new_geometry_emits_a_fresh_reset() {
    let (p, reg) = setup().await;
    let mut a = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 3 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hsmall".to_vec(),
    })
    .await
    .unwrap();
    let u = next(&mut a).await;
    assert_eq!(u.geometry(), Some((20, 3)));

    feed.send(PaneEvent::Reset { cols: 40, rows: 5 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hwide".to_vec(),
    })
    .await
    .unwrap();
    let u = next(&mut a).await;
    assert!(
        matches!(u, PaneUpdate::Reset { .. }),
        "a geometry change is never a patch"
    );
    assert_eq!(u.geometry(), Some((40, 5)));
    assert_eq!(u.rows().len(), 5, "a reset carries every row");
    assert_eq!(text_of(&u)[0], "wide");
}

#[tokio::test]
async fn a_pane_with_no_output_still_resets_so_a_restart_is_visible() {
    let (p, reg) = setup().await;
    let mut a = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 8, rows: 2 }).await.unwrap();
    let u = next(&mut a).await;
    assert!(matches!(u, PaneUpdate::Reset { .. }));
    assert_eq!(u.geometry(), Some((8, 2)));
}

#[tokio::test]
async fn input_reaches_the_provider() {
    let (p, reg) = setup().await;
    let _a = reg.watch("p").await.unwrap();
    reg.write("p", Input::Bytes(b"\x1b[5~".to_vec())).await.unwrap();
    reg.write("p", Input::Keys(vec!["ctrl+c".into()])).await.unwrap();
    let w = p.writes.lock().await;
    assert_eq!(w.len(), 2);
    assert!(matches!(&w[0].1, Input::Bytes(b) if b == b"\x1b[5~"));
    assert!(matches!(&w[1].1, Input::Keys(k) if k == &["ctrl+c".to_string()]));
}

#[tokio::test]
async fn the_first_watcher_waits_for_real_geometry_instead_of_a_placeholder() {
    let (p, reg) = setup().await;
    let opened = tokio::spawn({
        let reg = reg.clone();
        async move { reg.watch("p").await.unwrap() }
    });
    tokio::time::sleep(Duration::from_millis(30)).await;
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 67, rows: 55 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hup".to_vec(),
    })
    .await
    .unwrap();

    let w = opened.await.unwrap();
    assert!(w.is_ready());
    assert_eq!(
        w.initial().geometry(),
        Some((67, 55)),
        "never hand a client a 1x1 grid"
    );
    assert_eq!(text_of(w.initial())[0], "up");
}

#[tokio::test]
async fn a_restart_at_the_same_geometry_never_blanks_the_grid() {
    let (p, reg) = setup().await;
    let mut a = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 3 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hkeep me".to_vec(),
    })
    .await
    .unwrap();
    assert_eq!(text_of(&next(&mut a).await)[0], "keep me");

    feed.send(PaneEvent::Reset { cols: 20, rows: 3 }).await.unwrap();
    let quiet = tokio::time::timeout(Duration::from_millis(600), a.recv()).await;
    assert!(
        quiet.is_err(),
        "a same-size restart must not push anything at a valid grid"
    );
    assert_eq!(text_of(reg.watch("p").await.unwrap().initial())[0], "keep me");

    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hback".to_vec(),
    })
    .await
    .unwrap();
    let u = next(&mut a).await;
    assert!(
        matches!(u, PaneUpdate::Reset { .. }),
        "the repaint is a reset, exactly one of them"
    );
    assert_eq!(text_of(&u)[0], "back");
}

#[tokio::test]
async fn the_registry_forwards_the_providers_topology_signal() {
    let (p, reg) = setup().await;
    let mut topo = reg.topology();
    p.topology.send_modify(|r| *r += 1);
    tokio::time::timeout(Duration::from_secs(1), topo.changed())
        .await
        .unwrap()
        .unwrap();
    assert_eq!(*topo.borrow(), 1);
    assert_eq!(reg.list_panes().await.unwrap()[0].pane_id, "p");
}

fn brisk() -> HistoryPolicy {
    HistoryPolicy {
        row_budget: 4,
        fastest: Duration::from_millis(20),
        quiet: Duration::from_millis(40),
        idle: Duration::from_millis(400),
    }
}

fn read(lines: &[String], viewport_rows: u16) -> RawScrollback {
    read_at(lines, viewport_rows, 16)
}

fn read_at(lines: &[String], viewport_rows: u16, cols: u16) -> RawScrollback {
    RawScrollback {
        text: lines.iter().map(|l| format!("{l}\n")).collect(),
        cols: Some(cols),
        viewport_rows,
        truncated: true,
    }
}

/// The cadence the poller advertises is the cadence it uses.
///
/// `interval_for_rate` exists so a pane trickling output is read at the rate its output justifies
/// rather than at the floor. It was computed correctly and then thrown away: the wait is a
/// `select!` against `activity.woken`, and `saw_frame` notifies on **every** frame while `Notify`
/// stores a permit — so any frame arriving during the wait ended it, and only `fastest` was ever
/// served. Measured against a real herdr at 10 lines/s: the policy chose 2 s on 397 of 413 polls
/// and the interval that actually followed had a median of 102 ms (#282).
///
/// The wake is for output *starting*, which is the one thing the estimate cannot know in advance —
/// so it belongs to the idle wait and to nothing else, which is what the comment above it always
/// said.
#[tokio::test]
async fn a_pane_trickling_output_is_polled_at_the_cadence_its_rate_earns() {
    let p = Arc::new(Scripted::default());
    let policy = brisk();
    let reg = PaneRegistry::with_config(
        p.clone(),
        RegistryConfig {
            history: policy,
            ..RegistryConfig::default()
        },
    );
    {
        let mut q = p.reads.lock().await;
        // One row per read, so the measured rate stays low and the policy keeps choosing `quiet`.
        for i in 1..=200 {
            q.push_back(read(&[format!("line-{i}")], 1));
        }
    }

    let _w = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    let settle = Duration::from_millis(200);
    tokio::time::sleep(settle).await;

    // A frame every `fastest`, which is the shape that defeated the cadence: often enough that a
    // wake always lands inside the wait, slow enough that the rate never justifies the floor.
    let before = p.reads_done.load(Ordering::SeqCst);
    let window = Duration::from_millis(600);
    let until = tokio::time::Instant::now() + window;
    while tokio::time::Instant::now() < until {
        let _ = feed
            .send(PaneEvent::Bytes {
                bytes: b"x".to_vec(),
                full: false,
            })
            .await;
        tokio::time::sleep(policy.fastest).await;
    }
    let reads = p.reads_done.load(Ordering::SeqCst) - before;

    // `quiet` is 40ms and the window is 600ms, so the cadence admits ~15 reads; the floor is 20ms
    // and admits ~30. The bound is generous on purpose — the claim is that the poller is on the
    // cadence rather than on the floor, not that it hits a particular count.
    let earned = window.as_millis() / policy.quiet.as_millis();
    assert!(
        (reads as u128) <= earned + earned / 2,
        "the poller ran at its floor rather than the {}ms cadence its rate earned: {reads} reads in {}ms",
        policy.quiet.as_millis(),
        window.as_millis(),
    );
}

#[tokio::test]
async fn a_watched_pane_stitches_its_history_across_reads() {
    let p = Arc::new(Scripted::default());
    let reg = PaneRegistry::with_config(
        p.clone(),
        RegistryConfig {
            history: brisk(),
            ..RegistryConfig::default()
        },
    );
    let numbered =
        |from: usize, to: usize| -> Vec<String> { (from..=to).map(|i| format!("line-{i}")).collect() };
    {
        let mut q = p.reads.lock().await;
        q.push_back(read(&numbered(1, 5), 1));
        q.push_back(read(&numbered(3, 9), 1));
    }

    let _w = reg.watch("p").await.unwrap();
    tokio::time::sleep(Duration::from_millis(220)).await;

    let doc = reg.scrollback("p").await.unwrap().expect("history");
    let lines: Vec<String> = doc
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    assert_eq!(lines, numbered(1, 8), "two overlapping reads become one ring");
    assert!(doc.capped, "the reads were against herdr's cap");
    assert_eq!(doc.total_rows, 8);
}

#[tokio::test]
async fn history_is_torn_down_with_the_last_watcher() {
    let p = Arc::new(Scripted::default());
    let reg = PaneRegistry::with_config(
        p.clone(),
        RegistryConfig {
            history: brisk(),
            ..RegistryConfig::default()
        },
    );
    p.reads
        .lock()
        .await
        .push_back(read(&["a".into(), "b".into(), "v".into()], 1));

    let w = reg.watch("p").await.unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(reg.scrollback("p").await.unwrap().unwrap().rows.len(), 2);

    drop(w);
    tokio::time::sleep(Duration::from_millis(120)).await;
    assert_eq!(reg.watcher_count("p"), 0);
    let fresh = reg.scrollback("p").await.unwrap().unwrap();
    assert_eq!(
        fresh.from_top, 0,
        "an unwatched pane starts a ring from the one read it can do"
    );
}

#[tokio::test]
async fn new_hyperlinks_ride_a_patch_as_a_delta_in_arrival_order() {
    let (p, reg) = setup().await;
    let mut a = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 40, rows: 3 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1H\x1b]8;;https://one\x1b\\ONE\x1b]8;;\x1b\\".to_vec(),
    })
    .await
    .unwrap();
    let u = next(&mut a).await;
    match &u {
        PaneUpdate::Reset { links, .. } => assert_eq!(links.as_slice(), ["https://one"]),
        other => panic!("{other:?}"),
    }

    feed.send(PaneEvent::Bytes {
        full: false,
        bytes: b"\x1b[2;1H\x1b]8;;https://two\x1b\\TWO\x1b]8;;\x1b\\".to_vec(),
    })
    .await
    .unwrap();
    let u = next(&mut a).await;
    match &u {
        PaneUpdate::Patch { new_links, .. } => {
            assert_eq!(
                new_links.as_slice(),
                ["https://two"],
                "only what the client lacks"
            );
        }
        other => panic!("{other:?}"),
    }
    assert_eq!(u.rows()[0].cells[0].link, Some(1), "ids index the appended table");

    let mut enc = kampr_core::wire::Encoder::new();
    let v = serde_json::to_value(enc.encode("n/p", &u).last().unwrap()).unwrap();
    assert_eq!(v["links"], serde_json::json!(["https://two"]));
    assert_eq!(v["rows"][0]["runs"][0]["l"], 1);
}

#[test]
fn the_cadence_is_derived_from_the_measured_row_rate() {
    let policy = HistoryPolicy::default();
    assert_eq!(policy.interval_for_rate(1000.0), Duration::from_millis(400));
    assert_eq!(policy.interval_for_rate(2000.0), Duration::from_millis(200));
    assert_eq!(
        policy.interval_for_rate(100_000.0),
        policy.fastest,
        "past the floor no cadence can follow the pane"
    );
    assert_eq!(
        policy.interval_for_rate(1.0),
        policy.quiet,
        "a trickle is not worth chasing"
    );
    assert_eq!(policy.interval_for_rate(0.0), policy.quiet);
}

#[test]
fn the_cadence_keeps_every_followable_rate_under_herdrs_thousand_row_cap() {
    let policy = HistoryPolicy::default();
    for rate in [10.0f64, 100.0, 400.0, 900.0, 1000.0, 2500.0, 4000.0] {
        let interval = policy.interval_for_rate(rate);
        if interval == policy.fastest {
            continue;
        }
        let rows_between_reads = rate * interval.as_secs_f64();
        assert!(
            rows_between_reads < 1000.0,
            "{rate} rows/s would put {rows_between_reads} rows between reads"
        );
    }
}

#[test]
fn one_quiet_sample_between_bursts_does_not_throw_away_the_estimate() {
    // The failure this exists to prevent: a 100 ms poll lands between two bursts, measures zero,
    // and relaxes to the quiet cadence — at which point the next burst gaps.
    let policy = HistoryPolicy::default();
    let mut rate = RowRate::default();
    let burst = Duration::from_millis(40);
    let lull = Duration::from_millis(100);

    rate.observe(300, burst);
    assert_eq!(policy.interval_for_rate(rate.get()), policy.fastest);
    let after_lull = rate.observe(0, lull);
    assert!(
        after_lull > 0.0,
        "one empty sample must decay the estimate, not erase it"
    );
    assert_eq!(
        policy.interval_for_rate(after_lull),
        policy.fastest,
        "still mid-stream, so still fast"
    );
}

#[test]
fn the_estimate_decays_to_quiet_once_the_pane_really_stops() {
    let policy = HistoryPolicy::default();
    let mut rate = RowRate::default();
    rate.observe(4000, Duration::from_millis(100));
    let mut last = policy.fastest;
    for _ in 0..20 {
        last = policy.interval_for_rate(rate.observe(0, Duration::from_millis(100)));
    }
    assert_eq!(
        last, policy.quiet,
        "a stopped pane must not be polled fast forever"
    );
}

#[test]
fn a_sample_too_short_to_mean_anything_is_ignored() {
    let mut rate = RowRate::default();
    rate.observe(500, Duration::from_millis(100));
    let steady = rate.get();
    assert_eq!(rate.observe(0, Duration::from_micros(50)), steady);
}

fn patient() -> HistoryPolicy {
    HistoryPolicy {
        row_budget: 4,
        fastest: Duration::from_millis(20),
        quiet: Duration::from_millis(40),
        idle: Duration::from_secs(600),
    }
}

async fn watched(p: Arc<Scripted>, policy: HistoryPolicy) -> (Arc<PaneRegistry>, Watcher) {
    let reg = PaneRegistry::with_config(
        p.clone(),
        RegistryConfig {
            history: policy,
            ..RegistryConfig::default()
        },
    );
    let w = reg.watch("p").await.unwrap();
    (reg, w)
}

#[tokio::test(start_paused = true)]
async fn an_idle_pane_is_not_polled_at_all() {
    let p = Arc::new(Scripted::default());
    p.reads.lock().await.push_back(read(&["a".into(), "v".into()], 1));
    let (_reg, _w) = watched(p.clone(), patient()).await;

    tokio::time::sleep(Duration::from_millis(500)).await;
    let settled = p.reads_done.load(Ordering::SeqCst);
    tokio::time::sleep(Duration::from_secs(60)).await;
    assert_eq!(
        p.reads_done.load(Ordering::SeqCst),
        settled,
        "a pane producing nothing must cost no socket traffic"
    );
}

#[tokio::test(start_paused = true)]
async fn a_frame_wakes_the_parked_poller() {
    let p = Arc::new(Scripted::default());
    p.reads.lock().await.push_back(read(&["a".into(), "v".into()], 1));
    let (_reg, _w) = watched(p.clone(), patient()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;
    let parked = p.reads_done.load(Ordering::SeqCst);

    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 2 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"out".to_vec(),
    })
    .await
    .unwrap();
    tokio::time::sleep(Duration::from_millis(60)).await;

    assert!(
        p.reads_done.load(Ordering::SeqCst) > parked,
        "output is the wake-up, not a timer"
    );
}

#[tokio::test(start_paused = true)]
async fn a_gap_drops_the_poller_to_its_fastest_cadence() {
    let p = Arc::new(Scripted::default());
    p.stall_when_empty.store(true, Ordering::SeqCst);
    let numbered =
        |from: usize, to: usize| -> Vec<String> { (from..=to).map(|i| format!("line-{i}")).collect() };
    {
        let mut q = p.reads.lock().await;
        q.push_back(read(&numbered(1, 5), 1));
        q.push_back(read(&numbered(900, 906), 1));
    }
    let (reg, _w) = watched(p.clone(), patient()).await;
    tokio::time::sleep(Duration::from_millis(500)).await;

    let status = reg.history_status("p").expect("watched");
    assert_eq!(
        status.poll,
        patient().fastest,
        "a gap means read flat out until it stops"
    );
    assert!(status.rows_per_sec > 0.0);
}

#[tokio::test(start_paused = true)]
async fn a_fast_pane_is_polled_faster_than_a_slow_one() {
    let numbered =
        |from: usize, to: usize| -> Vec<String> { (from..=to).map(|i| format!("line-{i}")).collect() };
    let policy = HistoryPolicy {
        row_budget: 20,
        fastest: Duration::from_millis(5),
        quiet: Duration::from_millis(400),
        idle: Duration::from_secs(600),
    };

    let slow = Arc::new(Scripted::default());
    slow.stall_when_empty.store(true, Ordering::SeqCst);
    {
        let mut q = slow.reads.lock().await;
        q.push_back(read(&numbered(1, 3), 1));
        q.push_back(read(&numbered(1, 5), 1));
    }
    let (slow_reg, _sw) = watched(slow.clone(), policy).await;

    let fast = Arc::new(Scripted::default());
    fast.stall_when_empty.store(true, Ordering::SeqCst);
    {
        let mut q = fast.reads.lock().await;
        q.push_back(read(&numbered(1, 3), 1));
        q.push_back(read(&numbered(1, 600), 1));
    }
    let (fast_reg, _fw) = watched(fast.clone(), policy).await;

    tokio::time::sleep(Duration::from_millis(300)).await;
    let slow_rate = slow_reg.history_status("p").unwrap().rows_per_sec;
    let fast_rate = fast_reg.history_status("p").unwrap().rows_per_sec;
    assert!(
        fast_rate > slow_rate * 10.0,
        "measured rates should separate: slow {slow_rate}, fast {fast_rate}"
    );
}

#[tokio::test(start_paused = true)]
async fn a_pane_with_no_ring_yet_is_still_read_once_it_starts_producing() {
    let p = Arc::new(Scripted::default());
    // Nothing queued: the provider reports no history at all, the way herdr does for a pane that
    // has not scrolled yet. Treating that like an alt-screen pane and parking would miss the
    // whole first burst — which is exactly when a ring appears.
    let (_reg, _w) = watched(p.clone(), patient()).await;
    tokio::time::sleep(Duration::from_millis(200)).await;
    let parked = p.reads_done.load(Ordering::SeqCst);

    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 2 }).await.unwrap();
    for _ in 0..6 {
        feed.send(PaneEvent::Bytes {
            full: false,
            bytes: b"x\r\n".to_vec(),
        })
        .await
        .unwrap();
        tokio::time::sleep(Duration::from_millis(30)).await;
    }

    let after = p.reads_done.load(Ordering::SeqCst);
    assert!(
        after >= parked + 3,
        "output must keep the poller looking for a ring, saw {} reads",
        after - parked
    );
}

/// Probe #112: a watched pane logged `pane re-wrapped; ring restarted` over and over while its
/// `from_top` climbed by the ring's whole depth each time. A pane whose measured width moves once
/// owes one restart, not one per read — and every read after it must go back to stitching.
#[tokio::test]
async fn a_single_width_change_costs_a_single_restart() {
    let p = Arc::new(Scripted::default());
    let reg = PaneRegistry::with_config(
        p.clone(),
        RegistryConfig {
            history: brisk(),
            ..RegistryConfig::default()
        },
    );
    let numbered =
        |from: usize, to: usize| -> Vec<String> { (from..=to).map(|i| format!("line-{i}")).collect() };
    {
        let mut q = p.reads.lock().await;
        q.push_back(read(&numbered(1, 5), 1));
        q.push_back(read_at(&numbered(3, 9), 1, 40));
    }

    let _w = reg.watch("p").await.unwrap();
    tokio::time::sleep(Duration::from_millis(220)).await;

    let restarted = reg.scrollback("p").await.unwrap().expect("history");
    let again = reg.scrollback("p").await.unwrap().expect("history");
    assert_eq!(
        (again.from_top, again.total_rows),
        (restarted.from_top, restarted.total_rows),
        "re-reading the same history at the same width must not restart the ring"
    );
    assert_eq!(again.total_rows, 6, "the rows the restart kept are still there");
}

/// A full-screen program has the pane, so herdr answers `pane.read recent` with the live viewport
/// and nothing above it (#244, #246). The node used to read that silence as history disagreeing with the
/// ring: it discarded every row and rebased, and a rebase is indistinguishable from growth on the
/// wire, so the client threw its own copy away and was handed the same rows back at a new base a
/// moment later. On a phone that is pressing ↑ for shell history and landing in scrollback.
#[tokio::test]
async fn a_read_that_is_only_the_viewport_does_not_cost_the_ring_its_history() {
    let p = Arc::new(Scripted::default());
    let reg = PaneRegistry::with_config(
        p.clone(),
        RegistryConfig {
            history: brisk(),
            ..RegistryConfig::default()
        },
    );
    let numbered =
        |from: usize, to: usize| -> Vec<String> { (from..=to).map(|i| format!("line-{i}")).collect() };
    {
        let mut q = p.reads.lock().await;
        q.push_back(read(&numbered(1, 5), 1));
        // Every read from here on is this one: the program keeps the screen.
        q.push_back(read(&["only-the-viewport".into()], 1));
    }

    let _w = reg.watch("p").await.unwrap();
    tokio::time::sleep(Duration::from_millis(220)).await;

    let doc = reg.scrollback("p").await.unwrap().expect("history");
    let lines: Vec<String> = doc
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    assert_eq!(lines, numbered(1, 4), "the operator's history is still there");
    assert_eq!(doc.total_rows, 4);
    assert_eq!(
        doc.from_top, 0,
        "nothing was discarded, so nothing may be rebased past what the client holds"
    );
}

/// `watch` took one process-global lock and held it across the two-second first-frame wait, so
/// four panes on a silent provider opened at 2 s, 4 s, 6 s and 8 s — and the worst case is the
/// worst failure, because a spawn that fails sends no `Reset` at all and every pane waits the lot.
/// The client fans out exactly this way: it re-sends `watch` for every watched pane on reconnect.
#[tokio::test(flavor = "multi_thread")]
async fn opening_one_pane_does_not_wait_out_another_panes_first_frame() {
    let p = Arc::new(Scripted::default());
    let reg = PaneRegistry::with_config(
        p.clone(),
        RegistryConfig {
            first_grid_wait: Duration::from_millis(400),
            ..RegistryConfig::default()
        },
    );
    let started = std::time::Instant::now();
    let (a, b) = tokio::join!(reg.watch("a"), reg.watch("b"));
    let elapsed = started.elapsed();
    a.expect("a");
    b.expect("b");
    assert!(
        elapsed < Duration::from_millis(700),
        "two silent panes opened serially: {elapsed:?}"
    );
}

/// Probe #12: every herdr frame ends with an absolute cursor address and carries `ESC[?25h/l`, so
/// a frame that moves nothing but the cursor is the *normal* shape for ←/→/Home at a prompt and
/// for a program hiding the caret. The client paints the caret from the last `grid.*` message and
/// nothing else, so a caret with no cell change behind it sat at the wrong column until some
/// unrelated cell moved and dragged the right cursor along with it.
#[tokio::test]
async fn a_frame_that_moves_only_the_cursor_is_still_published() {
    let (p, reg) = setup().await;
    let mut w = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 3 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hprompt> ".to_vec(),
    })
    .await
    .unwrap();
    let first = next(&mut w).await;
    assert!(first.is_reset());

    feed.send(PaneEvent::Bytes {
        full: false,
        bytes: b"\x1b[1;3H".to_vec(),
    })
    .await
    .unwrap();
    match next(&mut w).await {
        PaneUpdate::Patch { rows, cursor, .. } => {
            assert!(rows.is_empty(), "no cell changed, so no row should travel");
            assert_eq!(
                cursor,
                Cursor {
                    col: 2,
                    row: 0,
                    visible: true
                }
            );
        }
        other => panic!("a cursor-only frame published {other:?}"),
    }

    feed.send(PaneEvent::Bytes {
        full: false,
        bytes: b"\x1b[?25l".to_vec(),
    })
    .await
    .unwrap();
    match next(&mut w).await {
        PaneUpdate::Patch { cursor, .. } => assert!(!cursor.visible),
        other => panic!("a hidden caret published {other:?}"),
    }
}

/// Nothing asserted a cursor on a *patch* — the only cursor assertions in the suite were on a
/// hand-built `GridPatch` and on a `grid.reset` — so replacing it with `Cursor::default()` in
/// `pump` passed the whole suite.
#[tokio::test]
async fn a_patch_carries_the_cursor_the_frame_left_behind() {
    let (p, reg) = setup().await;
    let mut w = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 3 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Ha".to_vec(),
    })
    .await
    .unwrap();
    assert!(next(&mut w).await.is_reset());

    feed.send(PaneEvent::Bytes {
        full: false,
        bytes: b"\x1b[2;1Hbcd".to_vec(),
    })
    .await
    .unwrap();
    match next(&mut w).await {
        PaneUpdate::Patch { rows, cursor, .. } => {
            assert_eq!(rows.len(), 1);
            assert_eq!(
                cursor,
                Cursor {
                    col: 3,
                    row: 1,
                    visible: true
                }
            );
        }
        other => panic!("expected a patch, got {other:?}"),
    }
}

/// A pane whose `pane.read` keeps failing looked exactly like a pane with nothing to say: the
/// pacer relaxed to the idle backstop, history stopped growing and only a `debug!` knew. Quiet is
/// a fact about the pane; a failing read is a fact about the node, and the two must not settle
/// into the same cadence.
#[tokio::test(flavor = "multi_thread")]
async fn a_pane_whose_reads_keep_failing_is_not_treated_as_a_quiet_one() {
    let p = Arc::new(Scripted::default());
    p.reads_fail.store(true, Ordering::SeqCst);
    let reg = PaneRegistry::with_config(
        p.clone(),
        RegistryConfig {
            first_grid_wait: Duration::from_millis(10),
            history: brisk(),
            ..RegistryConfig::default()
        },
    );
    let _w = reg.watch("p").await.unwrap();
    tokio::time::sleep(Duration::from_millis(300)).await;
    let reads = p.reads_done.load(Ordering::SeqCst);
    assert!(
        reads >= 4,
        "300ms at a 40ms quiet cadence is several reads, saw {reads} — the poller parked on the \
         idle backstop as though the pane had simply gone quiet"
    );
}

/// Probe #268. The defect the browser report was made of: one pane's grid frozen at an old screen
/// while that same pane's conversation kept answering on the same healthy socket.
///
/// `PaneEntry` owns a `broadcast::Sender` of its own, so the channel never closes when the pump
/// that feeds it goes — and `Watcher::recv` therefore cannot tell a dead feeder from a pane nobody
/// is typing in. It simply parks. Every surface stays honest-looking: the socket is up, the herd
/// model is fresh, `watcher_count` still counts this viewer, and the pane is dead for good.
///
/// This is probe #233's shape one layer in, and the rule it breaks is the one this project names
/// above all others — a failure that wears a plausible-looking success.
#[tokio::test]
async fn a_watcher_whose_feeder_died_is_told_rather_than_left_parked() {
    let (p, reg) = setup().await;
    let mut w = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 3 }).await.unwrap();
    feed.send(PaneEvent::Bytes {
        full: true,
        bytes: b"\x1b[1;1Hhello".to_vec(),
    })
    .await
    .unwrap();
    assert_eq!(text_of(&next(&mut w).await)[0], "hello");

    // Exactly what `supervise` returning does: the provider's end of the stream goes, and nothing
    // anywhere is told about it.
    drop(feed);
    p.feeds.lock().await.remove("p");

    let ended = tokio::time::timeout(Duration::from_secs(2), w.recv()).await;
    assert!(
        ended.is_ok(),
        "a watcher whose feeder died parked for ever instead of reporting it",
    );
    assert!(
        matches!(ended.unwrap(), Err(kampr_core::registry::WatchError::Closed)),
        "the pane went quiet without saying it had stopped",
    );
}

/// And the recovery that makes it survivable: a dead entry is not a pane. Re-opening the pane —
/// the one thing an operator would try — must re-open the provider rather than re-attach to the
/// corpse. `hold_while` pins the entry across a re-watch by design (#252), so without this the
/// stall outlives every close and reopen the operator can perform.
#[tokio::test]
async fn reopening_a_pane_whose_feeder_died_opens_a_live_one() {
    let (p, reg) = setup().await;
    let w = reg.watch("p").await.unwrap();
    let feed = p.feed("p").await;
    feed.send(PaneEvent::Reset { cols: 20, rows: 3 }).await.unwrap();
    assert_eq!(p.opens.load(Ordering::SeqCst), 1);

    drop(feed);
    p.feeds.lock().await.remove("p");
    // The dead pump has to actually finish before the entry can be seen to be dead.
    tokio::time::sleep(Duration::from_millis(50)).await;

    let _second = reg.watch("p").await.unwrap();
    assert_eq!(
        p.opens.load(Ordering::SeqCst),
        2,
        "a re-watch re-attached to a pane with nothing feeding it",
    );
    drop(w);
}
