//! A file the agent handed to the operator is the message, not a step on the way to one.
//!
//! Shapes from Claude 2.1.295: the call's `input` carries `files` (absolute or relative to the
//! record's `cwd`) and an optional `caption`; the result is a line of `file_uuid`s that name an
//! upload no node can reach.

use crate::common::*;
use kampr_journal::{Attachment, Block, FileRef};
use serde_json::{Value, json};

fn sent(input: Value) -> Vec<Value> {
    vec![
        json!({
            "type": "assistant", "uuid": "s1", "cwd": "/home/u/demo",
            "timestamp": "2026-10-19T10:52:25.000Z",
            "message": { "content": [
                { "type": "tool_use", "id": "toolu_send", "name": "SendUserFile", "input": input }
            ] }
        }),
        json!({
            "type": "user", "uuid": "s2", "cwd": "/home/u/demo",
            "timestamp": "2026-10-19T10:52:25.600Z",
            "message": { "content": [
                { "type": "tool_result", "tool_use_id": "toolu_send",
                  "content": "2 files delivered to user.\n  /tmp/shot.jpg → file_uuid: 4f99" }
            ] },
            "toolUseResult": { "attachments": [] }
        }),
    ]
}

fn header(path: &str, kind: &str, mime: Option<&str>, name: &str) -> Attachment {
    Attachment {
        id: FileRef::new(path).encode(),
        kind: kind.into(),
        mime: mime.map(str::to_string),
        bytes: None,
        name: Some(name.into()),
    }
}

#[test]
fn a_sent_file_is_its_caption_and_its_files_rather_than_a_tool_card() {
    let mut scratch = scratch_claude(
        "sent-files",
        &sent(json!({
            "files": ["/tmp/shot.jpg", "report.md"],
            "caption": "Before and after.",
            "status": "proactive"
        })),
    );
    let turns = scratch.turns();

    assert!(
        tool_blocks(&turns).is_empty(),
        "no card to fold into a run — {turns:#?}"
    );
    assert_eq!(turns.len(), 1, "the result adds nothing of its own — {turns:#?}");
    let blocks = &turns[0].blocks;
    assert_eq!(blocks[0], Block::md("Before and after."));
    assert_eq!(
        attachments(&turns),
        vec![
            &header("/tmp/shot.jpg", "image", Some("image/jpeg"), "shot.jpg"),
            &header("/home/u/demo/report.md", "file", None, "report.md"),
        ],
        "a relative path is the record's own working directory's"
    );
}

#[test]
fn a_sent_file_with_no_caption_is_its_files_alone() {
    let mut scratch = scratch_claude("sent-bare", &sent(json!({ "files": ["/tmp/a.zip"] })));
    let turns = scratch.turns();

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].blocks.len(), 1, "{turns:#?}");
    assert_eq!(
        attachments(&turns),
        vec![&header("/tmp/a.zip", "file", None, "a.zip")]
    );
}
