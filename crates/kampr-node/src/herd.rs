use kampr_core::provider::AgentStatus;
use kampr_core::wire::{HerdDelta, NodeEntry, PaneEntry, ServerMsg};
use std::collections::{HashMap, HashSet};
use std::time::{Duration, Instant};
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
        let panes_now: HashSet<&str> = self.panes.iter().map(|p| p.id.as_str()).collect();
        let nodes_now: HashSet<&str> = self.nodes.iter().map(|n| n.id.as_str()).collect();
        let removed_ids: Vec<String> = previous
            .panes
            .iter()
            .filter(|p| !panes_now.contains(p.id.as_str()))
            .map(|p| p.id.clone())
            .chain(
                previous
                    .nodes
                    .iter()
                    .filter(|n| !nodes_now.contains(n.id.as_str()))
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
}

/// The clock behind `updated_at` for a pane nothing better stamped.
///
/// A pane whose harness keeps a transcript arrives already stamped with when that conversation
/// last moved, and a peer's arrive stamped by the peer; both are left alone. Everything else is
/// stamped here, and only for what somebody would call the pane *doing* something — a viewer
/// arriving, a geometry change or a new scrollback row is not it. A client marks herdr's `done`
/// read against this value, so each of those re-raised a flag the operator had already put down.
///
/// Remembered past a pane's absence, because a herd rebuilt while herdr was unreachable carries
/// none and the panes coming back are the same panes. And the herd a node first builds stamps
/// nothing: "now" there is when this process started, not when the pane was touched.
#[derive(Default)]
pub struct Stamps {
    seen: HashMap<String, Seen>,
    primed: bool,
}

struct Seen {
    doing: Doing,
    at: Option<String>,
    last: Instant,
}

type Doing = (
    Option<String>,
    AgentStatus,
    Option<String>,
    Option<String>,
    Option<String>,
);

const FORGET: Duration = Duration::from_secs(600);

fn doing(pane: &PaneEntry) -> Doing {
    (
        pane.agent.clone(),
        pane.agent_status,
        pane.cwd.clone(),
        pane.cmd.clone(),
        pane.argv.clone(),
    )
}

impl Stamps {
    pub fn stamp(&mut self, model: &mut HerdModel) {
        let now = OffsetDateTime::now_utc().format(&Rfc3339).ok();
        let at = Instant::now();
        for pane in &mut model.panes {
            let doing = doing(pane);
            if pane.updated_at.is_none() {
                pane.updated_at = match self.seen.get(&pane.id) {
                    Some(seen) if seen.doing == doing => seen.at.clone(),
                    None if !self.primed => None,
                    _ => now.clone(),
                };
            }
            self.seen.insert(
                pane.id.clone(),
                Seen {
                    doing,
                    at: pane.updated_at.clone(),
                    last: at,
                },
            );
        }
        self.primed = true;
        self.seen.retain(|_, seen| at.duration_since(seen.last) < FORGET);
    }
}

fn same_node(a: &NodeEntry, b: &NodeEntry) -> bool {
    a.name == b.name
        && a.kind == b.kind
        && a.online == b.online
        && a.herdr_version == b.herdr_version
        && a.build == b.build
        && a.update == b.update
        && a.detail == b.detail
}

