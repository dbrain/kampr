use crate::common::*;
use kampr_journal::{AgyAdapter, CodexAdapter, Facets, JournalAdapter, TranscriptRoot};

fn codex() -> CodexAdapter {
    CodexAdapter::new(TranscriptRoot::new(harness_facets_root("codex")).expect("a root"))
}

fn agy() -> AgyAdapter {
    AgyAdapter::new(TranscriptRoot::new(harness_facets_root("agy")).expect("a root"))
}

fn codex_facets() -> Facets {
    codex().facets(&codex_facets_transcript(), None)
}

fn agy_facets() -> Facets {
    agy().facets(&agy_facets_transcript(), None)
}

fn turn_ids(journal: &mut dyn kampr_journal::Journal) -> Vec<String> {
    drain(journal).iter().map(|t| t.id.clone()).collect()
}

#[test]
fn a_codex_timing_carries_the_duration_the_harness_recorded_rather_than_one_inferred_from_two_stamps() {
    let facets = codex_facets();

    let timings: Vec<(&str, u64, Option<u32>)> = facets
        .timings
        .iter()
        .map(|t| (t.turn.as_str(), t.duration_ms, t.messages))
        .collect();
    assert_eq!(
        timings,
        [
            ("x12", 63_552, None),
            ("x20", 16_242, None),
            ("x24", 786_363, None)
        ],
        "`task_complete` hands over `duration_ms` outright, and Codex counts no messages"
    );
}

/// The `turn_id` on `task_complete` is the harness's own handle for a turn — it appears on
/// `turn_context` and `task_started` and nowhere the parser ever mints an id from. Emitting it
/// would put a timing on the wire that no turn a client holds can carry.
#[test]
fn a_codex_timing_names_a_turn_the_parser_produced_and_never_the_harnesss_own_turn_id() {
    let facets = codex_facets();
    let mut journal = codex().open_path(codex_facets_transcript());
    let ids = turn_ids(journal.as_mut());

    assert!(!facets.timings.is_empty(), "the fixture records three turns");
    for timing in &facets.timings {
        assert!(
            ids.contains(&timing.turn),
            "{} is not a turn this transcript produced",
            timing.turn
        );
    }
    assert!(
        !ids.iter().any(|id| id.contains('-')),
        "the parser mints `x<n>`, so a uuid-shaped turn is the harness's handle leaking through"
    );
}

/// A naive "last response item before the `task_complete`" walk lands on a `developer` message, a
/// user record that is nothing but `<environment_context>`, a `reasoning` item or a tool
/// *output* — none of which the parser turns into anything.
///
/// **The third turn here is the one that catches it, and it is a real shape**: one of the 118
/// `task_complete` records on this machine ends a turn that wrote no closing message at all,
/// because the harness hit its usage limit and stopped after a patch. Its last response item is
/// the `custom_tool_call_output`, which revises the card its call opened rather than being a turn
/// of its own.
#[test]
fn a_codex_record_the_parser_drops_never_becomes_the_turn_a_timing_hangs_off() {
    let facets = codex_facets();
    let mut journal = codex().open_path(codex_facets_transcript());
    let ids = turn_ids(journal.as_mut());

    assert_eq!(
        ids,
        ["x6", "x9", "x12", "x17", "x18", "x20", "x22", "x24"],
        "lines 2, 3, 8, 10, 19, 23 and 27 are records the parser produces no turn for"
    );
    let named: Vec<&str> = facets.timings.iter().map(|t| t.turn.as_str()).collect();
    assert!(
        !named.contains(&"x10") && !named.contains(&"x19") && !named.contains(&"x27"),
        "a tool output revises the turn its call opened; it is not one of its own"
    );
}

/// `context_compacted`'s whole payload is `{"type":"context_compacted"}` and the `compacted`
/// record beside it carries `replacement_history` with no counts anywhere — so a compaction can
/// say *here* and nothing else. Counting the history's entries would be a number the harness
/// never wrote.
#[test]
fn a_codex_compaction_says_where_it_fell_and_invents_no_token_counts() {
    let facets = codex_facets();

    assert_eq!(facets.compactions.len(), 1);
    let compaction = &facets.compactions[0];
    assert_eq!(compaction.at.as_deref(), Some("2026-08-20T05:52:09.001Z"));
    assert_eq!(compaction.trigger, None);
    assert_eq!(compaction.pre_tokens, None);
    assert_eq!(compaction.post_tokens, None);
    assert_eq!(compaction.dropped_tokens, None);
    assert_eq!(
        serde_json::to_value(compaction).expect("serialises"),
        serde_json::json!({"at": "2026-08-20T05:52:09.001Z"}),
        "a client is told the position and is told nothing it could mistake for a count"
    );
}

#[test]
fn the_codex_mode_is_the_last_turn_context_and_is_recorded_on_codexs_own_axes() {
    let mode = codex_facets().mode.expect("a mode");

    assert_eq!(mode.mode.as_deref(), Some("default"));
    assert_eq!(
        mode.permission.as_deref(),
        Some("on-request"),
        "the second `turn_context` moved the approval policy off `never`"
    );
}

/// The 2700 `payload.name` hits across this machine's rollouts are tool-call names, and there is
/// no queue record of any kind.
#[test]
fn codex_has_no_session_title_and_no_queue_so_it_offers_neither() {
    let facets = codex_facets();

    assert_eq!(facets.title, None);
    assert_eq!(facets.queued, []);
}

#[test]
fn an_agy_checkpoint_is_a_compaction_boundary_with_no_counts_behind_it() {
    let facets = agy_facets();

    let at: Vec<Option<&str>> = facets.compactions.iter().map(|c| c.at.as_deref()).collect();
    assert_eq!(at, [Some("2026-08-26T05:19:26Z"), Some("2026-08-26T06:02:11Z")]);
    for compaction in &facets.compactions {
        assert_eq!(
            serde_json::to_value(compaction).expect("serialises"),
            serde_json::json!({"at": compaction.at}),
            "the record is prose and a step index; nothing in it is a token count"
        );
    }
}

/// The gap between two `created_at` stamps holds the operator reading, thinking and typing.
/// #322 refuses it by name: it is not a duration the harness recorded, and a facet filled from a
/// field that merely reads like one cannot be told apart from a real one on the wire.
#[test]
fn agy_never_turns_the_gap_between_two_steps_into_a_timing() {
    let facets = agy_facets();

    assert_eq!(facets.timings, []);
    assert_eq!(facets.title, None);
    assert_eq!(facets.queued, []);
    assert_eq!(facets.mode, None);
}
