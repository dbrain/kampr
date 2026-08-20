mod common;

use std::sync::Arc;

use common::*;
use kampr_journal::{ClaudeAdapter, CodexAdapter, Registry, SessionRef, TranscriptRoot};

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
    assert!(!registry().has_conversation(None, None));
}

#[test]
fn a_harness_with_no_adapter_has_no_conversation() {
    let session = SessionRef::id("gemini", CLAUDE_SESSION);
    assert!(!registry().has_conversation(Some("gemini"), Some(&session)));
    assert!(registry().open(Some("gemini"), Some(&session)).unwrap().is_none());
}

#[test]
fn a_stale_session_announcement_is_ignored() {
    // Herdr keeps reporting the last session a pane announced (probe #38), so a pane relaunched
    // as claude can still be advertising the codex session it used to run.
    let stale = SessionRef::id("codex", CODEX_SESSION);
    let registry = registry();
    assert!(!registry.has_conversation(Some("claude"), Some(&stale)));
    assert!(registry.open(Some("claude"), Some(&stale)).unwrap().is_none());
}

#[test]
fn a_matching_session_opens_its_transcript() {
    let registry = registry();
    let session = SessionRef::id("claude", CLAUDE_SESSION);
    assert!(registry.has_conversation(Some("claude"), Some(&session)));

    let mut journal = registry
        .open(Some("claude"), Some(&session))
        .unwrap()
        .expect("adapter selected");
    assert_eq!(drain(journal.as_mut()).len(), 5);

    let session = SessionRef::id("codex", CODEX_SESSION);
    let mut journal = registry
        .open(Some("codex"), Some(&session))
        .unwrap()
        .expect("adapter selected");
    assert_eq!(drain(journal.as_mut()).len(), 5);
}

#[test]
fn a_home_with_both_harnesses_registers_both() {
    let home = scratch_dir("home");
    std::os::unix::fs::symlink(claude_root(), home.join(".claude")).unwrap();
    std::os::unix::fs::symlink(codex_root(), home.join(".codex")).unwrap();

    let registry = kampr_journal::registry_from_home(&home);
    let session = SessionRef::id("claude", CLAUDE_SESSION);
    let journal = registry
        .open(Some("claude"), Some(&session))
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
