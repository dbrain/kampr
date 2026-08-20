use anyhow::Result;
use async_trait::async_trait;
use kampr_core::provider::{Input, PaneEvent, PaneInfo, PaneStream, Provider, RawScrollback};
use kampr_core::registry::{PaneRegistry, PaneUpdate, RegistryConfig, Watcher};
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
            cols: 20,
            rows: 3,
            ..PaneInfo::default()
        }])
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        let (tx, rx) = mpsc::channel(32);
        self.feeds.lock().await.insert(pane_id.to_string(), tx);
        self.opens.fetch_add(1, Ordering::SeqCst);
        Ok(PaneStream::new(rx))
    }

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()> {
        self.writes.lock().await.push((pane_id.to_string(), input));
        Ok(())
    }

    async fn read_scrollback(&self, _pane_id: &str) -> Result<Option<RawScrollback>> {
        if let Some(next) = self.reads.lock().await.pop_front() {
            *self.last_read.lock().await = Some(next.clone());
            return Ok(Some(next));
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

fn read(lines: &[String], viewport_rows: u16) -> RawScrollback {
    RawScrollback {
        text: lines.iter().map(|l| format!("{l}\n")).collect(),
        cols: 16,
        viewport_rows,
        truncated: true,
    }
}

#[tokio::test]
async fn a_watched_pane_stitches_its_history_across_reads() {
    let p = Arc::new(Scripted::default());
    let reg = PaneRegistry::with_config(
        p.clone(),
        RegistryConfig {
            scrollback_poll: Duration::from_millis(40),
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
            scrollback_poll: Duration::from_millis(40),
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
