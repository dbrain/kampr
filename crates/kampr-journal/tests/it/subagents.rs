//! The conversations a conversation launched.
//!
//! Claude Code stopped writing `isSidechain` records into the main transcript — `jq
//! 'select(.isSidechain==true)' <transcript> | wc -l` answers 0 against 2.1.248 and 2.1.250 — and
//! moved the content to `<session>/subagents/agent-<id>.jsonl`, in the same record grammar the
//! main transcript uses. `tests/fixtures/claude/projects/-home-u-agents` mirrors that layout, down
//! to the `.meta.json` beside each transcript, the `tool-results/` directory and
//! `custom-title.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::common::*;
use kampr_journal::{
    Block, ClaudeAdapter, JournalAdapter, Registry, SessionRef, SubRef, ToolState, TranscriptRoot, Turn,
};

const SESSION: &str = "7a2f1d00-0000-4000-8000-00000000000a";
const LAUNCHED: &str = "I will read the wire encoder before I answer.";
const NESTED: &str = "`ClientMsg::Manage` carries `op` and `pane`.";

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.register(Arc::new(ClaudeAdapter::new(
        TranscriptRoot::new(claude_root()).unwrap(),
    )));
    registry
}

fn transcript() -> PathBuf {
    claude_root().join(format!("projects/-home-u-agents/{SESSION}.jsonl"))
}

fn subagent(name: &str) -> PathBuf {
    claude_root().join(format!(
        "projects/-home-u-agents/{SESSION}/subagents/{name}.jsonl"
    ))
}

fn opened(path: &Path) -> Vec<Turn> {
    let adapter = ClaudeAdapter::new(TranscriptRoot::new(claude_root()).unwrap());
    let mut journal = adapter.open_path(path.canonicalize().unwrap());
    drain(journal.as_mut())
}

fn launches(turns: &[Turn]) -> Vec<&Block> {
    turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter(|b| matches!(b, Block::Sub { .. }))
        .collect()
}

fn handle(block: &Block) -> &str {
    match block {
        Block::Sub { id, .. } => id.as_str(),
        other => panic!("expected a launch, got {other:?}"),
    }
}

/// The cheap half, and most of the legibility: the card for an `Agent` call says *what kind of
/// agent* and *what it was asked*, off the `.meta.json` the harness wrote beside the transcript.
///
/// The launching tool's own input carries the description, so a card that only showed that would
/// look almost right — the `agentType` is the half that exists nowhere but the meta file, and it
/// is the half that tells an operator an `Explore` from a `general-purpose`.
#[test]
fn an_agent_card_says_what_kind_of_agent_it_launched_and_not_just_what_it_was_asked() {
    let turns = opened(&transcript());
    let cards = tool_blocks(&turns);

    assert_eq!(cards.len(), 1);
    assert_eq!(
        cards[0],
        &Block::Tool {
            name: "Agent".into(),
            summary: Some("Explore — Map the manage op end-to-end path".into()),
            lines: Some(1),
            state: ToolState::Done,
        }
    );
}

/// A subagent's words are the subagent's. Inlining them into the parent's turn list would have an
/// installed phone render them as the parent's own reply, which is a lie about who said what — so
/// the parent carries a *handle* and nothing else, and the words arrive only when the handle is
/// opened.
#[test]
fn a_launched_agents_words_are_addressable_without_being_spoken_in_the_parents_voice() {
    let turns = opened(&transcript());

    assert!(
        !md_texts(&turns).iter().any(|t| t.contains(LAUNCHED)),
        "the subagent's own reply must not appear in the parent's turns: {:?}",
        md_texts(&turns)
    );

    let found = launches(&turns);
    assert_eq!(found.len(), 1);
    let Block::Sub {
        id,
        kind,
        title,
        depth,
    } = found[0]
    else {
        unreachable!()
    };
    assert!(!id.is_empty());
    assert_eq!(kind.as_deref(), Some("Explore"));
    assert_eq!(title.as_deref(), Some("Map the manage op end-to-end path"));
    assert_eq!(depth, &Some(1));

    let mut sub = registry()
        .open_sub(id, &transcript())
        .expect("the handle opens the transcript it names");
    assert!(sub.path().ends_with("subagents/agent-4b7c9e21.jsonl"));
    let said = drain(sub.as_mut());
    assert!(md_texts(&said).contains(&LAUNCHED));
}

