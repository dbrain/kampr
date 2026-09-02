//! Antigravity CLI, against a real conversation.
//!
//! `tests/fixtures/agy` is a verbatim capture of `agy` 1.1.18 driven through a headless
//! `herdr server --session kampr-agy`, under a relocated `HOME` so nothing of the operator's own
//! was read or written. Both files the harness writes are kept side by side, byte for byte and
//! whole, because the difference between them is the reason one of them is the one opened.

use std::fs::File;
use std::os::unix::fs::MetadataExt;
use std::path::Path;
use std::time::SystemTime;

use crate::common;
use crate::common::*;
use kampr_journal::Started;
use kampr_journal::{
    AgyAdapter, Block, JournalAdapter, JournalError, PaneProcess, Role, SessionRef, ToolState,
    TranscriptRoot, Turn,
};

fn adapter() -> AgyAdapter {
    AgyAdapter::new(TranscriptRoot::new(agy_root()).expect("root"))
}

fn turns() -> Vec<Turn> {
    let adapter = adapter();
    let mut journal = adapter.open(&SessionRef::id("agy", AGY_SESSION)).expect("open");
    drain(journal.as_mut())
}

/// Both transcripts sit in the same directory and only one of them is the conversation. The
/// truncated file is what the harness feeds its own model; Kampr exists to show what the agent
/// actually said.
#[test]
fn the_full_transcript_is_the_one_opened() {
    let found = adapter().locate(&SessionRef::id("agy", AGY_SESSION)).unwrap();
    assert_eq!(found, agy_transcript().canonicalize().unwrap());
    assert!(
        found.ends_with("transcript_full.jsonl"),
        "the truncated sibling is right beside it: {found:?}"
    );
    assert!(
        found.parent().unwrap().join("transcript.jsonl").is_file(),
        "and the fixture keeps it, so the choice is a choice"
    );
}

