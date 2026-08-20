use async_trait::async_trait;
use kampr_core::provider::{Input, PaneEvent, PaneInfo, PaneStream, Provider, RawScrollback};
use kampr_core::registry::RegistryConfig;
use kampr_core::{PaneRegistry, wire::Encoder};
use kampr_node::herd::HerdModel;
use kampr_node::outbox::Outbox;
use kampr_node::session::{PaneStreamCtx, pump_pane};
use kampr_node::wire::Wire;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};

/// A pane that emits exactly what a test tells it to, so the backpressure rule can be proved
/// without a terminal, a socket or a phone.
struct Scripted {
    events: std::sync::Mutex<Option<mpsc::Receiver<PaneEvent>>>,
    topology: watch::Sender<u64>,
}

#[async_trait]
impl Provider for Scripted {
    async fn list_panes(&self) -> anyhow::Result<Vec<PaneInfo>> {
        Ok(vec![PaneInfo {
            pane_id: "w1:p1".into(),
            cols: 20,
            rows: 4,
            ..PaneInfo::default()
        }])
    }

    async fn watch_pane(&self, _pane_id: &str) -> anyhow::Result<PaneStream> {
        let rx = self
            .events
            .lock()
            .unwrap()
            .take()
            .expect("the scripted pane is watched once");
        Ok(PaneStream::new(rx))
    }

    async fn write_pane(&self, _pane_id: &str, _input: Input) -> anyhow::Result<()> {
        Ok(())
    }

    async fn read_scrollback(&self, _pane_id: &str) -> anyhow::Result<Option<RawScrollback>> {
        Ok(None)
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.topology.subscribe()
    }
}

fn setup(queue: usize) -> (Arc<PaneRegistry>, mpsc::Sender<PaneEvent>, Arc<Outbox>, Arc<Wire>) {
    let (tx, rx) = mpsc::channel(256);
    let (topology, _) = watch::channel(0);
    let provider = Arc::new(Scripted {
        events: std::sync::Mutex::new(Some(rx)),
        topology,
    });
    let registry = PaneRegistry::with_config(
        provider,
        RegistryConfig {
            broadcast_capacity: 8,
            first_grid_wait: Duration::from_millis(500),
            ..RegistryConfig::default()
        },
    );
    let outbox = Arc::new(Outbox::new(queue));
    let wire = Arc::new(Wire::new(outbox.clone()));
    (registry, tx, outbox, wire)
}

fn spawn_pump(
    registry: Arc<PaneRegistry>,
    wire: Arc<Wire>,
) -> (tokio::task::JoinHandle<()>, watch::Sender<Arc<HerdModel>>) {
    let (herd_tx, herd_rx) = watch::channel(Arc::new(HerdModel::default()));
    let handle = tokio::spawn(pump_pane(PaneStreamCtx {
        registry,
        // Nothing in this test reaches herdr; a socket that does not exist proves it.
        herdr: kampr_herdr::Herdr::new("/nonexistent/kampr-test.sock"),
        herd: herd_rx,
        wire,
        global: "01J/w1:p1".into(),
        local: "w1:p1".into(),
        scrollback: false,
    }));
    (handle, herd_tx)
}

async fn drain(outbox: &Outbox) -> Vec<Value> {
    let mut frames = Vec::new();
    while let Ok(Some(frame)) = tokio::time::timeout(Duration::from_millis(50), outbox.next()).await {
        frames.push(serde_json::from_str(&frame.json).unwrap());
    }
    frames
}

fn ansi(text: &str) -> PaneEvent {
    PaneEvent::Bytes {
        full: false,
        bytes: text.as_bytes().to_vec(),
    }
}

#[tokio::test]
async fn a_watched_pane_opens_with_a_reset_and_then_patches() {
    let (registry, events, outbox, wire) = setup(64);
    events.send(PaneEvent::Reset { cols: 20, rows: 4 }).await.unwrap();
    events
        .send(PaneEvent::Bytes {
            full: true,
            bytes: b"hello".to_vec(),
        })
        .await
        .unwrap();
    let (pump, _herd) = spawn_pump(registry, wire);

    tokio::time::sleep(Duration::from_millis(120)).await;
    events.send(ansi("\r\nworld")).await.unwrap();
    tokio::time::sleep(Duration::from_millis(120)).await;

    let frames = drain(&outbox).await;
    let kinds: Vec<&str> = frames.iter().map(|f| f["t"].as_str().unwrap()).collect();
    assert_eq!(kinds.first(), Some(&"grid.reset"), "{kinds:?}");
    assert!(kinds.contains(&"grid.patch"), "{kinds:?}");
    assert_eq!(frames[0]["cols"], 20);
    assert_eq!(frames[0]["rows"], 4);
    pump.abort();
}

