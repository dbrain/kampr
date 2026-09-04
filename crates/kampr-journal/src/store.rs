use std::collections::{BTreeSet, HashMap};

use crate::model::Turn;

/// Turns keyed by a stable id taken from the transcript record. A tool turn is revised in place
/// when its result lands in a later record, so consumers must match on `Turn::id` and replace
/// rather than blindly append.
#[derive(Debug, Default)]
pub struct TurnStore {
    turns: Vec<Turn>,
    index: HashMap<String, usize>,
    changed: BTreeSet<usize>,
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

    /// Takes a turn back: the same id carrying no blocks, which is how the wire withdraws one.
    ///
    /// **Emptied rather than removed.** Every position this store hands out — a page's cursor, a
    /// tool card's index inside its turn — is an index into `turns`, and removing an element
    /// shifts every one of them. A client that is holding the turn is told to drop it because a
    /// turn with no blocks is not drawn, and a page that still contains it draws nothing for it.
    pub fn retire(&mut self, id: &str) {
        let Some(&at) = self.index.get(id) else {
            return;
        };
        if self.turns[at].blocks.is_empty() {
            return;
        }
        self.turns[at].blocks.clear();
        self.mark(at);
    }

    pub fn revise(&mut self, id: &str) -> Option<&mut Turn> {
        let at = *self.index.get(id)?;
        self.mark(at);
        self.turns.get_mut(at)
    }

    pub fn drain_changed(&mut self) -> Vec<Turn> {
        std::mem::take(&mut self.changed)
            .into_iter()
            .map(|i| self.turns[i].clone())
            .collect()
    }

    pub fn turns(&self) -> &[Turn] {
        &self.turns
    }

    pub fn position(&self, id: &str) -> Option<usize> {
        self.index.get(id).copied()
    }

    /// A set, and not a `Vec` scanned for the id already in it. The scan is over one entry per
    /// turn the poll has seen, so the first poll of a transcript — which is every record in it —
    /// was quadratic: 10 000 turns 22 ms, 40 000 221 ms, 160 000 **2.93 s**, against 118 ms for
    /// the same 160 000 records revising a single turn. It runs on the blocking pool the
    /// attachment route shares, once per watching socket.
    fn mark(&mut self, at: usize) {
        self.changed.insert(at);
    }
}
