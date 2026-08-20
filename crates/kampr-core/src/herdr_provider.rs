use crate::backoff::Backoff;
use crate::provider::{AgentStatus, Input, PaneEvent, PaneInfo, PaneStream, Provider, RawScrollback};
use anyhow::{Context, Result, bail};
use async_trait::async_trait;
use kampr_herdr::{Herdr, Observer, Snapshot, StreamEvent};
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, warn};

/// `pane.updated` is what actually fires when the desk resizes — `layout.updated` does not
/// (probed 2026-08-20, herdr 0.8.2). Both are subscribed, and the poll below is the backstop for
/// anything neither reports — including `pane.agent_status_changed`, which cannot be listed here
/// because herdr rejects a subscription to it without a `pane_id`, and one rejection kills the
/// whole `events.subscribe` call.
const TOPOLOGY_EVENTS: &[&str] = &[
    "layout.updated",
    "pane.created",
    "pane.closed",
    "pane.updated",
    "pane.exited",
    "pane.moved",
    "pane.agent_detected",
    "tab.created",
    "tab.closed",
    "tab.renamed",
    "tab.moved",
    "workspace.created",
    "workspace.closed",
    "workspace.renamed",
    "workspace.moved",
];

#[derive(Debug, Clone)]
pub struct HerdrConfig {
    pub binary: String,
    pub backoff: Backoff,
    /// Re-snapshot even when no event arrived; also the reconnect probe for a dead socket.
    pub poll_interval: Duration,
    /// A burst of events after one structural change collapses into a single snapshot.
    pub settle: Duration,
}

impl Default for HerdrConfig {
    fn default() -> Self {
        Self {
            binary: "herdr".into(),
            backoff: Backoff::default(),
            poll_interval: Duration::from_secs(5),
            settle: Duration::from_millis(60),
        }
    }
}

struct Inner {
    herdr: Herdr,
    config: HerdrConfig,
    snapshot: watch::Sender<Arc<Snapshot>>,
    revision: watch::Sender<u64>,
}

pub struct HerdrProvider {
    inner: Arc<Inner>,
    topology_task: tokio::task::JoinHandle<()>,
}

impl Drop for HerdrProvider {
    fn drop(&mut self) {
        self.topology_task.abort();
    }
}

impl HerdrProvider {
    pub async fn connect(herdr: Herdr, config: HerdrConfig) -> Result<Self> {
        let snapshot = herdr
            .snapshot()
            .await
            .with_context(|| format!("herdr socket {}", herdr.socket().display()))?;
        Ok(Self::from_snapshot(herdr, config, snapshot))
    }

    fn from_snapshot(herdr: Herdr, config: HerdrConfig, snapshot: Snapshot) -> Self {
        let (snap_tx, _) = watch::channel(Arc::new(snapshot));
        let (rev_tx, _) = watch::channel(0);
        let inner = Arc::new(Inner {
            herdr,
            config,
            snapshot: snap_tx,
            revision: rev_tx,
        });
        let topology_task = tokio::spawn(topology(inner.clone()));
        Self { inner, topology_task }
    }

    pub fn snapshot(&self) -> Arc<Snapshot> {
        self.inner.snapshot.borrow().clone()
    }

    pub async fn refresh(&self) -> Result<Arc<Snapshot>> {
        self.inner.refresh().await
    }

    pub fn herdr_version(&self) -> String {
        self.inner.snapshot.borrow().version.clone()
    }
}

impl Inner {
    async fn refresh(&self) -> Result<Arc<Snapshot>> {
        let snapshot = Arc::new(self.herdr.snapshot().await?);
        let changed = fingerprint(&self.snapshot.borrow()) != fingerprint(&snapshot);
        if changed {
            self.snapshot.send_replace(snapshot.clone());
            self.revision.send_modify(|r| *r += 1);
        }
        Ok(snapshot)
    }
}