fn same(a: &PaneEntry, b: &PaneEntry) -> bool {
    a.updated_at == b.updated_at
        && a.workspace == b.workspace
        && a.tab == b.tab
        && a.cwd == b.cwd
        && a.label == b.label
        && a.agent == b.agent
        && a.agent_status == b.agent_status
        && a.cols == b.cols
        && a.rows == b.rows
        && a.scrollback_rows == b.scrollback_rows
        && a.has_conversation == b.has_conversation
        && a.converses == b.converses
        && a.watchers == b.watchers
        && a.detail == b.detail
        && a.cmd == b.cmd
        && a.argv == b.argv
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

    fn stamped(stamps: &mut Stamps, mut model: HerdModel) -> HerdModel {
        stamps.stamp(&mut model);
        model
    }

    fn primed() -> (Stamps, Option<String>) {
        let mut stamps = Stamps::default();
        stamped(&mut stamps, HerdModel::default());
        let first = stamped(&mut stamps, model(&[("w1:p1", 74)]));
        let at = first.panes[0].updated_at.clone();
        assert!(
            at.is_some(),
            "a pane that opened while the node was running is stamped"
        );
        (stamps, at)
    }

    #[test]
    fn a_node_that_has_just_started_claims_no_pane_was_touched_just_now() {
        let mut stamps = Stamps::default();
        let first = stamped(&mut stamps, model(&[("w1:p1", 74)]));
        assert_eq!(first.panes[0].updated_at, None);
        let second = stamped(&mut stamps, model(&[("w1:p1", 74)]));
        assert_eq!(
            second.panes[0].updated_at, None,
            "and nothing it did since says otherwise"
        );
    }

    /// A client marks a `done` read against this stamp, so every one of these re-raised a flag the
    /// operator had already put down — a viewer leaving the pane they had just read among them.
    #[test]
    fn looking_at_a_pane_or_reshaping_it_is_not_touching_it() {
        let (mut stamps, at) = primed();
        let mut watched = model(&[("w1:p1", 94)]);
        watched.panes[0] = watched.panes[0].clone().with_watchers(2);
        watched.panes[0].scrollback_rows = 400;
        watched.panes[0].detail = Some("no picture".into());
        watched.panes[0].label = Some("renamed".into());
        assert_eq!(stamped(&mut stamps, watched).panes[0].updated_at, at);
    }

    #[test]
    fn a_pane_that_starts_doing_something_is_stamped_again() {
        let (mut stamps, at) = primed();
        let mut working = model(&[("w1:p1", 74)]);
        working.panes[0].agent_status = AgentStatus::Working;
        assert_ne!(stamped(&mut stamps, working).panes[0].updated_at, at);

        let (mut stamps, at) = primed();
        let mut job = model(&[("w1:p1", 74)]);
        job.panes[0].cmd = Some("cargo".into());
        assert_ne!(stamped(&mut stamps, job).panes[0].updated_at, at);
    }

    /// A herd rebuilt while herdr was unreachable carries no panes, and the ones that come back are
    /// the same panes.
    #[test]
    fn a_pane_that_drops_out_of_one_herd_comes_back_with_its_stamp() {
        let (mut stamps, at) = primed();
        stamped(&mut stamps, HerdModel::default());
        assert_eq!(
            stamped(&mut stamps, model(&[("w1:p1", 74)])).panes[0].updated_at,
            at
        );
    }

    /// The transcript's own clock, or a peer's: either is the same answer from every node that
    /// relays it, which a stamp taken here could never be.
    #[test]
    fn a_pane_that_arrives_stamped_keeps_the_stamp_it_came_with() {
        let mut stamps = Stamps::default();
        let mut heard = model(&[("w1:p1", 74)]);
        heard.panes[0].updated_at = Some("2026-09-30T00:15:34Z".into());
        let kept = stamped(&mut stamps, heard);
        assert_eq!(kept.panes[0].updated_at.as_deref(), Some("2026-09-30T00:15:34Z"));
    }

    /// A conversation moving is news for a client whether or not anything else about the pane did.
    #[test]
    fn a_new_stamp_is_a_change() {
        let before = model(&[("w1:p1", 74)]);
        let mut after = model(&[("w1:p1", 74)]);
        after.panes[0].updated_at = Some("2026-09-30T00:15:34Z".into());
        let (_, changed, _) = patch(&after.diff(&before).unwrap());
        assert_eq!(changed, ["01J/w1:p1"]);
    }

    /// A viewer joining or leaving is a change to the pane like any other: a client that is told
    /// once and never again would show a stale "someone else is here" for the life of the session.
    #[test]
    fn a_watcher_arriving_is_a_change() {
        let before = model(&[("w1:p1", 74)]);
        let mut after = model(&[("w1:p1", 74)]);
        after.panes[0] = after.panes[0].clone().with_watchers(2);
        let (added, changed, removed) = patch(&after.diff(&before).unwrap());
        assert!(added.is_empty() && removed.is_empty());
        assert_eq!(changed, ["01J/w1:p1"]);
        assert!(after.diff(&after).is_none());
    }

    /// What a pane is *running* is now part of what it is called, and a client that was told once
    /// would go on showing `kampr (cargo test)` long after the build finished. Nothing else in the
    /// pane moves when a job starts — same workspace, same tab, same cwd, same geometry — so this
    /// is the only thing that carries it.
    #[test]
    fn a_job_starting_and_finishing_is_a_change_like_any_other() {
        let before = model(&[("w1:p1", 74)]);
        let mut after = model(&[("w1:p1", 74)]);
        after.panes[0].cmd = Some("cargo".into());
        after.panes[0].argv = Some("cargo test".into());
        let (added, changed, removed) = patch(&after.diff(&before).unwrap());
        assert!(added.is_empty() && removed.is_empty());
        assert_eq!(changed, ["01J/w1:p1"]);
        assert!(after.diff(&after).is_none());

        let (_, back, _) = patch(&before.diff(&after).unwrap());
        assert_eq!(back, ["01J/w1:p1"], "and the job ending is a change too");
    }

    fn node(build: &str, update: Option<&str>) -> HerdModel {
        HerdModel {
            nodes: vec![NodeEntry {
                id: "01J".into(),
                name: "front".into(),
                kind: "local".into(),
                online: true,
                reachable: None,
                rtt_ms: None,
                herdr_version: None,
                build: Some(build.to_string()),
                update: update.map(str::to_string),
                detail: None,
            }],
            panes: Vec::new(),
        }
    }

    /// The check lands once a day, long after the `herd` a client was greeted with. If it is not
    /// a diffable change the client hears about it at the next sweep at best and never at worst,
    /// which is a field that works only for whoever reconnects after it.
    #[test]
    fn a_release_check_landing_is_a_change_a_client_is_told_about() {
        let before = node("0.1.0", None);
        let after = node("0.1.0", Some("0.1.2"));
        let ServerMsg::HerdPatch { changed, .. } = after.diff(&before).expect("a patch") else {
            panic!("expected a herd patch");
        };
        assert_eq!(changed.nodes.len(), 1);
        assert_eq!(changed.nodes[0].update.as_deref(), Some("0.1.2"));
        assert!(
            after.diff(&after).is_none(),
            "a settled model kept emitting patches"
        );

        // And an update that was taken by an install is a change in the other direction.
        assert!(before.diff(&after).is_some(), "the update line would never clear");
    }

    /// A node that restarts onto a new binary keeps its id, so the build is the only thing that
    /// moved — and a client that was told once at `hello` would still be showing the old one.
    #[test]
    fn a_node_that_moved_to_a_new_build_is_a_change_too() {
        assert!(node("0.1.2", None).diff(&node("0.1.0", None)).is_some());
    }

    #[test]
    fn global_ids_are_node_scoped() {
        assert_eq!(model(&[("w3:p2", 74)]).panes[0].id, "01J/w3:p2");
    }
}
