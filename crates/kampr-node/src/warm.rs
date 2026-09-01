//! What a conversation cost to open, kept for the next reader of the same pane.
//!
//! **The grid is warm and the conversation was not.** A pane's stream is held across a re-watch by
//! the registry (#252), so the grid a reader comes back to is the one they left; the conversation
//! was built by the pump that `watch` spawned and destroyed by the `unwatch` that stopped it, so
//! every return to a pane found the transcript again, parsed the whole of it, and folded it again.
//! Measured on a 30.7 MB transcript: **1.99 s to the first conversation message and 0.86 s more
//! for the facets, on the first open and on every re-watch alike** (#409). That is the panel a
//! reader saw sitting on an old conversation after they had come back to it.
//!
//! Held by the node rather than by the session, which is the difference between surviving a view
//! switch and surviving a phone going into a pocket: a dropped socket is a new [`Session`], and
//! warmth kept there would be thrown away exactly when a reconnecting client most needs it.
//!
//! **Taken, not shared.** A [`Journal`](kampr_journal::Journal) hands back what changed *since the
//! last read*, so two pumps polling one would each get half of it. A watcher takes the entry out
//! of this table for as long as it holds the pane and puts it back when its handle drops, so at
//! most one pump can ever be reading it — and a second session watching the same pane meanwhile
//! opens its own, exactly as it did before any of this existed.

use crate::convo::{Warm, Warmth, warmth};
use std::sync::Mutex;

/// How many conversations a node keeps warm for panes nobody is watching.
///
/// A parsed transcript is the size of the file it came from, and this is memory held for a reader
/// who may not come back. Four is the widest mosaic a phone draws (`MosaicSwitcher`), so a reader
/// who steps out of one and back into it finds every pane of it warm, and nothing beyond the set
/// they were actually looking at is kept.
const KEEP: usize = 4;

/// Newest last, and short enough that a scan is cheaper than a map.
#[derive(Default)]
pub struct ConvoWarmth(Mutex<Vec<(String, Warmth)>>);

impl ConvoWarmth {
    /// The conversation this pane was left in the middle of, or a cold one. Either way it is now
    /// the caller's alone until [`put`](Self::put) hands it back.
    pub fn take(&self, pane: &str) -> Warmth {
        let mut kept = self.0.lock().unwrap();
        match kept.iter().position(|(id, _)| id == pane) {
            Some(at) => kept.remove(at).1,
            None => warmth(),
        }
    }

    /// Hands a pane's conversation back when its watcher lets go.
    ///
    /// A conversation that never opened is not worth keeping and is not kept: it holds no parse
    /// and no fold, so putting it here would only evict one that does.
    pub fn put(&self, pane: &str, warm: Warmth) {
        if warm.lock().map(|w| Warm::cold(&w)).unwrap_or(true) {
            return;
        }
        let mut kept = self.0.lock().unwrap();
        kept.retain(|(id, _)| id != pane);
        kept.push((pane.to_string(), warm));
        let over = kept.len().saturating_sub(KEEP);
        kept.drain(..over);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn opened(path: &str) -> Warmth {
        let warm = warmth();
        warm.lock().unwrap().opened = Some(PathBuf::from(path));
        warm
    }

    fn held(table: &ConvoWarmth, pane: &str) -> Option<PathBuf> {
        let taken = table.take(pane);
        let path = taken.lock().unwrap().opened.clone();
        table.put(pane, taken);
        path
    }

    #[test]
    fn a_pane_let_go_of_is_handed_back_to_whoever_watches_it_next() {
        let table = ConvoWarmth::default();
        table.put("p1", opened("/t/one.jsonl"));
        assert_eq!(held(&table, "p1"), Some(PathBuf::from("/t/one.jsonl")));
    }

    /// A journal hands back what changed *since the last read*, so two pumps reading one would
    /// each get half of it. Taking is what makes that impossible.
    #[test]
    fn a_pane_already_taken_is_not_handed_out_a_second_time() {
        let table = ConvoWarmth::default();
        table.put("p1", opened("/t/one.jsonl"));
        let first = table.take("p1");
        assert_eq!(table.take("p1").lock().unwrap().opened, None);
        assert_eq!(first.lock().unwrap().opened, Some(PathBuf::from("/t/one.jsonl")),);
    }

    /// A parsed transcript is the size of the file it came from, so this table is a cache and has
    /// to have a floor to fall through. The oldest goes.
    #[test]
    fn the_oldest_conversation_goes_when_there_are_more_than_a_mosaic_of_them() {
        let table = ConvoWarmth::default();
        for n in 0..KEEP + 2 {
            table.put(&format!("p{n}"), opened(&format!("/t/{n}.jsonl")));
        }
        assert_eq!(held(&table, "p0"), None);
        assert_eq!(held(&table, "p1"), None);
        assert_eq!(
            held(&table, &format!("p{}", KEEP + 1)),
            Some(PathBuf::from(format!("/t/{}.jsonl", KEEP + 1))),
        );
    }

    /// Watching a pane again moves it to the front of the queue rather than adding a second entry
    /// for it — a reader flipping between two panes would otherwise evict everything else with
    /// copies of the two they were looking at.
    #[test]
    fn a_pane_watched_twice_is_one_entry_and_the_newest_one() {
        let table = ConvoWarmth::default();
        table.put("p1", opened("/t/one.jsonl"));
        table.put("p1", opened("/t/two.jsonl"));
        for n in 0..KEEP - 1 {
            table.put(&format!("p{n}x"), opened("/t/filler.jsonl"));
        }
        assert_eq!(held(&table, "p1"), Some(PathBuf::from("/t/two.jsonl")));
    }

    /// A pane whose conversation never opened holds no parse and no fold, so keeping it would only
    /// evict one that does. A pane on another host is exactly this case: the node that owns it
    /// does the reading.
    #[test]
    fn a_conversation_that_never_opened_is_not_worth_a_place() {
        let table = ConvoWarmth::default();
        table.put("p1", opened("/t/one.jsonl"));
        for n in 0..KEEP + 2 {
            table.put(&format!("relayed{n}"), warmth());
        }
        assert_eq!(held(&table, "p1"), Some(PathBuf::from("/t/one.jsonl")));
    }
}