#[async_trait]
impl Provider for HerdrProvider {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        let snapshot = self.inner.snapshot.borrow().clone();
        Ok(snapshot.panes.iter().map(|p| pane_info(&snapshot, p)).collect())
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        let (tx, rx) = mpsc::channel(64);
        let task = tokio::spawn(supervise(self.inner.clone(), pane_id.to_string(), tx));
        Ok(PaneStream::supervised(rx, task))
    }

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()> {
        match input {
            // `pane.send_text` takes a JSON string, so input must be UTF-8 representable.
            // Every escape sequence herdr's key grammar rejects is (probe #8/#9).
            Input::Bytes(bytes) => {
                let text = String::from_utf8(bytes)
                    .map_err(|_| anyhow::anyhow!("pane input must be valid UTF-8"))?;
                self.inner.herdr.send_text(pane_id, &text).await
            }
            Input::Keys(keys) => self.inner.herdr.send_keys(pane_id, &keys).await,
        }
    }

    async fn read_scrollback(&self, pane_id: &str) -> Result<Option<RawScrollback>> {
        let snapshot = self.inner.refresh().await?;
        let pane = snapshot.pane(pane_id).context("unknown pane")?;
        if !pane.scrollback_is_safe_to_read() {
            return Ok(None);
        }
        let scroll = pane.scroll.context("pane reported no scroll state")?;
        let (cols, _) = snapshot.geometry(pane_id).context("pane has no layout rect")?;
        let want = scroll.max_offset_from_bottom + scroll.viewport_rows;
        let read = self.inner.herdr.read_scrollback(pane_id, want).await?;
        Ok(Some(RawScrollback {
            text: read.text,
            cols: cols as u16,
            viewport_rows: scroll.viewport_rows as u16,
            scrollback_rows: scroll.max_offset_from_bottom as u32,
            truncated: read.truncated,
        }))
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.inner.revision.subscribe()
    }
}

fn pane_info(snapshot: &Snapshot, pane: &kampr_herdr::Pane) -> PaneInfo {
    let (cols, rows) = snapshot.geometry(&pane.pane_id).unwrap_or((0, 0));
    let workspace = snapshot
        .workspaces
        .iter()
        .find(|w| w.workspace_id == pane.workspace_id)
        .map(|w| w.label.clone().unwrap_or_else(|| w.number.to_string()));
    let tab = snapshot
        .tabs
        .iter()
        .find(|t| t.tab_id == pane.tab_id)
        .and_then(|t| t.label.clone());
    PaneInfo {
        pane_id: pane.pane_id.clone(),
        workspace,
        tab,
        cwd: pane.cwd.clone(),
        label: pane.label.clone(),
        agent: pane.agent.clone(),
        agent_status: AgentStatus::from(pane.agent_status),
        cols: cols as u16,
        rows: rows as u16,
        scrollback_rows: if pane.scrollback_is_safe_to_read() {
            pane.scroll.map_or(0, |s| s.max_offset_from_bottom as u32)
        } else {
            0
        },
    }
}

fn fingerprint(snapshot: &Snapshot) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for p in &snapshot.panes {
        p.pane_id.hash(&mut h);
        p.workspace_id.hash(&mut h);
        p.tab_id.hash(&mut h);
        p.cwd.hash(&mut h);
        p.label.hash(&mut h);
        p.agent.hash(&mut h);
        (p.agent_status as u8).hash(&mut h);
        if let Some(s) = p.scroll {
            (s.max_offset_from_bottom, s.viewport_rows).hash(&mut h);
        }
    }
    for l in &snapshot.layouts {
        for lp in &l.panes {
            lp.pane_id.hash(&mut h);
            (lp.rect.width, lp.rect.height).hash(&mut h);
        }
    }
    h.finish()
}

async fn topology(inner: Arc<Inner>) {
    let mut backoff = inner.config.backoff.start();
    loop {
        if let Err(e) = inner.refresh().await {
            debug!(error = %e, "herdr snapshot failed; retrying");
            backoff.sleep().await;
            continue;
        }
        match inner.herdr.subscribe(TOPOLOGY_EVENTS).await {
            Ok(mut sub) => {
                backoff.reset();
                // `read_line` is not cancel-safe, so the subscription gets its own task and the
                // poll races a channel receive instead of the socket read itself.
                let (events, mut rx) = mpsc::channel::<()>(8);
                let reader = tokio::spawn(async move {
                    while let Ok(Some(_)) = sub.next().await {
                        if events.send(()).await.is_err() {
                            return;
                        }
                    }
                });
                let mut poll = tokio::time::interval(inner.config.poll_interval);
                poll.tick().await;
                loop {
                    let live = tokio::select! {
                        event = rx.recv() => {
                            if event.is_none() {
                                break;
                            }
                            tokio::time::sleep(inner.config.settle).await;
                            true
                        }
                        _ = poll.tick() => true,
                    };
                    if !live || inner.refresh().await.is_err() {
                        break;
                    }
                }
                reader.abort();
                debug!("herdr event subscription ended; reconnecting");
            }
            Err(e) => debug!(error = %e, "events.subscribe failed"),
        }
        backoff.sleep().await;
    }
}

