use crate::common;
use crate::common::*;
use kampr_journal::{Block, CodexAdapter, JournalAdapter, Role, SessionRef, ToolState, TranscriptRoot};

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
            lines: Some(1),
            state: ToolState::Done,
        },
        "one line of output under four of Codex's own bookkeeping, and the card counts the output"
    );
    assert_eq!(
        call.blocks[1],
        Block::Code {
            lang: Some("bash".into()),
            text: "herdr pane list --json".into(),
            role: None,
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
    let scratch = scratch_dir("codex-fail");
    let dir = scratch.join("sessions/2026/08/18");
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
        matches!(&call.blocks[1], Block::Code { lang: None, text, .. } if text.starts_with("const r =")),
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
    let record = serde_json::json!({
        "type": "response_item",
        "timestamp": "2026-08-18T14:11:36.000Z",
        "payload": { "type": "message", "role": "user", "content": [
            { "type": "input_text", "text": "does this look right?" },
            { "type": "input_image",
              "image_url": "data:image/png;base64,iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==" }
        ] }
    });
    let mut scratch = scratch_codex("codex-image", &[record]);
    let turns = scratch.turns();

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].role, Role::User);
    assert_eq!(turns[0].blocks[0], Block::md("does this look right?"));
    assert_eq!(md_texts(&turns), vec!["does this look right?", "[image · png]"]);
    let att = attachments(&turns);
    assert_eq!(att.len(), 1);
    assert_eq!(att[0].mime.as_deref(), Some("image/png"));
    assert_eq!(att[0].bytes, Some(70));
    let wire = serde_json::to_string(&turns).unwrap();
    assert!(!wire.contains("base64"), "{wire}");
}

// ---------------------------------------------------------------------------------------------
// Identity: the thread a pid is on.
// ---------------------------------------------------------------------------------------------

use kampr_journal::PaneProcess;
use std::fs::File;
use std::path::Path;

const HELD_THREAD: &str = "01a04b4e-d231-7d81-9fd8-971e2b0ca9d0";
const SECOND_THREAD: &str = "01a04b61-71ac-7c42-9f0e-1d2b3c4d5e6f";

/// A codex home holding a rollout and a writer lock for each id. The lock files are empty, which
/// is what made them look like nothing worth reading — the kernel holds the meaning, not the file.
fn locks_home(tag: &str, ids: &[&str]) -> common::ScratchDir {
    let home = scratch_dir(tag);
    std::fs::create_dir_all(home.join("thread-writer-locks")).unwrap();
    let dir = home.join("sessions/2026/08/18");
    std::fs::create_dir_all(&dir).unwrap();
    let source = std::fs::read_to_string(codex_transcript()).unwrap();
    for id in ids {
        std::fs::write(
            dir.join(format!("rollout-2026-08-18T14-11-36-{id}.jsonl")),
            &source,
        )
        .unwrap();
        File::create(home.join(format!("thread-writer-locks/{id}.lock"))).unwrap();
    }
    home
}

fn me() -> PaneProcess {
    PaneProcess {
        pid: std::process::id(),
        start: None,
        started: None,
    }
}

fn located(home: &Path, process: &PaneProcess) -> Option<String> {
    CodexAdapter::new(TranscriptRoot::new(home).unwrap())
        .locate_by_process(process)
        .ok()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
}

fn hold(home: &Path, id: &str) -> File {
    let lock = File::options()
        .write(true)
        .open(home.join(format!("thread-writer-locks/{id}.lock")))
        .unwrap();
    lock.lock().unwrap();
    lock
}

/// Against the real kernel, with nothing mocked: this test process takes the same `flock` codex
/// takes, and the adapter finds the thread through `/proc/locks`.
///
/// The adapter used to say codex published no process-to-thread map at all, on the strength of
/// the lock files being empty — so it fell through to the directory search, which is a time bound
/// and not an identity.
#[test]
fn the_thread_whose_writer_lock_a_pid_holds_is_the_one_served() {
    let home = locks_home("codex-held", &[SECOND_THREAD, HELD_THREAD]);
    assert_eq!(
        located(&home, &me()),
        None,
        "two lock files and neither held: without this the test proves nothing"
    );

    let _lock = hold(&home, HELD_THREAD);

    assert_eq!(
        located(&home, &me()).as_deref(),
        Some(format!("rollout-2026-08-18T14-11-36-{HELD_THREAD}.jsonl").as_str()),
        "the file nobody holds is a thread that has already ended"
    );
    assert_eq!(
        located(&home, &PaneProcess { pid: 1, ..me() }),
        None,
        "and the lock names one pid, not any pid"
    );
}

/// **`/new` does not move a codex lock, it takes a second one and keeps the first** — measured,
/// and the opposite of what `agy` does. A rule of "exactly one held lock or nothing" would refuse
/// every codex session that has ever used `/new`, so the newest lock file is what answers.
#[test]
fn a_codex_process_that_has_opened_a_second_thread_is_on_the_newer_one() {
    let home = locks_home("codex-new", &[HELD_THREAD]);
    let _first = hold(&home, HELD_THREAD);
    std::fs::create_dir_all(home.join("sessions/2026/08/18")).unwrap();
    let source = std::fs::read_to_string(codex_transcript()).unwrap();
    std::fs::write(
        home.join(format!(
            "sessions/2026/08/18/rollout-2026-08-18T14-11-36-{SECOND_THREAD}.jsonl"
        )),
        &source,
    )
    .unwrap();
    File::create(home.join(format!("thread-writer-locks/{SECOND_THREAD}.lock"))).unwrap();
    let _second = hold(&home, SECOND_THREAD);

    assert_eq!(
        located(&home, &me()).as_deref(),
        Some(format!("rollout-2026-08-18T14-11-36-{SECOND_THREAD}.jsonl").as_str()),
    );
}
