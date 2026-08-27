use crate::provider::{Input, PaneEvent, PaneInfo, PaneStream, Provider};
use crate::scrollback::{Ingest, ScrollbackDoc, ScrollbackRing};
use crate::wire::Cursor;
use anyhow::Result;
use kampr_term::{Emulator, RowDiff};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::Notify;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;
use tokio::time::Instant;
use tracing::{debug, info, warn};

#[derive(Debug, Clone)]
pub enum PaneUpdate {
    Reset {
        cols: u16,
        rows: u16,
        rows_data: Arc<Vec<RowDiff>>,
        cursor: Cursor,
        links: Arc<Vec<String>>,
    },
    Patch {
        rows: Arc<Vec<RowDiff>>,
        cursor: Cursor,
        new_links: Arc<Vec<String>>,
    },
}

impl PaneUpdate {
    pub fn rows(&self) -> &[RowDiff] {
        match self {
            Self::Reset { rows_data, .. } => rows_data,
            Self::Patch { rows, .. } => rows,
        }
    }

    pub fn geometry(&self) -> Option<(u16, u16)> {
        match self {
            Self::Reset { cols, rows, .. } => Some((*cols, *rows)),
            Self::Patch { .. } => None,
        }
    }

    pub fn is_reset(&self) -> bool {
        matches!(self, Self::Reset { .. })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum WatchError {
    #[error("the pane is no longer being watched")]
    Closed,
}

#[derive(Debug, Clone, Copy)]
pub struct RegistryConfig {
    /// Per-pane fan-out depth. Overflowing it costs a watcher one `grid.reset`, never a stall.
    pub broadcast_capacity: usize,
    /// How long a pending reset waits for its first frame before publishing anyway, so a pane
    /// that produces no output still reports its geometry.
    pub reset_flush_after: Duration,
    /// How long the very first watcher waits for real geometry, so no client is ever handed the
    /// placeholder grid that exists before the provider's first `Reset`.
    pub first_grid_wait: Duration,
    pub history: HistoryPolicy,
    pub scrollback_max_rows: usize,
}

/// How often a watched pane's history is re-read.
///
/// Successive reads must overlap or the ring cannot be stitched, and they stop overlapping the
/// moment more than herdr's 1000-row read cap accumulates between them (probe #51). The gap
/// threshold is therefore `1000 / rows-per-second`, and the cadence is derived from a *measured*
/// row rate rather than a guess: the growth of the ring between two reads is exactly the quantity
/// that matters. Frame content cannot supply it — herdr coalesces a burst to end state, so
/// `seq 1 20000` arrives as three frames (probe #23/#25) and counting newlines in them would
/// under-count by three orders of magnitude.
#[derive(Debug, Clone, Copy)]
pub struct HistoryPolicy {
    /// Rows allowed to accumulate between reads. Well under the 1000-row cap, because the rate
    /// estimate always lags the pane by one interval.
    pub row_budget: u32,
    pub fastest: Duration,
    /// Ceiling while the pane is producing frames at all.
    pub quiet: Duration,
    /// A pane producing nothing is not polled on this timer at all — it waits here to be woken
    /// by a frame, and this is only the backstop.
    pub idle: Duration,
}

impl Default for HistoryPolicy {
    fn default() -> Self {
        Self {
            row_budget: 400,
            fastest: Duration::from_millis(100),
            quiet: Duration::from_secs(2),
            idle: Duration::from_secs(30),
        }
    }
}

impl HistoryPolicy {
    /// `row_budget` rows' worth of time at the estimated rate, bounded at both ends.
    ///
    /// Below the `fastest` clamp the pane is producing faster than any cadence can follow and a
    /// gap is unavoidable; above it, the returned interval admits `row_budget` rows, which is the
    /// margin against herdr's 1000-row cap.
    pub fn interval_for_rate(&self, rows_per_sec: f64) -> Duration {
        if rows_per_sec <= 0.0 {
            return self.quiet;
        }
        Duration::try_from_secs_f64(self.row_budget as f64 / rows_per_sec)
            .unwrap_or(self.quiet)
            .clamp(self.fastest, self.quiet)
    }
}

/// Smoothed rows-per-second.
///
/// Terminal output is bursty — a shell loop emits three hundred rows in ten milliseconds and then
/// waits — so a single sample is worthless. An unsmoothed estimate reads zero the moment a poll
/// lands between bursts and drops the cadence back to `quiet` mid-stream, which is exactly how a
/// gap happens. The average decays instead.
#[derive(Debug, Clone, Copy, Default)]
pub struct RowRate {
    average: f64,
}

impl RowRate {
    const ALPHA: f64 = 0.4;
    /// Samples shorter than this measure scheduling noise, not the pane.
    const FLOOR: f64 = 0.001;

    pub fn observe(&mut self, rows: usize, elapsed: Duration) -> f64 {
        let seconds = elapsed.as_secs_f64();
        if seconds < Self::FLOOR {
            return self.average;
        }
        let sample = rows as f64 / seconds;
        self.average = if self.average == 0.0 {
            sample
        } else {
            Self::ALPHA * sample + (1.0 - Self::ALPHA) * self.average
        };
        self.average
    }

    pub fn get(&self) -> f64 {
        self.average
    }
}

/// What the adaptive poller is currently doing. Diagnostics only; nothing on the wire.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HistoryStatus {
    pub poll: Duration,
    pub rows_per_sec: f64,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            broadcast_capacity: 256,
            reset_flush_after: Duration::from_millis(300),
            first_grid_wait: Duration::from_secs(2),
            history: HistoryPolicy::default(),
            scrollback_max_rows: crate::scrollback::DEFAULT_MAX_ROWS,
        }
    }
}

