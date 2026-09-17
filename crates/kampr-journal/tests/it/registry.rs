use std::sync::Arc;

use crate::common::*;
use kampr_journal::{
    ClaudeAdapter, CodexAdapter, Harness, OmpAdapter, PaneProcess, Registry, SessionRef, TranscriptRoot,
};
use std::path::{Path, PathBuf};

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.register(Arc::new(ClaudeAdapter::new(
        TranscriptRoot::new(claude_root()).unwrap(),
    )));
    registry.register(Arc::new(CodexAdapter::new(
        TranscriptRoot::new(codex_root()).unwrap(),
    )));
    registry
}

#[test]
fn a_shell_pane_has_no_conversation() {
    assert!(!registry().serves(None));
    assert!(
        registry()
            .locate(None, None, None, &Harness::Unknown)
            .unwrap()
            .is_none()
    );
}

#[test]
fn a_harness_with_no_adapter_has_no_conversation() {
    let session = SessionRef::id("gemini", CLAUDE_SESSION);
    assert!(!registry().serves(Some("gemini")));
    assert!(
        registry()
            .open(Some("gemini"), Some(&session), None, &Harness::Unknown)
            .unwrap()
            .is_none()
    );
}

/// `has_conversation` is "a transcript resolves", and a node that serves no adapter at all
/// resolves nothing — so a pane's claim can never outrun `caps.conversation`.
#[test]
fn a_pane_conversation_can_never_outrun_the_node_capability() {
    let registry = registry();
    for agent in [None, Some("claude"), Some("codex"), Some("agy"), Some("gemini")] {
        assert!(!registry.serves(agent) || registry.serves_any());
    }
    let empty = Registry::new();
    assert!(!empty.serves_any());
    assert!(!empty.serves(Some("claude")));
    assert!(
        empty
            .locate(
                Some("claude"),
                None,
                Some(Path::new("/home/u/demo")),
                &Harness::Unknown
            )
            .unwrap()
            .is_none()
    );
}

/// An adapter for the harness is not a conversation. A `claude` whose working directory nothing
/// has ever run in resolves to nothing — which is the pane the New sheet creates, opening on the
/// Conversation view — and `locate` is what says so before the herd claims otherwise.
#[test]
fn a_harness_with_no_transcript_on_disk_resolves_to_nothing() {
    let registry = registry();
    assert!(registry.serves(Some("claude")), "the adapter is registered");
    assert!(
        registry
            .locate(
                Some("claude"),
                None,
                Some(Path::new("/home/u/never-used")),
                &Harness::Unknown
            )
            .unwrap()
            .is_none(),
        "a directory with no transcript has no conversation"
    );
    assert!(
        registry
            .locate(
                Some("claude"),
                None,
                Some(Path::new("/home/u/demo")),
                &Harness::Unknown
            )
            .unwrap()
            .is_some(),
        "and one with a transcript does"
    );
}

#[test]
fn a_stale_session_announcement_falls_back_to_the_working_directory() {
    // Herdr keeps reporting the last session a pane announced (probe #38), so a pane relaunched
    // as claude can still be advertising the codex session it used to run. The stale reference is
    // ignored; the pane's cwd is what is left to go on.
    let stale = SessionRef::id("codex", CODEX_SESSION);
    let registry = registry();
    assert!(
        registry
            .open(Some("claude"), Some(&stale), None, &Harness::Unknown)
            .unwrap()
            .is_none()
    );
    let mut journal = registry
        .open(
            Some("claude"),
            Some(&stale),
            Some(Path::new("/home/u/demo")),
            &Harness::Unknown,
        )
        .unwrap()
        .expect("the cwd still names a claude transcript");
    assert!(journal.path().ends_with(format!("{CLAUDE_SESSION}.jsonl")));
    assert_eq!(drain(journal.as_mut()).len(), 5);
}

#[test]
fn a_matching_session_opens_its_transcript() {
    let registry = registry();
    let session = SessionRef::id("claude", CLAUDE_SESSION);
    assert!(registry.serves(Some("claude")));

    let mut journal = registry
        .open(Some("claude"), Some(&session), None, &Harness::Unknown)
        .unwrap()
        .expect("adapter selected");
    assert_eq!(drain(journal.as_mut()).len(), 5);

    let session = SessionRef::id("codex", CODEX_SESSION);
    let mut journal = registry
        .open(Some("codex"), Some(&session), None, &Harness::Unknown)
        .unwrap()
        .expect("adapter selected");
    assert_eq!(drain(journal.as_mut()).len(), 5);
}

