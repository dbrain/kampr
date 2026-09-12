//! Searching a transcript where it lives.
//!
//! The client's own search is over the turns it holds, which on a long session is the newest page
//! and whatever the reader has paged back to. This is the whole transcript, so what it owes the
//! reader is a count that means what it says and a hit that can be aimed at: a turn id, how far
//! back it is, and enough of the line to read where it cannot yet be scrolled to.

use crate::common::*;
use kampr_journal::{Block, ConvoFound, FileJournal, Journal, Role, ToolState, Turn, search_turns};

const CAP: usize = 40;

fn turn(id: &str, role: Role, blocks: Vec<Block>) -> Turn {
    let mut turn = Turn::new(id, role, Some("2026-08-23T09:00:00.000Z".into()));
    turn.blocks = blocks;
    turn
}

fn said(id: &str, text: &str) -> Turn {
    turn(id, Role::Assistant, vec![Block::md(text)])
}

fn found(turns: &[Turn], query: &str) -> ConvoFound {
    search_turns(turns, query, CAP)
}

fn ids(found: &ConvoFound) -> Vec<&str> {
    found.hits.iter().map(|h| h.turn.as_str()).collect()
}

#[test]
fn a_hit_carries_the_turn_it_is_in_how_far_back_it_is_and_the_line_it_matched() {
    let turns = vec![
        said("a-1", "the scrollbar column is the one it keeps back"),
        said("a-2", "nothing to find here"),
        said("a-3", "and the scrollbar again"),
    ];
    let out = found(&turns, "scrollbar");
    assert_eq!(out.total, 2);
    // Newest first: the reader is standing at the newest end and pages backwards from it.
    assert_eq!(ids(&out), ["a-3", "a-1"]);
    assert_eq!(out.hits[0].from_end, 0);
    assert_eq!(out.hits[1].from_end, 2);
    assert_eq!(out.hits[1].text, "the scrollbar column is the one it keeps back");
    assert_eq!(out.hits[0].role, Role::Assistant);
}

#[test]
fn a_turn_is_one_result_however_many_times_it_says_the_word() {
    let turns = vec![said("a-1", "scrollbar, scrollbar, scrollbar")];
    let out = found(&turns, "scrollbar");
    assert_eq!(out.total, 1, "one turn, one result");
    assert_eq!(out.hits[0].hits, 3, "and it says how many times it holds it");
}

#[test]
fn the_count_is_every_matching_turn_and_the_list_is_capped() {
    let turns: Vec<Turn> = (0..30)
        .map(|n| said(&format!("a-{n}"), "the scrollbar"))
        .collect();
    let out = search_turns(&turns, "scrollbar", 4);
    assert_eq!(out.total, 30);
    assert_eq!(out.hits.len(), 4, "the list is capped");
    assert_eq!(
        ids(&out),
        ["a-29", "a-28", "a-27", "a-26"],
        "and it is the newest four"
    );
}

#[test]
fn the_match_is_case_insensitive_and_needs_two_characters_like_the_client_that_asks() {
    let turns = vec![said("a-1", "The Scrollbar Column")];
    assert_eq!(found(&turns, "scrollbar").total, 1);
    assert_eq!(found(&turns, "SCROLLBAR").total, 1);
    assert_eq!(found(&turns, "s").total, 0, "one character is not a search");
    assert_eq!(found(&turns, "").total, 0);
}

