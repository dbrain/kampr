mod common;

use std::sync::Arc;

use common::*;
use kampr_journal::{ClaudeAdapter, CodexAdapter, Registry, SessionRef, TranscriptRoot};
use std::path::Path;

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
    assert!(!registry().has_conversation(None));
}

#[test]
fn a_harness_with_no_adapter_has_no_conversation() {
    let session = SessionRef::id("gemini", CLAUDE_SESSION);
    assert!(!registry().has_conversation(Some("gemini")));
    assert!(
        registry()
            .open(Some("gemini"), Some(&session), None)
            .unwrap()
            .is_none()
    );
}

/// `has_conversation` is "a journal adapter exists for this harness" — exactly what the wire
/// document says it means — so it can never be true on a node whose `caps.conversation` is false.
#[test]
fn a_pane_conversation_can_never_outrun_the_node_capability() {
    let registry = registry();
    for agent in [None, Some("claude"), Some("codex"), Some("gemini")] {
        assert!(!registry.has_conversation(agent) || registry.serves_any());
    }
    let empty = Registry::new();
    assert!(!empty.serves_any());
    assert!(!empty.has_conversation(Some("claude")));
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
            .open(Some("claude"), Some(&stale), None)
            .unwrap()
            .is_none()
    );
    let mut journal = registry
        .open(Some("claude"), Some(&stale), Some(Path::new("/home/u/demo")))
        .unwrap()
        .expect("the cwd still names a claude transcript");
    assert!(journal.path().ends_with(format!("{CLAUDE_SESSION}.jsonl")));
    assert_eq!(drain(journal.as_mut()).len(), 5);
}

#[test]
fn a_matching_session_opens_its_transcript() {
    let registry = registry();
    let session = SessionRef::id("claude", CLAUDE_SESSION);
    assert!(registry.has_conversation(Some("claude")));

    let mut journal = registry
        .open(Some("claude"), Some(&session), None)
        .unwrap()
        .expect("adapter selected");
    assert_eq!(drain(journal.as_mut()).len(), 5);

    let session = SessionRef::id("codex", CODEX_SESSION);
    let mut journal = registry
        .open(Some("codex"), Some(&session), None)
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
        .open(Some("claude"), None, Some(demo))
        .unwrap()
        .expect("claude resolves from cwd");
    assert!(claude.path().ends_with(format!("{CLAUDE_SESSION}.jsonl")));
    assert_eq!(drain(claude.as_mut()).len(), 5);

    // Two codex rollouts declare this cwd; the newest wins.
    let mut codex = registry
        .open(Some("codex"), None, Some(demo))
        .unwrap()
        .expect("codex resolves from cwd");
    assert!(codex.path().to_string_lossy().contains("2026/08/20"));
    assert!(!drain(codex.as_mut()).is_empty());

    assert!(
        registry
            .open(Some("claude"), None, Some(Path::new("/home/u/nowhere")))
            .unwrap()
            .is_none(),
        "a cwd no transcript claims resolves to nothing rather than to somebody else's"
    );
}

#[test]
fn a_home_with_both_harnesses_registers_both() {
    let home = scratch_dir("home");
    std::os::unix::fs::symlink(claude_root(), home.join(".claude")).unwrap();
    std::os::unix::fs::symlink(codex_root(), home.join(".codex")).unwrap();

    let registry = kampr_journal::registry_from_home(&home);
    let session = SessionRef::id("claude", CLAUDE_SESSION);
    let journal = registry
        .open(Some("claude"), Some(&session), None)
        .unwrap()
        .expect("claude adapter registered from home");
    assert!(journal.path().ends_with(format!("{CLAUDE_SESSION}.jsonl")));
    assert!(registry.get("codex").is_some());
}

#[test]
fn a_home_with_no_harness_registers_nothing() {
    let home = scratch_dir("bare-home");
    let registry = kampr_journal::registry_from_home(&home);
    assert!(registry.get("claude").is_none());
    assert!(registry.get("codex").is_none());
}