/// `spawnDepth` is 2 on one of these, so nesting is not hypothetical and the shape may not assume
/// one level: the conversation reached through a handle is an ordinary conversation, and the
/// handles on *its* cards resolve exactly the same way.
#[test]
fn a_subagent_that_launches_its_own_is_reachable_one_level_further_down() {
    let turns = opened(&subagent("agent-4b7c9e21"));
    let found = launches(&turns);
    assert_eq!(found.len(), 1, "the nested launch: {:?}", turns);
    let Block::Sub { kind, depth, .. } = found[0] else {
        unreachable!()
    };
    assert_eq!(kind.as_deref(), Some("general-purpose"));
    assert_eq!(depth, &Some(2), "the meta file's own count, not an inference");

    let id = handle(found[0]);
    for anchor in [transcript(), subagent("agent-4b7c9e21")] {
        let mut nested = registry()
            .open_sub(id, &anchor)
            .expect("a nested handle resolves against the session, at any depth");
        assert!(md_texts(&drain(nested.as_mut())).contains(&NESTED));
    }
}

/// A handle arrives from the network, so it gets the two independent checks an attachment id
/// gets: the path is resolved through the adapter's own root, and the result must also be
/// something *this* pane's session launched. The root holds every project on the machine, and
/// passing only the first check would hand a pane any conversation on the host.
#[test]
fn a_handle_naming_a_transcript_this_session_never_launched_is_refused() {
    let root = TranscriptRoot::new(claude_root()).unwrap();
    let elsewhere = SubRef::new("claude", &root, &claude_transcript().canonicalize().unwrap());

    assert!(
        registry()
            .open_sub(&elsewhere.encode(), &claude_transcript())
            .is_err(),
        "another pane's whole conversation is inside the root and perfectly readable"
    );
    assert!(registry().open_sub(&elsewhere.encode(), &transcript()).is_err());
    for rubbish in ["", "not-base64-at-all!!", &"A".repeat(9000)] {
        assert!(registry().open_sub(rubbish, &transcript()).is_err());
    }
}

/// An `agentId` comes out of a transcript and is pasted into a filename, and the `agent-` prefix
/// is **not** enough on its own: a launched agent's own directory is one of the two places a
/// nested transcript is looked for, so `agent-<id>` can be a real directory, and a `..` that
/// starts inside one is a `..` the kernel will follow. Four of them reach `projects/`, and the
/// next segment is another project's conversation.
#[test]
fn an_agent_id_that_walks_out_of_its_own_session_names_no_transcript() {
    let escape = "4b7c9e21/../../../../-home-u-other/agent-secret";
    let mut scratch = scratch_claude("escape", &agent_call(escape));
    std::fs::create_dir_all(
        scratch
            .root
            .join("projects/-home-u-demo/session/subagents/agent-4b7c9e21"),
    )
    .unwrap();
    let elsewhere = scratch.root.join("projects/-home-u-other");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(
        elsewhere.join("agent-secret.jsonl"),
        lines(&[serde_json::json!({
            "type": "assistant", "uuid": "v1", "timestamp": "2026-08-27T09:00:00.000Z",
            "message": { "content": [{ "type": "text", "text": "another project's words" }] }
        })]),
    )
    .unwrap();
    assert!(
        scratch
            .root
            .join(format!(
                "projects/-home-u-demo/session/subagents/agent-{escape}.jsonl"
            ))
            .is_file(),
        "the traversal has to land on a real file or this test proves nothing"
    );

    assert!(launches(&scratch.turns()).is_empty());
}

