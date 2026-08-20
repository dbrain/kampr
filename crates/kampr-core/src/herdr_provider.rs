use crate::backoff::Backoff;
use crate::provider::{AgentStatus, Input, PaneEvent, PaneInfo, PaneStream, Provider, RawScrollback};
use anyhow::{Context, Result};
use async_trait::async_trait;
use kampr_herdr::{Herdr, Observer, Snapshot, StreamEvent, Sub, rpc::Subscription};
use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

/// **Nothing here fires when the desk client resizes** (probe #52): `layout.updated` covers
/// structural change only, and a native geometry change is detectable *only* by polling. The
/// `poll_interval` below is therefore load-bearing, not a backstop — drop it and a resized pane
/// keeps streaming at a stale width forever.
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

/// The one event the whole triage story rests on, and the one that cannot be subscribed to for
/// the session as a whole: herdr requires a `pane_id`, and a single entry without one rejects the
/// entire `events.subscribe` call (probe #54). So it is subscribed once per agent pane, and the
/// list is rebuilt whenever the agent-pane set moves.
const STATUS_EVENT: &str = "pane.agent_status_changed";

/// Well past herdr's 1000-line read cap; over-asking clamps rather than failing.
const READ_CEILING: u64 = 4096;

#[derive(Debug, Clone)]
pub struct HerdrConfig {
    pub binary: String,
    pub backoff: Backoff,
    /// The only way a desk resize is ever noticed (probe #52), and the reconnect probe for a
    /// dead socket.
    pub poll_interval: Duration,
    /// A burst of events after one structural change collapses into a single snapshot.
    pub settle: Duration,
    /// How often a live stream re-measures the pane's true PTY width. Nothing announces a
    /// PTY/rect divergence either (probe #68), so this is the only thing that notices one.
    pub width_poll: Duration,
    /// The floor between two `events.subscribe` calls.
    ///
    /// `pane.agent_status_changed` is per pane, so opening a workspace of ten agents changes the
    /// subscription set ten times. Without this each change is a fresh socket and a fresh
    /// subscribe, which is a burst aimed at herdr for no gain: the poll is still the source of
    /// truth, so collapsing a burst costs one interval of event latency on the new panes and
    /// nothing else.
    pub resubscribe_min: Duration,
}

impl Default for HerdrConfig {
    fn default() -> Self {
        Self {
            binary: "herdr".into(),
            backoff: Backoff::default(),
            poll_interval: Duration::from_secs(3),
            settle: Duration::from_millis(60),
            width_poll: Duration::from_secs(3),
            resubscribe_min: Duration::from_millis(500),
        }
    }
}

/// Whether herdr is answering, and whether it ever has.
///
/// `ever` is the difference between "the herd is empty because everything closed" and "the herd
/// is empty because this node has never reached a herdr at all", and a client cannot say why
/// without being told which.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Health {
    pub online: bool,
    pub ever: bool,
    pub detail: Option<String>,
}

struct Inner {
    herdr: Herdr,
    config: HerdrConfig,
    snapshot: watch::Sender<Arc<Snapshot>>,
    revision: watch::Sender<u64>,
    health: watch::Sender<Health>,
    widths: Mutex<HashMap<String, Measured>>,
}

/// What this pane's width readings have established while the rect has held still.
///
/// `floor` is a running max rather than the latest sample: terminal content shrinks between
/// repaints, and one narrow reading must not restart the stream at a width that crops the next
/// wide line. `exact` is the newest *proven* width and overrides the floor and the rect both.
/// Cleared when the rect moves, which is the only cue a node gets that the pane may have been
/// re-sized underneath it.
#[derive(Debug, Clone, Copy, Default)]
struct Measured {
    rect: u16,
    floor: u16,
    exact: Option<u16>,
}

impl Measured {
    fn cols(&self) -> u16 {
        self.exact.unwrap_or_else(|| self.rect.max(self.floor))
    }
}

