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
        .locate_by_cwd(std::path::Path::new("/home/u/live"), None)
        .expect("a transcript for the cwd");
    assert_eq!(
        found,
        long.canonicalize().unwrap(),
        "the live conversation lost to a dead one because only its first records were read"
    );
}

/// A 1×1 PNG, so a test can ask whether real image bytes escaped as well as whether the image
/// was named. Both records below are the shapes Claude 2.1.220–2.1.236 actually writes: a paste
/// arrives as an `image` block beside the text in `message.content`, and a `Read` of a picture
/// arrives as a `tool_result` whose content array holds an `image` and no text at all.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

fn pasted(text: Option<&str>) -> serde_json::Value {
    let image = serde_json::json!({
        "type": "image",
        "source": { "type": "base64", "media_type": "image/png", "data": PNG }
    });
    let content: Vec<serde_json::Value> = match text {
        Some(text) => vec![serde_json::json!({ "type": "text", "text": text }), image],
        None => vec![image],
    };
    serde_json::json!({
        "type": "user",
        "uuid": "549c13ed-c2b4-4013-b072-f26304a5bb6c",
        "timestamp": "2026-08-20T02:56:27.681Z",
        "imagePasteIds": [1],
        "message": { "role": "user", "content": content }
    })
}

#[test]
fn a_pasted_screenshot_is_named_beside_the_words_it_came_with() {
    let mut scratch = scratch_claude("paste", &[pasted(Some("does this look right?"))]);
    let turns = scratch.turns();

    assert_eq!(turns.len(), 1);
    assert_eq!(turns[0].role, Role::User);
    assert_eq!(
        turns[0].blocks[0],
        Block::md("does this look right?"),
        "a turn that renders only the words reads as though no screenshot was ever sent"
    );
    let Block::Md { text, att } = &turns[0].blocks[1] else {
        panic!("expected a marker, got {:?}", turns[0].blocks[1]);
    };
    assert_eq!(text, "[image · png]", "the marker an installed phone renders");
    let att = att
        .as_ref()
        .expect("a header the client can fetch the bytes with");
    assert_eq!(att.kind, "image");
    assert_eq!(att.mime.as_deref(), Some("image/png"));
    assert_eq!(att.bytes, Some(70));
    assert_eq!(
        att.name, None,
        "a paste carries no filename and inventing one is a lie about the transcript"
    );
    assert!(!att.id.is_empty());
}

#[test]
fn a_message_that_is_nothing_but_an_image_is_still_a_turn() {
    let mut scratch = scratch_claude("paste-only", &[pasted(None)]);
    let turns = scratch.turns();

    assert_eq!(
        turns.len(),
        1,
        "dropping it leaves the answer to a question the transcript never shows being asked"
    );
    assert_eq!(md_texts(&turns), vec!["[image · png]"]);
    assert_eq!(attachments(&turns).len(), 1);
}

#[test]
fn image_bytes_never_reach_the_wire() {
    let read = serde_json::json!({
        "type": "assistant", "uuid": "d1", "timestamp": "2026-08-20T02:56:30.000Z",
        "message": { "content": [
            { "type": "tool_use", "id": "toolu_1", "name": "Read",
              "input": { "file_path": "/home/u/demo/shot.png" } }
        ] }
    });
    let result = serde_json::json!({
        "type": "user", "uuid": "d2", "timestamp": "2026-08-20T02:56:31.000Z",
        "message": { "content": [
            { "type": "tool_result", "tool_use_id": "toolu_1",
              "content": [ { "type": "image",
                             "source": { "type": "base64", "media_type": "image/png", "data": PNG } } ] }
        ] },
        "toolUseResult": { "type": "image", "file": { "base64": PNG, "type": "image/png" } }
    });
    let mut scratch = scratch_claude("bytes", &[pasted(Some("look")), read, result]);
    scratch.turns();

    let wire = serde_json::to_string(&scratch.journal.page_before(None, 10).turns).unwrap();
    assert!(wire.contains("[image · png]"), "{wire}");
    assert!(wire.contains("\"att\""), "{wire}");
    assert!(
        !wire.contains(&PNG[..40]),
        "a screenshot is megabytes and the websocket carries it to a phone: {wire}"
    );
}

/// Claude emits several `tool_use` blocks in one assistant record and the results come back in
/// separate records, in whatever order the calls finish — so a result has to settle onto the card
/// its *own* call opened. Taking the first `Block::Tool` in the turn put every result on the first
/// card: the second tool's state, its line count and its diff all landed on the first one, and the
/// first tool's own result was overwritten by whichever came last.
#[test]
fn parallel_tool_calls_each_settle_onto_their_own_card() {
    let calls = serde_json::json!({
        "type": "assistant", "uuid": "p1", "timestamp": "2026-08-26T01:00:00.000Z",
        "message": { "content": [
            { "type": "tool_use", "id": "toolu_a", "name": "Read",
              "input": { "file_path": "/home/u/demo/notes.md" } },
            { "type": "tool_use", "id": "toolu_b", "name": "Grep",
              "input": { "pattern": "needle" } }
        ] }
    });
    // The second call answers first, which is the whole point of running them in parallel.
    let grep = serde_json::json!({
        "type": "user", "uuid": "p2", "timestamp": "2026-08-26T01:00:01.000Z",
        "message": { "content": [
            { "type": "tool_result", "tool_use_id": "toolu_b", "content": "one\ntwo\nthree" }
        ] }
    });
    let read = serde_json::json!({
        "type": "user", "uuid": "p3", "timestamp": "2026-08-26T01:00:02.000Z",
        "message": { "content": [
            { "type": "tool_result", "tool_use_id": "toolu_a", "content": "no such file",
              "is_error": true }
        ] }
    });
    let mut scratch = scratch_claude("parallel", &[calls, grep, read]);
    let turns = scratch.turns();

    assert_eq!(
        turns.iter().filter(|t| t.id == "p1").count(),
        1,
        "the turn both calls settled onto is delivered once, however many times it was marked"
    );
    let turn = turns.iter().find(|t| t.id == "p1").expect("the calling turn");
    let cards: Vec<&Block> = turn
        .blocks
        .iter()
        .filter(|b| matches!(b, Block::Tool { .. }))
        .collect();
    assert_eq!(cards.len(), 2, "one card per call: {:?}", turn.blocks);
    assert_eq!(
        cards[0],
        &Block::Tool {
            name: "Read".into(),
            summary: Some("/home/u/demo/notes.md".into()),
            lines: Some(1),
            state: ToolState::Error,
        },
        "the failing Read took the Grep's three lines and its success"
    );
    assert_eq!(
        cards[1],
        &Block::Tool {
            name: "Grep".into(),
            summary: Some("needle".into()),
            lines: Some(3),
            state: ToolState::Done,
        },
        "the Grep never settled at all"
    );
}