/// Herdr 0.8.2 detects Claude and Codex by scraping the screen and never populates
/// `pane.agent_session` — verified live against a real `claude` in a headless session. The pane's
/// working directory is therefore the only handle a node actually gets, and it has to be enough.
#[test]
fn a_pane_with_no_session_announcement_resolves_from_its_cwd() {
    let registry = registry();
    let demo = Path::new("/home/u/demo");

    let mut claude = registry
        .open(Some("claude"), None, Some(demo), &Harness::Unknown)
        .unwrap()
        .expect("claude resolves from cwd");
    assert!(claude.path().ends_with(format!("{CLAUDE_SESSION}.jsonl")));
    assert_eq!(drain(claude.as_mut()).len(), 5);

    // Two codex rollouts declare this cwd; the newest wins.
    let mut codex = registry
        .open(Some("codex"), None, Some(demo), &Harness::Unknown)
        .unwrap()
        .expect("codex resolves from cwd");
    assert!(codex.path().to_string_lossy().contains("2026/08/20"));
    assert!(!drain(codex.as_mut()).is_empty());

    assert!(
        registry
            .open(
                Some("claude"),
                None,
                Some(Path::new("/home/u/nowhere")),
                &Harness::Unknown
            )
            .unwrap()
            .is_none(),
        "a cwd no transcript claims resolves to nothing rather than to somebody else's"
    );
}

#[test]
fn a_home_registers_every_harness_it_has_a_root_for() {
    let home = scratch_dir("home");
    std::os::unix::fs::symlink(claude_root(), home.join(".claude")).unwrap();
    std::os::unix::fs::symlink(codex_root(), home.join(".codex")).unwrap();
    std::fs::create_dir(home.join(".gemini")).unwrap();
    std::os::unix::fs::symlink(agy_root(), home.join(".gemini/antigravity-cli")).unwrap();

    let registry = kampr_journal::registry_from_home(&home);
    let session = SessionRef::id("claude", CLAUDE_SESSION);
    let journal = registry
        .open(Some("claude"), Some(&session), None, &Harness::Unknown)
        .unwrap()
        .expect("claude adapter registered from home");
    assert!(journal.path().ends_with(format!("{CLAUDE_SESSION}.jsonl")));
    assert!(registry.get("codex").is_some());

    let agy = registry
        .open(
            Some("agy"),
            Some(&SessionRef::id("agy", AGY_SESSION)),
            None,
            &Harness::Unknown,
        )
        .unwrap()
        .expect("agy adapter registered from home");
    assert!(agy.path().ends_with("transcript_full.jsonl"), "{:?}", agy.path());
}

/// `agy` keeps its conversations under `~/.gemini`, which `gemini-cli` also writes to — so a
/// machine that has only ever run the older harness has the outer directory and none of the
/// inner one, and registering on the outer would claim conversations that are not there.
#[test]
fn a_gemini_home_with_no_antigravity_directory_registers_nothing() {
    let home = scratch_dir("gemini-only");
    std::fs::create_dir(home.join(".gemini")).unwrap();
    assert!(kampr_journal::registry_from_home(&home).get("agy").is_none());
}

#[test]
fn a_home_with_no_harness_registers_nothing() {
    let home = scratch_dir("bare-home");
    let registry = kampr_journal::registry_from_home(&home);
    assert!(registry.get("claude").is_none());
    assert!(registry.get("codex").is_none());
    assert!(registry.get("agy").is_none());
}

/// "Which session is this pid on, and is it live" is a [`kampr_journal::JournalAdapter`]
/// capability, not a Claude one. Claude answers it from a marker directory it happens to write;
/// a harness that writes nothing says so, and a registry holding only such harnesses answers
/// nothing rather than guessing — which is what keeps Codex from becoming second-class because
/// Claude grew a convenient file.
#[test]
fn a_harness_that_publishes_no_marker_claims_none_of_a_panes_processes() {
    let pipeline = [kampr_journal::PaneProcess::look_up(std::process::id())];

    let mut only_codex = Registry::new();
    only_codex.register(Arc::new(CodexAdapter::new(
        TranscriptRoot::new(codex_root()).unwrap(),
    )));
    assert!(only_codex.marker(&pipeline).is_none());
    assert!(Registry::new().marker(&pipeline).is_none());
    assert!(
        registry().marker(&pipeline).is_none(),
        "this test's own pid is on no harness session"
    );
}

