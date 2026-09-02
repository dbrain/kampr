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

/// What a fleet run is doing, as measured rather than as guessed.
///
/// `Quiet` is deliberately not a kind of `Waiting`. Probes #331 and #332 describe hosts whose
/// state cannot be read at all, and a board that rendered those as questions would send somebody
/// to a host that is only slow — the same defect as [#233] seen from the other side, where a
/// surface answered confidently while one of its paths was dead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FleetState {
    Running,
    /// Parked in a read on fd 0, with whatever it last said.
    Waiting(Box<crate::question::Question>),
    /// Silent, with nothing readable behind it. **Not a question.**
    Quiet {
        seconds: u64,
    },
    Exited {
        code: Option<i32>,
        signal: Option<i32>,
    },
}

impl FleetState {
    pub fn question(&self) -> Option<&crate::question::Question> {
        match self {
            Self::Waiting(q) => Some(q),
            _ => None,
        }
    }

    /// Whether the run has ended, however it ended.
    pub fn finished(&self) -> bool {
        matches!(self, Self::Exited { .. })
    }

    /// `true` only for an exit that actually succeeded. A signal is not a zero exit and must never
    /// round to one.
    pub fn succeeded(&self) -> bool {
        matches!(
            self,
            Self::Exited {
                code: Some(0),
                signal: None
            }
        )
    }
}

/// A pane that is a fleet run rather than anything herdr knows about.
///
/// Its presence is what files a pane under the fleet board instead of beside the operator's own
/// workspaces — the grouping is a property of the pane, not a filter the client has to remember.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FleetPane {
    /// The run this pane belongs to. One `pacman -Syu` across five hosts is five panes and one
    /// cohort.
    pub cohort: String,
    pub command: String,
    pub state: FleetState,
    /// The supervisor cannot read its own child: a job that escalates refuses its own parent
    /// (probe #334's privilege half). Nothing about the *process* is readable, so the state comes
    /// off the screen and the board has to say so rather than let the host look idle.
    pub blind: bool,
    pub started_unix: i64,
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
    /// The foreground job's process name, and its whole command line. **Legitimately absent most
    /// of the time**: herdr answers with the shell whenever the pane is sitting at its prompt, and
    /// on a machine that sources ble.sh it answers with the shell even while a job runs, because
    /// the job never leaves the shell's process group (probe #297).
    pub cmd: Option<String>,
    pub argv: Option<String>,
    /// Set on a fleet run and `None` on everything herdr owns. A client groups by this rather
    /// than by parsing the pane id.
    pub fleet: Option<FleetPane>,
    /// Why this pane cannot be streamed, in the words an operator has to act on. `None` is the
    /// ordinary state; a pane that carries one has a supervisor retrying behind it, so it clears
    /// itself when the fault does.
    pub detail: Option<String>,
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
    /// The width the rows were wrapped at, and `None` until one has been *proved*. The layout
    /// rect is not an answer (probe #68), and a ring restarted on a rect that resolves to the
    /// PTY's own width a moment later throws away history nothing was wrong with.
    pub cols: Option<u16>,
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

    /// Whether a harness owns this pane's screen — which is *why* [`Self::read_scrollback`] has no
    /// ring to offer, as distinct from a pane that has simply not scrolled yet.
    ///
    /// The two look identical from the read and are not the same news. A harness holds the
    /// alternate screen for its whole life, so herdr's ring stays away and the rows a node has
    /// already accumulated are the shell session that ran *before* it — not this pane's history
    /// any more (`ScrollbackRing::superseded`). A pager takes the screen for a moment and gives it
    /// back, and the ring outlives it (probe #244).
    ///
    /// Answered from cached state and never from the socket: it is asked on the same poll the read
    /// is, for every watched pane.
    fn harness_owns_the_screen(&self, _pane_id: &str) -> bool {
        false
    }

    /// Whether this implementation is the one that owns `pane_id`.
    ///
    /// Only [`Composite`] asks. The default is "yes", which makes a single provider the answer to
    /// everything — the arrangement every node had before there were two.
    fn owns(&self, _pane_id: &str) -> bool {
        true
    }
}

/// Two sources of panes behind one seam.
///
/// The node's panes come from herdr; its fleet runs come from ptys the node forked itself, and
/// neither knows about the other. Order matters and is the routing rule: the first provider that
/// claims a pane gets it, so a discriminating provider goes before the catch-all.
pub struct Composite {
    providers: Vec<std::sync::Arc<dyn Provider>>,
    topology: watch::Sender<u64>,
    _pumps: Vec<JoinHandle<()>>,
}

impl Composite {
    pub fn new(providers: Vec<std::sync::Arc<dyn Provider>>) -> std::sync::Arc<Self> {
        let (topology, _) = watch::channel(0);
        // One revision for the union: a client watching the composite must wake for a change in
        // either half, and neither half can know about the other's.
        let pumps = providers
            .iter()
            .map(|provider| {
                let mut source = provider.topology();
                let out = topology.clone();
                tokio::spawn(async move {
                    while source.changed().await.is_ok() {
                        out.send_modify(|n| *n += 1);
                    }
                })
            })
            .collect();
        std::sync::Arc::new(Self {
            providers,
            topology,
            _pumps: pumps,
        })
    }

    fn route(&self, pane_id: &str) -> Result<&std::sync::Arc<dyn Provider>> {
        self.providers
            .iter()
            .find(|p| p.owns(pane_id))
            .ok_or_else(|| anyhow::anyhow!("no provider owns {pane_id}"))
    }
}

#[async_trait]
impl Provider for Composite {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        let mut all = Vec::new();
        for provider in &self.providers {
            // **One source failing must not blank the other.** A herdr that has gone away would
            // otherwise take the fleet board down with it, which is the same defect as a node that
            // looks healthy while one of its two paths is dead (probe #233), inverted.
            match provider.list_panes().await {
                Ok(panes) => all.extend(panes),
                Err(e) => tracing::warn!("a provider could not list its panes: {e:#}"),
            }
        }
        Ok(all)
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        self.route(pane_id)?.watch_pane(pane_id).await
    }

    async fn write_pane(&self, pane_id: &str, input: Input) -> Result<()> {
        self.route(pane_id)?.write_pane(pane_id, input).await
    }

    async fn read_scrollback(&self, pane_id: &str) -> Result<Option<RawScrollback>> {
        self.route(pane_id)?.read_scrollback(pane_id).await
    }

    fn harness_owns_the_screen(&self, pane_id: &str) -> bool {
        self.route(pane_id)
            .is_ok_and(|p| p.harness_owns_the_screen(pane_id))
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.topology.subscribe()
    }
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