enum Stop {
    Closed(String),
    GeometryChanged,
    ConsumerGone,
}

/// Owns restart. `terminal.closed` is routine — a pane that runs `clear`, a herdr restart, a desk
/// resize — so every one of them comes back as a `Reset`, never an error.
async fn supervise(inner: Arc<Inner>, pane_id: String, tx: mpsc::Sender<PaneEvent>) {
    let mut snapshots = inner.snapshot.subscribe();
    let mut backoff = inner.config.backoff.start();
    loop {
        let Some((cols, rows)) = resolve_geometry(&pane_id, &mut snapshots).await else {
            return;
        };
        if tx.send(PaneEvent::Reset { cols, rows }).await.is_err() {
            return;
        }
        let observer = Observer::spawn(
            &inner.config.binary,
            inner.herdr.socket(),
            &pane_id,
            cols as u32,
            rows as u32,
        );
        let mut observer = match observer {
            Ok(o) => o,
            Err(e) => {
                warn!(pane = %pane_id, error = %e, "could not spawn observe");
                backoff.sleep().await;
                continue;
            }
        };
        let (stop, streamed) = run_observer(&mut observer, &tx, &mut snapshots, &pane_id, (cols, rows)).await;
        observer.shutdown().await;
        if streamed {
            backoff.reset();
        }
        match stop {
            Stop::ConsumerGone => return,
            Stop::GeometryChanged => debug!(pane = %pane_id, "native geometry changed; restarting"),
            Stop::Closed(reason) => {
                debug!(pane = %pane_id, %reason, "observer closed; restarting");
                backoff.sleep().await;
            }
        }
    }
}

async fn resolve_geometry(
    pane_id: &str,
    snapshots: &mut watch::Receiver<Arc<Snapshot>>,
) -> Option<(u16, u16)> {
    loop {
        if let Some((c, r)) = snapshots.borrow_and_update().geometry(pane_id)
            && c > 0
            && r > 0
        {
            return Some((c as u16, r as u16));
        }
        snapshots.changed().await.ok()?;
    }
}

async fn run_observer(
    observer: &mut Observer,
    tx: &mpsc::Sender<PaneEvent>,
    snapshots: &mut watch::Receiver<Arc<Snapshot>>,
    pane_id: &str,
    geometry: (u16, u16),
) -> (Stop, bool) {
    let mut streamed = false;
    loop {
        tokio::select! {
            event = observer.events.recv() => match event {
                Some(StreamEvent::Frame { full, bytes, .. }) => {
                    streamed = true;
                    if tx.send(PaneEvent::Bytes { full, bytes }).await.is_err() {
                        return (Stop::ConsumerGone, streamed);
                    }
                }
                Some(StreamEvent::Closed { reason }) => return (Stop::Closed(reason), streamed),
                None => return (Stop::Closed("observe exited".into()), streamed),
            },
            changed = snapshots.changed() => {
                if changed.is_err() {
                    return (Stop::ConsumerGone, streamed);
                }
                let now = snapshots.borrow_and_update().geometry(pane_id);
                if let Some((c, r)) = now
                    && (c as u16, r as u16) != geometry
                    && c > 0
                    && r > 0
                {
                    return (Stop::GeometryChanged, streamed);
                }
            }
        }
    }
}

pub async fn connect_with_retry(herdr: Herdr, config: HerdrConfig) -> Result<HerdrProvider> {
    let mut backoff = config.backoff.start();
    let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
    loop {
        match HerdrProvider::connect(herdr.clone(), config.clone()).await {
            Ok(p) => return Ok(p),
            Err(e) if tokio::time::Instant::now() < deadline => {
                debug!(error = %e, "waiting for herdr");
                backoff.sleep().await;
            }
            Err(e) => bail!(e),
        }
    }
}
