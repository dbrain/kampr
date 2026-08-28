mod common;

use std::path::PathBuf;

use common::*;
use kampr_journal::{
    AgyAdapter, ClaudeAdapter, CodexAdapter, Facets, JournalAdapter, Registry, SessionMarker, TitleSource,
    TranscriptRoot,
};

fn claude() -> ClaudeAdapter {
    ClaudeAdapter::new(TranscriptRoot::new(facets_root()).expect("a root"))
}

fn facets_of(session: &str, marker: Option<&SessionMarker>) -> Facets {
    claude().facets(&facets_transcript(session), marker)
}

fn marker_named(name: &str, source: Option<&str>) -> SessionMarker {
    SessionMarker {
        agent: "claude".into(),
        pid: 1400898,
        session: FACETS_TITLED.into(),
        cwd: Some(PathBuf::from("/home/u/facets")),
        name: Some(name.into()),
        name_source: source.map(str::to_string),
        status: Some("busy".into()),
        transcript: None,
    }
}

/// Codex and agy fill what probe #322 measured on them and not one field more: neither writes a
/// session title or keeps a queue, and agy records no mode and no duration either. `tests/
/// harness_facets.rs` is where what they *do* fill is asserted.
#[test]
fn a_harness_with_nothing_to_say_says_nothing() {
    let codex = CodexAdapter::new(TranscriptRoot::new(codex_root()).expect("a root"));
    let agy = AgyAdapter::new(TranscriptRoot::new(agy_root()).expect("a root"));

    let codex = codex.facets(&codex_transcript(), None);
    assert_eq!(codex.title, None);
    assert_eq!(codex.queued, []);

    let agy = agy.facets(&agy_transcript(), None);
    assert_eq!(agy.title, None);
    assert_eq!(agy.queued, []);
    assert_eq!(agy.timings, []);
    assert_eq!(agy.mode, None);

    assert_eq!(
        serde_json::to_value(Facets::default()).expect("serialises"),
        serde_json::json!({}),
        "every facet is absent by default, so a client draws nothing for what it does not get"
    );
}

#[test]
fn the_title_the_operator_typed_beats_the_one_the_harness_generated() {
    let title = facets_of(FACETS_TITLED, None).title.expect("a title");

    assert_eq!(title.text, "the width inference rewrite");
    assert_eq!(title.source, TitleSource::Manual);
}

#[test]
fn a_generated_title_stands_only_where_nothing_manual_exists() {
    let title = facets_of(FACETS_GENERATED, None).title.expect("a title");

    assert_eq!(
        title.text, "Probe rows and measurements",
        "a harness rewrites its generated title as the conversation moves, so the last one is the \
         one it has now"
    );
    assert_eq!(title.source, TitleSource::Generated);
}

#[test]
fn a_manual_title_in_the_transcript_counts_as_manual_too() {
    let title = facets_of(FACETS_RECORDED, None).title.expect("a title");

    assert_eq!(title.text, "the fit ladder");
    assert_eq!(title.source, TitleSource::Manual);
}

#[test]
fn the_harness_session_name_is_the_last_resort_and_never_beats_a_title() {
    let marker = marker_named("kampr-fb", Some("derived"));

    assert_eq!(
        facets_of(FACETS_TITLED, Some(&marker))
            .title
            .expect("a title")
            .text,
        "the width inference rewrite",
        "a name the harness derived does not displace a title the operator typed"
    );
    assert_eq!(
        facets_of(FACETS_GENERATED, Some(&marker))
            .title
            .expect("a title")
            .text,
        "Probe rows and measurements",
        "nor one the harness generated"
    );

    let named = facets_of(FACETS_UNTITLED, Some(&marker)).title.expect("a title");
    assert_eq!(named.text, "kampr-fb");
    assert_eq!(
        named.source,
        TitleSource::Generated,
        "`nameSource` is measured as auto, derived and absent (#311) — none of them is a person"
    );
    assert_eq!(
        facets_of(FACETS_UNTITLED, None).title,
        None,
        "and a session with no name anywhere carries no title at all"
    );
}

#[test]
fn a_turn_that_was_timed_carries_the_duration_the_harness_recorded() {
    let facets = facets_of(FACETS_TITLED, None);

    let timings: Vec<(&str, u64, Option<u32>)> = facets
        .timings
        .iter()
        .map(|t| (t.turn.as_str(), t.duration_ms, t.messages))
        .collect();
    assert_eq!(
        timings,
        [
            ("22222222-0000-4000-8000-000000000002", 315_990, Some(144)),
            ("66666666-0000-4000-8000-000000000006", 204_378, Some(198)),
        ],
        "a timing names the turn it closes, so nothing has to infer one from timestamps"
    );

    let mut journal = claude().open_path(facets_transcript(FACETS_TITLED));
    let ids = drain(journal.as_mut())
        .iter()
        .map(|t| t.id.clone())
        .collect::<Vec<_>>();
    for timing in &facets.timings {
        assert!(
            ids.contains(&timing.turn),
            "{} is not a turn this transcript produced",
            timing.turn
        );
    }
}