/// The other half of this rule is `Search.kt`'s `blockText`, and the two have to agree: a hit the
/// count promises and the screen cannot show is worse than a screen that is too long.
#[test]
fn what_is_searched_is_what_the_client_draws_of_each_block() {
    let turns = vec![
        turn(
            "a-1",
            Role::Assistant,
            vec![Block::Tool {
                name: "Bash".into(),
                summary: Some("cargo test --workspace".into()),
                lines: Some(12),
                state: ToolState::Done,
            }],
        ),
        turn(
            "a-2",
            Role::Assistant,
            vec![Block::Diff {
                path: Some("crates/kampr-node/src/session.rs".into()),
                text: "-old\n+new".into(),
            }],
        ),
        turn(
            "a-3",
            Role::Assistant,
            vec![Block::Code {
                lang: Some("rust".into()),
                text: "fn widen() {}".into(),
                role: None,
            }],
        ),
        turn(
            "a-4",
            Role::Assistant,
            vec![Block::Sub {
                id: "s-1".into(),
                kind: Some("Explore".into()),
                title: Some("the width inference".into()),
                depth: None,
            }],
        ),
    ];
    assert_eq!(ids(&found(&turns, "cargo test")), ["a-1"], "a tool's label");
    assert_eq!(ids(&found(&turns, "session.rs")), ["a-2"], "a patch's path");
    assert_eq!(ids(&found(&turns, "+new")), ["a-2"], "and its body");
    assert_eq!(ids(&found(&turns, "widen")), ["a-3"], "a code block");
    assert_eq!(
        ids(&found(&turns, "Explore")),
        ["a-4"],
        "a launched conversation's type"
    );
    assert_eq!(
        ids(&found(&turns, "width inference")),
        ["a-4"],
        "and what it was asked"
    );
}

#[test]
fn a_long_line_is_clipped_around_the_match_rather_than_shipped_whole() {
    let before = "x".repeat(400);
    let after = "y".repeat(400);
    let turns = vec![said("a-1", &format!("{before} scrollbar {after}"))];
    let out = found(&turns, "scrollbar");
    let text = &out.hits[0].text;
    assert!(
        text.chars().count() <= 200,
        "{} chars is not an excerpt",
        text.chars().count()
    );
    assert!(text.contains("scrollbar"), "the excerpt holds the match: {text}");
    assert!(
        text.starts_with('…') && text.ends_with('…'),
        "and says it was cut: {text}"
    );
}

#[test]
fn the_line_the_match_is_on_is_the_line_not_the_whole_turn() {
    let turns = vec![said("a-1", "first line\nthe scrollbar line\nthird line")];
    assert_eq!(found(&turns, "scrollbar").hits[0].text, "the scrollbar line");
}

/// The scan reads bytes when the query is ASCII, which is the ordinary case and the fast one — and
/// a transcript is full of em dashes, so it has to be right about an ASCII needle inside a haystack
/// that is not ASCII at all. UTF-8 makes that safe (#541); this is the test that says so.
#[test]
fn an_ascii_query_is_found_inside_prose_that_is_not_ascii() {
    let turns = vec![said(
        "a-1",
        "the seam — and the scrollbar — is the pane's own box",
    )];
    let out = found(&turns, "scrollbar");
    assert_eq!(out.total, 1);
    assert_eq!(
        out.hits[0].text,
        "the seam — and the scrollbar — is the pane's own box"
    );
    assert_eq!(
        ids(&found(&turns, "— and")),
        ["a-1"],
        "and a query with one in it"
    );
}

/// The other path, and the reason it is kept: a query that is not ASCII cannot be lowered by byte,
/// so it is compared character by character.
#[test]
fn a_query_that_is_not_ascii_still_matches_and_still_ignores_case() {
    let turns = vec![said("a-1", "the Grüße in the probe log")];
    assert_eq!(found(&turns, "grüße").total, 1, "lowered to match");
    assert_eq!(found(&turns, "Grüße").hits[0].text, "the Grüße in the probe log");
}

/// The same search over a real transcript, through the journal the node actually holds — the level
/// at which the turns are the folded ones a client is handed rather than a list built by hand.
#[test]
fn a_real_transcript_is_searched_through_the_journal_the_node_holds() {
    let mut journal = FileJournal::new(claude_transcript(), claude_parser(), None);
    journal.poll().unwrap();
    let all = journal.page_before(None, 100).turns;
    let word = all
        .iter()
        .find_map(|t| match t.blocks.first() {
            Some(Block::Md { text, .. }) => text.split_whitespace().find(|w| w.len() > 5),
            _ => None,
        })
        .expect("a word in the fixture's prose")
        .to_string();
    let out = journal.search(&word, CAP);
    assert!(out.total >= 1, "the fixture's own prose is found: {word}");
    let newest = &out.hits[0];
    assert!(
        all.iter().any(|t| t.id == newest.turn),
        "a hit names a turn the transcript has",
    );
}
