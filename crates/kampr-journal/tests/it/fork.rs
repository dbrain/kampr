//! A transcript two processes wrote is a tree, and a reader must not draw it interleaved (#655).

use crate::common::*;
use serde_json::json;

fn rec(uuid: &str, parent: &str, kind: &str, text: &str) -> serde_json::Value {
    json!({
        "type": kind,
        "uuid": uuid,
        "parentUuid": parent,
        "timestamp": "2026-08-20T02:56:27.000Z",
        "message": { "role": kind, "content": text },
    })
}

/// The turns a client would draw: the ones with anything in them, in the order the store keeps
/// them.
fn shown(turns: &[kampr_journal::Turn]) -> Vec<&str> {
    turns
        .iter()
        .filter(|t| !t.blocks.is_empty())
        .map(|t| t.id.as_str())
        .collect()
}

#[test]
fn a_branched_transcript_is_read_as_one_line_not_two() {
    let mut scratch = scratch_claude(
        "fork",
        &[
            rec("p", "root", "user", "q"),
            rec("a1", "p", "assistant", "a1"),
            rec("b1", "p", "assistant", "b1"),
            rec("a2", "a1", "assistant", "a2"),
            rec("b2", "b1", "assistant", "b2"),
        ],
    );
    let turns = scratch.turns();
    assert_eq!(
        shown(&turns),
        ["p", "b1", "b2"],
        "the branch written last is the one shown, in its own order"
    );
    assert_eq!(md_texts(&turns), ["q", "b1", "b2"]);
}

#[test]
fn a_branch_written_again_after_the_other_is_shown_again() {
    let mut scratch = scratch_claude(
        "fork-back",
        &[
            rec("p", "root", "user", "q"),
            rec("a1", "p", "assistant", "a1"),
            rec("b1", "p", "assistant", "b1"),
            rec("a2", "a1", "assistant", "a2"),
        ],
    );
    let turns = scratch.turns();
    assert_eq!(shown(&turns), ["p", "a1", "a2"], "a2 was written last");

    append(
        &scratch.transcript,
        &rec("b3", "b1", "assistant", "b3").to_string(),
    );
    let grown = scratch.turns();
    assert_eq!(
        shown(&grown),
        ["b1", "b3"],
        "b wrote last, so b's line is the one standing — a1 and a2 withdrawn, b1 restored"
    );
    assert!(
        grown.iter().any(|t| t.id == "a1" && t.blocks.is_empty()),
        "a1 is withdrawn, not deleted"
    );
}

#[test]
fn a_line_that_never_branched_is_undisturbed() {
    let mut scratch = scratch_claude(
        "fork-none",
        &[
            rec("p", "root", "user", "q"),
            rec("a1", "p", "assistant", "a1"),
            rec("u2", "a1", "user", "more"),
            rec("a2", "u2", "assistant", "a2"),
        ],
    );
    let turns = scratch.turns();
    assert_eq!(shown(&turns), ["p", "a1", "u2", "a2"]);
    assert_eq!(md_texts(&turns), ["q", "a1", "more", "a2"]);
}

fn append(path: &std::path::Path, line: &str) {
    use std::io::Write;
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(format!("{line}\n").as_bytes()).unwrap();
    f.flush().unwrap();
}
