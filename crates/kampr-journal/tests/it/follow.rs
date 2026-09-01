use std::io::Write;

use crate::common::*;
use kampr_journal::{Block, FileJournal, Journal, ToolState};

fn append(path: &std::path::Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
    f.flush().unwrap();
}

#[test]
fn appended_records_arrive_without_replaying_the_file() {
    let source: Vec<String> = std::fs::read_to_string(claude_transcript())
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    let split = 8; // through the Read tool_result

    let scratch = scratch_dir("follow");
    let path = scratch.join("session.jsonl");
    std::fs::write(&path, source[..split].join("\n") + "\n").unwrap();

    let mut journal = FileJournal::new(path.clone(), claude_parser(), Some(kampr_journal::claude::live));
    let first = journal.poll().unwrap();
    assert_eq!(first.len(), 2);

    assert!(
        journal.poll().unwrap().is_empty(),
        "a poll with no growth yields nothing"
    );

    append(&path, &(source[split..].join("\n") + "\n"));
    let second = journal.poll().unwrap();
    let ids: Vec<&str> = second.iter().map(|t| t.id.as_str()).collect();
    assert_eq!(
        ids,
        [
            "aa803b51-afc2-4dd4-8c0c-cd27526951ea",
            "d880dc91-044a-4449-accb-ae813a6bc922",
            "b3721c3d-3c26-4165-922a-640d5adfcd2d"
        ]
    );

    assert_eq!(
        journal.page_before(None, 10).turns.len(),
        5,
        "the whole transcript is available for paging after following"
    );
}

#[test]
fn a_settled_tool_is_re_emitted_under_the_same_id() {
    let source: Vec<String> = std::fs::read_to_string(claude_transcript())
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();
    // Cut between the Bash tool_use and its tool_result.
    let split = source.len() - 1;

    let scratch = scratch_dir("settle");
    let path = scratch.join("session.jsonl");
    std::fs::write(&path, source[..split].join("\n") + "\n").unwrap();

    let mut journal = FileJournal::new(path.clone(), claude_parser(), Some(kampr_journal::claude::live));
    let first = journal.poll().unwrap();
    let bash = first.last().unwrap();
    assert!(matches!(
        &bash.blocks[0],
        Block::Tool {
            state: ToolState::Running,
            ..
        }
    ));

    append(&path, &(source[split..].join("\n") + "\n"));
    let second = journal.poll().unwrap();
    assert_eq!(second.len(), 1);
    assert_eq!(second[0].id, bash.id);
    assert!(matches!(
        &second[0].blocks[0],
        Block::Tool {
            state: ToolState::Done,
            lines: Some(2),
            ..
        }
    ));
}

#[test]
fn a_half_written_line_waits_for_its_newline() {
    let source: Vec<String> = std::fs::read_to_string(claude_transcript())
        .unwrap()
        .lines()
        .map(str::to_string)
        .collect();

    let scratch = scratch_dir("torn");
    let path = scratch.join("session.jsonl");
    std::fs::write(&path, source[..5].join("\n") + "\n").unwrap();

    let mut journal = FileJournal::new(path.clone(), claude_parser(), Some(kampr_journal::claude::live));
    assert_eq!(journal.poll().unwrap().len(), 1);

    let record = &source[5];
    let (head, tail) = record.split_at(record.len() / 2);
    append(&path, head);
    assert!(
        journal.poll().unwrap().is_empty(),
        "a torn line must not be parsed"
    );

    append(&path, &format!("{tail}\n"));
    assert_eq!(journal.poll().unwrap().len(), 1);
}

#[test]
fn a_truncated_transcript_is_re_read_from_the_start() {
    let source = std::fs::read_to_string(claude_transcript()).unwrap();
    let scratch = scratch_dir("truncate");
    let path = scratch.join("session.jsonl");
    std::fs::write(&path, &source).unwrap();

    let mut journal = FileJournal::new(path.clone(), claude_parser(), Some(kampr_journal::claude::live));
    assert_eq!(journal.poll().unwrap().len(), 5);

    std::fs::write(
        &path,
        source.lines().take(4).collect::<Vec<_>>().join("\n") + "\n",
    )
    .unwrap();
    let after = journal.poll().unwrap();
    assert_eq!(after.len(), 1);
    assert_eq!(journal.page_before(None, 10).turns.len(), 1);
}
