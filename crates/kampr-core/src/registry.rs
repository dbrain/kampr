use crate::provider::{Input, PaneEvent, PaneInfo, PaneStream, Provider};
use crate::scrollback::{self, ScrollbackDoc};
use crate::wire::Cursor;
use anyhow::Result;
use kampr_term::{Emulator, RowDiff};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Weak};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::sync::broadcast::error::RecvError;
use tokio::task::JoinHandle;

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
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            broadcast_capacity: 256,
            reset_flush_after: Duration::from_millis(300),
            first_grid_wait: Duration::from_secs(2),
        }
    }
}

struct PaneState {
    term: Emulator,
    links_sent: usize,
    pending_reset: bool,
    ready: bool,
}

struct PaneEntry {
    pane_id: String,
    state: Arc<Mutex<PaneState>>,
    tx: broadcast::Sender<PaneUpdate>,
    task: JoinHandle<()>,
}

impl Drop for PaneEntry {
    fn drop(&mut self) {
        self.task.abort();
    }
}

/// One emulator per pane, shared by every viewer and refcounted by [`Watcher`]s. The provider
/// stream is opened on the first watcher and dropped with the last.
pub struct PaneRegistry {
    provider: Arc<dyn Provider>,
    config: RegistryConfig,
    panes: Mutex<HashMap<String, Weak<PaneEntry>>>,
    opening: tokio::sync::Mutex<()>,
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

    pub async fn scrollback(&self, pane_id: &str) -> Result<Option<ScrollbackDoc>> {
        Ok(self
            .provider
            .read_scrollback(pane_id)
            .await?
            .map(|raw| scrollback::render(&raw)))
    }

    pub fn watcher_count(&self, pane_id: &str) -> usize {
        self.lookup(pane_id).map_or(0, |e| Arc::strong_count(&e) - 1)
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
        }));
        let (tx, _) = broadcast::channel(self.config.broadcast_capacity);
        let task = tokio::spawn(pump(
            stream,
            state.clone(),
            tx.clone(),
            self.config.reset_flush_after,
        ));
        let entry = Arc::new(PaneEntry {
            pane_id: pane_id.to_string(),
            state,
            tx,
            task,
        });
        self.panes
            .lock()
            .unwrap()
            .insert(pane_id.to_string(), Arc::downgrade(&entry));
        let mut watcher = self.attach(entry);
        if let Ok(Ok(first)) = tokio::time::timeout(self.config.first_grid_wait, watcher.recv()).await
            && first.is_reset()
        {
            watcher.initial = first;
            watcher.ready = true;
        }
        Ok(watcher)
    }

    fn lookup(&self, pane_id: &str) -> Option<Arc<PaneEntry>> {
        self.panes.lock().unwrap().get(pane_id).and_then(Weak::upgrade)
    }

    /// Subscribes before releasing the state lock, so the grid handed to a joiner and the stream
    /// of patches that follows it cannot interleave.
    fn attach(&self, entry: Arc<PaneEntry>) -> Watcher {
        let state = entry.state.lock().unwrap();
        let ready = state.ready;
        let rx = entry.tx.subscribe();
        let initial = full_update(&state);
        drop(state);
        Watcher {
            entry,
            rx,
            initial,
            ready,
        }
    }
}

pub struct Watcher {
    entry: Arc<PaneEntry>,
    rx: broadcast::Receiver<PaneUpdate>,
    initial: PaneUpdate,
    ready: bool,
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
    /// patches it can never drain.
    pub async fn recv(&mut self) -> Result<PaneUpdate, WatchError> {
        match self.rx.recv().await {
            Ok(u) => Ok(u),
            Err(RecvError::Lagged(_)) => Ok(full_update(&self.entry.state.lock().unwrap())),
            Err(RecvError::Closed) => Err(WatchError::Closed),
        }
    }
}

fn full_update(state: &PaneState) -> PaneUpdate {
    let grid = state.term.grid();
    let rows_data = (0..grid.rows())
        .map(|r| RowDiff {
            row: r,
            cells: grid.row(r).to_vec(),
        })
        .collect();
    let (col, row, visible) = state.term.cursor();
    PaneUpdate::Reset {
        cols: grid.cols(),
        rows: grid.rows(),
        rows_data: Arc::new(rows_data),
        cursor: Cursor { col, row, visible },
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
    let _ = tx.send(full_update(&st));
}

async fn pump(
    mut stream: PaneStream,
    state: Arc<Mutex<PaneState>>,
    tx: broadcast::Sender<PaneUpdate>,
    flush_after: Duration,
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
                if full {
                    st.term.reset();
                    st.links_sent = 0;
                    st.pending_reset = true;
                }
                st.term.feed(&bytes);
                let dirty = st.term.take_dirty();
                let (col, row, visible) = st.term.cursor();
                let cursor = Cursor { col, row, visible };
                if st.pending_reset {
                    st.pending_reset = false;
                    st.ready = true;
                    st.links_sent = st.term.grid().links.len();
                    let _ = tx.send(full_update(&st));
                } else if !dirty.is_empty() {
                    let links = &st.term.grid().links;
                    let new_links = links[st.links_sent.min(links.len())..].to_vec();
                    st.links_sent = links.len();
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
