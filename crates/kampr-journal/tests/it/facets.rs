use std::path::PathBuf;

use crate::common::*;
use kampr_journal::{
    AgyAdapter, ClaudeAdapter, CodexAdapter, FacetFeed, Facets, JournalAdapter, Registry, SessionMarker,
    TitleSource, TranscriptRoot,
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
            .fold(Some("claude"))
            .moved(&transcript, None)
            .expect("facets")
            .title
            .expect("a title")
            .text,
        "the width inference rewrite"
    );
    assert_eq!(
        registry.fold(Some("codex")).moved(&transcript, None),
        None,
        "the codex adapter reads codex records, and a claude transcript holds none"
    );
    assert_eq!(registry.fold(None).moved(&transcript, None), None);
    assert_eq!(
        registry.fold(Some("agy")).moved(&transcript, None),
        None,
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

fn enqueue(text: &str, at: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "queue-operation", "operation": "enqueue", "timestamp": at, "content": text
    })
}

fn prompt(text: &str) -> serde_json::Value {
    serde_json::json!({ "type": "user", "uuid": text, "message": { "content": text } })
}

fn append(transcript: &std::path::Path, body: &str) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(transcript)
        .expect("a transcript to append to");
    file.write_all(body.as_bytes()).expect("appended");
}

fn queued_texts(facets: &Facets) -> Vec<&str> {
    facets.queued.iter().map(|q| q.text.as_str()).collect()
}

fn feed(scratch: &Scratch) -> FacetFeed {
    scratch.journals.fold(Some("claude"))
}

/// The reported defect: a prompt sent from a phone while the agent is working shows up in the
/// pane immediately and in the conversation not at all, because the facets were collected once
/// when the transcript was opened and never again.
#[test]
fn a_prompt_queued_after_the_facets_were_collected_is_folded_onto_them_rather_than_missed() {
    let scratch = scratch_claude("queued-late", &[prompt("go and read the log")]);
    let mut feed = feed(&scratch);

    assert_eq!(
        feed.moved(&scratch.transcript, None),
        None,
        "a session with nothing queued and nothing titled has nothing to say"
    );

    append(
        &scratch.transcript,
        &lines(&[enqueue("and copy the config across", "2026-08-28T02:10:59.658Z")]),
    );

    let moved = feed
        .moved(&scratch.transcript, None)
        .expect("the queued prompt reaches the client while it is still waiting");
    assert_eq!(queued_texts(&moved), ["and copy the config across"]);
    assert_eq!(moved.queued[0].at.as_deref(), Some("2026-08-28T02:10:59.658Z"));

    append(
        &scratch.transcript,
        &lines(&[serde_json::json!({
            "type": "queue-operation", "operation": "dequeue", "content": null
        })]),
    );
    assert_eq!(
        queued_texts(
            &feed
                .moved(&scratch.transcript, None)
                .expect("the queue emptying moved it")
        ),
        Vec::<&str>::new(),
        "the harness taking the prompt is a change like any other"
    );
}

#[test]
fn facets_that_have_not_moved_are_not_published_a_second_time() {
    let scratch = scratch_claude(
        "unmoved",
        &[
            serde_json::json!({ "type": "ai-title", "aiTitle": "the width inference rewrite" }),
            prompt("first"),
        ],
    );
    let mut feed = feed(&scratch);

    assert!(
        feed.moved(&scratch.transcript, None).is_some(),
        "the title is new"
    );
    assert_eq!(
        feed.moved(&scratch.transcript, None),
        None,
        "the same transcript, unchanged, is not a second frame"
    );

    append(&scratch.transcript, &lines(&[prompt("second")]));
    assert_eq!(
        feed.moved(&scratch.transcript, None),
        None,
        "a turn is not a facet: the conversation grew and none of these five moved"
    );
}

