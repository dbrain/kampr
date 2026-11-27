use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use anyhow::{Result, bail};
use async_trait::async_trait;
use kampr_core::provider::{AgentStatus, Input, PaneInfo, PaneStream, Provider, RawScrollback};
use kampr_keep::{Mode, SessionDir};
use tokio::sync::watch;

pub const PREFIX: &str = "c:";
pub const AGENT: &str = "claude";

pub fn pane_id(session: &str) -> String {
    format!("{PREFIX}{session}")
}

pub fn session_of(local: &str) -> Option<&str> {
    local.strip_prefix(PREFIX).filter(|s| !s.is_empty())
}

/// What a headless Claude pane is: a keeper session with no terminal. The source and the provider
/// share it, so a create the source makes is a pane the herd lists, and the topology bump is what
/// tells the herd to look again.
pub struct Herd {
    topology: watch::Sender<u64>,
    sessions: Mutex<HashSet<String>>,
}

impl Herd {
    /// The herd of a node that starts with sessions already running: every live pipe-mode keeper
    /// session under `root` is a headless Claude, since nothing else is started in pipe mode.
    /// The keeper outlives the node, so the panes must too.
    pub fn at(root: &Path) -> (Self, watch::Receiver<u64>) {
        let (topology, rx) = watch::channel(0);
        let running = kampr_keep::list(root)
            .unwrap_or_default()
            .into_iter()
            .filter(SessionDir::alive)
            .filter(|s| s.spec().is_ok_and(|spec| spec.mode == Mode::Pipe))
            .map(|s| s.id().to_string())
            .collect();
        (
            Self {
                topology,
                sessions: Mutex::new(running),
            },
            rx,
        )
    }

    pub fn add(&self, id: &str) {
        self.sessions.lock().unwrap().insert(id.to_string());
        self.topology.send_modify(|n| *n += 1);
    }

    pub fn remove(&self, id: &str) {
        self.sessions.lock().unwrap().remove(id);
        self.topology.send_modify(|n| *n += 1);
    }

    pub fn sessions(&self) -> Vec<String> {
        self.sessions.lock().unwrap().iter().cloned().collect()
    }
}

/// The headless Claude keeper sessions as panes of this node's herd. They have no terminal:
/// nothing streams them, and the node serves their conversation itself.
pub struct ClaudeProvider {
    herd: Arc<Herd>,
    root: PathBuf,
}

impl ClaudeProvider {
    pub fn new(herd: Arc<Herd>, root: PathBuf) -> Self {
        Self { herd, root }
    }

    fn cwd_of(&self, id: &str) -> Option<String> {
        let spec: kampr_keep::Spec =
            serde_json::from_str(&std::fs::read_to_string(self.root.join(id).join("spec.json")).ok()?)
                .ok()?;
        spec.cwd
    }
}

#[async_trait]
impl Provider for ClaudeProvider {
    async fn list_panes(&self) -> Result<Vec<PaneInfo>> {
        Ok(self
            .herd
            .sessions()
            .into_iter()
            .map(|id| PaneInfo {
                pane_id: pane_id(&id),
                cwd: self.cwd_of(&id),
                label: None,
                agent: Some(AGENT.into()),
                agent_status: AgentStatus::Idle,
                cols: None,
                rows: 0,
                ..PaneInfo::default()
            })
            .collect())
    }

    async fn watch_pane(&self, pane_id: &str) -> Result<PaneStream> {
        bail!("{pane_id} is a headless Claude session and has no terminal to stream")
    }

    async fn write_pane(&self, _pane_id: &str, _input: Input) -> Result<()> {
        bail!("a headless Claude pane is served by its conversation, not a terminal")
    }

    async fn read_scrollback(&self, _pane_id: &str) -> Result<Option<RawScrollback>> {
        Ok(None)
    }

    fn topology(&self) -> watch::Receiver<u64> {
        self.herd.topology.subscribe()
    }

    fn owns(&self, pane_id: &str) -> bool {
        pane_id.starts_with(PREFIX)
    }
}
