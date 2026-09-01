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

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use crate::common;
use crate::common::scratch_dir;
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
fn home_with(tag: &str, transcripts: &[&str]) -> common::ScratchDir {
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

/// The same fresh pane, with the operator still working in the same directory from a
/// shell — which is the ordinary case, not a contrived one.
///
/// The marker names the session and the session has written nothing, so the start-time
/// bound is the only thing still holding the search back, and a run started *after* this
/// pane's `claude` walks straight past it. The pane knows exactly which session it is
/// and serves somebody else's words anyway.
#[test]
fn a_pane_whose_session_has_written_nothing_shows_nothing_though_a_shell_run_is_newer() {
    let home = home_with("fresh-pane-crowded", &[DECOY, NEWER]);

    assert_eq!(
        located(&home, &Harness::Unknown).as_deref(),
        Some(NEWER),
        "recency answers with the shell run — without which this test proves nothing"
    );
    assert_eq!(
        located(&home, &Harness::Running(live_process())).as_deref(),
        None,
        "the marker names a session whose transcript does not exist yet, and that is an \
         answer about this pane rather than a reason to go guessing in the directory"
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

/// The bound is a comparison of two instants, and only one of them is an instant.
///
/// A harness start comes from procfs at nanoseconds; the other side is a stamp the harness wrote,
/// and every harness in tree writes a fraction — claude and codex to the millisecond (#285). Read
/// at whole seconds, `c5eec836`'s last record lands 462 ms *before* the process that wrote it
/// instead of after, and the pane's own transcript is refused for as long as that process lives:
/// a `has_conversation` that is false while the file sits there being written to, and nothing
/// short of a new second in the file to clear it (#415).
#[test]
fn a_transcript_written_in_the_second_the_harness_started_is_still_the_one_it_is_writing() {
    let home = home_with("same-second", &[DECOY, QUIT, RUNNING]);
    std::fs::remove_file(home.join("sessions").join(format!("{LIVE_PID}.json"))).unwrap();
    // `c5eec836`'s last record is 2026-08-22T10:45:36.462Z.
    let same_second = |millis: u64| PaneProcess {
        started: Some(SystemTime::UNIX_EPOCH + Duration::from_millis(1_787_395_536_000 + millis)),
        ..live_process()
    };

    assert_eq!(
        located(&home, &Harness::Running(same_second(200))).as_deref(),
        Some(RUNNING),
        "a record written 262 ms after the process started is the process's own",
    );
    // And the second is not a free pass in the other direction: flooring *both* sides would hand
    // this process the transcript of the run it replaced, which is the bound's whole job (#260).
    assert_eq!(
        located(&home, &Harness::Running(same_second(700))).as_deref(),
        None
    );
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

/// Probe #260: a transcript whose first user message carries a pasted image is one record wide
/// enough to push the `cwd` it declares past the head window — 288 KB against a 256 KB budget, on
/// a live 13.5 MB transcript on `pleader`. Nothing in the head names the directory, so the search
/// dropped it and answered with the previous conversation in the same pane, which is the exact
/// shape of the operator's report.
///
/// A file *inside* `projects/<slug(cwd)>` that never declares any directory is this directory's:
/// claude put it there. One that declares a different directory is still refused, which is the
/// half that stops a near-miss serving another project's conversation.
#[test]
fn a_transcript_whose_first_message_is_too_big_to_see_past_is_still_this_directorys() {
    let home = scratch_dir("blind-head");
    let project = home.join("projects/-tmp-kident-proj");
    std::fs::create_dir_all(&project).unwrap();
    std::fs::create_dir_all(home.join("sessions")).unwrap();

    let previous = "aaaaaaaa-0000-4000-8000-000000000001.jsonl";
    std::fs::write(project.join(previous), transcript(CWD, 0, LIVE_STARTED + 10)).unwrap();
    let blind = "bbbbbbbb-0000-4000-8000-000000000002.jsonl";
    std::fs::write(
        project.join(blind),
        transcript(CWD, 300 * 1024, LIVE_STARTED + 20),
    )
    .unwrap();
    // The same record shape, in a directory whose name says another project: the head is the only
    // thing that could refuse it, and it must still refuse it.
    let elsewhere = home.join("projects/-tmp-kident-other");
    std::fs::create_dir_all(&elsewhere).unwrap();
    std::fs::write(
        elsewhere.join("cccccccc-0000-4000-8000-000000000003.jsonl"),
        transcript("/tmp/kident/other", 300 * 1024, LIVE_STARTED + 30),
    )
    .unwrap();

    let unmapped = live_process();
    assert_eq!(
        located(&home, &Harness::Running(unmapped)).as_deref(),
        Some(blind),
        "the newest conversation in the pane's own directory, not the one before it"
    );
    assert_eq!(
        registry(&home)
            .locate(
                Some("claude"),
                None,
                Some(Path::new("/tmp/kident/nowhere")),
                &Harness::Unknown,
            )
            .unwrap(),
        None,
        "a directory nothing was ever run in takes no transcript from another project"
    );
}

/// One transcript: the header records claude writes before any conversation, a user record whose
/// `cwd` sits behind `pad` bytes of message, and the assistant reply that gives the tail its
/// stamp. `pad` is what decides whether the `cwd` is inside the head window or past it.
fn transcript(cwd: &str, pad: usize, at: u64) -> String {
    let stamp = |secs: u64| {
        time::OffsetDateTime::from_unix_timestamp(secs as i64)
            .unwrap()
            .format(&time::format_description::well_known::Rfc3339)
            .unwrap()
    };
    let mut out = String::new();
    for header in ["mode", "permission-mode", "atis-latch"] {
        out.push_str(&format!("{{\"type\":\"{header}\"}}\n"));
    }
    out.push_str(&format!(
        "{{\"type\":\"user\",\"uuid\":\"u-{at}\",\"message\":{{\"role\":\"user\",\"content\":[{{\"type\":\"text\",\"text\":\"{}\"}}]}},\"timestamp\":\"{}\",\"cwd\":\"{cwd}\"}}\n",
        "x".repeat(pad),
        stamp(at)
    ));
    out.push_str(&format!(
        "{{\"type\":\"assistant\",\"uuid\":\"a-{at}\",\"message\":{{\"role\":\"assistant\",\"content\":[{{\"type\":\"text\",\"text\":\"ok\"}}]}},\"timestamp\":\"{}\"}}\n",
        stamp(at + 1)
    ));
    out
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

/// The pane is an agent pane the moment the agent opens, and that is minutes before there is
/// anything to read: the marker is written at session start and the transcript is not created
/// until the first prompt is submitted — 13:00:13 against 13:02:55, measured.
///
/// So a marker with no transcript is an **agent pane with an empty conversation**, which is a
/// different answer from "this pane has no conversation" and has to stay a different answer.
#[test]
fn a_pane_running_claude_is_an_agent_pane_before_it_has_written_a_transcript() {
    let home = home_with("marker-first", &[]);
    let pipeline = [PaneProcess::default(), live_process()];

    assert_eq!(
        located(&home, &Harness::Running(live_process())),
        None,
        "nothing on disk resolves — without which this test proves nothing"
    );

    let found = registry(&home)
        .marker(&pipeline)
        .expect("the session the pane's own process opened");
    assert_eq!(found.agent, "claude");
    assert_eq!(found.pid, LIVE_PID);
    assert_eq!(found.session, "c5eec836-44cf-4563-829c-cfdc322b3254");
    assert_eq!(found.cwd.as_deref(), Some(Path::new(CWD)));
    assert_eq!(found.name.as_deref(), Some("proj-4f"));
    assert_eq!(found.name_source.as_deref(), Some("derived"));
    assert_eq!(found.status.as_deref(), Some("idle"));
    assert_eq!(
        found.transcript, None,
        "an empty conversation, not the absence of one"
    );
}

/// Nothing here reads a process name, and that is the whole point. `process_info` reports only
/// `bash` for every job ble.sh runs in the shell's own process group (#297), which is every
/// interactive shell on the operator's machine — so a pane whose visible command says nothing at
/// all is still matched exactly, because the match is on **pid** against the marker directory.
#[test]
fn a_pane_whose_visible_command_is_only_bash_is_still_matched_on_pid() {
    let home = home_with("pipeline", &[DECOY, QUIT, RUNNING]);
    let shell = PaneProcess {
        pid: 1_463_000,
        start: Some("28770000".into()),
        started: None,
    };
    let pager = PaneProcess {
        pid: 1_463_001,
        ..shell.clone()
    };

    for alone in [&shell, &pager] {
        assert!(
            registry(&home).marker(std::slice::from_ref(alone)).is_none(),
            "neither of the other members of the pipeline is on a session"
        );
    }
    let found = registry(&home)
        .marker(&[shell, pager, live_process()])
        .expect("the harness anywhere in the pipeline");
    assert_eq!(found.pid, LIVE_PID);
    assert!(found.transcript.expect("its transcript").ends_with(RUNNING));
}

/// The `procStart` check survives the widening. A pid is a small integer the kernel hands out
/// again, and a whole pipeline of them is a whole pipeline of chances to believe a marker left
/// behind by a process that has gone.
#[test]
fn a_marker_left_by_a_dead_process_is_not_claimed_by_the_pid_that_replaced_it() {
    let home = home_with("pipeline-reused", &[DECOY, QUIT, RUNNING]);
    let reused = PaneProcess {
        start: Some("99999999".into()),
        ..live_process()
    };

    assert!(
        registry(&home).marker(&[live_process()]).is_some(),
        "the real process still resolves — without which this test proves nothing"
    );
    assert!(registry(&home).marker(&[reused]).is_none());
    assert!(registry(&home).marker(&[]).is_none());
}
