//! Native sessions split from one another, kept as herdr keeps a tab: a tree of splits whose
//! leaves are sessions. Only ever a picture for clients to lay out — splitting starts a session
//! of its own, and nothing here sizes one.

use anyhow::{Context, Result};
use kampr_core::tile::{Cut, Split, Tile};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeSet, HashMap};
use std::path::{Path, PathBuf};

const FILE: &str = "layouts.json";
pub const TAB_PREFIX: &str = "k:";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Node {
    Pane {
        pane: String,
    },
    Split {
        split: Split,
        ratio: u16,
        first: Box<Node>,
        second: Box<Node>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Group {
    group: String,
    root: Node,
}

#[derive(Debug, Default)]
pub(crate) struct Layouts {
    path: PathBuf,
    groups: Vec<Group>,
}

impl Layouts {
    /// A file nobody can read is no layout at all: each session stands alone, as before splits.
    pub(crate) fn load(root: &Path) -> Self {
        let path = root.join(FILE);
        let groups = std::fs::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default();
        Self { path, groups }
    }

    fn save(&self) -> Result<()> {
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, serde_json::to_vec(&self.groups)?)
            .with_context(|| format!("writing {}", tmp.display()))?;
        std::fs::rename(&tmp, &self.path).with_context(|| format!("replacing {}", self.path.display()))
    }

    /// `new` takes the second half of `at`'s box, as a herdr split puts the new pane right of or
    /// below the one it split.
    pub(crate) fn split(&mut self, at: &str, new: &str, split: Split, ratio: u16) -> Result<()> {
        let fresh = |pane: &str| Node::Pane {
            pane: pane.to_string(),
        };
        let made = Node::Split {
            split,
            ratio,
            first: Box::new(fresh(at)),
            second: Box::new(fresh(new)),
        };
        match self.groups.iter_mut().find_map(|g| leaf(&mut g.root, at)) {
            Some(node) => *node = made,
            None => self.groups.push(Group {
                group: tab_of(at),
                root: made,
            }),
        }
        self.save()
    }

    /// A tree of sessions already made, as a tab of its own named for its first.
    pub(crate) fn add(&mut self, root: Node) -> Result<String> {
        let group = tab_of(first(&root));
        if matches!(root, Node::Split { .. }) {
            self.groups.push(Group {
                group: group.clone(),
                root,
            });
            self.save()?;
        }
        Ok(group)
    }

    pub(crate) fn tree(&self, tab: &str) -> Option<Node> {
        if let Some(group) = self.groups.iter().find(|g| g.group == tab) {
            return Some(group.root.clone());
        }
        let pane = format!("{}{}", crate::PREFIX, tab.strip_prefix(TAB_PREFIX)?);
        let split = self.groups.iter().any(|g| contains(&g.root, &pane));
        (!split).then_some(Node::Pane { pane })
    }

    /// Drops every session not in `alive`, closing its split up behind it, and writes the file
    /// back only when that changed something.
    pub(crate) fn keep(&mut self, alive: &BTreeSet<String>) -> Result<()> {
        let before = self.groups.clone();
        self.groups = std::mem::take(&mut self.groups)
            .into_iter()
            .filter_map(|g| {
                let root = prune(g.root, alive)?;
                matches!(root, Node::Split { .. }).then_some(Group { root, ..g })
            })
            .collect();
        match self.groups == before {
            true => Ok(()),
            false => self.save(),
        }
    }

    pub(crate) fn tiles(&self) -> HashMap<String, Tile> {
        let mut tiles = HashMap::new();
        for group in &self.groups {
            walk(&group.root, &group.group, &mut Vec::new(), &mut tiles);
        }
        tiles
    }
}

/// The tab a session stands in when nothing is split from it.
pub fn tab_of(pane: &str) -> String {
    format!("{TAB_PREFIX}{}", pane.trim_start_matches(crate::PREFIX))
}

fn first(node: &Node) -> &str {
    match node {
        Node::Pane { pane } => pane,
        Node::Split { first: half, .. } => first(half),
    }
}

fn contains(node: &Node, pane: &str) -> bool {
    match node {
        Node::Pane { pane: here } => here == pane,
        Node::Split { first, second, .. } => contains(first, pane) || contains(second, pane),
    }
}

fn leaf<'a>(node: &'a mut Node, pane: &str) -> Option<&'a mut Node> {
    match node {
        Node::Pane { pane: here } if here == pane => Some(node),
        Node::Pane { .. } => None,
        Node::Split { first, second, .. } => {
            if let Some(found) = leaf(first, pane) {
                return Some(found);
            }
            leaf(second, pane)
        }
    }
}

fn prune(node: Node, alive: &BTreeSet<String>) -> Option<Node> {
    match node {
        Node::Pane { ref pane } => alive.contains(pane).then_some(node),
        Node::Split {
            split,
            ratio,
            first,
            second,
        } => match (prune(*first, alive), prune(*second, alive)) {
            (Some(first), Some(second)) => Some(Node::Split {
                split,
                ratio,
                first: Box::new(first),
                second: Box::new(second),
            }),
            (one, other) => one.or(other),
        },
    }
}

fn walk(node: &Node, group: &str, path: &mut Vec<Cut>, tiles: &mut HashMap<String, Tile>) {
    match node {
        Node::Pane { pane } => {
            tiles.insert(
                pane.clone(),
                Tile {
                    group: group.to_string(),
                    path: path.clone(),
                },
            );
        }
        Node::Split {
            split,
            ratio,
            first,
            second,
        } => {
            for (half, second_half) in [(first, false), (second, true)] {
                path.push(Cut {
                    split: *split,
                    ratio: *ratio,
                    second: second_half,
                });
                walk(half, group, path, tiles);
                path.pop();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn alive(panes: &[&str]) -> BTreeSet<String> {
        panes.iter().map(|p| p.to_string()).collect()
    }

    #[test]
    fn splits_nest_survive_a_reload_and_close_up_behind_a_session_that_went() {
        let root = tempfile::tempdir().expect("a root");
        let mut layouts = Layouts::load(root.path());
        layouts.split("n:a", "n:b", Split::Right, 500).unwrap();
        layouts.split("n:b", "n:c", Split::Down, 500).unwrap();
        layouts.split("n:x", "n:y", Split::Down, 400).unwrap();

        let tiles = Layouts::load(root.path()).tiles();
        let c = &tiles["n:c"];
        assert_eq!(c.group, "k:a");
        assert_eq!(
            c.path,
            vec![
                Cut {
                    split: Split::Right,
                    ratio: 500,
                    second: true
                },
                Cut {
                    split: Split::Down,
                    ratio: 500,
                    second: true
                },
            ]
        );
        assert_eq!(tiles["n:y"].group, "k:x");

        layouts.keep(&alive(&["n:a", "n:c", "n:x"])).unwrap();
        let tiles = Layouts::load(root.path()).tiles();
        assert_eq!(
            tiles["n:c"].path,
            vec![Cut {
                split: Split::Right,
                ratio: 500,
                second: true
            }],
            "b's half went to c"
        );
        assert!(!tiles.contains_key("n:x"), "a group of one is no group");
    }
}