struct PaneState {
    term: Emulator,
    links_sent: usize,
    pending_reset: bool,
    ready: bool,
    cursor_sent: Cursor,
}

/// Output seen on the pane's frame stream. The poller waits on this rather than on a timer, so a
/// quiet pane costs one parked task and no socket traffic at all.
#[derive(Default)]
struct Activity {
    frames: AtomicU64,
    woken: Notify,
}

impl Activity {
    fn saw_frame(&self) {
        self.frames.fetch_add(1, Ordering::Relaxed);
        self.woken.notify_one();
    }

    fn count(&self) -> u64 {
        self.frames.load(Ordering::Relaxed)
    }
}

struct PaneEntry {
    pane_id: String,
    watchers: AtomicU64,
    state: Arc<Mutex<PaneState>>,
    history: Arc<Mutex<ScrollbackRing>>,
    status: Arc<Mutex<HistoryStatus>>,
    tx: broadcast::Sender<PaneUpdate>,
    /// Whether [`pump`] is still running. The entry owns a `Sender` of its own so the broadcast
    /// channel can outlive its feeder, which is what made a dead pump indistinguishable from a
    /// pane nobody is typing in: `recv` simply parked, for ever, while the socket carrying it
    /// stayed up and answered every other question correctly (#268, and #233's shape one layer
    /// in). The pump holds the other half of this and nothing else does, so it goes when the task
    /// does — including when the task panics or is aborted, which no explicit flag would catch.
    alive: tokio::sync::watch::Receiver<()>,
    tasks: [JoinHandle<()>; 2],
}

impl PaneEntry {
    fn feeding(&self) -> bool {
        self.alive.has_changed().is_ok()
    }
}

impl Drop for PaneEntry {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

/// One emulator per pane, shared by every viewer and refcounted by [`Watcher`]s. The provider
/// stream is opened on the first watcher and dropped with the last.
pub struct PaneRegistry {
    provider: Arc<dyn Provider>,
    config: RegistryConfig,
    panes: Mutex<HashMap<String, Weak<PaneEntry>>>,
    opening: tokio::sync::Mutex<()>,
    /// A revision, not a count: it only ever goes up, and a reader wants the edge.
    watcher_changes: Arc<tokio::sync::watch::Sender<u64>>,
}

impl PaneRegistry {
    pub fn new(provider: Arc<dyn Provider>) -> Arc<Self> {
        Self::with_config(provider, RegistryConfig::default())
    }

    pub fn with_config(provider: Arc<dyn Provider>, config: RegistryConfig) -> Arc<Self> {
        Arc::new(Self {
            provider,
            config,
            panes: Mutex::new(HashMap::new()),
            opening: tokio::sync::Mutex::new(()),
            watcher_changes: Arc::new(tokio::sync::watch::channel(0).0),
        })
    }

    pub async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        self.provider.list_panes().await
    }

    /// Bumps when the pane list or its geometry may have changed — the cue to re-read
    /// [`Self::list_panes`] and send a herd patch.
    pub fn topology(&self) -> tokio::sync::watch::Receiver<u64> {
        self.provider.topology()
    }

    pub async fn write(&self, pane_id: &str, input: Input) -> Result<()> {
        self.provider.write_pane(pane_id, input).await
    }

