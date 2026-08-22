//! Which conversation a pane is showing, against a real one.
//!
//! Every fixture under `tests/fixtures/identity` is a verbatim capture of `claude` 2.1.239 driven
//! through the operator's own two reports, in a headless `herdr server --session kident`:
//!
//! | file | what wrote it |
//! |---|---|
//! | `ab4daea8-….jsonl` | `claude -p` in `/tmp/kident/proj`, from a shell. **No pane ever ran it.** |
//! | `8ae22034-….jsonl` | the pane's first interactive `claude`, pid 1456224, since quit |
//! | `c5eec836-….jsonl` | the pane's *second* interactive `claude`, pid 1463543, still running |
//! | `d22cc625-….jsonl` | a second shell `claude -p`, run *while* that pane sat idle — so the newest |
//! | `sessions/1463543.json` | what that live `claude` wrote about itself, byte for byte |
//!
//! Records of type `attachment`, `bridge-session` and `file-history-snapshot` were dropped whole
//! — kilobytes of directory listings, and account identifiers. Every record that remains is
//! exactly what the harness wrote.
//!
//! All four transcripts declare the *same* `cwd`, which is the entire defect: "the newest
//! transcript in this directory" answered `ab4daea8` to a pane that had just started its own
//! session, and `8ae22034` to a pane whose agent had been quit and restarted.

mod common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use common::scratch_dir;
use kampr_journal::{
    ClaudeAdapter, Harness, JournalError, PaneProcess, Registry, SessionRef, TranscriptRoot,
};

const CWD: &str = "/tmp/kident/proj";

/// The live `claude`: pid, `/proc/<pid>/stat` field 22, and `/proc/<pid>`'s modification time,
/// all read off the running process while it held the pane.
const LIVE_PID: u32 = 1_463_543;
const LIVE_START: &str = "28776850";
const LIVE_STARTED: u64 = 1_787_395_452;

const RUNNING: &str = "c5eec836-44cf-4563-829c-cfdc322b3254.jsonl";
const QUIT: &str = "8ae22034-2bfc-42e0-a09f-0a79c080dcba.jsonl";
const DECOY: &str = "ab4daea8-c481-4b08-a63d-23ea065f393d.jsonl";
/// A second shell run, started while the pane's own session was already open and idle. Its last
/// record is newer than anything the pane wrote, so recency puts it first and it is the one
/// transcript here that the process-start bound cannot exclude.
const NEWER: &str = "d22cc625-bb84-4776-880a-6f54edf42386.jsonl";

fn identity_root() -> PathBuf {
    common::fixtures().join("identity")
}

/// A claude home holding only the named transcripts, plus the live session's own record of
/// itself. Copying rather than pointing at the fixture is what lets a test describe the *moment*
/// — a pane whose fresh session has not written a line yet is the case both reports describe,
/// and it is a directory with the older transcripts in it and nothing else.
fn home_with(tag: &str, transcripts: &[&str]) -> PathBuf {
    let home = scratch_dir(tag);
    let project = home.join("projects/-tmp-kident-proj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(home.join("sessions")).unwrap();
    let from = identity_root();
    for name in transcripts {
        std::fs::copy(
            from.join("projects/-tmp-kident-proj").join(name),
            project.join(name),
        )
        .unwrap();
    }
    std::fs::copy(
        from.join("sessions").join(format!("{LIVE_PID}.json")),
        home.join("sessions").join(format!("{LIVE_PID}.json")),
    )
    .unwrap();
    home
}

fn registry(home: &Path) -> Registry {
    let mut registry = Registry::new();
    registry.register(Arc::new(ClaudeAdapter::new(TranscriptRoot::new(home).unwrap())));
    registry
}

fn live_process() -> PaneProcess {
    PaneProcess {
        pid: LIVE_PID,
        start: Some(LIVE_START.into()),
        started: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(LIVE_STARTED)),
    }
}

fn located(home: &Path, harness: &Harness) -> Option<String> {
    registry(home)
        .locate(Some("claude"), None, Some(Path::new(CWD)), harness)
        .unwrap()
        .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
}