/// The empty id, and the one long enough to be a filesystem's problem rather than a transcript's.
#[test]
fn an_agent_id_that_is_not_one_names_no_transcript() {
    for id in ["", &"a".repeat(4096)] {
        let mut scratch = scratch_claude("not-an-id", &agent_call(id));
        assert!(launches(&scratch.turns()).is_empty(), "{id:?} minted a handle");
    }
}

fn agent_call(agent_id: &str) -> [serde_json::Value; 2] {
    [
        serde_json::json!({
            "type": "assistant", "uuid": "g1", "timestamp": "2026-08-27T09:00:04.000Z",
            "message": { "content": [
                { "type": "tool_use", "id": "toolu_g", "name": "Agent",
                  "input": { "subagent_type": "general-purpose",
                             "description": "Read the wire encoder",
                             "prompt": "Read crates/kampr-core/src/wire.rs." } }
            ] }
        }),
        serde_json::json!({
            "type": "user", "uuid": "g2", "timestamp": "2026-08-27T09:00:05.000Z",
            "message": { "content": [
                { "type": "tool_result", "tool_use_id": "toolu_g", "content": "Agent launched" }
            ] },
            "toolUseResult": { "agentId": agent_id, "description": "Read the wire encoder",
                               "isAsync": true, "status": "async_launched" }
        }),
    ]
}

/// The seam is the adapter's, not Claude's. A harness that has never been measured to launch a
/// conversation mints no handles, and asking it to open one is a refusal rather than a panic —
/// which is what keeps Codex and agy able to fill this in without the shape changing under them.
#[test]
fn a_harness_that_launches_nothing_is_asked_the_same_question_and_answers_no() {
    let registry = registry();
    let session = SessionRef::id("claude", CLAUDE_SESSION);
    let path = registry.get("claude").unwrap().locate(&session).unwrap();
    let mut journal = registry.get("claude").unwrap().open_path(path);

    assert!(
        launches(&drain(journal.as_mut())).is_empty(),
        "a transcript with no Agent call in it carries no launches"
    );
}

/// **A subagent's transcript grows while it runs, and the reason to open one is to watch it work.**
/// A reader who has to close and re-open it to see the next step has been handed a snapshot of
/// something live — so the journal a launched conversation is opened as has to keep answering
/// `poll` with what the file has grown by, exactly as the pane's own does.
/// **A subagent transcript is nothing but sidechain records, and the parser drops those.**
///
/// In a pane's own transcript `isSidechain` marks a record belonging to something the agent
/// launched, and inlining it would put a subagent's words in the parent's voice. In the launched
/// conversation's own file it marks every record there is. Opening one through `open_sub` — the
/// only way a reader reaches it — must therefore keep them, or the panel is empty: a real
/// transcript put through `open_path` yields **0 turns**, which is what a fixture built without
/// the field could never show.
#[test]
fn a_launched_conversation_is_read_even_though_every_record_in_it_is_a_sidechain() {
    let scratch = scratch_dir("sub-sidechain");
    let root = scratch.join("root");
    let session = "7a2f1d00-0000-4000-8000-00000000000a";
    let dir = root.join(format!("projects/-home-u-agents/{session}/subagents"));
    std::fs::create_dir_all(&dir).expect("subagent dir");
    let live = dir.join("agent-4b7c9e21.jsonl");
    std::fs::write(&live, format!("{}\n", step("s1", "the launched agent spoke"))).expect("write");

    let contained = TranscriptRoot::new(root).unwrap();
    let handle = SubRef::new("claude", &contained, &live.canonicalize().unwrap());
    let adapter = ClaudeAdapter::new(contained);
    let mut opened = adapter.open_sub(&handle).expect("the launched conversation");

    let turns = drain(opened.as_mut());

    assert_eq!(
        turns.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["s1"],
        "a launched conversation read as its own came back empty",
    );
}

