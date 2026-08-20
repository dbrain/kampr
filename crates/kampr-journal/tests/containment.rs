mod common;

use common::*;
use kampr_journal::{ClaudeAdapter, CodexAdapter, JournalAdapter, JournalError, SessionRef, TranscriptRoot};

#[test]
fn a_path_inside_the_root_resolves() {
    let root = TranscriptRoot::new(claude_root()).unwrap();
    let inside = format!("projects/-home-u-demo/{CLAUDE_SESSION}.jsonl");
    assert_eq!(
        root.contain(&inside).unwrap(),
        claude_transcript().canonicalize().unwrap()
    );
}

#[test]
fn an_absolute_path_outside_the_root_is_refused() {
    let root = TranscriptRoot::new(claude_root()).unwrap();
    assert!(matches!(
        root.contain("/etc/passwd"),
        Err(JournalError::Escape(_))
    ));
}

#[test]
fn dot_dot_cannot_climb_out_of_the_root() {
    let root = TranscriptRoot::new(claude_root()).unwrap();
    for escape in ["../codex/sessions", "projects/../../codex"] {
        assert!(
            matches!(root.contain(escape), Err(JournalError::Escape(_))),
            "{escape} resolves to a real directory outside the root"
        );
    }
    assert!(
        root.contain("../../../../../../etc/passwd").is_err(),
        "climbing past the filesystem root must not resolve either"
    );
}

#[test]
fn a_symlink_pointing_out_of_the_root_is_refused() {
    let dir = scratch_dir("symlink");
    let root_dir = dir.join("root");
    let outside = dir.join("outside");
    std::fs::create_dir_all(&root_dir).unwrap();
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("secret.jsonl"), "{}\n").unwrap();
    std::os::unix::fs::symlink(outside.join("secret.jsonl"), root_dir.join("link.jsonl")).unwrap();

    let root = TranscriptRoot::new(&root_dir).unwrap();
    assert!(matches!(root.contain("link.jsonl"), Err(JournalError::Escape(_))));
}

#[test]
fn a_session_id_may_not_carry_path_syntax() {
    let root = TranscriptRoot::new(claude_root()).unwrap();
    for hostile in [
        "..",
        ".",
        "../../etc/passwd",
        "a/b",
        "/abs",
        "",
        "id\0",
        "id with space",
    ] {
        assert!(
            matches!(root.check_id(hostile), Err(JournalError::Escape(_))),
            "{hostile:?} must not be accepted as a session id"
        );
    }
    assert!(root.check_id(CLAUDE_SESSION).is_ok());
}

#[test]
fn adapters_refuse_a_pane_supplied_escape() {
    let claude = ClaudeAdapter::new(TranscriptRoot::new(claude_root()).unwrap());
    assert!(matches!(
        claude.locate(&SessionRef::path("claude", "/etc/passwd")),
        Err(JournalError::Escape(_))
    ));
    assert!(matches!(
        claude.locate(&SessionRef::id("claude", "../../../../etc/passwd")),
        Err(JournalError::Escape(_))
    ));

    let codex = CodexAdapter::new(TranscriptRoot::new(codex_root()).unwrap());
    assert!(matches!(
        codex.locate(&SessionRef::path("codex", "../claude/projects")),
        Err(JournalError::Escape(_))
    ));
    assert!(matches!(
        codex.locate(&SessionRef::id("codex", "../../etc/passwd")),
        Err(JournalError::Escape(_))
    ));
}

#[test]
fn an_unknown_session_is_not_found_rather_than_leaking() {
    let claude = ClaudeAdapter::new(TranscriptRoot::new(claude_root()).unwrap());
    assert!(matches!(
        claude.locate(&SessionRef::id("claude", "00000000-dead-beef-0000-000000000000")),
        Err(JournalError::NotFound(_))
    ));
}
