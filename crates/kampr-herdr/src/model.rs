use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct SnapshotReply {
    pub snapshot: Snapshot,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Snapshot {
    pub version: String,
    pub protocol: u32,
    #[serde(default)]
    pub workspaces: Vec<Workspace>,
    #[serde(default)]
    pub tabs: Vec<Tab>,
    #[serde(default)]
    pub panes: Vec<Pane>,
    #[serde(default)]
    pub layouts: Vec<Layout>,
    pub focused_pane_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Workspace {
    pub workspace_id: String,
    pub number: u32,
    pub label: Option<String>,
    #[serde(default)]
    pub agent_status: AgentStatus,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Tab {
    pub tab_id: String,
    pub workspace_id: String,
    pub label: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    pub workspace_id: String,
    pub tab_id: String,
    pub cwd: Option<String>,
    pub label: Option<String>,
    /// Absent on panes with no detected harness — this is the agent-vs-shell discriminator.
    pub agent: Option<String>,
    #[serde(default)]
    pub agent_status: AgentStatus,
    pub agent_session: Option<AgentSession>,
    pub scroll: Option<Scroll>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentSession {
    pub agent: String,
    pub kind: String,
    pub value: String,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Scroll {
    pub offset_from_bottom: u64,
    /// Zero on alt-screen panes: there is no scrollback ring to reach.
    pub max_offset_from_bottom: u64,
    pub viewport_rows: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Idle,
    Working,
    Blocked,
    Done,
    #[default]
    Unknown,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Layout {
    pub tab_id: String,
    pub area: Rect,
    #[serde(default)]
    pub panes: Vec<LayoutPane>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct LayoutPane {
    pub pane_id: String,
    pub rect: Rect,
}

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct Rect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

fn stem(arg: &str) -> &str {
    let file = arg.rsplit('/').next().unwrap_or(arg);
    file.split_once('.').map_or(file, |(stem, _)| stem)
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessInfoReply {
    pub process_info: ProcessInfo,
}

/// What herdr knows about the processes inside a pane. `foreground_processes` is the job the
/// terminal is actually attached to — the harness itself, where one is running — and the pid it
/// carries is the only handle a node gets on *which* session a pane is having.
#[derive(Debug, Clone, Deserialize)]
pub struct ProcessInfo {
    #[serde(default)]
    pub foreground_processes: Vec<ForegroundProcess>,
}

impl ProcessInfo {
    /// The process to treat as the pane's harness, **or nothing**.
    ///
    /// A pane can report several — a shell and the job it launched — so this is a search for the
    /// one that *is* the harness rather than for whatever the terminal happens to be attached to.
    /// Falling back to the shell would be worse than answering nothing: its start time predates
    /// every transcript in the directory, so it re-admits exactly the neighbouring sessions a
    /// harness's start time exists to exclude, and it does so at the moment an agent has just
    /// been quit — which is the moment the operator noticed.
    ///
    /// A launcher counts, because `node …/cli.js` is the harness under another name.
    pub fn harness(&self, agent: &str) -> Option<u32> {
        self.foreground_processes
            .iter()
            .find(|p| p.name == agent || p.argv.iter().any(|arg| stem(arg) == agent))
            .map(|p| p.pid)
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct ForegroundProcess {
    pub pid: u32,
    pub name: String,
    #[serde(default)]
    pub argv: Vec<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ReadReply {
    pub read: Read,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Read {
    pub text: String,
    pub truncated: bool,
}

impl Snapshot {
    /// What a node holds before it has ever reached herdr. The node binds and serves from this
    /// rather than refusing to start, so an unreachable herd is an empty herd, not an exit.
    pub fn empty() -> Self {
        Self {
            version: String::new(),
            protocol: 0,
            workspaces: Vec::new(),
            tabs: Vec::new(),
            panes: Vec::new(),
            layouts: Vec::new(),
            focused_pane_id: None,
        }
    }

    pub fn pane(&self, pane_id: &str) -> Option<&Pane> {
        self.panes.iter().find(|p| p.pane_id == pane_id)
    }

    /// Native grid size, from the tab layout rather than the pane record: `scroll.viewport_rows`
    /// gives rows but nothing in the pane record gives columns.
    pub fn geometry(&self, pane_id: &str) -> Option<(u32, u32)> {
        let lp = self
            .layouts
            .iter()
            .flat_map(|l| &l.panes)
            .find(|p| p.pane_id == pane_id)?;
        Some((lp.rect.width, lp.rect.height))
    }
}

impl Pane {
    pub fn is_agent(&self) -> bool {
        self.agent.is_some()
    }

    /// True only when herdr actually holds scrollback for this pane. Alt-screen panes
    /// report zero, and asking for more than the viewport on a *detected agent* pane makes
    /// herdr harvest via the agent's mouse-scroll interface — slow, and it moves the
    /// operator's screen. Both hazards are excluded here.
    pub fn scrollback_is_safe_to_read(&self) -> bool {
        !self.is_agent() && self.scroll.is_some_and(|s| s.max_offset_from_bottom > 0)
    }
}
