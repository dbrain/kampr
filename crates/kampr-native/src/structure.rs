//! Workspaces and tabs of Kampr sessions: herdr's containers, kept by the node (docs/17).

use crate::layout::Node;
use crate::provider::{NativeProvider, PREFIX};
use anyhow::Result;

impl NativeProvider {
    pub fn new_workspace(&self, name: &str) -> Result<String> {
        let id = self.inner.workspaces.lock().unwrap().create(name)?;
        self.inner.bump();
        Ok(id)
    }

    /// An id or a name, made when nothing has it.
    pub fn resolve_workspace(&self, wanted: &str) -> Result<String> {
        self.inner.workspaces.lock().unwrap().resolve(wanted)
    }

    pub fn rename_workspace(&self, id: &str, name: &str) -> Result<bool> {
        let renamed = self.inner.workspaces.lock().unwrap().rename(id, name)?;
        self.inner.bump();
        Ok(renamed)
    }

    pub fn workspace_of(&self, pane: &str) -> Option<String> {
        self.session(pane)?.spec().ok()?.workspace
    }

    pub fn workspace_members(&self, id: &str) -> Vec<String> {
        kampr_keep::list(&self.inner.config.root)
            .unwrap_or_default()
            .into_iter()
            .filter(|s| s.spec().ok().and_then(|spec| spec.workspace).as_deref() == Some(id))
            .map(|s| format!("{PREFIX}{}", s.id()))
            .collect()
    }

    /// The tab's tree, or `None` for a tab nobody has: a standalone session's own tab is its id.
    pub fn tab(&self, tab: &str) -> Option<Node> {
        let tree = self.inner.layouts.lock().unwrap().tree(tab)?;
        let mut leaves = Vec::new();
        leaves_of(&tree, &mut leaves);
        leaves
            .iter()
            .all(|pane| self.session(pane).is_some())
            .then_some(tree)
    }

    pub fn tab_of_pane(&self, pane: &str) -> String {
        match self.inner.layouts.lock().unwrap().tiles().remove(pane) {
            Some(tile) => tile.group,
            None => crate::layout::tab_of(pane),
        }
    }

    pub fn tab_members(&self, tab: &str) -> Vec<String> {
        let mut leaves = Vec::new();
        if let Some(tree) = self.tab(tab) {
            leaves_of(&tree, &mut leaves);
        }
        leaves
    }

    /// Sessions already made, laid out as `root` says, as a tab of their own.
    pub fn place(&self, root: Node) -> Result<String> {
        let tab = self.inner.layouts.lock().unwrap().add(root)?;
        self.inner.bump();
        Ok(tab)
    }
}

pub fn leaves_of(node: &Node, out: &mut Vec<String>) {
    match node {
        Node::Pane { pane } => out.push(pane.clone()),
        Node::Split { first, second, .. } => {
            leaves_of(first, out);
            leaves_of(second, out);
        }
    }
}