/// One pair of reads, resolved into what it proves.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Reading {
    /// A lower bound: herdr trims each rendered row, so short content reads narrow.
    floor: u16,
    /// The width itself, when the reads prove it.
    exact: Option<u16>,
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
    /// **Never fails and never blocks on herdr.** The connection is a supervised loop that
    /// retries for as long as the process lives, so a node binds its port and serves its own
    /// "herdr is not running" state instead of exiting into a restart loop.
    pub fn spawn(herdr: Herdr, config: HerdrConfig) -> Self {
        let (snap_tx, _) = watch::channel(Arc::new(Snapshot::empty()));
        let (rev_tx, _) = watch::channel(0);
        let (health_tx, _) = watch::channel(Health::default());
        let inner = Arc::new(Inner {
            herdr,
            config,
            snapshot: snap_tx,
            revision: rev_tx,
            health: health_tx,
            widths: Mutex::new(HashMap::new()),
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

    pub fn health(&self) -> Health {
        self.inner.health.borrow().clone()
    }

    pub fn watch_health(&self) -> watch::Receiver<Health> {
        self.inner.health.subscribe()
    }

    /// `None` until herdr has answered once — an unknown version rather than a fabricated one.
    pub fn herdr_version(&self) -> Option<String> {
        let version = self.inner.snapshot.borrow().version.clone();
        (!version.is_empty()).then_some(version)
    }
}

impl Inner {
    async fn refresh(&self) -> Result<Arc<Snapshot>> {
        let snapshot = match self.herdr.snapshot().await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                self.went_offline(&e);
                return Err(e).with_context(|| format!("herdr socket {}", self.herdr.socket().display()));
            }
        };
        self.came_online();
        let changed = fingerprint(&self.snapshot.borrow()) != fingerprint(&snapshot);
        if changed {
            self.snapshot.send_replace(snapshot.clone());
            self.revision.send_modify(|r| *r += 1);
        }
        Ok(snapshot)
    }

    /// The log line an operator sees at the default level, once per outage rather than once per
    /// retry — a socket that has been down for a week must not be a week of identical warnings.
    fn went_offline(&self, error: &anyhow::Error) {
        let detail = format!("{}: {error}", self.herdr.socket().display());
        let first = {
            let h = self.health.borrow();
            h.online || h.detail.is_none()
        };
        self.health.send_if_modified(|h| {
            let changed = h.online;
            h.online = false;
            h.detail = Some(detail);
            changed
        });
        if first {
            warn!(
                socket = %self.herdr.socket().display(),
                error = %error,
                "herdr is not reachable; will keep retrying on the poll loop. Is the Herdr server running?"
            );
        }
    }

    fn came_online(&self) {
        let recovered = !self.health.borrow().online;
        self.health.send_if_modified(|h| {
            let changed = !h.online || !h.ever;
            h.online = true;
            h.ever = true;
            h.detail = None;
            changed
        });
        if recovered {
            info!(socket = %self.herdr.socket().display(), "herdr is answering");
        }
    }

    /// Probe #68/#84: in a headless session the PTY does not follow the layout rect, so the rect
    /// is fiction and observing at it crops every row. The reads render at the **true** PTY
    /// width, so they, not the rect, are what an observe stream is sized from.
    async fn observe_cols(&self, pane_id: &str, rect: u16) -> u16 {
        let reading = self.read_width(pane_id).await;
        let mut widths = self.widths.lock().unwrap();
        let entry = widths.entry(pane_id.to_string()).or_default();
        if entry.rect != rect {
            *entry = Measured {
                rect,
                ..Measured::default()
            };
        }
        entry.floor = entry.floor.max(reading.floor);
        // The newest proof wins: an older one described a pane that may since have been resized.
        entry.exact = reading.exact.or(entry.exact);
        entry.cols()
    }

    async fn read_width(&self, pane_id: &str) -> Reading {
        let rows = self
            .snapshot
            .borrow()
            .pane(pane_id)
            .and_then(|p| p.scroll)
            .map_or(0, |s| s.viewport_rows);
        if rows == 0 {
            return Reading::default();
        }
        match self.herdr.read_wrapped_and_logical(pane_id, rows).await {
            Ok((physical, logical)) => reading(&physical.text, &logical.text),
            Err(e) => {
                debug!(pane = %pane_id, error = %e, "could not measure the pane width");
                Reading::default()
            }
        }
    }

    /// The corrected width without touching the socket, for callers that only have a snapshot.
    fn known_cols(&self, pane_id: &str, rect: u16) -> u16 {
        let widths = self.widths.lock().unwrap();
        match widths.get(pane_id) {
            Some(m) if m.rect == rect => m.cols(),
            _ => rect,
        }
    }
}

