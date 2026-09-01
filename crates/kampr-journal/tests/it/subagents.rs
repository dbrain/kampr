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

/// The launch is real and the file is not there: an agent that has been asked for but has written
/// nothing yet, or a transcript that has been cleaned up. A card that offers to open nothing is
/// worse than one that does not offer, so no handle is minted — and the summary falls back to what
/// the launching call itself said rather than to a kind invented for it.
#[test]
fn an_agent_call_whose_transcript_is_not_on_disk_offers_nothing_to_open() {
    let mut scratch = scratch_claude("no-subagent", &agent_call("nowhere1234"));
    let turns = scratch.turns();

    assert!(launches(&turns).is_empty());
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
