mod common;

use common::*;
use kampr_journal::{Block, ClaudeAdapter, JournalAdapter, Role, SessionRef, ToolState, TranscriptRoot};

fn journal() -> Box<dyn kampr_journal::Journal> {
    let adapter = ClaudeAdapter::new(TranscriptRoot::new(claude_root()).unwrap());
    adapter
        .open(&SessionRef::id("claude", CLAUDE_SESSION))
        .expect("open")
}

#[test]
fn locates_a_session_by_id_under_the_root() {
    let adapter = ClaudeAdapter::new(TranscriptRoot::new(claude_root()).unwrap());
    let found = adapter.locate(&SessionRef::id("claude", CLAUDE_SESSION)).unwrap();
    assert_eq!(found, claude_transcript().canonicalize().unwrap());
}

#[test]
fn parses_only_conversation_records() {
    let mut journal = journal();
    let turns = drain(journal.as_mut());

    let ids: Vec<&str> = turns.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "86ce419f-c0d3-4f51-bfc5-cbded73665d3",
            "ea8efd25-b0e1-46bf-8520-714c93dea66f",
            "aa803b51-afc2-4dd4-8c0c-cd27526951ea",
            "d880dc91-044a-4449-accb-ae813a6bc922",
            "b3721c3d-3c26-4165-922a-640d5adfcd2d",
        ],
        "mode/permission-mode/attachment/system records and thinking-only records carry no turn"
    );
    assert_eq!(turns[0].role, Role::User);
    assert_eq!(turns[1].role, Role::Assistant);
    assert_eq!(
        turns[0].at.as_deref(),
        Some("2026-08-17T03:47:14.049Z"),
        "timestamps pass through as recorded"
    );
}

#[test]
fn markdown_is_passed_through_verbatim() {
    let mut journal = journal();
    let turns = drain(journal.as_mut());

    let source = std::fs::read_to_string(claude_transcript()).unwrap();
    let recorded: String = source
        .lines()
        .filter_map(|l| serde_json::from_str::<serde_json::Value>(l).ok())
        .find_map(|v| {
            v.pointer("/message/content/0/text")
                .and_then(|t| t.as_str())
                .map(str::to_string)
        })
        .expect("fixture has an assistant text block");

    assert!(recorded.contains("| Key | Accepted |"));
    assert_eq!(
        md_texts(&turns),
        vec!["which keys does the grammar accept?", recorded.as_str()]
    );
}

#[test]
fn a_tool_use_settles_when_its_result_lands() {
    let mut journal = journal();
    let turns = drain(journal.as_mut());

    let tools = tool_blocks(&turns);
    let named: Vec<(&str, &ToolState, &Option<u32>, &Option<String>)> = tools
        .iter()
        .map(|b| match b {
            Block::Tool {
                name,
                state,
                lines,
                summary,
            } => (name.as_str(), state, lines, summary),
            _ => unreachable!(),
        })
        .collect();

    assert_eq!(named.len(), 3);
    assert_eq!(named[0].0, "Read");
    assert_eq!(named[0].1, &ToolState::Done);
    assert_eq!(named[0].2, &Some(2));
    assert_eq!(named[0].3.as_deref(), Some("/home/u/demo/notes.md"));
    assert_eq!(named[1].0, "Edit");
    assert_eq!(named[2].0, "Bash");
    assert_eq!(
        named[2].3.as_deref(),
        Some("list panes"),
        "Bash summarises from its description, not its command line"
    );
}

#[test]
fn a_bash_command_becomes_a_code_block() {
    let mut journal = journal();
    let turns = drain(journal.as_mut());

    let bash = turns
        .iter()
        .find(|t| t.id == "b3721c3d-3c26-4165-922a-640d5adfcd2d")
        .unwrap();
    assert_eq!(bash.blocks.len(), 2);
    assert_eq!(
        bash.blocks[1],
        Block::Code {
            lang: Some("bash".into()),
            text: "herdr pane list --json".into(),
        }
    );
}

#[test]
fn an_edit_result_appends_a_diff_block() {
    let mut journal = journal();
    let turns = drain(journal.as_mut());

    let diffs = diff_blocks(&turns);
    assert_eq!(diffs.len(), 1, "only Edit carries a structuredPatch");
    assert_eq!(
        diffs[0],
        &Block::Diff {
            path: Some("/home/u/demo/notes.md".into()),
            text: "@@ -1,1 +1,1 @@\n-old line\n+new line\n".into(),
        }
    );
}

#[test]
fn pages_backwards_from_the_newest_turn() {
    let mut journal = journal();
    drain(journal.as_mut());

    let newest = journal.page_before(None, 2);
    assert_eq!(newest.turns.len(), 2);
    assert!(newest.more);
    assert_eq!(
        newest.cursor.as_deref(),
        Some("d880dc91-044a-4449-accb-ae813a6bc922")
    );

    let older = journal.page_before(newest.cursor.as_deref(), 2);
    let ids: Vec<&str> = older.turns.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "ea8efd25-b0e1-46bf-8520-714c93dea66f",
            "aa803b51-afc2-4dd4-8c0c-cd27526951ea"
        ]
    );
    assert!(older.more);

    let oldest = journal.page_before(older.cursor.as_deref(), 2);
    assert_eq!(oldest.turns.len(), 1);
    assert!(!oldest.more);
}

/// A conversation's recency is when it was **last** written, not when it opened.
///
/// Ranking by the head meant a long session that started yesterday lost to a five-minute one
/// started this morning — and it is precisely the long session that a pane is still sitting in.
/// Measured against a real pane: a 12-hour-dead 20 KB transcript was served in preference to the
/// 9.9 MB one the operator was watching.
#[test]
fn the_newest_transcript_is_the_last_written_not_the_first_opened() {
    let home = scratch_dir("recency");
    let project = home.join("projects/-home-u-live");
    std::fs::create_dir_all(&project).unwrap();

    let record = |at: &str, text: &str| {
        serde_json::json!({
            "type": "user", "uuid": text, "timestamp": at, "cwd": "/home/u/live",
            "message": { "content": text }
        })
        .to_string()
            + "\n"
    };

    // Opened yesterday, still being written into a minute ago. Every record the old ranking read
    // — the first forty — is yesterday's; the ones after it are today's.
    let long = project.join("9f1c0b2e-0000-4000-8000-0000000000aa.jsonl");
    let mut body = String::new();
    for n in 0..60 {
        body += &record(&format!("2026-08-20T08:{n:02}:00Z"), &format!("long-open-{n}"));
    }
    for n in 0..60 {
        body += &record(&format!("2026-08-21T10:{n:02}:00Z"), &format!("long-live-{n}"));
    }
    std::fs::write(&long, &body).unwrap();

    // Opened this morning, dead since. Newer head, older tail.
    let short = project.join("9f1c0b2e-0000-4000-8000-0000000000bb.jsonl");
    std::fs::write(
        &short,
        record("2026-08-21T09:00:00Z", "short-open") + &record("2026-08-21T09:05:00Z", "short-last"),
    )
    .unwrap();

    let adapter = ClaudeAdapter::new(TranscriptRoot::new(&home).unwrap());
    let found = adapter
        .locate_by_cwd(std::path::Path::new("/home/u/live"))
        .expect("a transcript for the cwd");
    assert_eq!(
        found,
        long.canonicalize().unwrap(),
        "the live conversation lost to a dead one because only its first records were read"
    );
}