#[test]
fn a_launched_conversation_goes_on_answering_as_its_transcript_grows() {
    let scratch = scratch_dir("sub-tail");
    let root = scratch.join("root");
    let session = "7a2f1d00-0000-4000-8000-00000000000a";
    let dir = root.join(format!("projects/-home-u-agents/{session}/subagents"));
    std::fs::create_dir_all(&dir).expect("subagent dir");
    let live = dir.join("agent-4b7c9e21.jsonl");
    std::fs::write(&live, format!("{}\n", step("s1", "first step"))).expect("write");

    let contained = TranscriptRoot::new(root).unwrap();
    let handle = SubRef::new("claude", &contained, &live.canonicalize().unwrap());
    let adapter = ClaudeAdapter::new(contained);
    let mut journal = adapter.open_sub(&handle).expect("the launched conversation");
    assert_eq!(drain(journal.as_mut()).len(), 1);

    // The agent takes another step while the reader is watching.
    let mut file = std::fs::OpenOptions::new()
        .append(true)
        .open(&live)
        .expect("append");
    use std::io::Write;
    writeln!(file, "{}", step("s2", "second step")).expect("append");

    let grown = drain(journal.as_mut());

    assert_eq!(
        grown.iter().map(|t| t.id.as_str()).collect::<Vec<_>>(),
        ["s2"],
        "a followed conversation answered with {grown:?} rather than the step it had just grown by",
    );
}

/// A record shaped as a **real** subagent transcript writes one. Every record in one carries
/// `isSidechain: true` — 134 of 134 in the transcript this was read from — and the parser drops
/// exactly that in a pane's own transcript, where it means somebody else's words.
fn step(uuid: &str, text: &str) -> String {
    serde_json::json!({
        "type": "assistant",
        "uuid": uuid,
        "isSidechain": true,
        "timestamp": "2026-08-28T02:44:30.000Z",
        "message": { "role": "assistant", "content": [{ "type": "text", "text": text }] }
    })
    .to_string()
}

const SUBAGENTS: &str = "projects/-home-u-demo/session/subagents";

fn wrote_meta(root: &Path, agent_id: &str, kind: &str, description: &str, call: &str, depth: u32) {
    let dir = root.join(SUBAGENTS);
    std::fs::create_dir_all(&dir).expect("a subagents directory");
    std::fs::write(
        dir.join(format!("agent-{agent_id}.meta.json")),
        serde_json::json!({
            "agentType": kind,
            "description": description,
            "toolUseId": call,
            "spawnDepth": depth,
        })
        .to_string(),
    )
    .expect("a meta");
}

fn wrote_transcript(root: &Path, agent_id: &str, text: &str) {
    let dir = root.join(SUBAGENTS);
    std::fs::create_dir_all(&dir).expect("a subagents directory");
    std::fs::write(
        dir.join(format!("agent-{agent_id}.jsonl")),
        format!("{}\n", step("s1", text)),
    )
    .expect("a transcript");
}

/// One assistant record carrying an `Agent` call per `(tool_use_id, subagent_type, description)`,
/// which is how three concurrent launches actually arrive.
fn calls(specs: &[(&str, &str, &str)]) -> serde_json::Value {
    let blocks: Vec<serde_json::Value> = specs
        .iter()
        .map(|(call, kind, description)| {
            serde_json::json!({
                "type": "tool_use", "id": call, "name": "Agent",
                "input": { "subagent_type": kind, "description": description,
                           "run_in_background": false,
                           "prompt": "Read crates/kampr-core/src/wire.rs." }
            })
        })
        .collect();
    serde_json::json!({
        "type": "assistant", "uuid": "c1", "timestamp": "2026-08-27T09:00:04.000Z",
        "message": { "content": blocks }
    })
}

fn result(call: &str, agent_id: &str, description: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "user", "uuid": format!("r-{call}"), "timestamp": "2026-08-27T09:02:00.000Z",
        "message": { "content": [
            { "type": "tool_result", "tool_use_id": call, "content": "Agent result" }
        ] },
        "toolUseResult": { "agentId": agent_id, "description": description,
                           "isAsync": true, "status": "async_launched" }
    })
}