    /// Reads once more before rendering, so a client's history is current at the moment it asks
    /// and the ring gets one more chance to stitch.
    pub async fn scrollback(&self, pane_id: &str) -> Result<Option<ScrollbackDoc>> {
        let Some(raw) = self.provider.read_scrollback(pane_id).await? else {
            return Ok(None);
        };
        match self.lookup(pane_id) {
            Some(entry) => {
                let mut ring = entry.history.lock().unwrap();
                ring.ingest(&raw);
                Ok(Some(ring.render()))
            }
            None => {
                let mut ring = ScrollbackRing::new(self.config.scrollback_max_rows);
                ring.ingest(&raw);
                Ok(Some(ring.render()))
            }
        }
    }

    /// The pane's visible grid as plain text, one string per row with trailing blanks trimmed.
    ///
    /// The emulator only exists while somebody is streaming the pane, so this is `None` for an
    /// unwatched pane and never opens anything to answer. It is the same grid the client is
    /// looking at — no `pane.read`, no socket call, no second emulation.
    pub fn screen(&self, pane_id: &str) -> Option<Vec<String>> {
        let entry = self.lookup(pane_id)?;
        let state = entry.state.lock().unwrap();
        if !state.ready {
            return None;
        }
        let grid = state.term.grid();
        Some((0..grid.rows()).map(|r| grid.row_text(r)).collect())
    }

    /// What cadence the pane's history poller has settled on, and the row rate it measured.
    pub fn history_status(&self, pane_id: &str) -> Option<HistoryStatus> {
        self.lookup(pane_id).map(|e| *e.status.lock().unwrap())
    }

    /// Counted rather than read off the refcount, because a [`PaneHold`] is a strong reference
    /// that nobody is looking through: the herd carries this number.
    pub fn watcher_count(&self, pane_id: &str) -> usize {
        self.lookup(pane_id)
            .map_or(0, |e| e.watchers.load(Ordering::Relaxed) as usize)
    }

    /// Bumps whenever a viewer joins or leaves any pane.
    ///
    /// The herd carries `watchers`, and nothing else moves when a second phone opens a pane that
    /// is already streaming — no herdr event, no change to any snapshot. Without this the count
    /// would sit stale until the next reconciliation sweep, which is measured in tens of seconds.
    pub fn watchers_changed(&self) -> tokio::sync::watch::Receiver<u64> {
        self.watcher_changes.subscribe()
    }

    pub fn watched_panes(&self) -> Vec<String> {
        let mut panes = self.panes.lock().unwrap();
        panes.retain(|_, w| w.strong_count() > 0);
        panes.keys().cloned().collect()
    }

    pub async fn watch(&self, pane_id: &str) -> Result<Watcher> {
        if let Some(entry) = self.lookup(pane_id) {
            return Ok(self.attach(entry));
        }
        let _opening = self.opening.lock().await;
        if let Some(entry) = self.lookup(pane_id) {
            return Ok(self.attach(entry));
        }

        let stream = self.provider.watch_pane(pane_id).await?;
        let state = Arc::new(Mutex::new(PaneState {
            term: Emulator::new(1, 1),
            links_sent: 0,
            pending_reset: false,
            ready: false,
            cursor_sent: Cursor::default(),
        }));
        let (tx, _) = broadcast::channel(self.config.broadcast_capacity);
        let history = Arc::new(Mutex::new(ScrollbackRing::new(self.config.scrollback_max_rows)));
        let activity = Arc::new(Activity::default());
        let status = Arc::new(Mutex::new(HistoryStatus {
            poll: self.config.history.fastest,
            rows_per_sec: 0.0,
        }));
        let (alive_tx, alive) = tokio::sync::watch::channel(());
        let tasks = [
            tokio::spawn(pump(
                stream,
                state.clone(),
                tx.clone(),
                self.config.reset_flush_after,
                activity.clone(),
                alive_tx,
            )),
            tokio::spawn(accumulate_history(
                self.provider.clone(),
                pane_id.to_string(),
                history.clone(),
                status.clone(),
                activity,
                self.config.history,
            )),
        ];
        let entry = Arc::new(PaneEntry {
            pane_id: pane_id.to_string(),
            watchers: AtomicU64::new(0),
            state,
            history,
            status,
            tx,
            alive,
            tasks,
        });
        self.panes
            .lock()
            .unwrap()
            .insert(pane_id.to_string(), Arc::downgrade(&entry));
        // The lock is a create-once guard and nothing more, and the entry is in the map: a
        // concurrent `watch` for this pane now takes the fast path above. Held any longer it
        // serialises the first-frame wait across *every* pane — four silent panes opening at 2 s,
        // 4 s, 6 s and 8 s, on the reconnect where a client re-watches all of them at once.
        drop(_opening);
        let mut watcher = self.attach(entry);
        if let Ok(Ok(first)) = tokio::time::timeout(self.config.first_grid_wait, watcher.recv()).await
            && first.is_reset()
        {
            watcher.initial = first;
            watcher.ready = true;
        }
        Ok(watcher)
    }