/// What choosing the other file would have cost, measured on the same two captures rather than
/// asserted. Both losses are real: a tool result cut in half, and every argument re-encoded as a
/// JSON string, quotes and all.
#[test]
fn the_truncated_transcript_loses_output_and_retypes_every_argument() {
    let full = std::fs::read_to_string(agy_transcript()).unwrap();
    let short = std::fs::read_to_string(agy_transcript().parent().unwrap().join("transcript.jsonl")).unwrap();

    let marked = short
        .lines()
        .filter(|l| l.contains("\"truncated_fields\""))
        .count();
    assert_eq!(
        marked, 1,
        "the harness marks what it cut, and this capture has one such record"
    );
    let cut = short
        .lines()
        .find(|l| l.contains("\"truncated_fields\""))
        .unwrap();
    let whole = full
        .lines()
        .nth(short.lines().position(|l| l == cut).unwrap())
        .unwrap();
    assert!(
        whole.len() > cut.len() + 4000,
        "and it cut four kilobytes of a shell result out of the middle of it: {} against {}",
        cut.len(),
        whole.len()
    );
    assert!(
        !full.contains("\"truncated_fields\""),
        "and the full file marks nothing, because it cuts nothing"
    );

    assert!(
        full.contains(r#""TargetContent":"hello from the probe""#),
        "the full file types its arguments"
    );
    assert!(
        short.contains(r#""TargetContent":"\"hello from the probe\"""#),
        "the truncated file re-encodes every one of them as a JSON string"
    );
    assert!(
        short.contains(r#""AllowMultiple":"false""#),
        "booleans included: {}",
        "a summary read out of this file would carry the quotes"
    );
}

#[test]
fn a_user_turn_carries_the_request_and_not_its_envelope() {
    let turns = turns();
    let first = &turns[0];
    assert_eq!(first.role, Role::User);
    assert_eq!(
        md_texts(&turns[..1]),
        [
            "Read notes.md, then append three more lines of your choosing to it with a shell command, then run `ls -la` and explain in four or five sentences what the directory holds and what you changed."
        ],
        "the `<USER_REQUEST>` wrapper, the local time and the model-selection notice are the \
         harness talking to its own model"
    );
    assert!(
        !md_texts(&turns).iter().any(|t| t.contains("ADDITIONAL_METADATA")
            || t.contains("USER_SETTINGS_CHANGE")
            || t.contains("USER_REQUEST")),
        "and none of the four user turns carries any of it"
    );
}

/// `{{ CHECKPOINT 0 }}` is the harness telling its model what it dropped from the context. It is
/// not something anybody said.
#[test]
fn a_system_checkpoint_is_not_a_turn() {
    assert!(
        !md_texts(&turns()).iter().any(|t| t.contains("CHECKPOINT")),
        "the capture has one and it carries no turn"
    );
    let turns = turns();
    assert_eq!(
        turns.len(),
        23,
        "thirty-two records: seven prompts, seven answers and nine calls are turns; the eight \
         results revise a call rather than making one, and the checkpoint makes nothing"
    );
}

/// The thinking summary rides on the same record as the answer. The answer is the turn.
#[test]
fn thinking_is_not_published_beside_the_answer() {
    let turns = turns();
    let answer = md_texts(&turns)
        .into_iter()
        .find(|t| t.starts_with("The current working directory is a Git repository"))
        .expect("the record carrying both");
    assert!(
        !answer.contains("Initiating Exploration") && !answer.contains("Analyzing the Steps"),
        "{answer:?}"
    );
}

#[test]
fn a_shell_call_becomes_a_tool_and_a_code_block() {
    let turns = turns();
    let call = turns
        .iter()
        .find(|t| matches!(t.blocks.first(), Some(Block::Tool { name, .. }) if name == "run_command"))
        .expect("a run_command turn");
    assert_eq!(
        call.blocks[0],
        Block::Tool {
            name: "run_command".into(),
            summary: Some("Append lines to notes.md".into()),
            lines: Some(3),
            state: ToolState::Done,
        },
        "agy writes its own one-line summary of every call, so nothing has to be derived, and the \
         count is the result under the exit-status line rather than the line itself"
    );
    assert_eq!(
        call.blocks[1],
        Block::Code {
            role: None,
            lang: Some("bash".into()),
            text: "cat << 'EOF' >> notes.md\nLine 1: Antigravity probe confirmed.\nLine 2: System operational and responsive.\nLine 3: Logging completed successfully.\nEOF".into(),
        }
    );
}

/// There is no call id anywhere in the format. A result is the record that comes *next*, and
/// nothing else — which is what makes the missing-result case below a correctness question rather
/// than a cosmetic one.
#[test]
fn a_nonzero_exit_marks_the_tool_as_failed() {
    let turns = turns();
    let failed = tool_blocks(&turns)
        .into_iter()
        .filter(|b| {
            matches!(
                b,
                Block::Tool {
                    state: ToolState::Error,
                    ..
                }
            )
        })
        .collect::<Vec<_>>();
    assert_eq!(failed.len(), 1, "the capture has exactly one failing command");
    assert!(
        matches!(failed[0], Block::Tool { summary: Some(s), .. } if s.contains("missing file")),
        "{:?}",
        failed[0]
    );
}

/// **A tool that fails hard writes no result record at all** — the capture's `step_index` runs
/// `24, 26`. Pairing on position without pairing on *adjacency* would hand that call the next
/// call's result and mark the wrong tool done.
#[test]
fn a_call_whose_result_never_came_stays_running_and_does_not_steal_the_next_one() {
    let turns = turns();
    let running: Vec<&Block> = tool_blocks(&turns)
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
        .collect();
    assert_eq!(running.len(), 1, "exactly one call in the capture never answered");
    assert!(
        matches!(running[0], Block::Tool { name, .. } if name == "view_file"),
        "{:?}",
        running[0]
    );

    let viewers: Vec<&Block> = tool_blocks(&turns)
        .into_iter()
        .filter(|b| matches!(b, Block::Tool { name, .. } if name == "view_file"))
        .collect();
    assert_eq!(viewers.len(), 4);
    assert_eq!(
        viewers
            .iter()
            .filter(|b| matches!(
                b,
                Block::Tool {
                    state: ToolState::Done,
                    ..
                }
            ))
            .count(),
        3,
        "the three that were answered are done, and the drift would have made it four"
    );
}

#[test]
fn an_edit_becomes_a_diff_block() {
    let turns = turns();
    let diffs = diff_blocks(&turns);
    assert_eq!(diffs.len(), 1);
    assert_eq!(
        diffs[0],
        &Block::Diff {
            path: Some(
                "/tmp/claude-1000/-home-dbrain-dev-kampr/88b9dd03-2675-4287-a9b6-21b7f2b4391c/scratchpad/agy/work/notes.md"
                    .into()
            ),
            text: "@@ -1,4 +1,4 @@\n-hello from the probe\n+hello from the kampr probe\n Line 1: Antigravity probe confirmed.\n Line 2: System operational and responsive.\n Line 3: Logging completed successfully.".into(),
        },
        "the harness puts a unified diff in the result, fenced by its own markers"
    );
    assert!(
        !md_texts(&turns).iter().any(|t| t.contains("diff_block_start")),
        "and the fence itself is layout"
    );
}

#[test]
fn every_turn_has_a_stable_distinct_id() {
    let turns = turns();
    let mut ids: Vec<&str> = turns.iter().map(|t| t.id.as_str()).collect();
    let before = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), before, "a repeated id silently replaces a turn");
    assert_eq!(turns, self::turns(), "and a second read produces the same ones");
}

/// Nothing `agy` writes before it exits says which directory a conversation belongs to. The two
/// files that do — `cache/last_conversations.json` and `conversation_summaries.db` — are written
/// at exit, so while a conversation is live they name the *previous* one, which is precisely the
/// wrong answer. Nothing is the honest one.
#[test]
fn a_working_directory_names_no_conversation() {
    let err = adapter()
        .locate_by_cwd(Path::new("/tmp/anything"), None)
        .unwrap_err();
    assert!(matches!(err, JournalError::NotFound(_)), "{err:?}");
    let transcript = std::fs::read_to_string(agy_transcript()).unwrap();
    for record in transcript.lines() {
        let record: serde_json::Value = serde_json::from_str(record).unwrap();
        assert!(
            record.get("cwd").is_none() && record.get("workspace").is_none(),
            "if a record ever declares one, this test is what should change first"
        );
    }
}

// ---------------------------------------------------------------------------------------------
// Identity: the conversation a pid is on.
// ---------------------------------------------------------------------------------------------

fn presence_home(tag: &str, ids: &[&str]) -> common::ScratchDir {
    let home = scratch_dir(tag);
    std::fs::create_dir_all(home.join("presence")).unwrap();
    for id in ids {
        let brain = home.join(format!("brain/{id}/.system_generated/logs"));
        std::fs::create_dir_all(&brain).unwrap();
        std::fs::copy(agy_transcript(), brain.join("transcript_full.jsonl")).unwrap();
        File::create(home.join(format!("presence/{id}.lock"))).unwrap();
    }
    home
}

fn me() -> PaneProcess {
    PaneProcess {
        pid: std::process::id(),
        start: None,
        started: Started::At(SystemTime::UNIX_EPOCH),
    }
}

fn located(home: &Path, process: &PaneProcess) -> Option<String> {
    AgyAdapter::new(TranscriptRoot::new(home).unwrap())
        .locate_by_process(process)
        .ok()
        .map(|p| {
            p.parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .unwrap()
                .to_string_lossy()
                .to_string()
        })
}

const HELD: &str = "b9463836-cff5-42eb-a2f0-ff47ea827f2b";
const STALE: &str = "bcfba2ec-595e-4e58-9bf9-87d90fd5acc5";

/// The whole handle, against the real kernel: this test process takes the same `flock` `agy`
/// takes, and the adapter finds it through `/proc/locks` with nothing mocked.
#[test]
fn the_conversation_whose_presence_lock_a_pid_holds_is_the_one_served() {
    let home = presence_home("agy-held", &[STALE, AGY_SESSION, HELD]);
    assert_eq!(
        located(&home, &me()),
        None,
        "three presence files and no lock held: without this the test proves nothing"
    );

    let lock = File::options()
        .write(true)
        .open(home.join(format!("presence/{HELD}.lock")))
        .unwrap();
    lock.lock().unwrap();

    assert_eq!(
        located(&home, &me()).as_deref(),
        Some(HELD),
        "the two files nobody holds are conversations that have already ended"
    );

    let other = PaneProcess { pid: 1, ..me() };
    assert_eq!(
        located(&home, &other),
        None,
        "and the lock names one pid, not any pid"
    );
}

/// The lock *file* outlives the conversation — `agy` unlinks nothing on exit. Only the kernel's
/// answer separates a live conversation from three dead ones.
#[test]
fn a_presence_file_nobody_holds_is_not_a_conversation() {
    let home = presence_home("agy-stale", &[STALE, AGY_SESSION]);
    assert_eq!(located(&home, &me()), None);
    assert!(
        home.join(format!("presence/{STALE}.lock")).is_file(),
        "the files are still there, which is the point"
    );
}

/// A conversation with a lock and no transcript yet is a pane that has resolved nothing. Serving
/// its directory would be serving whatever else is in the root.
#[test]
fn a_held_lock_with_no_transcript_behind_it_resolves_to_nothing() {
    let home = scratch_dir("agy-nofile");
    std::fs::create_dir_all(home.join("presence")).unwrap();
    let path = home.join(format!("presence/{HELD}.lock"));
    File::create(&path).unwrap();
    let lock = File::options().write(true).open(&path).unwrap();
    lock.lock().unwrap();
    assert_eq!(located(&home, &me()), None);
}

/// **One read of `/proc/locks` is not evidence.** It is a `seq_file`, and its iteration restarts
/// by index every time the kernel's ~4 KiB buffer is drained, so a lock released *before* that
/// boundary between two of the `read` calls behind one `read_to_string` shifts every later record
/// up one and the record sitting on the boundary is never printed. Measured on a 112-line table by
/// re-reading it inside `holder` at the instant it answered wrongly: the dropped record was a lock
/// the caller was still holding, and it came straight back on the next read.
///
/// Dropping a record is the dangerous direction, not the noisy one. A pid holding two presence
/// locks whose second lock falls out of the view resolves to the first — the wrong-conversation
/// answer this module exists to refuse — so a lock seen in *any* read is a lock held.
#[test]
fn a_lock_one_table_read_dropped_is_not_a_lock_released() {
    let home = presence_home("agy-dropped", &[AGY_SESSION, HELD]);
    let pid = std::process::id();
    let both = fake_table(&home, pid, &[HELD, AGY_SESSION]);
    let dropped = fake_table(&home, pid, &[HELD]);
    let presence = home.join("presence");

    for view in [
        vec![dropped.clone(), both.clone(), both.clone(), both.clone()],
        vec![both.clone(), dropped.clone(), dropped.clone(), dropped.clone()],
    ] {
        let mut reads = view.into_iter();
        assert_eq!(
            kampr_journal::agy::holder_from(&presence, pid, || reads.next()),
            None,
            "a read that lost one of the two locks is not a read that says there is one"
        );
    }

    let mut reads = std::iter::repeat_n(dropped, 4);
    assert_eq!(
        kampr_journal::agy::holder_from(&presence, pid, || reads.next()).as_deref(),
        Some(HELD),
        "and a lock no read ever saw is not held — without which the two above prove nothing"
    );
}

/// `/proc/locks` as the kernel prints it, for locks this process is not really holding. The
/// device field is the kernel's own `major:minor` encoding, so a line the kernel wrote about a
/// lock this process *is* holding is captured and its inode swapped, rather than the encoding
/// being re-derived here and drifting from `presence.rs`.
fn fake_table(home: &Path, pid: u32, ids: &[&str]) -> String {
    let witness_path = home.join("presence/witness.lock");
    File::create(&witness_path).unwrap();
    let witness = File::options().write(true).open(&witness_path).unwrap();
    witness.lock().unwrap();
    let held = std::fs::metadata(&witness_path).unwrap().ino();
    let line = (0..64)
        .find_map(|_| {
            let table = std::fs::read_to_string("/proc/locks").unwrap();
            table
                .lines()
                .find(|line| {
                    kampr_journal::agy::flocks(line)
                        .first()
                        .is_some_and(|(owner, _, _, inode)| *owner == pid && *inode == held)
                })
                .map(str::to_string)
        })
        .expect("the kernel prints a lock this process holds");
    drop(witness);
    std::fs::remove_file(&witness_path).unwrap();

    let (head, _) = line
        .rsplit_once(':')
        .expect("the kernel writes major:minor:inode");
    ids.iter()
        .map(|id| {
            let inode = std::fs::metadata(home.join(format!("presence/{id}.lock")))
                .unwrap()
                .ino();
            format!("{head}:{inode} 0 EOF\n")
        })
        .collect()
}

fn locks(name: &str) -> String {
    std::fs::read_to_string(fixtures().join("identity-agy").join(format!("{name}.txt"))).unwrap()
}

/// `/proc/locks` verbatim, captured while the probe's own `agy` held its lock. World-readable,
/// which is the reason this and not the process's file descriptor table.
#[test]
fn the_kernel_lock_table_is_read_as_the_kernel_writes_it() {
    let held = kampr_journal::agy::flocks(&locks("proc-locks"));
    assert!(
        held.contains(&(2_945_050, 0, 41, 827_551)),
        "the agy line: pid 2945050, device 00:29, inode 827551"
    );
    assert!(
        held.iter().all(|(_, _, _, inode)| *inode != 29_264_970),
        "a POSIX record lock is not an flock and is not this"
    );
    assert_eq!(held.len(), 17, "the capture holds 116 locks, 17 of them flocks");
}

/// A second `agy` opening a conversation the first one still has is a *waiter*, and the kernel
/// lists it under the same index with a `->`. It has not got the lock, so it is not on that
/// conversation. Captured by making two `flock(1)` processes contend for one file.
#[test]
fn a_blocked_waiter_is_not_the_holder() {
    let held = kampr_journal::agy::flocks(&locks("proc-locks-blocked"));
    assert!(held.contains(&(2_958_060, 0, 41, 827_585)), "the holder is there");
    assert!(
        !held.contains(&(2_958_064, 0, 41, 827_585)),
        "and the process queued behind it is not"
    );
}

/// One process, two held locks, is a shape nothing observed produces and nothing can resolve:
/// there is no way to tell which of the two the operator is looking at, and showing the wrong
/// conversation is worse than showing none.
#[test]
fn a_pid_holding_two_presence_locks_names_neither() {
    let home = presence_home("agy-two", &[AGY_SESSION, HELD]);
    let first = File::options()
        .write(true)
        .open(home.join(format!("presence/{HELD}.lock")))
        .unwrap();
    first.lock().unwrap();
    assert_eq!(
        located(&home, &me()).as_deref(),
        Some(HELD),
        "one lock resolves — without which the second assertion proves nothing"
    );

    let second = File::options()
        .write(true)
        .open(home.join(format!("presence/{AGY_SESSION}.lock")))
        .unwrap();
    second.lock().unwrap();
    assert_eq!(located(&home, &me()), None);
}