/// #259: `/clear` opens a new transcript, and a pane resolves onto it under the same path shape.
/// A fold that kept its accumulator would show the finished session's queue on the fresh one.
#[test]
fn a_transcript_replaced_under_the_fold_is_read_from_the_start_rather_than_folded_onto() {
    let scratch = scratch_claude(
        "cleared",
        &[
            serde_json::json!({ "type": "ai-title", "aiTitle": "the session that was cleared" }),
            enqueue("still waiting when it was cleared", "2026-08-28T02:10:59.658Z"),
            prompt("a turn that made the file long"),
        ],
    );
    let mut feed = feed(&scratch);
    assert_eq!(
        queued_texts(&feed.moved(&scratch.transcript, None).expect("facets")),
        ["still waiting when it was cleared"]
    );

    std::fs::write(
        &scratch.transcript,
        lines(&[serde_json::json!({ "type": "ai-title", "aiTitle": "after the clear" })]),
    )
    .expect("the transcript replaced");

    let moved = feed
        .moved(&scratch.transcript, None)
        .expect("a shorter file moved it");
    assert_eq!(moved.title.as_ref().expect("a title").text, "after the clear");
    assert_eq!(
        queued_texts(&moved),
        Vec::<&str>::new(),
        "the queue belonged to the session that was cleared"
    );
}

/// A transcript is appended to while it is read, so the last record of a poll is regularly half
/// written. Folding half of it and its remainder as a record of its own is how a queued prompt
/// arrives twice — or, once the halves parse as nothing, never at all.
#[test]
fn a_record_still_being_written_is_folded_once_it_is_whole_and_never_twice() {
    let scratch = scratch_claude("torn", &[prompt("go and read the log")]);
    let mut feed = feed(&scratch);
    feed.moved(&scratch.transcript, None);

    let record = enqueue("and copy the config across", "2026-08-28T02:10:59.658Z").to_string();
    let (head, tail) = record.split_at(record.len() / 2);
    append(&scratch.transcript, head);
    assert_eq!(
        feed.moved(&scratch.transcript, None),
        None,
        "half a record is not a record"
    );

    append(&scratch.transcript, &format!("{tail}\n"));
    assert_eq!(
        queued_texts(&feed.moved(&scratch.transcript, None).expect("the whole record")),
        ["and copy the config across"]
    );
    assert_eq!(
        feed.moved(&scratch.transcript, None),
        None,
        "and it is not folded a second time when the newline arrives"
    );
}

/// The resumable fold is every harness's, not Claude's: a `Fold` that restarted its record count
/// on the second read would name Codex's timings after the wrong turns, and one that lost its
/// accumulator would drop everything the first read collected.
#[test]
fn reading_a_transcript_in_two_parts_says_exactly_what_reading_it_whole_says() {
    let dir = scratch_dir("resumed");
    let harnesses: [(&str, Box<dyn JournalAdapter>, PathBuf); 3] = [
        ("claude", Box::new(claude()), facets_transcript(FACETS_TITLED)),
        (
            "codex",
            Box::new(CodexAdapter::new(
                TranscriptRoot::new(harness_facets_root("codex")).expect("a root"),
            )),
            codex_facets_transcript(),
        ),
        (
            "agy",
            Box::new(AgyAdapter::new(
                TranscriptRoot::new(harness_facets_root("agy")).expect("a root"),
            )),
            agy_facets_transcript(),
        ),
    ];

    for (agent, adapter, fixture) in harnesses {
        let body = std::fs::read_to_string(&fixture).expect("a fixture");
        let records: Vec<&str> = body.lines().collect();
        let path = dir.join(format!("{agent}.jsonl"));
        let head: String = records[..records.len() / 2]
            .iter()
            .map(|line| format!("{line}\n"))
            .collect();

        std::fs::write(&path, &head).expect("the first half");
        let mut fold = adapter.fold().expect("a resumable fold");
        fold.facets(&path, None);
        std::fs::write(&path, &body).expect("the rest");

        assert_eq!(
            fold.facets(&path, None),
            adapter.facets(&path, None),
            "{agent} read in two parts is not what {agent} read whole"
        );
    }
}