fn sub_of(block: &Block) -> (Option<&str>, Option<&str>, Option<u32>) {
    match block {
        Block::Sub {
            kind, title, depth, ..
        } => (kind.as_deref(), title.as_deref(), *depth),
        other => panic!("expected a launch, got {other:?}"),
    }
}

/// **The result record beats its own transcript to disk.** Measured at +0.101 s and +0.777 s on
/// two async launches — and `settle` runs exactly once per `tool_use_id`, from the one call site
/// in `ingest_block`, with no retry anywhere. So a poll landing inside that window used to drop
/// the card for the whole life of the session: the operator's only way into the conversation,
/// gone because a file was a tenth of a second late.
#[test]
fn a_launch_whose_transcript_has_not_landed_yet_still_mints_a_card_to_open() {
    let mut scratch = scratch_claude("sub-race", &agent_call("4b7c9e21"));
    let turns = scratch.turns();

    let found = launches(&turns);
    assert_eq!(
        found.len(),
        1,
        "the transcript lands up to 0.8 s after the record that names it: {turns:?}"
    );
    assert_eq!(sub_of(found[0]).1, Some("Read the wire encoder"));
}

/// The handle is the file's name, not a proof it is there — `sub.rs` says so outright: *the
/// transcript on disk is already the store*. Resolution happens at **open** time, through
/// `TranscriptRoot::contain`, which canonicalises and so refuses until the file exists. A card
/// minted a tenth of a second early therefore opens the moment the agent writes its first line.
#[test]
fn a_card_minted_before_its_transcript_landed_opens_it_the_moment_the_file_appears() {
    let mut scratch = scratch_claude("sub-lands", &agent_call("4b7c9e21"));
    let turns = scratch.turns();
    let id = handle(launches(&turns)[0]).to_string();

    assert!(
        scratch.journals.open_sub(&id, &scratch.transcript).is_err(),
        "a handle naming a file that is not there yet opens nothing"
    );

    wrote_transcript(&scratch.root, "4b7c9e21", LAUNCHED);

    let mut sub = scratch
        .journals
        .open_sub(&id, &scratch.transcript)
        .expect("the same handle, once the file it names exists");
    assert!(md_texts(&drain(sub.as_mut())).contains(&LAUNCHED));
}

/// **A launch whose file never appears keeps its card, and the card refuses to open.** The two
/// cases are indistinguishable at mint time — the transcript is 3–5 ms late or it is never
/// coming — and only one of them is common, so the card is minted either way and the question is
/// settled where it can be answered honestly: at open. A refusal the client already renders is a
/// better answer than a card silently missing for the rest of the session.
#[test]
fn an_agent_call_whose_transcript_never_appears_offers_a_card_that_refuses_to_open() {
    let mut scratch = scratch_claude("no-subagent", &agent_call("nowhere1234"));
    let turns = scratch.turns();

    let found = launches(&turns);
    assert_eq!(found.len(), 1);
    assert!(
        scratch
            .journals
            .open_sub(handle(found[0]), &scratch.transcript)
            .is_err(),
        "there is nothing on disk for this handle to resolve to"
    );
    assert_eq!(
        tool_blocks(&turns)[0],
        &Block::Tool {
            name: "Agent".into(),
            summary: Some("Read the wire encoder".into()),
            lines: Some(1),
            state: ToolState::Done,
        }
    );
}

