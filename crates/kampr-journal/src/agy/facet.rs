use std::path::Path;

use crate::facet::{Compaction, FacetFold, Facets};
use crate::marker::SessionMarker;
use crate::scan::{Appended, Cursor};

use super::record::Record;

/// One facet, and position only. A `SYSTEM` / `CHECKPOINT` step is the harness telling its model
/// what it dropped — the same boundary Codex's `context_compacted` marks — and it carries a step
/// index, a timestamp and prose. There is no token count anywhere in it and no trigger it did not
/// choose itself.
///
/// **The `created_at` deltas between steps are deliberately not a timing** (#322): the gap
/// between two of them holds the operator reading and typing, which is a different fact from a
/// duration the harness recorded, and a facet filled from a field that merely reads like one
/// cannot be told apart from a real one once it is on the wire.
pub fn collect(transcript: &Path) -> Facets {
    Fold::default().facets(transcript, None)
}

/// The same fold, kept between reads.
#[derive(Default)]
pub struct Fold {
    cursor: Cursor,
    accumulated: Facets,
}

impl FacetFold for Fold {
    fn facets(&mut self, transcript: &Path, _marker: Option<&SessionMarker>) -> Facets {
        let mut appended = Appended::open(transcript, self.cursor);
        if appended.restarted() {
            *self = Self::default();
        }
        for line in appended.by_ref() {
            self.push(&line);
        }
        self.cursor = appended.cursor();
        self.accumulated.clone()
    }
}

impl Fold {
    fn push(&mut self, line: &str) {
        let Ok(record) = serde_json::from_str::<Record>(line) else {
            return;
        };
        if (record.source.as_deref(), record.kind.as_deref()) == (Some("SYSTEM"), Some("CHECKPOINT")) {
            self.accumulated.compactions.push(Compaction {
                at: record.created_at,
                ..Compaction::default()
            });
        }
    }
}
