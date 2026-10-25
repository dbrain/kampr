use crate::mapper::{TurnBuilder, Workers};
use crate::wire::ContentBlock;
use kampr_journal::{LIVE_ID, Role, Turn};

/// The reserved `live` turn, built from what the attach socket streams while a turn runs and
/// withdrawn once a `recorded` marker says the transcript holds it.
#[derive(Debug, Default)]
pub struct Live {
    builder: TurnBuilder,
    shown: bool,
}

impl Live {
    pub fn push(&mut self, block: &ContentBlock) {
        if !matches!(block, ContentBlock::UserMessage { .. }) {
            self.builder.push(block);
        }
    }

    pub fn turn(&mut self, workers: &Workers) -> Option<Turn> {
        let blocks = self.builder.blocks(workers, false);
        if blocks.is_empty() {
            return None;
        }
        self.shown = true;
        Some(Turn {
            blocks,
            ..Turn::new(LIVE_ID, Role::Assistant, None)
        })
    }

    /// Starts over, and hands back the withdrawal owed for a turn the client was shown.
    pub fn reset(&mut self) -> Option<Turn> {
        let shown = std::mem::take(&mut self.shown);
        self.builder = TurnBuilder::default();
        shown.then(|| Turn::new(LIVE_ID, Role::Assistant, None))
    }
}