/// **A synchronous launch writes no result for 65–146 s**, and until 2.1.252 every launch was
/// asynchronous so the result was the only way in. `agent-<id>.meta.json` lands 12–20 ms after
/// the call and carries the `toolUseId` that made it — measured on 4 of 4 synchronous launches —
/// so the card is minted from that instead, and the operator can watch the agent while it is
/// still working rather than two minutes after it stopped.
#[test]
fn a_synchronous_launch_is_openable_from_the_meta_its_own_call_id_names() {
    let mut scratch = scratch_claude(
        "sub-sync",
        &[calls(&[("toolu_sync", "Explore", "Map the manage op")])],
    );
    wrote_meta(
        &scratch.root,
        "5c1d0e33",
        "Explore",
        "Map the manage op",
        "toolu_sync",
        1,
    );
    wrote_transcript(&scratch.root, "5c1d0e33", LAUNCHED);
    let turns = scratch.turns();

    let found = launches(&turns);
    assert_eq!(found.len(), 1, "no result has been written yet: {turns:?}");
    assert_eq!(
        sub_of(found[0]),
        (Some("Explore"), Some("Map the manage op"), Some(1))
    );
    assert_eq!(
        tool_blocks(&turns)[0],
        &Block::Tool {
            name: "Agent".into(),
            summary: Some("Explore — Map the manage op".into()),
            lines: None,
            state: ToolState::Running,
        },
        "a launch still running is a running card, labelled off the meta"
    );

    let mut sub = scratch
        .journals
        .open_sub(handle(found[0]), &scratch.transcript)
        .expect("the running agent's own transcript");
    assert!(md_texts(&drain(sub.as_mut())).contains(&LAUNCHED));
}

/// The result arrives eventually and `settle` mints handles too. One `tool_use_id` is one launch
/// and one card: a second `Block::Sub` would put the same conversation on the turn twice, and a
/// client that opens the second gets a duplicate view of the first.
#[test]
fn a_card_minted_at_the_call_is_not_minted_again_when_the_result_arrives() {
    let mut scratch = scratch_claude(
        "sub-once",
        &[
            calls(&[("toolu_sync", "Explore", "Map the manage op")]),
            result("toolu_sync", "5c1d0e33", "Map the manage op"),
        ],
    );
    wrote_meta(
        &scratch.root,
        "5c1d0e33",
        "Explore",
        "Map the manage op",
        "toolu_sync",
        1,
    );
    wrote_transcript(&scratch.root, "5c1d0e33", LAUNCHED);
    let turns = scratch.turns();

    assert_eq!(launches(&turns).len(), 1, "one launch, one card: {turns:?}");
    assert_eq!(
        tool_blocks(&turns)[0],
        &Block::Tool {
            name: "Agent".into(),
            summary: Some("Explore — Map the manage op".into()),
            lines: Some(1),
            state: ToolState::Done,
        },
        "the card minted early must still settle when its result lands"
    );
}

/// **Three launches in flight at once each wrote their own meta with their own `toolUseId`.**
/// File-creation order would have matched them by luck; the `toolUseId` is the harness's own
/// answer to which call made which agent, and it is the only one this may use.
#[test]
fn concurrent_launches_get_their_own_cards_and_not_each_others() {
    let mut scratch = scratch_claude(
        "sub-concurrent",
        &[calls(&[
            ("toolu_a", "Explore", "the first question"),
            ("toolu_b", "general-purpose", "the second question"),
            ("toolu_c", "Plan", "the third question"),
        ])],
    );
    for (agent, call, kind, description) in [
        ("aaa11111", "toolu_a", "Explore", "the first question"),
        ("bbb22222", "toolu_b", "general-purpose", "the second question"),
        ("ccc33333", "toolu_c", "Plan", "the third question"),
    ] {
        wrote_meta(&scratch.root, agent, kind, description, call, 1);
        wrote_transcript(&scratch.root, agent, description);
    }
    let turns = scratch.turns();

    let found = launches(&turns);
    assert_eq!(found.len(), 3, "{turns:?}");
    assert_eq!(
        found.iter().map(|b| sub_of(b)).collect::<Vec<_>>(),
        [
            (Some("Explore"), Some("the first question"), Some(1)),
            (Some("general-purpose"), Some("the second question"), Some(1)),
            (Some("Plan"), Some("the third question"), Some(1)),
        ]
    );
    for (block, said) in found
        .iter()
        .zip(["the first question", "the second question", "the third question"])
    {
        let mut sub = scratch
            .journals
            .open_sub(handle(block), &scratch.transcript)
            .expect("each card opens the agent it names");
        assert!(
            md_texts(&drain(sub.as_mut())).contains(&said),
            "a card opened somebody else's agent"
        );
    }
}
