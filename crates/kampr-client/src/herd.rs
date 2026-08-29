use kampr_core::provider::AgentStatus;
use kampr_core::wire::{HerdDelta, NodeEntry, PaneEntry};

/// The whole model, as the node published it.
///
/// The same shape and the same rules as the Kotlin client's `model/Herd.kt`, deliberately: three
/// clients that disagree about what a herd is are three different products.
#[derive(Debug, Clone, Default)]
pub struct Herd {
    pub nodes: Vec<NodeEntry>,
    pub panes: Vec<PaneEntry>,
    /// Whether a `herd` has ever arrived. Before the first one there is nothing to be absent from.
    pub known: bool,
    /// The last herd that arrived, on a socket that has since dropped — the last thing that was
    /// true rather than a statement about now.
    pub stale: bool,
}

/// One fan-out: the same command, on however many hosts it was sent to.
#[derive(Debug, Clone)]
pub struct Cohort<'a> {
    pub id: String,
    pub command: String,
    pub started_unix: i64,
    /// Needs-you first, then still going, then done — the order somebody scanning the board reads
    /// in, and the only order in which the top of the list is the part that matters.
    pub panes: Vec<&'a PaneEntry>,
}

impl Cohort<'_> {
    pub fn waiting(&self) -> usize {
        self.count("waiting")
    }

    pub fn running(&self) -> usize {
        self.count("running")
    }

    pub fn quiet(&self) -> usize {
        self.count("quiet")
    }

    /// Finished with an exit code of zero and no signal. A run the kernel killed is finished and
    /// is **not** a success.
    pub fn succeeded(&self) -> usize {
        self.fleet()
            .filter(|f| f.state == "exited" && f.exit_code == Some(0) && f.signal.is_none())
            .count()
    }

    pub fn failed(&self) -> usize {
        self.fleet()
            .filter(|f| f.state == "exited" && !(f.exit_code == Some(0) && f.signal.is_none()))
            .count()
    }

    pub fn finished(&self) -> bool {
        self.fleet().all(|f| f.state == "exited")
    }

    fn count(&self, state: &str) -> usize {
        self.fleet().filter(|f| f.state == state).count()
    }

    fn fleet(&self) -> impl Iterator<Item = &kampr_core::wire::FleetEntry> {
        self.panes.iter().filter_map(|p| p.fleet.as_ref())
    }
}

#[derive(Debug, Clone)]
pub struct NodeGroup<'a> {
    pub node: &'a NodeEntry,
    pub panes: Vec<&'a PaneEntry>,
}

/// Which half of a pane left. A shell that exits takes its pane; a node that goes takes every pane
/// it had, and may be back on its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Gone {
    Shell,
    Node,
}

impl Herd {
    pub fn apply(&mut self, nodes: Vec<NodeEntry>, panes: Vec<PaneEntry>) {
        self.nodes = nodes;
        self.panes = panes;
        self.known = true;
        self.stale = false;
    }

    /// **A patch carries nodes as well as panes.** A herd going away is a node flipping to
    /// `online: false`, so a client that only ever applied the pane half left an outage invisible.
    pub fn apply_patch(&mut self, added: HerdDelta, changed: HerdDelta, removed: &[String]) {
        for node in added.nodes.into_iter().chain(changed.nodes) {
            match self.nodes.iter_mut().find(|n| n.id == node.id) {
                Some(existing) => *existing = node,
                None => self.nodes.push(node),
            }
        }
        for pane in added.panes.into_iter().chain(changed.panes) {
            match self.panes.iter_mut().find(|p| p.id == pane.id) {
                Some(existing) => *existing = pane,
                None => self.panes.push(pane),
            }
        }
        // A removal names a pane or a node, and it is dropped from whichever list holds it.
        self.nodes.retain(|n| !removed.contains(&n.id));
        self.panes.retain(|p| !removed.contains(&p.id));
        // A node that left takes its panes with it: a pane whose node is not in the herd cannot
        // be watched, addressed or explained. An empty node list is a herd still assembling
        // rather than a herd with nothing in it.
        if !self.nodes.is_empty() {
            self.panes
                .retain(|p| self.nodes.iter().any(|n| n.id == p.node_id));
        }
        self.known = true;
        self.stale = false;
    }

