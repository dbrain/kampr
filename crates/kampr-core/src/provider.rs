use anyhow::Result;
use async_trait::async_trait;
use kampr_journal::Harness;
use serde::{Deserialize, Serialize};
use tokio::sync::{mpsc, watch};
use tokio::task::JoinHandle;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneInfo {
    pub pane_id: String,
    /// Herdr's own ids for the two containers. A pane id carries its workspace (`w3:p2`) but not
    /// its tab, so a client with only the label could never address `tab.rename` or `tab.close`.
    pub workspace_id: Option<String>,
    pub tab_id: Option<String>,
    pub workspace: Option<String>,
    pub tab: Option<String>,
    pub cwd: Option<String>,
    pub label: Option<String>,
    /// `None` on a shell pane. This is the agent-vs-shell discriminator, and it also decides
    /// whether scrollback may be read at all.
    pub agent: Option<String>,
    /// The harness process this pane is running. **This, not `cwd`, is what identifies a
    /// conversation**: every run in a directory writes a new transcript, so the newest of them
    /// belongs to whoever ran last rather than to this pane.
    pub agent_harness: Harness,
    pub agent_status: AgentStatus,
    /// `None` until the width has been *proven*. In a headless session the PTY does not follow
    /// the layout rect — a pane whose rect reads 47 is really 93 wide — so the rect is a number
    /// no row was ever wrapped at, and an unmeasured pane has nothing honest to report.
    pub cols: Option<u16>,
    /// Herdr's own `scroll.viewport_rows`, which is the PTY's, not the rect's.
    pub rows: u16,
    /// Rows of history *above* the viewport, and zero whenever reading them would be unsafe.
    pub scrollback_rows: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Input {
    Bytes(Vec<u8>),
    Keys(Vec<String>),
}

#[derive(Debug, Clone)]
pub enum PaneEvent {
    /// The stream started or restarted at this geometry. Everything the consumer holds for the
    /// pane is stale: rebuild from the bytes that follow.
    Reset {
        cols: u16,
        rows: u16,
    },
    Bytes {
        full: bool,
        bytes: Vec<u8>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawScrollback {
    pub text: String,
    pub cols: u16,
    pub viewport_rows: u16,
    /// Set by herdr when more history existed than it returned — the read cap, in practice.
    pub truncated: bool,
}

/// A pane's event stream. Dropping it stops the provider's supervision — for the herdr provider
/// that kills the `observe` child, which is the only way to stop it.
pub struct PaneStream {
    events: mpsc::Receiver<PaneEvent>,
    task: Option<JoinHandle<()>>,
}

impl PaneStream {
    pub fn new(events: mpsc::Receiver<PaneEvent>) -> Self {
        Self { events, task: None }
    }

    pub fn supervised(events: mpsc::Receiver<PaneEvent>, task: JoinHandle<()>) -> Self {
        Self {
            events,
            task: Some(task),
        }
    }

    pub async fn recv(&mut self) -> Option<PaneEvent> {
        self.events.recv().await
    }
}

impl Drop for PaneStream {
    fn drop(&mut self) {
        if let Some(t) = self.task.take() {
            t.abort();
        }
    }
}

/// The seam between a node and whatever is actually hosting terminals. Herdr is the first
/// implementation; an Android local-PTY provider joins the same mesh by implementing this.
#[async_trait]
pub trait Provider: Send + Sync + 'static {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>>;

    /// Opens a supervised stream. Implementations own restart and reconnect, so a consumer sees
    /// a `Reset` rather than an error.
    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream>;

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()>;

    /// `None` when the pane has no readable history — the implementation owns that judgement.
    async fn read_scrollback(&self, pane_id: &str) -> Result<Option<RawScrollback>>;

    /// Bumps whenever the pane list or its geometry may have changed.
    fn topology(&self) -> watch::Receiver<u64>;
}

impl From<kampr_herdr::AgentStatus> for AgentStatus {
    fn from(s: kampr_herdr::AgentStatus) -> Self {
        use kampr_herdr::AgentStatus as H;
        match s {
            H::Idle => Self::Idle,
            H::Working => Self::Working,
            H::Blocked => Self::Blocked,
            H::Done => Self::Done,
            H::Unknown => Self::Unknown,
        }
    }
}
