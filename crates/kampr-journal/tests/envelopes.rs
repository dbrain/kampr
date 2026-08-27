mod common;

use common::*;
use kampr_journal::{Block, FileJournal, Journal, Role};

// A background agent finishing writes a `user` record nobody typed (#286). It reached the phone as
// "You: <task-notification>…", and because the client starts a new exchange at every user turn it
// also cut the answer it interrupted in two.
const CLAUDE: &str = r#"{"type":"user","uuid":"u-1","timestamp":"2026-08-24T09:00:00.000Z","message":{"role":"user","content":[{"type":"text","text":"have another look at the width inference"}]}}
{"type":"assistant","uuid":"a-1","timestamp":"2026-08-24T09:00:04.000Z","message":{"role":"assistant","content":[{"type":"text","text":"Reading the registry."}]}}
{"type":"user","uuid":"u-2","timestamp":"2026-08-24T09:00:09.000Z","message":{"role":"user","content":[{"type":"text","text":"<task-notification>\n<task-id>b88dc5bqu</task-id>\n<status>completed</status>\n<summary>Background command \"cargo test\" completed (exit code 0)</summary>\n</task-notification>"}]}}
{"type":"assistant","uuid":"a-2","timestamp":"2026-08-24T09:00:12.000Z","message":{"role":"assistant","content":[{"type":"text","text":"The rect is 47 and the PTY is 93."}]}}
{"type":"user","uuid":"u-3","timestamp":"2026-08-24T09:00:20.000Z","message":{"role":"user","content":[{"type":"text","text":"<command-name>/compact</command-name>\n<command-message>compact</command-message>\n<command-args></command-args>"}]}}
{"type":"user","uuid":"u-4","timestamp":"2026-08-24T09:00:31.000Z","message":{"role":"user","content":[{"type":"text","text":"ship it<system-reminder>The date has changed.</system-reminder>"}]}}
"#;

const CODEX: &str = r#"{"type":"response_item","timestamp":"2026-08-26T11:01:00.000Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>\n<cwd>/home/u/dev/kampr</cwd>\n<shell>bash</shell>\n</environment_context>"}]}}
{"type":"response_item","timestamp":"2026-08-26T11:01:01.000Z","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"review the mesh"}]}}
{"type":"response_item","timestamp":"2026-08-26T11:01:06.000Z","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":"The hub trusts a peer's own node list."}]}}
"#;

fn said(turns: &[kampr_journal::Turn]) -> Vec<(Role, String)> {
    turns
        .iter()
        .map(|turn| {
            let text = turn
                .blocks
                .iter()
                .filter_map(|b| match b {
                    Block::Md { text, .. } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(" ");
            (turn.role, text)
        })
        .collect()
}

#[test]
fn a_claude_transcript_carries_only_what_the_operator_typed() {
    let scratch = scratch_dir("envelope-claude");
    let path = scratch.join("session.jsonl");
    std::fs::write(&path, CLAUDE).unwrap();

    let mut journal = FileJournal::new(path, claude_parser(), Some(kampr_journal::claude::live));
    let turns = journal.poll().unwrap();

    assert_eq!(
        said(&turns),
        vec![
            (Role::User, "have another look at the width inference".to_string()),
            (Role::Assistant, "Reading the registry.".to_string()),
            (Role::Assistant, "The rect is 47 and the PTY is 93.".to_string()),
            (Role::User, "ship it".to_string()),
        ],
        "a notification and a slash command are the harness talking to itself, and a reminder \
         stapled to a prompt is not part of the prompt"
    );
}

// The half that matters to the reader: with the notification gone the two halves of the answer it
// interrupted are one reply again, which is the unit the conversation view groups and collapses by.
#[test]
fn an_answer_a_notification_landed_in_the_middle_of_stays_one_answer() {
    let scratch = scratch_dir("envelope-exchange");
    let path = scratch.join("session.jsonl");
    std::fs::write(&path, CLAUDE).unwrap();

    let mut journal = FileJournal::new(path, claude_parser(), Some(kampr_journal::claude::live));
    let turns = journal.poll().unwrap();
    let roles: Vec<Role> = turns.iter().map(|t| t.role).collect();
    let breaks = roles.iter().filter(|r| **r == Role::User).count();

    assert_eq!(breaks, 2, "two prompts, so two exchanges");
    assert_eq!(
        roles,
        [Role::User, Role::Assistant, Role::Assistant, Role::User],
        "the answer is not split by something nobody said"
    );
}

#[test]
fn a_codex_transcript_does_not_open_on_the_harness_describing_the_machine() {
    let scratch = scratch_dir("envelope-codex");
    let path = scratch.join("session.jsonl");
    std::fs::write(&path, CODEX).unwrap();

    let mut journal = FileJournal::new(path, codex_parser(), None);
    let turns = journal.poll().unwrap();

    assert_eq!(
        said(&turns),
        vec![
            (Role::User, "review the mesh".to_string()),
            (
                Role::Assistant,
                "The hub trusts a peer's own node list.".to_string()
            ),
        ]
    );
}
