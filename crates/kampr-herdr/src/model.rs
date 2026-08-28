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
pub struct PaneReply {
    pub pane: Pane,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Pane {
    pub pane_id: String,
    /// Whatever source is currently winning herdr's per-source metadata table for this pane —
    /// absent from `session.snapshot` and present on `pane.get` (probe #294).
    #[serde(default)]
    pub title: Option<String>,
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
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProcessInfo {
    #[serde(default)]
    pub foreground_processes: Vec<ForegroundProcess>,
    /// Equal to [`Self::shell_pid`] whenever the pane is at its prompt — and also whenever
    /// ble.sh is running the job, because it keeps it in the shell's own group (probe #297).
    #[serde(default)]
    pub foreground_process_group_id: Option<u32>,
    #[serde(default)]
    pub shell_pid: Option<u32>,
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
    /// The foreground job, **or nothing at all**, which is the ordinary answer.
    ///
    /// Two separate reasons for nothing, and neither is a fault. A pane sitting at its prompt has
    /// its shell in the foreground and no job to name. And a machine that sources ble.sh reports
    /// the shell however busy the pane is, because ble.sh runs the job inside the shell's own
    /// process group — which is every interactive shell on the operator's own machine (probe
    /// #297). So this answers `None` far oftener than it answers, and every caller has to degrade
    /// rather than render a blank.
    ///
    /// A pipeline names every member (`sleep 9 | cat` comes back as both, probe #297), so the
    /// line is their lines joined the way a shell would have written it.
    pub fn command(&self) -> Option<Command> {
        if let (Some(group), Some(shell)) = (self.foreground_process_group_id, self.shell_pid)
            && group == shell
        {
            return None;
        }
        let jobs: Vec<&ForegroundProcess> = self
            .foreground_processes
            .iter()
            .filter(|p| !SHELLS.contains(&p.name.trim_start_matches('-')))
            .collect();
        let first = jobs.first()?;
        Some(Command {
            name: first.name.trim_start_matches('-').to_string(),
            line: jobs.iter().map(|p| p.line()).collect::<Vec<_>>().join(" | "),
        })
    }

    /// A launcher counts, because `node …/cli.js` is the harness under another name.
    pub fn harness(&self, agent: &str) -> Option<u32> {
        self.foreground_processes
            .iter()
            .find(|p| p.name == agent || p.argv.iter().any(|arg| stem(arg) == agent))
            .map(|p| p.pid)
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct ForegroundProcess {
    pub pid: u32,
    pub name: String,
    #[serde(default)]
    pub argv: Vec<String>,
    /// Pre-joined by herdr, and the only form that survives an argument with a space in it
    /// (probe #297).
    #[serde(default)]
    pub cmdline: Option<String>,
}

impl ForegroundProcess {
    fn line(&self) -> String {
        match self.cmdline.as_deref().map(str::trim).filter(|l| !l.is_empty()) {
            Some(line) => line.to_string(),
            None if self.argv.is_empty() => self.name.clone(),
            None => self.argv.join(" "),
        }
    }
}

/// The job a pane is running, as a name and as a whole command line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub name: String,
    pub line: String,
}

/// Shell names, which are never the answer to "what is this pane running".
///
/// A login shell arrives as `-bash`, so the leading dash is stripped before the comparison.
const SHELLS: &[&str] = &[
    "sh", "bash", "zsh", "fish", "dash", "ash", "ksh", "mksh", "tcsh", "csh", "nu", "elvish", "xonsh",
];

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

    /// True only when herdr actually holds scrollback for this pane — alt-screen panes report
    /// zero, and so does an agent that clears the scrollback when it takes the screen.
    ///
    /// **A detected harness is not the second half of this.** The inherited rule also excluded
    /// agent panes, on the documented hazard that reading above the viewport there makes herdr
    /// harvest through the agent's mouse-scroll interface and move the operator's screen. It does
    /// not: measured against a live `codex` and a live `claude`, both herdr-detected and both
    /// holding a ring, `lines: 5000` comes back in **1 ms** with the whole ring and the viewport
    /// untouched, and a pane deliberately marked an agent while running a mouse-mode program
    /// received no wheel bytes at all (probe #231). Excluding them cost the node history on the
    /// one kind of pane it exists to serve.
    ///
    /// What is slow is a live harness whose ring is *empty* — Claude Code clears the scrollback
    /// when it takes the screen: every read past `viewport_rows` there costs a flat ~375 ms and
    /// returns the viewport anyway. That is this predicate's remaining half, so the only way to
    /// pay it is for the agent to clear its ring between the snapshot this was asked of and the
    /// read that follows.
    pub fn scrollback_is_safe_to_read(&self) -> bool {
        self.scroll.is_some_and(|s| s.max_offset_from_bottom > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::Pane;

    fn pane(agent: Option<&str>, max_offset_from_bottom: u64) -> Pane {
        serde_json::from_value(serde_json::json!({
            "pane_id": "w1:p1",
            "workspace_id": "w1",
            "tab_id": "w1:t1",
            "cwd": null,
            "label": null,
            "agent": agent,
            "agent_session": null,
            "scroll": {
                "offset_from_bottom": 0,
                "max_offset_from_bottom": max_offset_from_bottom,
                "viewport_rows": 40,
            },
        }))
        .unwrap()
    }

    /// Probe #231 — reading above the viewport on a detected-agent pane. The hazard the interlock
    /// inherited was never measured: a real `codex` and a real `claude`, both herdr-detected,
    /// both holding a ring, answer `lines: 5000` in **1 ms** with every row of the ring and the
    /// viewport exactly where it was. What is slow is a live harness with an *empty* ring, and
    /// that is the case the other half of the interlock already excludes.
    #[test]
    fn a_ring_is_a_ring_whether_or_not_a_harness_is_in_the_pane() {
        assert!(pane(None, 361).scrollback_is_safe_to_read());
        assert!(
            pane(Some("claude"), 384).scrollback_is_safe_to_read(),
            "an agent pane holding a ring reads like any other pane"
        );
    }

    /// The half of the interlock that measurement keeps: an alt-screen pane reports no ring, and
    /// a live harness with no ring is the one read that costs ~375 ms rather than under one.
    #[test]
    fn a_pane_with_no_ring_is_never_read_above_its_viewport() {
        assert!(!pane(None, 0).scrollback_is_safe_to_read());
        assert!(!pane(Some("claude"), 0).scrollback_is_safe_to_read());
    }
}