/// A harness whose collector cannot be resumed. It leaves `fold` alone, and the registry has to
/// answer with one that re-reads the transcript rather than one that freezes its facets at
/// whatever the conversation opened with.
struct Unresumable(ClaudeAdapter);

impl JournalAdapter for Unresumable {
    fn agent(&self) -> &str {
        "unresumable"
    }

    fn root(&self) -> &TranscriptRoot {
        self.0.root()
    }

    fn locate(&self, session: &kampr_journal::SessionRef) -> Result<PathBuf, kampr_journal::JournalError> {
        self.0.locate(session)
    }

    fn locate_by_cwd(
        &self,
        cwd: &std::path::Path,
        since: Option<std::time::SystemTime>,
    ) -> Result<PathBuf, kampr_journal::JournalError> {
        self.0.locate_by_cwd(cwd, since)
    }

    fn parser(&self) -> Box<dyn kampr_journal::TranscriptParser> {
        self.0.parser()
    }

    fn facets(&self, transcript: &std::path::Path, marker: Option<&SessionMarker>) -> Facets {
        self.0.facets(transcript, marker)
    }
}

#[test]
fn a_harness_with_no_resumable_fold_reads_its_transcript_again_rather_than_going_static() {
    let scratch = scratch_claude("unresumable", &[prompt("go and read the log")]);
    let mut registry = Registry::new();
    registry.register(std::sync::Arc::new(Unresumable(claude())));
    let mut feed = registry.fold(Some("unresumable"));
    assert_eq!(feed.moved(&scratch.transcript, None), None);

    append(
        &scratch.transcript,
        &lines(&[enqueue("and copy the config across", "2026-08-28T02:10:59.658Z")]),
    );

    assert_eq!(
        queued_texts(
            &feed
                .moved(&scratch.transcript, None)
                .expect("the whole file, read again")
        ),
        ["and copy the config across"],
        "the fallback costs what a whole-transcript read costs; it does not cost the facets"
    );
}

/// The herd path needs the levels apart, not the winner.
///
/// A pane entry is titled from the same transcript the conversation is, but its weakest level is
/// a different string: the herd drops a name the harness derived for itself (#311) where the
/// conversation keeps it. Handing back a resolved [`kampr_journal::Title`] would leave that
/// caller unable to substitute anything without re-reading the file.
#[test]
fn a_fold_hands_back_the_title_levels_so_a_caller_can_replace_the_weakest() {
    let mut registry = Registry::new();
    registry.register(std::sync::Arc::new(claude()));
    let mut fold = registry.folder(Some("claude"));

    let titles = fold.titles(&facets_transcript(FACETS_TITLED), None);

    assert_eq!(titles.manual.as_deref(), Some("the width inference rewrite"));
    assert_eq!(titles.generated.as_deref(), Some("Inferring a pane's width"));
    assert_eq!(titles.named.as_deref(), Some("kampr-fb"));
    assert_eq!(
        titles.resolve().expect("a title").text,
        "the width inference rewrite",
        "and resolving them is the same answer the conversation surface publishes"
    );
}

#[test]
fn a_harness_whose_fold_can_only_be_read_whole_still_says_which_level_its_title_came_from() {
    let mut registry = Registry::new();
    registry.register(std::sync::Arc::new(Unresumable(claude())));

    let manual = registry
        .folder(Some("unresumable"))
        .titles(&facets_transcript(FACETS_TITLED), None);
    assert_eq!(manual.manual.as_deref(), Some("the width inference rewrite"));
    assert_eq!(manual.generated, None);

    let generated = registry
        .folder(Some("unresumable"))
        .titles(&facets_transcript(FACETS_GENERATED), None);
    assert_eq!(generated.manual, None);
    assert_eq!(
        generated.generated.as_deref(),
        Some("Probe rows and measurements")
    );

    assert_eq!(
        registry
            .folder(None)
            .titles(&facets_transcript(FACETS_TITLED), None),
        kampr_journal::Titles::default(),
        "a pane with no harness is titled by nothing rather than by somebody else's transcript"
    );
}