    pub fn node(&self, id: &str) -> Option<&NodeEntry> {
        self.nodes.iter().find(|n| n.id == id)
    }

    pub fn pane(&self, id: &str) -> Option<&PaneEntry> {
        self.panes.iter().find(|p| p.id == id)
    }

    /// Panes grouped by the node that owns them, local nodes first — the sidebar's data model.
    /// An offline node keeps its group, empty or not: dropping it empties a node out of the UI at
    /// the moment the operator most needs to see that it exists and is unreachable.
    /// Every fleet run, gathered into the fan-outs that produced them, newest first.
    pub fn cohorts(&self) -> Vec<Cohort<'_>> {
        let mut by_id: std::collections::HashMap<&str, Vec<&PaneEntry>> = std::collections::HashMap::new();
        for pane in self.panes.iter() {
            if let Some(fleet) = &pane.fleet {
                by_id.entry(fleet.cohort.as_str()).or_default().push(pane);
            }
        }
        let mut cohorts: Vec<Cohort<'_>> = by_id
            .into_iter()
            .map(|(id, mut panes)| {
                panes.sort_by(|a, b| board_rank(a).cmp(&board_rank(b)).then_with(|| a.id.cmp(&b.id)));
                let first = panes.first().and_then(|p| p.fleet.as_ref());
                Cohort {
                    id: id.to_string(),
                    command: first.map(|f| f.command.clone()).unwrap_or_default(),
                    started_unix: panes
                        .iter()
                        .filter_map(|p| p.fleet.as_ref().map(|f| f.started_unix))
                        .min()
                        .unwrap_or_default(),
                    panes,
                }
            })
            .collect();
        cohorts.sort_by(|a, b| b.started_unix.cmp(&a.started_unix).then_with(|| a.id.cmp(&b.id)));
        cohorts
    }

    pub fn groups(&self) -> Vec<NodeGroup<'_>> {
        let mut ordered: Vec<&NodeEntry> = self.nodes.iter().collect();
        ordered.sort_by_key(|n| n.kind != "local");
        ordered
            .into_iter()
            .map(|node| {
                // **Fleet runs are not on the operator's desk and must not be listed as if they
                // were.** They are ptys the node forked for one command, with no workspace and no
                // place in anyone's layout; they belong to their cohort and are reached from the
                // fleet board.
                let mut panes: Vec<&PaneEntry> = self
                    .panes
                    .iter()
                    .filter(|p| p.node_id == node.id && p.fleet.is_none())
                    .collect();
                panes.sort_by(|a, b| {
                    rank(a.agent_status)
                        .cmp(&rank(b.agent_status))
                        .then_with(|| a.workspace.cmp(&b.workspace))
                        .then_with(|| a.id.cmp(&b.id))
                });
                NodeGroup { node, panes }
            })
            .collect()
    }

    /// Only a herd that has arrived and is current can say a pane is absent: reading absence out
    /// of a stale one reports a shell closed every time the socket drops.
    pub fn gone(&self, pane_id: &str) -> Option<Gone> {
        if !self.known || self.stale || self.pane(pane_id).is_some() {
            return None;
        }
        let node = pane_id.split_once('/').map_or(pane_id, |(node, _)| node);
        match self.node(node).is_some() {
            true => Some(Gone::Shell),
            false => Some(Gone::Node),
        }
    }
}

/// The board's order: what needs somebody, then what is still going, then what is merely quiet,
/// then what failed, then what worked.
///
/// Failures sort **above** successes among the finished, because the finished half of the board is
/// read to find what went wrong.
fn board_rank(pane: &PaneEntry) -> u8 {
    let Some(fleet) = &pane.fleet else { return 9 };
    match fleet.state.as_str() {
        "waiting" => 0,
        "running" => 1,
        "quiet" => 2,
        "exited" if fleet.exit_code == Some(0) && fleet.signal.is_none() => 4,
        "exited" => 3,
        _ => 5,
    }
}

fn rank(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Blocked => 0,
        AgentStatus::Working => 1,
        AgentStatus::Done => 2,
        AgentStatus::Idle => 3,
        AgentStatus::Unknown => 4,
    }
}
