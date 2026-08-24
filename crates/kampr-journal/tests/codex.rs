mod common;

use common::*;
use kampr_journal::{
    Block, CodexAdapter, Journal, JournalAdapter, Role, SessionRef, ToolState, TranscriptRoot,
};

fn journal() -> Box<dyn kampr_journal::Journal> {
    let adapter = CodexAdapter::new(TranscriptRoot::new(codex_root()).unwrap());
    adapter
        .open(&SessionRef::id("codex", CODEX_SESSION))
        .expect("open")
}

#[test]
fn locates_a_rollout_by_id_under_the_date_tree() {
    let adapter = CodexAdapter::new(TranscriptRoot::new(codex_root()).unwrap());
    let found = adapter.locate(&SessionRef::id("codex", CODEX_SESSION)).unwrap();
    assert_eq!(found, codex_transcript().canonicalize().unwrap());
}

#[test]
fn parses_response_items_and_ignores_the_event_stream() {
    let mut journal = journal();
    let turns = drain(journal.as_mut());

    let ids: Vec<&str> = turns.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        ["x3", "x6", "x7", "x10", "x12"],
        "session_meta, turn_context, world_state, event_msg, developer and reasoning carry no turn"
    );
    assert_eq!(turns[0].role, Role::User);
    assert_eq!(turns[1].role, Role::Assistant);
}

#[test]
fn markdown_is_passed_through_verbatim() {
    let mut journal = journal();
    let turns = drain(journal.as_mut());

    let source = std::fs::read_to_string(codex_transcript()).unwrap();
    let recorded: String = source
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find(|v| v.pointer("/payload/phase").and_then(|p| p.as_str()) == Some("final_answer"))
        .and_then(|v| {
            v.pointer("/payload/content/0/text")
                .and_then(|t| t.as_str())
                .map(str::to_string)
        })
        .expect("fixture has a final answer");

    assert!(recorded.contains("| Key | Accepted |"));
    assert_eq!(md_texts(&turns).last(), Some(&recorded.as_str()));
}

#[test]
fn exec_command_becomes_a_tool_and_a_code_block() {
    let mut journal = journal();
    let turns = drain(journal.as_mut());

    let call = turns.iter().find(|t| t.id == "x7").unwrap();
    assert_eq!(
        call.blocks[0],
        Block::Tool {
            name: "exec_command".into(),
            summary: Some("herdr pane list --json".into()),
            lines: Some(5),
            state: ToolState::Done,
        }
    );
    assert_eq!(
        call.blocks[1],
        Block::Code {
            lang: Some("bash".into()),
            text: "herdr pane list --json".into(),
        }
    );
}

#[test]
fn apply_patch_becomes_a_diff_block() {
    let mut journal = journal();
    let turns = drain(journal.as_mut());

    let patch = turns.iter().find(|t| t.id == "x10").unwrap();
    assert_eq!(
        patch.blocks[0],
        Block::Tool {
            name: "apply_patch".into(),
            summary: Some("/home/u/demo/notes.md".into()),
            lines: Some(2),
            state: ToolState::Done,
        }
    );
    assert_eq!(patch.blocks[1], Block::Diff {
        path: Some("/home/u/demo/notes.md".into()),
        text: "*** Begin Patch\n*** Update File: /home/u/demo/notes.md\n@@\n-old line\n+new line\n*** End Patch".into(),
    });
}

#[test]
fn a_nonzero_exit_marks_the_tool_as_failed() {
    let mut source = std::fs::read_to_string(codex_transcript()).unwrap();
    source = source.replace("Process exited with code 0", "Process exited with code 1");
    let dir = scratch_dir("codex-fail").join("sessions/2026/08/18");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("rollout-2026-08-18T14-11-36-{CODEX_SESSION}.jsonl"));
    std::fs::write(&path, source).unwrap();

    let root = TranscriptRoot::new(
        dir.parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .parent()
            .unwrap(),
    )
    .unwrap();
    let adapter = CodexAdapter::new(root);
    let mut journal = adapter.open(&SessionRef::id("codex", CODEX_SESSION)).unwrap();
    let turns = drain(journal.as_mut());

    let call = turns.iter().find(|t| t.id == "x7").unwrap();
    assert!(matches!(
        &call.blocks[0],
        Block::Tool {
            state: ToolState::Error,
            ..
        }
    ));
}

/// Codex 0.147 routes shell work through code mode: a `custom_tool_call` named `exec` whose
/// input is JavaScript, not a patch.
const CODE_MODE_SESSION: &str = "01a01db9-177e-7ae3-99e3-9c42d9b6fc3d";

#[test]
fn code_mode_exec_is_a_code_block_not_a_diff() {
    let adapter = CodexAdapter::new(TranscriptRoot::new(codex_root()).unwrap());
    let mut journal = adapter.open(&SessionRef::id("codex", CODE_MODE_SESSION)).unwrap();
    let turns = drain(journal.as_mut());

    let call = turns.last().unwrap();
    assert!(matches!(
        &call.blocks[0],
        Block::Tool {
            name,
            state: ToolState::Running,
            ..
        } if name == "exec"
    ));
    assert!(
        matches!(&call.blocks[1], Block::Code { lang: None, text } if text.starts_with("const r =")),
        "code-mode input is JavaScript, so no language is claimed for it"
    );
    assert!(diff_blocks(&turns).is_empty());
}

/// A `custom_tool_call` with no matching output is a request still waiting on the operator
/// (probe #40): codex records it before approval, so it shows as running rather than done.
#[test]
fn an_unanswered_tool_call_stays_running() {
    let adapter = CodexAdapter::new(TranscriptRoot::new(codex_root()).unwrap());
    let mut journal = adapter.open(&SessionRef::id("codex", CODE_MODE_SESSION)).unwrap();
    let turns = drain(journal.as_mut());

    let running = tool_blocks(&turns)
        .into_iter()
        .filter(|b| {
            matches!(
                b,
                Block::Tool {
                    state: ToolState::Running,
                    ..
                }
            )
        })
        .count();
    assert_eq!(running, 1);
}

/// Codex attaches a picture as an `input_image` content item carrying a `data:` URL — measured on
/// this machine at `view_image` outputs up to 999 594 characters of base64 in one item.
#[test]
fn an_attached_image_is_named_rather_than_dropped() {
    let path = scratch_dir("codex-image").join("rollout.jsonl");
    let record = serde_json::json!({
        "type": "response_item",
        "timestamp": "2026-08-18T14:11:36.000Z",
        "payload": { "type": "message", "role": "user", "content": [
            { "type": "input_text", "text": "does this look right?" },
            { "type": "input_image",
              "image_url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==" }
        ] }
    });
    std::fs::write(&path, record.to_string() + "\n").unwrap();

    let mut journal = kampr_journal::FileJournal::new(path, codex_parser(), Some(kampr_journal::codex::live));
    let turns = journal.poll().unwrap();

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].role, Role::User);
    assert_eq!(
        turns[0].blocks,
        vec![
            Block::Md {
                text: "does this look right?".into()
            },
            Block::Md {
                text: "[image · png]".into()
            },
        ]
    );
    let wire = serde_json::to_string(&turns).unwrap();
    assert!(!wire.contains("base64"), "{wire}");
}