/// The constraint the herd path exists to solve: the richer titles used to cost a whole-transcript
/// read per pane per rebuild, and a node rebuilds its herd on every structural event.
///
/// Proved by poisoning what has already been read. The prefix is overwritten in place — same
/// length, so nothing shrinks and the fold has no reason to restart — with a `custom-title`
/// record that outranks every generated one. A fold that re-read the file would answer with the
/// poison; one that reads only the growth cannot see it.
#[test]
fn a_second_look_at_a_title_reads_only_what_the_transcript_has_grown_by() {
    let poison = r#"{"type":"custom-title","customTitle":"a whole-transcript read"}"#;
    let first = pad_to(r#"{"type":"ai-title","aiTitle":"the first look"}"#, poison.len());
    let scratch = scratch_claude("grown-by", &[]);
    std::fs::write(&scratch.transcript, format!("{first}\n")).expect("a transcript");
    let mut fold = scratch.journals.folder(Some("claude"));

    assert_eq!(
        fold.titles(&scratch.transcript, None).generated.as_deref(),
        Some("the first look")
    );

    let mut body = std::fs::read(&scratch.transcript).expect("the transcript");
    body[..poison.len()].copy_from_slice(poison.as_bytes());
    body.extend_from_slice(br#"{"type":"ai-title","aiTitle":"the second look"}"#);
    body.push(b'\n');
    std::fs::write(&scratch.transcript, body).expect("the transcript, poisoned and grown");

    let titles = fold.titles(&scratch.transcript, None);
    assert_eq!(titles.generated.as_deref(), Some("the second look"));
    assert_eq!(
        titles.manual, None,
        "the poisoned prefix was already behind the cursor, and a re-read would have found it"
    );
}

/// JSON tolerates whitespace inside an object, so a record can be padded to a byte length without
/// changing what it says.
fn pad_to(record: &str, len: usize) -> String {
    let mut padded = record.to_string();
    while padded.len() < len {
        padded.insert(padded.len() - 1, ' ');
    }
    padded
}

/// Measured on a real session: a `dequeue` carries **no content**, so the fold can only pop the
/// head by position — and once one unmatched enqueue sits at position 0, every later delivery pops
/// the wrong entry and the prompt that was actually answered stays for ever. The transcript
/// settles it without guessing: a prompt the harness has taken is written down as a user record,
/// and a queue holding one it has already answered is wrong however the operations folded.
#[test]
fn a_prompt_the_harness_has_since_delivered_is_not_still_waiting() {
    let scratch = scratch_claude(
        "delivered",
        &[
            serde_json::json!({ "type": "ai-title", "aiTitle": "the queue that would not drain" }),
            enqueue("check the release notes", "2026-08-29T10:02:20.000Z"),
            prompt("check the release notes"),
        ],
    );
    let mut feed = feed(&scratch);

    let moved = feed.moved(&scratch.transcript, None).expect("the title moved");
    assert_eq!(
        queued_texts(&moved),
        Vec::<&str>::new(),
        "the harness wrote this prompt down as delivered, so nothing is still waiting on it"
    );
}

/// **909 of the 992 `enqueue` records on this machine are `<task-notification>` envelopes** — the
/// harness handing itself the result of its own background work, never anything a person typed.
/// Drawn as queued prompts they are attributed to the operator, and they are filed after every
/// record on a view pinned to its end, so a few of them push the whole conversation off screen and
/// the pane reads as though it stopped updating.
#[test]
fn a_task_notification_is_the_harness_talking_to_itself_and_never_a_queued_prompt() {
    let scratch = scratch_claude(
        "plumbing",
        &[
            enqueue(
                "<task-notification>\n<task-id>blorajtic</task-id>\n</task-notification>",
                "2026-08-29T08:07:03.000Z",
            ),
            enqueue("and add a probe row for it", "2026-08-29T08:08:00.000Z"),
        ],
    );
    let mut feed = feed(&scratch);

    let moved = feed
        .moved(&scratch.transcript, None)
        .expect("the operator's own prompt is still queued");
    assert_eq!(queued_texts(&moved), ["and add a probe row for it"]);
}
