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
