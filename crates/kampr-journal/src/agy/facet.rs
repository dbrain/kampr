use std::path::Path;

use crate::facet::{Compaction, Facets};
use crate::scan::records;

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
    let mut facets = Facets::default();
    for line in records(transcript) {
        let Ok(record) = serde_json::from_str::<Record>(&line) else {
            continue;
        };
        if (record.source.as_deref(), record.kind.as_deref()) == (Some("SYSTEM"), Some("CHECKPOINT")) {
            facets.compactions.push(Compaction {
                at: record.created_at,
                ..Compaction::default()
            });
        }
    }
    facets
}