/// A phone that stops reading must not be able to grow the node's memory. The queue is bounded,
/// the pane's patches are dropped, and what the client eventually reads is one `grid.reset` —
/// not a backlog it can never drain.
#[tokio::test]
async fn a_client_that_stops_reading_is_reset_rather_than_buffered() {
    const QUEUE: usize = 8;
    let (registry, events, outbox, wire) = setup(QUEUE);
    events.send(PaneEvent::Reset { cols: 20, rows: 4 }).await.unwrap();
    events
        .send(PaneEvent::Bytes {
            full: true,
            bytes: b"start".to_vec(),
        })
        .await
        .unwrap();
    let (pump, _herd) = spawn_pump(registry, wire);
    tokio::time::sleep(Duration::from_millis(120)).await;

    // Nobody drains the outbox from here on.
    for n in 0..400 {
        if events.send(ansi(&format!("\r\nline {n}"))).await.is_err() {
            break;
        }
        if n % 32 == 0 {
            tokio::time::sleep(Duration::from_millis(2)).await;
        }
    }
    tokio::time::sleep(Duration::from_millis(300)).await;

    assert!(
        outbox.depth() <= QUEUE * 4,
        "the queue grew to {} against a cap of {QUEUE}",
        outbox.depth()
    );
    let (purges, dropped) = outbox.stats();
    assert!(purges > 0, "nothing was ever purged, so nothing was bounded");
    assert!(dropped > 0);

    // The client comes back. What it finds is a full grid, not a patch against state it lost.
    let frames = drain(&outbox).await;
    let last_grid = frames
        .iter()
        .rev()
        .find(|f| f["t"].as_str().is_some_and(|t| t.starts_with("grid.")))
        .expect("some grid frame survived");
    assert_eq!(
        last_grid["t"], "grid.reset",
        "a slow client is reset, never patched"
    );
    pump.abort();
}

/// The reset a purged client receives has to be a *complete* grid — the whole point is that it can
/// be applied with no history at all.
#[tokio::test]
async fn the_reset_after_a_purge_carries_the_whole_grid() {
    let (registry, events, outbox, wire) = setup(4);
    events.send(PaneEvent::Reset { cols: 20, rows: 4 }).await.unwrap();
    events
        .send(PaneEvent::Bytes {
            full: true,
            bytes: b"one".to_vec(),
        })
        .await
        .unwrap();
    let (pump, _herd) = spawn_pump(registry, wire);
    tokio::time::sleep(Duration::from_millis(120)).await;

    for n in 0..200 {
        let _ = events.send(ansi(&format!("\r\nrow {n}"))).await;
    }
    tokio::time::sleep(Duration::from_millis(250)).await;

    let frames = drain(&outbox).await;
    let reset = frames
        .iter()
        .rev()
        .find(|f| f["t"] == "grid.reset")
        .expect("a reset was sent");
    let rows = reset["rows_data"].as_array().unwrap();
    assert_eq!(rows.len(), 4, "every row of the pane, not a diff");
    let text: String = rows
        .iter()
        .flat_map(|r| r["runs"].as_array().cloned().unwrap_or_default())
        .filter_map(|run| run["x"].as_str().map(str::to_string))
        .collect();
    assert!(
        text.contains("row 199"),
        "the reset is the pane's end state: {text:?}"
    );
    pump.abort();
}

/// Style ids are only promised to be stable for the life of one connection, so two connections
/// interning the same pens must not be assumed to agree — and each must be told about its own.
#[tokio::test]
async fn each_connection_interns_its_own_styles() {
    let mut first = Encoder::new();
    let mut second = Encoder::new();
    let red = kampr_term::Cell {
        ch: 'x',
        fg: kampr_term::Color::Indexed(1),
        ..kampr_term::Cell::default()
    };
    let update = kampr_core::registry::PaneUpdate::Patch {
        rows: Arc::new(vec![kampr_term::RowDiff {
            row: 0,
            cells: vec![red],
        }]),
        cursor: kampr_core::wire::Cursor {
            col: 0,
            row: 0,
            visible: true,
        },
        new_links: Arc::new(vec![]),
    };
    for encoder in [&mut first, &mut second] {
        let messages = encoder.encode("p", &update);
        let styles = serde_json::to_value(&messages[0]).unwrap();
        assert_eq!(styles["t"], "styles", "a new pen is announced before it is used");
        assert_eq!(
            styles["from"], 1,
            "each connection starts its own table after the default pen"
        );
    }
}