/// Resolves one pair of reads into a width.
///
/// `recent` returns *physical* rows, wrapped at the PTY's own width; `recent_unwrapped` returns
/// the logical lines they came from. A logical line longer than the widest physical row proves a
/// soft wrap happened, and a soft wrap happens at exactly the PTY width — so that widest row is
/// the width, whatever the layout rect claims.
///
/// Without a wrap there is no proof, only a floor: herdr trims each row's trailing blanks, so a
/// screen holding a short prompt reads narrow. A floor is never an over-estimate, which is what
/// makes combining it with the rect by `max` safe.
fn reading(physical: &str, logical: &str) -> Reading {
    let floor = widest_row(physical);
    let wrapped = logical.lines().any(|line| line.chars().count() > floor as usize);
    Reading {
        floor,
        exact: (wrapped && floor > 0).then_some(floor),
    }
}

fn widest_row(text: &str) -> u16 {
    text.lines()
        .map(|line| line.chars().count().min(u16::MAX as usize) as u16)
        .max()
        .unwrap_or(0)
}

#[async_trait]
impl Provider for HerdrProvider {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        let snapshot = self.inner.snapshot.borrow().clone();
        Ok(snapshot
            .panes
            .iter()
            .map(|p| pane_info(&self.inner, &snapshot, p))
            .collect())
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
        let snapshot = self.inner.snapshot.borrow().clone();
        let pane = snapshot.pane(pane_id).context("unknown pane")?;
        if !pane.scrollback_is_safe_to_read() {
            return Ok(None);
        }
        let scroll = pane.scroll.context("pane reported no scroll state")?;
        // The ring is re-wrapped at this width, so it has to be the width the PTY actually
        // wrapped at, not the rect (probe #68).
        let (rect, _) = snapshot.geometry(pane_id).context("pane has no layout rect")?;
        let cols = self.inner.known_cols(pane_id, rect as u16);
        // Over-asking clamps to herdr's own cap (probe #51), so the request is deliberately far
        // past it: `truncated` then means "history exists above this", independent of how fresh
        // the cached snapshot's ring depth happens to be.
        let read = self
            .inner
            .herdr
            .read_scrollback(pane_id, READ_CEILING + scroll.viewport_rows)
            .await?;
        Ok(Some(RawScrollback {
            text: read.text,
            cols,
            viewport_rows: scroll.viewport_rows as u16,
            truncated: read.truncated,
        }))
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.inner.revision.subscribe()
    }
}