    /// Re-watching a pane must not close it in between.
    ///
    /// The registry holds only a `Weak` to a pane, so the last [`Watcher`] dropping takes the
    /// entry with it — the emulator, the stitched ring, and the spawned `observe` behind them. A
    /// caller that stops its old watch before starting the new one *is* that last watcher, so
    /// what it opens next is a fresh pane: a 1x1 emulator, a newly spawned observer, and
    /// `reset_flush_after` publishing a blank grid at the pane's real geometry over content the
    /// viewer was already looking at, until the new observer's first frame repaints it. Held
    /// across the swap, a re-watch is a re-attach: nothing is re-opened, nothing is republished,
    /// and the ring keeps the history it has stitched.
    pub fn hold_while(&self, pane_id: &str, stop: impl FnOnce()) -> Option<PaneHold> {
        let hold = self.lookup(pane_id).map(|entry| PaneHold { _entry: entry });
        stop();
        hold
    }

    /// **An entry whose pump has stopped is not a pane.** It still holds the last grid, so
    /// re-attaching to it hands a joiner a screen that looks right and then never moves again —
    /// and `hold_while` pins it across a re-watch by design (#252), so the stall would outlive
    /// every close and reopen an operator could perform. Refusing it here is what makes reopening
    /// the pane the recovery it looks like.
    fn lookup(&self, pane_id: &str) -> Option<Arc<PaneEntry>> {
        self.panes
            .lock()
            .unwrap()
            .get(pane_id)
            .and_then(Weak::upgrade)
            .filter(|entry| entry.feeding())
    }

    /// Subscribes before releasing the state lock, so the grid handed to a joiner and the stream
    /// of patches that follows it cannot interleave.
    fn attach(&self, entry: Arc<PaneEntry>) -> Watcher {
        let state = entry.state.lock().unwrap();
        let ready = state.ready;
        let rx = entry.tx.subscribe();
        let initial = full_update(&state);
        drop(state);
        entry.watchers.fetch_add(1, Ordering::Relaxed);
        self.watcher_changes.send_modify(|n| *n += 1);
        Watcher {
            alive: entry.alive.clone(),
            entry,
            rx,
            initial,
            ready,
            watcher_changes: self.watcher_changes.clone(),
        }
    }
}

/// A pane kept open while one watcher is handed over to the next. See [`PaneRegistry::hold_while`].
pub struct PaneHold {
    _entry: Arc<PaneEntry>,
}

pub struct Watcher {
    entry: Arc<PaneEntry>,
    rx: broadcast::Receiver<PaneUpdate>,
    alive: tokio::sync::watch::Receiver<()>,
    initial: PaneUpdate,
    ready: bool,
    watcher_changes: Arc<tokio::sync::watch::Sender<u64>>,
}

impl Drop for Watcher {
    fn drop(&mut self) {
        self.entry.watchers.fetch_sub(1, Ordering::Relaxed);
        self.watcher_changes.send_modify(|n| *n += 1);
    }
}

impl Watcher {
    pub fn pane_id(&self) -> &str {
        &self.entry.pane_id
    }

    /// False when the provider has not yet produced a frame, so [`Self::initial`] is a placeholder
    /// rather than the pane's real grid.
    pub fn is_ready(&self) -> bool {
        self.ready
    }

    /// The full grid as of the moment this watcher joined — always a `Reset`.
    pub fn initial(&self) -> &PaneUpdate {
        &self.initial
    }

    /// A watcher that falls behind is caught up with one full grid rather than a queue of
    /// patches it can never drain; a watcher whose feeder has stopped is told so.
    ///
    /// `biased`, because the two are not equal claims: whatever is already in the broadcast was
    /// produced before the pump died and is still the truth about the pane, so it is drained
    /// first and the death is reported only once there is nothing left to say.
    pub async fn recv(&mut self) -> Result<PaneUpdate, WatchError> {
        let received = tokio::select! {
            biased;
            r = self.rx.recv() => r,
            _ = self.alive.changed() => return Err(WatchError::Closed),
        };
        match received {
            Ok(u) => Ok(u),
            Err(RecvError::Lagged(_)) => Ok(full_update(&self.entry.state.lock().unwrap())),
            Err(RecvError::Closed) => Err(WatchError::Closed),
        }
    }
}

