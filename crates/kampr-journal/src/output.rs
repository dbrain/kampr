use crate::model::{Block, CodeRole, Turn};

/// Where a card's own blocks end: the run of input, patch and launch blocks written between it and
/// the next card. Appending to the turn instead would put one call's answer under the call after
/// it, which is exactly what a reader cannot check.
fn group_end(blocks: &[Block], at: usize) -> usize {
    blocks
        .iter()
        .enumerate()
        .skip(at + 1)
        .find(|(_, block)| !matches!(block, Block::Code { .. } | Block::Diff { .. } | Block::Sub { .. }))
        .map_or(blocks.len(), |(n, _)| n)
}

/// The result block for the card at `at`, revised where one is already there and inserted where it
/// is not. `Some(n)` is the index a block was inserted at, and nothing when the existing block was
/// rewritten — a result delivered twice must revise, never append, or the tool renders twice.
pub fn place(turn: &mut Turn, at: usize, text: String) -> Option<usize> {
    let end = group_end(&turn.blocks, at);
    for block in &mut turn.blocks[at + 1..end] {
        if let Block::Code {
            role: Some(CodeRole::Output),
            text: carried,
            ..
        } = block
        {
            *carried = text;
            return None;
        }
    }
    turn.blocks.insert(
        end,
        Block::Code {
            lang: None,
            text,
            role: Some(CodeRole::Output),
        },
    );
    Some(end)
}
