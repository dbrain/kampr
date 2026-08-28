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
    pub fn groups(&self) -> Vec<NodeGroup<'_>> {
        let mut ordered: Vec<&NodeEntry> = self.nodes.iter().collect();
        ordered.sort_by_key(|n| n.kind != "local");
        ordered
            .into_iter()
            .map(|node| {
                let mut panes: Vec<&PaneEntry> = self.panes.iter().filter(|p| p.node_id == node.id).collect();
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

fn rank(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Blocked => 0,
        AgentStatus::Working => 1,
        AgentStatus::Done => 2,
        AgentStatus::Idle => 3,
        AgentStatus::Unknown => 4,
    }
}