fn cursor_of(state: &PaneState) -> Cursor {
    let (col, row, visible) = state.term.cursor();
    Cursor { col, row, visible }
}

fn full_update(state: &PaneState) -> PaneUpdate {
    let grid = state.term.grid();
    let rows_data = (0..grid.rows())
        .map(|r| RowDiff {
            row: r as u32,
            cells: grid.row(r).to_vec(),
        })
        .collect();
    PaneUpdate::Reset {
        cols: grid.cols(),
        rows: grid.rows(),
        rows_data: Arc::new(rows_data),
        cursor: cursor_of(state),
        links: Arc::new(grid.links.clone()),
    }
}

fn publish_reset(state: &Arc<Mutex<PaneState>>, tx: &broadcast::Sender<PaneUpdate>) {
    let mut st = state.lock().unwrap();
    if !st.pending_reset {
        return;
    }
    st.pending_reset = false;
    st.ready = true;
    st.links_sent = st.term.grid().links.len();
    st.cursor_sent = cursor_of(&st);
    let _ = tx.send(full_update(&st));
}

/// `_alive` is held and never used: dropping it is the signal, and it drops however this task
/// ends — a stream that closed, a panic, an abort at teardown.
async fn pump(
    mut stream: PaneStream,
    state: Arc<Mutex<PaneState>>,
    tx: broadcast::Sender<PaneUpdate>,
    flush_after: Duration,
    activity: Arc<Activity>,
    _alive: tokio::sync::watch::Sender<()>,
) {
    loop {
        let pending = state.lock().unwrap().pending_reset;
        let event = if pending {
            match tokio::time::timeout(flush_after, stream.recv()).await {
                Ok(e) => e,
                Err(_) => {
                    publish_reset(&state, &tx);
                    continue;
                }
            }
        } else {
            stream.recv().await
        };
        let Some(event) = event else { return };

        let mut st = state.lock().unwrap();
        match event {
            PaneEvent::Reset { cols, rows } => {
                let same_size = (st.term.grid().cols(), st.term.grid().rows()) == (cols, rows);
                if !same_size {
                    st.term = Emulator::new(cols, rows);
                    st.links_sent = 0;
                }
                // A restart at the same size leaves every client's grid valid, so nothing is
                // published until the next full frame repaints it — no blank flash, no
                // redundant reset.
                st.pending_reset |= !same_size || !st.ready;
            }
            PaneEvent::Bytes { full, bytes } => {
                activity.saw_frame();
                if full {
                    st.term.reset();
                    st.links_sent = 0;
                    st.pending_reset = true;
                }
                st.term.feed(&bytes);
                let dirty = st.term.take_dirty();
                let cursor = cursor_of(&st);
                // Probe #12: every frame ends with an absolute cursor address and carries
                // `ESC[?25h/l`, so a frame that moves the caret and nothing else is the ordinary
                // shape of ←/→/Home at a prompt and of a program hiding it. The client paints the
                // caret from the last `grid.*` message and from nothing else, so a patch is owed
                // for a cursor that moved even when no cell did.
                if st.pending_reset {
                    st.pending_reset = false;
                    st.ready = true;
                    st.links_sent = st.term.grid().links.len();
                    st.cursor_sent = cursor;
                    let _ = tx.send(full_update(&st));
                } else if !dirty.is_empty() || cursor != st.cursor_sent {
                    let links = &st.term.grid().links;
                    let new_links = links[st.links_sent.min(links.len())..].to_vec();
                    st.links_sent = links.len();
                    st.cursor_sent = cursor;
                    let _ = tx.send(PaneUpdate::Patch {
                        rows: Arc::new(dirty),
                        cursor,
                        new_links: Arc::new(new_links),
                    });
                }
            }
        }
    }
}

