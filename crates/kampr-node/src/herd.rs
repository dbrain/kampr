use kampr_core::wire::{HerdDelta, NodeEntry, PaneEntry, ServerMsg};
use std::collections::HashMap;
use time::OffsetDateTime;
use time::format_description::well_known::Rfc3339;

#[derive(Debug, Clone, Default)]
pub struct HerdModel {
    pub nodes: Vec<NodeEntry>,
    pub panes: Vec<PaneEntry>,
}

impl HerdModel {
    pub fn message(&self) -> ServerMsg {
        ServerMsg::Herd {
            nodes: self.nodes.clone(),
            panes: self.panes.clone(),
        }
    }

    pub fn pane(&self, id: &str) -> Option<&PaneEntry> {
        self.panes.iter().find(|p| p.id == id)
    }

    pub fn node(&self, id: &str) -> Option<&NodeEntry> {
        self.nodes.iter().find(|n| n.id == id)
    }

    /// `None` when nothing moved, so a poll that finds no change sends nothing.
    ///
    /// Nodes are diffed alongside panes: a herdr going away is a node flipping to
    /// `online: false`, and a patch that only ever carried panes left that invisible.
    pub fn diff(&self, previous: &Self) -> Option<ServerMsg> {
        let before: HashMap<&str, &PaneEntry> = previous.panes.iter().map(|p| (p.id.as_str(), p)).collect();
        let mut added = HerdDelta::default();
        let mut changed = HerdDelta::default();
        for pane in &self.panes {
            match before.get(pane.id.as_str()) {
                None => added.panes.push(pane.clone()),
                Some(old) if !same(old, pane) => changed.panes.push(pane.clone()),
                Some(_) => {}
            }
        }
        let nodes_before: HashMap<&str, &NodeEntry> =
            previous.nodes.iter().map(|n| (n.id.as_str(), n)).collect();
        for node in &self.nodes {
            match nodes_before.get(node.id.as_str()) {
                None => added.nodes.push(node.clone()),
                Some(old) if !same_node(old, node) => changed.nodes.push(node.clone()),
                Some(_) => {}
            }
        }
        let panes_now: Vec<&str> = self.panes.iter().map(|p| p.id.as_str()).collect();
        let nodes_now: Vec<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();
        let removed_ids: Vec<String> = previous
            .panes
            .iter()
            .filter(|p| !panes_now.contains(&p.id.as_str()))
            .map(|p| p.id.clone())
            .chain(
                previous
                    .nodes
                    .iter()
                    .filter(|n| !nodes_now.contains(&n.id.as_str()))
                    .map(|n| n.id.clone()),
            )
            .collect();
        if added.is_empty() && changed.is_empty() && removed_ids.is_empty() {
            return None;
        }
        Some(ServerMsg::HerdPatch {
            added,
            changed,
            removed_ids,
        })
    }

    /// Which nodes changed their reachability between two models, newest state given.
    pub fn reachability_changes(&self, previous: &Self) -> Vec<(String, bool)> {
        self.nodes
            .iter()
            .filter(|node| {
                previous
                    .node(&node.id)
                    .is_none_or(|old| old.online != node.online)
            })
            .filter(|node| !(previous.nodes.is_empty() && node.online))
            .map(|node| (node.id.clone(), node.online))
            .collect()
    }

    /// Carries each pane's previous timestamp forward unless something about it actually moved.
    /// Herdr's snapshot carries no clock, so the node owns this one — and re-stamping every pane
    /// on every poll would make `updated_at` mean "when we last looked" instead of "when it last
    /// changed".
    pub fn stamp(&mut self, previous: &Self) {
        let before: HashMap<&str, &PaneEntry> = previous.panes.iter().map(|p| (p.id.as_str(), p)).collect();
        let now = OffsetDateTime::now_utc().format(&Rfc3339).ok();
        for pane in &mut self.panes {
            pane.updated_at = match before.get(pane.id.as_str()) {
                Some(old) if same(old, pane) => old.updated_at.clone(),
                _ => now.clone(),
            };
        }
    }
}

fn same_node(a: &NodeEntry, b: &NodeEntry) -> bool {
    a.name == b.name
        && a.kind == b.kind
        && a.online == b.online
        && a.herdr_version == b.herdr_version
        && a.detail == b.detail
}

fn same(a: &PaneEntry, b: &PaneEntry) -> bool {
    a.workspace == b.workspace
        && a.tab == b.tab
        && a.cwd == b.cwd
        && a.label == b.label
        && a.agent == b.agent
        && a.agent_status == b.agent_status
        && a.cols == b.cols
        && a.rows == b.rows
        && a.scrollback_rows == b.scrollback_rows
        && a.has_conversation == b.has_conversation
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_core::provider::{AgentStatus, PaneInfo};

    fn model(panes: &[(&str, u16)]) -> HerdModel {
        HerdModel {
            nodes: Vec::new(),
            panes: panes
                .iter()
                .map(|(id, cols)| {
                    PaneEntry::new(
                        "01J",
                        &PaneInfo {
                            pane_id: (*id).to_string(),
                            cols: Some(*cols),
                            rows: 30,
                            agent_status: AgentStatus::Unknown,
                            ..PaneInfo::default()
                        },
                        false,
                    )
                })
                .collect(),
        }
    }

    fn patch(msg: &ServerMsg) -> (Vec<String>, Vec<String>, Vec<String>) {
        match msg {
            ServerMsg::HerdPatch {
                added,
                changed,
                removed_ids,
            } => (
                added.panes.iter().map(|p| p.id.clone()).collect(),
                changed.panes.iter().map(|p| p.id.clone()).collect(),
                removed_ids.clone(),
            ),
            other => panic!("expected a herd patch, got {other:?}"),
        }
    }

    #[test]
    fn an_unchanged_herd_produces_no_patch() {
        let a = model(&[("w1:p1", 74)]);
        assert!(a.diff(&a).is_none());
    }

    #[test]
    fn a_geometry_change_shows_up_as_changed() {
        let before = model(&[("w1:p1", 74)]);
        let after = model(&[("w1:p1", 94)]);
        let (added, changed, removed) = patch(&after.diff(&before).unwrap());
        assert!(added.is_empty() && removed.is_empty());
        assert_eq!(changed, ["01J/w1:p1"]);
    }

    #[test]
    fn opened_and_closed_panes_land_in_the_right_buckets() {
        let before = model(&[("w1:p1", 74), ("w1:p2", 74)]);
        let after = model(&[("w1:p1", 74), ("w1:p3", 74)]);
        let (added, changed, removed) = patch(&after.diff(&before).unwrap());
        assert_eq!(added, ["01J/w1:p3"]);
        assert!(changed.is_empty());
        assert_eq!(removed, ["01J/w1:p2"]);
    }

    #[test]
    fn a_pane_that_did_not_move_keeps_its_timestamp() {
        let mut first = model(&[("w1:p1", 74)]);
        first.stamp(&HerdModel::default());
        let stamped = first.panes[0].updated_at.clone();
        assert!(stamped.is_some());

        let mut second = model(&[("w1:p1", 74)]);
        second.stamp(&first);
        assert_eq!(second.panes[0].updated_at, stamped);

        let mut third = model(&[("w1:p1", 94)]);
        third.stamp(&second);
        assert_ne!(third.panes[0].updated_at, stamped);
    }

    #[test]
    fn global_ids_are_node_scoped() {
        assert_eq!(model(&[("w3:p2", 74)]).panes[0].id, "01J/w3:p2");
    }
}