/// The operator, verbatim: *"i opened claude on a terminal that was there already that had never
/// opened claude and its showing me the most recent session?"*
///
/// The pane's own `claude` has been running for seconds and has written nothing. The only
/// transcript in the directory is a `claude -p` run from a shell — a conversation this pane has
/// never had, with somebody else's words in it.
#[test]
fn a_pane_whose_session_has_written_nothing_shows_nothing_rather_than_the_last_run() {
    let home = home_with("fresh-pane", &[DECOY]);

    assert_eq!(
        located(&home, &Harness::Unknown).as_deref(),
        Some(DECOY),
        "the directory alone still answers with the run that was never in this pane — \
         without which this test proves nothing"
    );
    assert_eq!(
        located(&home, &Harness::Running(live_process())).as_deref(),
        None,
        "the pane's own session has no transcript yet, and nothing is the honest answer"
    );
}

/// The operator, verbatim: *"existing session with claude -> closed claude -> opened again fresh
/// session -> conversation panel showing old and not updating to new at all"*.
///
/// The quit session's transcript is the newest thing in the directory and stays that way until
/// the fresh one is spoken to. It is the previous conversation in the same pane, which is exactly
/// what makes it convincing and exactly what makes serving it wrong.
#[test]
fn a_restarted_agent_does_not_keep_showing_the_session_that_was_quit() {
    let home = home_with("restarted", &[DECOY, QUIT]);

    assert_eq!(
        located(&home, &Harness::Unknown).as_deref(),
        Some(QUIT),
        "the directory alone serves the session the operator quit"
    );
    assert_eq!(located(&home, &Harness::Running(live_process())).as_deref(), None);
}

/// And once the fresh session has said something, that — not the newest file in the directory —
/// is what the pane shows, because the process named it.
///
/// **This is the case a start-time bound cannot reach.** The operator kept working in the same
/// directory from a shell while the pane sat idle, so the newest transcript in it was written
/// after the pane's `claude` started *and* is not the pane's. Only the pid tells them apart.
#[test]
fn the_transcript_a_pane_shows_is_the_one_its_own_process_is_on() {
    let home = home_with("running", &[DECOY, QUIT, RUNNING, NEWER]);

    assert_eq!(
        located(&home, &Harness::Unknown).as_deref(),
        Some(NEWER),
        "recency answers with the shell run — without which this test proves nothing"
    );
    assert_eq!(
        located(&home, &Harness::Running(live_process())).as_deref(),
        Some(RUNNING)
    );
}

/// `~/.claude/sessions/<pid>.json` is removed when a session exits, but a pid is a small integer
/// the kernel hands out again. `procStart` is what tells the two apart, and it has to be checked:
/// a stale file believed on the strength of its name alone is the same lie in a new place.
#[test]
fn a_session_file_left_behind_by_a_dead_process_is_not_this_ones() {
    let home = home_with("reused-pid", &[DECOY, QUIT, RUNNING]);
    let adapter = ClaudeAdapter::new(TranscriptRoot::new(&home).unwrap());
    let reused = PaneProcess {
        start: Some("99999999".into()),
        started: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(LIVE_STARTED + 3_600)),
        ..live_process()
    };

    assert!(
        kampr_journal::JournalAdapter::locate_by_process(&adapter, &live_process()).is_ok(),
        "the real process still resolves — without which this test proves nothing"
    );
    assert!(matches!(
        kampr_journal::JournalAdapter::locate_by_process(&adapter, &reused),
        Err(JournalError::NotFound(_))
    ));
    assert_eq!(
        located(&home, &Harness::Running(reused.clone())).as_deref(),
        None,
        "and the pane it belongs to shows nothing rather than the dead session's words"
    );
}

/// The harnesses that publish no map from a process to a session — Codex, and Claude before
/// 2.1.236 — still get a bound: a transcript whose last record was written before the pane's
/// harness started cannot be that harness's.
#[test]
fn a_harness_that_names_no_session_still_gets_the_transcripts_written_since_it_started() {
    let home = home_with("unmapped", &[DECOY, QUIT, RUNNING]);
    std::fs::remove_file(home.join("sessions").join(format!("{LIVE_PID}.json"))).unwrap();
    let unmapped = live_process();

    assert_eq!(
        located(&home, &Harness::Running(unmapped.clone())).as_deref(),
        Some(RUNNING),
        "the only transcript still being written after the process started"
    );

    // And it is a bound, not an identity: another run in the same directory while this one was
    // idle is newer than the process and indistinguishable without the pid.
    let crowded = home_with("unmapped-crowded", &[DECOY, QUIT, RUNNING, NEWER]);
    std::fs::remove_file(crowded.join("sessions").join(format!("{LIVE_PID}.json"))).unwrap();
    assert_eq!(
        located(&crowded, &Harness::Running(unmapped.clone())).as_deref(),
        Some(NEWER)
    );

    // Started later than every record on disk: the directory has nothing this process wrote.
    let later = PaneProcess {
        started: Some(SystemTime::UNIX_EPOCH + Duration::from_secs(LIVE_STARTED + 3_600)),
        ..unmapped
    };
    assert_eq!(located(&home, &Harness::Running(later.clone())).as_deref(), None);
}