/// Grows a pane's ring while it is watched, at a cadence set by the pane itself.
///
/// **The fast path exists to keep [`Ingest::Gap`] rare, and is not an optimisation target.** Two
/// reads can only be stitched while they overlap, and they stop overlapping once more than
/// herdr's 1000-row cap accumulates between them. Flattening this back to a fixed interval trades
/// a few idle socket calls for silently unreachable history on every busy pane — the interval is
/// derived from the measured row rate precisely so that trade is not made by accident.
async fn accumulate_history(
    provider: Arc<dyn Provider>,
    pane_id: String,
    ring: Arc<Mutex<ScrollbackRing>>,
    status: Arc<Mutex<HistoryStatus>>,
    activity: Arc<Activity>,
    policy: HistoryPolicy,
) {
    let mut previous = Instant::now();
    let mut seen_frames = activity.count();
    let mut rate = RowRate::default();
    let mut failures = 0usize;
    loop {
        let outcome = provider.read_scrollback(&pane_id).await;
        let now = Instant::now();
        let elapsed = now.saturating_duration_since(previous);
        previous = now;

        let frames = activity.count();
        let producing = frames != seen_frames;
        seen_frames = frames;

        let mut gapped = false;
        let mut failed = false;
        let added = match outcome {
            Ok(Some(raw)) => match ring.lock().unwrap().ingest(&raw) {
                Ingest::Fresh { rows } => rows,
                Ingest::Stitched { added } => added,
                Ingest::Gap { dropped } => {
                    warn!(pane = %pane_id, dropped, "history outran the poll; ring capped here");
                    gapped = true;
                    // The overlap failed, so at least a full read cap went past unseen. That is a
                    // lower bound on the rate, not a measurement of it.
                    HERDR_READ_CAP
                }
                Ingest::Rewrapped { dropped } => {
                    warn!(pane = %pane_id, dropped, "pane re-wrapped; ring restarted");
                    0
                }
            },
            // No ring to read: an alt-screen or agent pane, or a pane that has simply not
            // scrolled yet. The provider answers this from cached state without touching the
            // socket, so it costs nothing to keep asking at the quiet cadence — and a pane with
            // no ring *yet* is one frame away from having one.
            Ok(None) => 0,
            // **Not the same state as a quiet pane, and it must not settle into the same
            // cadence.** `Ok(None)` is the pane having no ring to offer; an error is this node
            // failing to ask, and a poller that parks on the idle backstop for it stops growing
            // history while every surface goes on looking healthy.
            Err(e) => {
                failed = true;
                if failures == 0 {
                    warn!(pane = %pane_id, error = %e, "scrollback read failed; history has stopped growing");
                }
                failures += 1;
                0
            }
        };
        if !failed && failures > 0 {
            info!(pane = %pane_id, failures, "scrollback reads are answering again");
            failures = 0;
        }

        let rows_per_sec = rate.observe(added, elapsed);
        let next = if gapped {
            policy.fastest
        } else {
            policy.interval_for_rate(rows_per_sec)
        };
        *status.lock().unwrap() = HistoryStatus {
            poll: next,
            rows_per_sec,
        };

        debug!(
            pane = %pane_id,
            added,
            producing,
            elapsed_ms = elapsed.as_millis(),
            next_ms = next.as_millis(),
            "history poll"
        );
        // A pane that has gone quiet waits on its own output rather than on a timer, which is
        // what makes an idle pane free: a frame cuts that wait short, because output *starting* is
        // the one moment the estimate cannot know about yet.
        //
        // Only that wait. `saw_frame` notifies on every frame and `Notify` holds a permit, so
        // racing the wake against a *computed* interval means any frame at all ends it and the
        // cadence collapses to `fastest` — measured at 20x, with the policy choosing 2s on 96% of
        // polls and the poller serving 102ms (#282). The estimate has nothing to learn from a
        // frame on a pane it already knows is producing; the interval is what that knowledge is
        // for. A permit left over from the producing stretch resolves the first idle wait
        // immediately, which is correct rather than spurious: it says a frame landed after the
        // reading that made this pane look quiet.
        let quiet = added == 0 && !producing && !gapped && !failed;
        let wait = if quiet { policy.idle } else { next };
        tokio::time::sleep(policy.fastest.min(wait)).await;
        if let Some(remainder) = wait.checked_sub(policy.fastest).filter(|r| !r.is_zero()) {
            match quiet {
                true => {
                    tokio::select! {
                        _ = tokio::time::sleep(remainder) => {}
                        _ = activity.woken.notified() => {}
                    }
                }
                false => tokio::time::sleep(remainder).await,
            }
        }
    }
}

/// Herdr returns at most this many rows per `pane.read recent`, with no offset (probe #51).
const HERDR_READ_CAP: usize = 1000;
