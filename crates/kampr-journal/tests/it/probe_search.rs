//! What a transcript search actually costs the node, measured on this machine's own transcripts
//! rather than on a fixture — a claim about milliseconds has to come from somewhere.
//!
//! Run it by hand, naming a transcript:
//!
//! ```text
//! KAMPR_PROBE_TRANSCRIPT=~/.claude/projects/-home-u-x/<session>.jsonl \
//!   cargo test -p kampr-journal --test it probe_search_cost -- --ignored --nocapture
//! ```
//!
//! Ignored by default: it reads a path outside the repo, and the suite must not depend on whose
//! machine it is running on.

use std::time::Instant;

use kampr_journal::{ClaudeAdapter, FileJournal, Journal, JournalAdapter, TranscriptRoot};

#[test]
#[ignore = "measures a transcript named by the environment"]
fn probe_search_cost() {
    let path = std::env::var("KAMPR_PROBE_TRANSCRIPT").expect("KAMPR_PROBE_TRANSCRIPT");
    let path = std::path::PathBuf::from(path);
    let bytes = std::fs::metadata(&path).expect("the transcript").len();
    // The root two levels above `projects/<slug>/<id>.jsonl`.
    let root = path
        .parent()
        .and_then(|p| p.parent())
        .and_then(|p| p.parent())
        .expect("a .claude root above the transcript")
        .to_path_buf();
    let adapter = ClaudeAdapter::new(TranscriptRoot::new(root).expect("a root"));
    let mut journal = FileJournal::new(path.clone(), adapter.parser(), None);

    let parse = Instant::now();
    journal.poll().expect("a parse");
    let parsed = parse.elapsed();
    let turns = journal.turn_ids().len();
    println!(
        "{} MiB, {turns} turns, parsed in {parsed:?}",
        bytes / (1024 * 1024)
    );

    // A rare word, a common one, and the two-character floor — the worst case the client can ask
    // for, since every shorter query is refused.
    for query in ["scrollbar", "the", "th"] {
        let mut runs = Vec::new();
        for _ in 0..5 {
            let at = Instant::now();
            let found = journal.search(query, 50);
            runs.push(at.elapsed());
            if runs.len() == 1 {
                println!(
                    "  {query:?}: {} matching turns, {} listed",
                    found.total,
                    found.hits.len()
                );
            }
        }
        runs.sort();
        println!("  {query:?}: median {:?}, worst {:?}", runs[2], runs[4]);
    }
}