/// The order the handles are tried in. An announcement is exact and comes from herdr; the process
/// is exact and comes from the harness; the directory is a guess and comes last. Herdr 0.8.2
/// never makes an announcement (probe #75), so this is about what happens when one starts.
#[test]
fn an_announced_session_is_believed_over_the_process_that_contradicts_it() {
    let home = home_with("announced", &[DECOY, QUIT, RUNNING]);
    let announced = SessionRef::id("claude", QUIT.trim_end_matches(".jsonl"));

    let path = registry(&home)
        .locate(
            Some("claude"),
            Some(&announced),
            Some(Path::new(CWD)),
            &Harness::Running(live_process()),
        )
        .unwrap()
        .expect("the announced session");
    assert!(path.ends_with(QUIT));
}

/// A pane running no harness at all, and a harness this node has no adapter for. Neither becomes
/// a conversation because a process was found for it.
#[test]
fn a_process_is_not_a_conversation_on_its_own() {
    let home = home_with("shell", &[DECOY, QUIT, RUNNING]);
    let registry = registry(&home);
    for agent in [None, Some("gemini")] {
        assert!(
            registry
                .locate(
                    agent,
                    None,
                    Some(Path::new(CWD)),
                    &Harness::Running(live_process())
                )
                .unwrap()
                .is_none()
        );
    }
}

/// A pid nothing on this machine has ever used resolves to no session file, and the containment
/// root refuses it rather than reading whatever `sessions/<pid>.json` happens to canonicalise to.
#[test]
fn an_unknown_pid_names_no_session() {
    let home = home_with("unknown-pid", &[RUNNING]);
    let adapter = ClaudeAdapter::new(TranscriptRoot::new(&home).unwrap());
    let unknown = PaneProcess {
        pid: 4_294_967_295,
        start: None,
        started: None,
    };
    assert!(matches!(
        kampr_journal::JournalAdapter::locate_by_process(&adapter, &unknown),
        Err(JournalError::NotFound(_))
    ));
}

/// Herdr detects a harness by scraping the screen, so a pane can go on claiming `claude` after
/// the process behind it is gone — and the directory it sits in is full of transcripts. The
/// harness being *absent* is information, not the absence of it.
#[test]
fn a_pane_whose_harness_is_gone_shows_nothing_even_though_its_directory_is_full() {
    let home = home_with("gone", &[DECOY, QUIT, RUNNING, NEWER]);
    assert_eq!(
        located(&home, &Harness::Unknown).as_deref(),
        Some(NEWER),
        "a host that cannot see processes still has only the directory to go on"
    );
    assert_eq!(located(&home, &Harness::Absent).as_deref(), None);
}

/// The one thing here that has to agree with the kernel rather than with a fixture: field 22 of
/// `/proc/<pid>/stat` is what Claude records as `procStart`, and reading field 21 or 23 instead
/// would produce a plausible number that never matches anything. Checked against this test's own
/// process, and against `/proc/self/stat` read a second way.
#[test]
fn a_process_start_is_read_from_the_field_claude_records() {
    let me = std::process::id();
    let looked = PaneProcess::look_up(me);
    assert_eq!(looked.pid, me);

    let stat = std::fs::read_to_string("/proc/self/stat").unwrap();
    let fields: Vec<&str> = stat[stat.rfind(") ").unwrap() + 2..].split_whitespace().collect();
    // Fields are 1-based and the slice starts at field 3, so `starttime` is field 22 here.
    let expected = fields[22 - 3];
    assert_eq!(looked.start.as_deref(), Some(expected));
    assert!(looked.started.is_some(), "procfs gives this process a start time");

    assert!(looked.owns(Some(expected)));
    assert!(looked.owns(None), "nothing recorded contradicts nothing");
    assert!(
        !looked.owns(Some("0")),
        "a different start is a different process"
    );
}
