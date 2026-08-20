use std::collections::HashMap;

use crate::model::Turn;

/// Turns keyed by a stable id taken from the transcript record. A tool turn is revised in place
/// when its result lands in a later record, so consumers must match on `Turn::id` and replace
/// rather than blindly append.
#[derive(Debug, Default)]
pub struct TurnStore {
    turns: Vec<Turn>,
    index: HashMap<String, usize>,
    changed: Vec<usize>,
}

impl TurnStore {
    pub fn push(&mut self, turn: Turn) {
        if let Some(&at) = self.index.get(&turn.id) {
            self.turns[at] = turn;
            self.mark(at);
            return;
        }
        let at = self.turns.len();
        self.index.insert(turn.id.clone(), at);
        self.turns.push(turn);
        self.mark(at);
    }

    pub fn revise(&mut self, id: &str) -> Option<&mut Turn> {
        let at = *self.index.get(id)?;
        self.mark(at);
        self.turns.get_mut(at)
    }

    pub fn drain_changed(&mut self) -> Vec<Turn> {
        let mut at = std::mem::take(&mut self.changed);
        at.sort_unstable();
        at.dedup();
        at.into_iter().map(|i| self.turns[i].clone()).collect()
    }

    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    pub fn position(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    fn mark(&mut self, at: usize) {
        if !self.changed.contains(&at) {
            self.changed.push(at);
        }
    }
}