const FOREIGN_SESSION: &str = "9f1c0b2e-0000-4000-8000-0000000000f0";
const OWN_SESSION: &str = "9f1c0b2e-0000-4000-8000-0000000000f1";

/// A sessions root holding one transcript that declares `cwd`, stamped at the moment of the call
/// so it reads as actively being written after this test's process started.
fn lively_cwd(tag: &str, session: &str) -> (ScratchDir, PathBuf, PathBuf) {
    let dir = scratch_dir(tag);
    let cwd = dir.join("cwd");
    std::fs::create_dir_all(&cwd).unwrap();
    let root = dir.join("root");
    let bucket = root.join("sessions").join("--lively--");
    std::fs::create_dir_all(&bucket).unwrap();
    let transcript = bucket.join(format!("1750000000000_{session}.jsonl"));
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    let header = serde_json::json!({
        "type": "session",
        "version": 3,
        "id": session,
        "timestamp": now,
        "cwd": cwd.to_string_lossy(),
    });
    std::fs::write(&transcript, format!("{header}\n")).unwrap();
    (dir, cwd, transcript)
}

fn pi_registry(root: &Path) -> Registry {
    let mut registry = Registry::new();
    registry.register(Arc::new(OmpAdapter::named(
        kampr_journal::omp::PI_AGENT,
        TranscriptRoot::new(root).unwrap(),
    )));
    registry
}

/// pi names its session only from the environment its tools carry (#542), so a fresh pi pane
/// names nothing at all while a neighbour in the same directory is mid-conversation — and the
/// directory must not answer for it.
#[test]
fn a_fresh_pi_pane_serves_nothing_while_a_neighbour_is_lively_in_its_cwd() {
    let process = PaneProcess::look_up(std::process::id());
    let (dir, cwd, _transcript) = lively_cwd("fresh-pi", FOREIGN_SESSION);
    let registry = pi_registry(&dir.join("root"));
    assert!(
        registry
            .locate(Some("pi"), None, Some(&cwd), &Harness::Running(process))
            .unwrap()
            .is_none(),
        "the directory holds only the neighbour's transcript"
    );
}

/// The environment handle is what reaches `locate` for a pi pane: the session the pane's own
/// processes name, announced. It must win over a lively cwd, and keeping the cwd out of a pi
/// pane's answer must not take it with it.
#[test]
fn a_pi_pane_serves_the_session_its_processes_name_despite_a_lively_cwd() {
    let process = PaneProcess::look_up(std::process::id());
    let (dir, cwd, _foreign) = lively_cwd("own-pi", FOREIGN_SESSION);
    let root = dir.join("root");
    let own = root
        .join("sessions")
        .join("--own--")
        .join(format!("1750000000000_{OWN_SESSION}.jsonl"));
    std::fs::create_dir_all(own.parent().unwrap()).unwrap();
    let now = time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap();
    let header = serde_json::json!({
        "type": "session",
        "version": 3,
        "id": OWN_SESSION,
        "timestamp": now,
        "cwd": "/home/u/elsewhere",
    });
    std::fs::write(&own, format!("{header}\n")).unwrap();

    let registry = pi_registry(&root);
    let session = SessionRef::id("pi", OWN_SESSION);
    let found = registry
        .locate(Some("pi"), Some(&session), Some(&cwd), &Harness::Running(process))
        .unwrap()
        .expect("the pane's own session resolves");
    assert_eq!(found, own);
}

/// The cwd handle stays for the harnesses whose process names its session on every tick: omp is
/// not sticky, and a lively cwd is exactly what it resolves from.
#[test]
fn a_non_sticky_pane_still_resolves_from_a_lively_cwd() {
    let process = PaneProcess::look_up(std::process::id());
    let (dir, cwd, foreign) = lively_cwd("non-sticky", FOREIGN_SESSION);
    let root = dir.join("root");
    let mut registry = Registry::new();
    registry.register(Arc::new(OmpAdapter::new(TranscriptRoot::new(&root).unwrap())));
    let found = registry
        .locate(Some("omp"), None, Some(&cwd), &Harness::Running(process))
        .unwrap()
        .expect("omp is not sticky, so the cwd answers");
    assert_eq!(found, foreign);
}