fn pane_info(inner: &Inner, snapshot: &Snapshot, pane: &kampr_herdr::Pane) -> PaneInfo {
    let (rect, rows) = snapshot.geometry(&pane.pane_id).unwrap_or((0, 0));
    // A watched pane has a measured width; an unwatched one has only the rect, which is what
    // `herd` has always carried.
    let cols = inner.known_cols(&pane.pane_id, rect as u16);
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
        workspace_id: Some(pane.workspace_id.clone()),
        tab_id: Some(pane.tab_id.clone()),
        workspace,
        tab,
        cwd: pane.cwd.clone(),
        label: pane.label.clone(),
        agent: pane.agent.clone(),
        agent_status: AgentStatus::from(pane.agent_status),
        cols,
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

/// The panes a `pane.agent_status_changed` subscription has to name, sorted so two snapshots
/// that carry the same agents compare equal whatever order herdr listed them in.
fn agent_panes(snapshot: &Snapshot) -> Vec<String> {
    let mut ids: Vec<String> = snapshot
        .panes
        .iter()
        .filter(|p| p.is_agent())
        .map(|p| p.pane_id.clone())
        .collect();
    ids.sort();
    ids
}

/// The whole subscription list: the session-wide topology kinds, plus one status entry per agent
/// pane. Building it in one place is what keeps probe #54 from coming back — every entry that
/// needs a `pane_id` gets one here or not at all.
pub fn subscriptions(agents: &[String]) -> Vec<Sub> {
    TOPOLOGY_EVENTS
        .iter()
        .map(|kind| Sub::kind(kind))
        .chain(agents.iter().map(|id| Sub::pane(STATUS_EVENT, id)))
        .collect()
}

/// Why the inner loop gave up its subscription.
enum Ended {
    /// The socket closed or a refresh failed — reconnect, with backoff.
    Broken,
    /// The agent-pane set moved, so the status entries are stale. Not a fault, so no backoff.
    PaneSetChanged,
}

async fn topology(inner: Arc<Inner>) {
    let mut backoff = inner.config.backoff.start();
    loop {
        if let Err(e) = inner.refresh().await {
            debug!(error = %e, "herdr snapshot failed; retrying");
            backoff.sleep().await;
            continue;
        }
        let agents = agent_panes(&inner.snapshot.borrow());
        match inner.herdr.subscribe(&subscriptions(&agents)).await {
            Ok(sub) => {
                backoff.reset();
                let subscribed_at = tokio::time::Instant::now();
                let ended = follow(&inner, sub, &agents).await;
                match ended {
                    Ended::PaneSetChanged => {
                        // Collapse a burst. A workspace opening ten agent panes moves the set ten
                        // times, and each move would otherwise be its own socket and its own
                        // subscribe. The poll is still the source of truth, so waiting out the
                        // window costs latency on the new panes and nothing else.
                        tokio::time::sleep_until(subscribed_at + inner.config.resubscribe_min).await;
                        continue;
                    }
                    Ended::Broken => debug!("herdr event subscription ended; reconnecting"),
                }
            }
            // A pane that closed between the snapshot and the subscribe answers `pane_not_found`
            // and takes the whole call with it (probe #89). That is a race, not a fault, and the
            // next pass re-derives the set from a fresh snapshot.
            Err(e) => debug!(error = %e, "events.subscribe failed"),
        }
        backoff.sleep().await;
    }
}

/// Drives one subscription until it breaks or the pane set it named goes stale.
///
/// **Events poke the poll; they never replace it.** Every event ends in the same `refresh` the
/// timer would have done, so a missed event costs one interval and never correctness — which is
/// what makes the per-pane status subscription safe to lose and safe to rebuild.
async fn follow(inner: &Arc<Inner>, mut sub: Subscription, agents: &[String]) -> Ended {
    // `read_line` is not cancel-safe, so the subscription gets its own task and the poll races a
    // channel receive instead of the socket read itself.
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
    let ended = loop {
        let live = tokio::select! {
            event = rx.recv() => {
                if event.is_none() {
                    break Ended::Broken;
                }
                tokio::time::sleep(inner.config.settle).await;
                true
            }
            _ = poll.tick() => true,
        };
        if !live || inner.refresh().await.is_err() {
            break Ended::Broken;
        }
        if agent_panes(&inner.snapshot.borrow()) != agents {
            break Ended::PaneSetChanged;
        }
    };
    reader.abort();
    ended
}

enum Stop {
    Closed(String),
    GeometryChanged,
    /// The PTY turned out to be wider than the stream was sized for — probe #68, where the rect
    /// is fiction and the divergence only shows once content fills the real width.
    WidthChanged {
        was: u16,
        now: u16,
    },
    ConsumerGone,
}

/// The rect the layout claims, the width the stream actually runs at, and the rows. The first two
/// differ whenever the PTY did not follow the rect, so change detection has to watch the rect
/// while the observer runs at the measured width.
#[derive(Debug, Clone, Copy)]
struct Geometry {
    rect: u16,
    cols: u16,
    rows: u16,
}

/// Owns restart. `terminal.closed` is routine — a pane that runs `clear`, a herdr restart, a desk
/// resize — so every one of them comes back as a `Reset`, never an error.
async fn supervise(inner: Arc<Inner>, pane_id: String, tx: mpsc::Sender<PaneEvent>) {
    let mut snapshots = inner.snapshot.subscribe();
    let mut backoff = inner.config.backoff.start();
    loop {
        let Some((rect, rows)) = resolve_geometry(&pane_id, &mut snapshots).await else {
            return;
        };
        let cols = inner.observe_cols(&pane_id, rect).await;
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
        // The probe is its own task: two socket round-trips inside the stream loop would stall
        // the frames it is meant to be sizing.
        let (width_tx, width_rx) = watch::channel(cols);
        let prober = tokio::spawn(probe_width(inner.clone(), pane_id.clone(), rect, width_tx));
        let (stop, streamed) = run_observer(
            &mut observer,
            &tx,
            &mut snapshots,
            width_rx,
            &pane_id,
            Geometry { rect, cols, rows },
        )
        .await;
        prober.abort();
        observer.shutdown().await;
        if streamed {
            backoff.reset();
        }
        match stop {
            Stop::ConsumerGone => return,
            Stop::GeometryChanged => debug!(pane = %pane_id, "native geometry changed; restarting"),
            Stop::WidthChanged { was, now } => {
                debug!(pane = %pane_id, was, now, "the pane is wider than the rect claimed; restarting")
            }
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

/// Re-measures the pane's true width while its stream runs. Nothing announces a PTY/rect
/// divergence (probe #68), so this poll is the only thing that notices content growing past the
/// width the stream was started at.
async fn probe_width(inner: Arc<Inner>, pane_id: String, rect: u16, tx: watch::Sender<u16>) {
    let mut poll = tokio::time::interval(inner.config.width_poll);
    poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
    poll.tick().await;
    loop {
        poll.tick().await;
        let cols = inner.observe_cols(&pane_id, rect).await;
        if tx.send(cols).is_err() {
            return;
        }
    }
}

async fn run_observer(
    observer: &mut Observer,
    tx: &mpsc::Sender<PaneEvent>,
    snapshots: &mut watch::Receiver<Arc<Snapshot>>,
    mut width: watch::Receiver<u16>,
    pane_id: &str,
    geometry: Geometry,
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
                    && (c as u16, r as u16) != (geometry.rect, geometry.rows)
                    && c > 0
                    && r > 0
                {
                    return (Stop::GeometryChanged, streamed);
                }
            }
            changed = width.changed() => {
                if changed.is_err() {
                    return (Stop::ConsumerGone, streamed);
                }
                let now = *width.borrow_and_update();
                if now != geometry.cols {
                    return (Stop::WidthChanged { was: geometry.cols, now }, streamed);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Measured, Reading, reading};
    use super::{STATUS_EVENT, TOPOLOGY_EVENTS, agent_panes, subscriptions};
    use kampr_herdr::Snapshot;

    fn snapshot(panes: &[(&str, Option<&str>)]) -> Snapshot {
        let json = serde_json::json!({
            "version": "0.8.2",
            "protocol": 20,
            "focused_pane_id": null,
            "panes": panes.iter().map(|(id, agent)| serde_json::json!({
                "pane_id": id,
                "workspace_id": "w1",
                "tab_id": "w1:t1",
                "cwd": null,
                "label": null,
                "agent": agent,
                "agent_status": "idle",
                "agent_session": null,
                "scroll": null,
            })).collect::<Vec<_>>(),
        });
        serde_json::from_value(json).expect("snapshot fixture")
    }

    /// Probe #54: one entry missing a required `pane_id` rejects the whole `events.subscribe`
    /// call, which is why the status event was unsubscribed at all. Every status entry has to
    /// carry one.
    #[test]
    fn every_status_entry_names_a_pane_and_no_topology_entry_does() {
        let subs = subscriptions(&["w1:p1".to_string(), "w3:p2".to_string()]);
        assert_eq!(subs.len(), TOPOLOGY_EVENTS.len() + 2);
        for sub in &subs {
            if sub.kind == STATUS_EVENT {
                assert!(sub.pane_id.is_some(), "{STATUS_EVENT} without a pane_id");
            } else {
                assert_eq!(sub.pane_id, None, "{} must not carry one", sub.kind);
            }
        }
        let named: Vec<&str> = subs
            .iter()
            .filter(|s| s.kind == STATUS_EVENT)
            .filter_map(|s| s.pane_id.as_deref())
            .collect();
        assert_eq!(named, ["w1:p1", "w3:p2"]);
    }

    #[test]
    fn a_herd_with_no_agents_still_subscribes_to_topology() {
        assert_eq!(subscriptions(&[]).len(), TOPOLOGY_EVENTS.len());
    }

    /// The resubscribe trigger. Shell panes are not agents and must not move it, or every new
    /// terminal costs a fresh socket for a status event that can never fire.
    #[test]
    fn only_agent_panes_move_the_subscription_set() {
        let shell = snapshot(&[("w1:p1", Some("claude")), ("w1:p2", None)]);
        assert_eq!(agent_panes(&shell), ["w1:p1"]);

        let another_shell = snapshot(&[("w1:p1", Some("claude")), ("w1:p2", None), ("w1:p3", None)]);
        assert_eq!(
            agent_panes(&another_shell),
            agent_panes(&shell),
            "a new shell pane is not a resubscribe"
        );

        let promoted = snapshot(&[("w1:p1", Some("claude")), ("w1:p2", Some("codex"))]);
        assert_ne!(
            agent_panes(&promoted),
            agent_panes(&shell),
            "a shell that became an agent is"
        );
    }

    /// Herdr does not promise an order, and an order change is not a pane-set change — comparing
    /// unsorted lists would resubscribe on every poll.
    #[test]
    fn the_pane_set_is_order_independent() {
        let a = snapshot(&[("w1:p2", Some("codex")), ("w1:p1", Some("claude"))]);
        let b = snapshot(&[("w1:p1", Some("claude")), ("w1:p2", Some("codex"))]);
        assert_eq!(agent_panes(&a), agent_panes(&b));
    }

    fn wide(width: usize, total: usize) -> (String, String) {
        let logical = "#".repeat(total);
        let mut physical: Vec<String> = logical
            .as_bytes()
            .chunks(width)
            .map(|c| String::from_utf8(c.to_vec()).unwrap())
            .collect();
        physical.insert(0, "$ printf".into());
        (physical.join("\n"), format!("$ printf\n{logical}"))
    }

    #[test]
    fn a_wrapped_line_proves_the_width_the_rect_is_lying_about() {
        // Measured live: rect 47 after a split, PTY still 93 (probes #68, #84).
        let (physical, logical) = wide(93, 400);
        assert_eq!(
            reading(&physical, &logical),
            Reading {
                floor: 93,
                exact: Some(93)
            }
        );
        let m = Measured {
            rect: 47,
            floor: 93,
            exact: Some(93),
        };
        assert_eq!(m.cols(), 93, "the proof beats the rect in both directions");
    }

    #[test]
    fn short_content_proves_nothing_and_falls_back_to_the_rect() {
        let screen = "$ stty size\n40 93";
        assert_eq!(
            reading(screen, screen),
            Reading {
                floor: 11,
                exact: None
            },
            "nothing wrapped, so the widest row is only a floor"
        );
        let m = Measured {
            rect: 94,
            floor: 11,
            exact: None,
        };
        assert_eq!(m.cols(), 94, "and a floor never narrows the stream");
        assert_eq!(reading("", ""), Reading::default());
    }

    /// Probe #69: even unsplit the rect is one column wider than the PTY, and the same proof
    /// fixes it — `observe` padded that column rather than cropping, but the grid was still a
    /// column wider than the pane.
    #[test]
    fn a_proof_below_the_rect_is_still_the_truth() {
        let (physical, logical) = wide(93, 400);
        let r = reading(&physical, &logical);
        assert_eq!(
            Measured {
                rect: 94,
                floor: r.floor,
                exact: r.exact,
            }
            .cols(),
            93
        );
    }

    #[test]
    fn a_floor_wider_than_the_rect_still_widens_the_stream() {
        // No wrap witness yet, but content already exceeds the rect: never crop what has been
        // seen, and let the next poll prove the rest.
        let m = Measured {
            rect: 47,
            floor: 80,
            exact: None,
        };
        assert_eq!(m.cols(), 80);
    }
}