#[test]
fn a_prompt_still_waiting_is_queued_and_one_the_turn_absorbed_is_not() {
    let facets = facets_of(FACETS_TITLED, None);

    let queued: Vec<&str> = facets.queued.iter().map(|q| q.text.as_str()).collect();
    assert_eq!(
        queued,
        ["and add a probe row for it"],
        "an enqueue the harness later removed is not a prompt anybody is still waiting on"
    );
    assert_eq!(facets.queued[0].at.as_deref(), Some("2026-08-27T11:55:02.001Z"));
}

/// `enqueue` and `remove` are half the vocabulary. Folding those two alone left **141** prompts
/// standing on a real session that had worked every one of them, because the ordinary delivery
/// leaves a `dequeue` with a null `content` and a `/clear` leaves a `popAll`.
#[test]
fn a_queue_is_worked_by_four_operations_and_folding_two_of_them_leaves_it_full() {
    let facets = facets_of(FACETS_QUEUE, None);

    let queued: Vec<&str> = facets.queued.iter().map(|q| q.text.as_str()).collect();
    assert_eq!(queued, ["fifth"]);
}

#[test]
fn a_session_the_harness_named_carries_that_name_with_no_marker_in_hand() {
    let title = facets_of(FACETS_QUEUE, None).title.expect("a title");

    assert_eq!(title.text, "kampr-queue");
    assert_eq!(title.source, TitleSource::Generated);
    assert_eq!(
        facets_of(FACETS_QUEUE, Some(&marker_named("kampr-live", None)))
            .title
            .expect("a title")
            .text,
        "kampr-live",
        "the marker is the live copy; the transcript records the name as of its last write"
    );
}

#[test]
fn the_mode_a_session_is_in_is_the_last_one_it_recorded() {
    let mode = facets_of(FACETS_TITLED, None).mode.expect("a mode");

    assert_eq!(mode.mode.as_deref(), Some("plan"));
    assert_eq!(mode.permission.as_deref(), Some("bypassPermissions"));
    assert_eq!(
        facets_of(FACETS_GENERATED, None).mode,
        None,
        "a session that never recorded one has no mode rather than a default one"
    );
}

#[test]
fn a_compaction_says_what_it_dropped_and_where_it_fell() {
    let facets = facets_of(FACETS_TITLED, None);

    assert_eq!(facets.compactions.len(), 1);
    let compaction = &facets.compactions[0];
    assert_eq!(compaction.trigger.as_deref(), Some("manual"));
    assert_eq!(compaction.pre_tokens, Some(756_165));
    assert_eq!(compaction.post_tokens, Some(18_709));
    assert_eq!(compaction.dropped_tokens, Some(737_456));
    assert_eq!(compaction.at.as_deref(), Some("2026-08-27T12:01:24.491Z"));
}

#[test]
fn a_registry_asks_the_adapter_the_pane_is_running_and_nobody_else() {
    let mut registry = Registry::new();
    registry.register(std::sync::Arc::new(claude()));
    registry.register(std::sync::Arc::new(CodexAdapter::new(
        TranscriptRoot::new(codex_root()).expect("a root"),
    )));

    let transcript = facets_transcript(FACETS_TITLED);
    assert_eq!(
        registry
            .facets(Some("claude"), &transcript, None)
            .title
            .expect("a title")
            .text,
        "the width inference rewrite"
    );
    assert_eq!(
        registry.facets(Some("codex"), &transcript, None),
        Facets::default(),
        "the codex adapter reads codex records, and a claude transcript holds none"
    );
    assert_eq!(registry.facets(None, &transcript, None), Facets::default());
    assert_eq!(
        registry.facets(Some("agy"), &transcript, None),
        Facets::default(),
        "an agent with no adapter registered is not an error, it is a session with no facets"
    );
}

/// A launched conversation is filed under the launching session's own directory, so the title
/// beside that session is the title of everything inside it — the walk back out of `subagents/`
/// is what makes that hold at any depth.
#[test]
fn a_conversation_one_of_these_launched_carries_the_session_title_it_was_launched_from() {
    let adapter = ClaudeAdapter::new(TranscriptRoot::new(claude_root()).expect("a root"));
    let launched = claude_root()
        .join("projects/-home-u-agents/7a2f1d00-0000-4000-8000-00000000000a/subagents")
        .join("agent-4b7c9e21.jsonl");

    let title = adapter.facets(&launched, None).title.expect("a title");
    assert_eq!(title.text, "manage ops");
    assert_eq!(title.source, TitleSource::Manual);
}

#[test]
fn a_transcript_that_is_not_there_is_a_session_with_no_facets() {
    assert_eq!(
        claude().facets(&facets_root().join("projects/-home-u-facets/nothing.jsonl"), None),
        Facets::default()
    );
}
