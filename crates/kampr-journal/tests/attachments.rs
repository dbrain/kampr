//! What an `att` id is worth, and what it must never be worth.
//!
//! The wire carries a header and the bytes stay on disk, so every one of these ids arrives back
//! at the node from the network. The refusals below are the whole reason the id is a locator
//! rather than a path.

mod common;

use common::*;
use kampr_journal::attach::{self, Att, Locator, MAX_BYTES};
use kampr_journal::{Attachment, Block, JournalError, Registry, Turn};
use serde_json::{Value, json};

/// A 1×1 PNG. 70 bytes decoded, which is what every `bytes` below is checked against.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";

/// A second, different PNG — a 1×1 GIF, so a mixed record's two attachments cannot be told apart
/// by luck.
const GIF: &str = "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

fn pasted(text: &str, images: &[(&str, &str)]) -> Value {
    let mut content = vec![json!({ "type": "text", "text": text })];
    content.extend(images.iter().map(|(mime, data)| {
        json!({ "type": "image", "source": { "type": "base64", "media_type": mime, "data": data } })
    }));
    json!({
        "type": "user",
        "uuid": "549c13ed-c2b4-4013-b072-f26304a5bb6c",
        "timestamp": "2026-08-20T02:56:27.681Z",
        "message": { "role": "user", "content": content }
    })
}

/// The shape a `Read` of a picture actually leaves: an `image` in the result's content array with
/// no text beside it, and the size and dimensions on `toolUseResult.file`.
fn read_a_picture() -> Vec<Value> {
    vec![
        json!({
            "type": "assistant", "uuid": "d1", "timestamp": "2026-08-20T02:56:30.000Z",
            "message": { "content": [
                { "type": "tool_use", "id": "toolu_1", "name": "Read",
                  "input": { "file_path": "/home/u/demo/shot.png" } }
            ] }
        }),
        json!({
            "type": "user", "uuid": "d2", "timestamp": "2026-08-20T02:56:31.000Z",
            "message": { "content": [
                { "type": "tool_result", "tool_use_id": "toolu_1",
                  "content": [ { "type": "image",
                                 "source": { "type": "base64", "media_type": "image/png", "data": PNG } } ] }
            ] },
            "toolUseResult": { "type": "image", "file": { "base64": PNG, "type": "image/png",
                               "originalSize": 70,
                               "dimensions": { "originalWidth": 1, "originalHeight": 1 } } }
        }),
    ]
}

fn codex_view_image(data: &str) -> Vec<Value> {
    vec![
        json!({
            "type": "response_item", "timestamp": "2026-08-18T14:11:36.000Z",
            "payload": { "type": "function_call", "name": "view_image",
                         "arguments": "{\"path\":\"/home/u/demo/shot.png\"}",
                         "call_id": "call_1" }
        }),
        json!({
            "type": "response_item", "timestamp": "2026-08-18T14:11:37.000Z",
            "payload": { "type": "function_call_output", "call_id": "call_1", "output": [
                { "type": "input_image", "image_url": format!("data:image/png;base64,{data}"),
                  "detail": "original" }
            ] }
        }),
    ]
}

fn only(turns: &[Turn]) -> &Attachment {
    let found = attachments(turns);
    assert_eq!(found.len(), 1, "expected exactly one attachment in {turns:?}");
    found[0]
}

fn decoded(png: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(png)
        .expect("a png")
}

#[test]
fn a_pasted_screenshot_fetches_back_byte_for_byte() {
    let mut scratch = scratch_claude("att-paste", &[pasted("look", &[("image/png", PNG)])]);
    let turns = scratch.turns();
    let att = only(&turns);

    assert_eq!(att.kind, "image");
    assert_eq!(att.mime.as_deref(), Some("image/png"));
    assert_eq!(att.bytes, Some(70));
    assert_eq!(att.name, None);

    let got = attach::fetch(&scratch.journals, &att.id, &scratch.transcript).expect("the bytes");
    assert_eq!(got.data, decoded(PNG));
    assert_eq!(got.data.len() as u64, att.bytes.unwrap());
    assert_eq!(got.mime.as_deref(), Some("image/png"));
}

/// The case the marker used to miss entirely: a tool result whose only content is an image has no
/// text in it, so a reader was shown a `Read` that finished and nothing it produced.
#[test]
fn a_read_of_a_picture_is_named_on_its_own_tool_turn_and_fetches_back() {
    let mut scratch = scratch_claude("att-read", &read_a_picture());
    let turns = scratch.turns();

    assert_eq!(md_texts(&turns), vec!["[image · png]"]);
    let tool = tool_blocks(&turns);
    assert!(
        matches!(tool[0], Block::Tool { state, .. } if *state == kampr_journal::ToolState::Done),
        "the picture must not cost the tool its result: {tool:?}"
    );
    let att = only(&turns);
    let got = attach::fetch(&scratch.journals, &att.id, &scratch.transcript).expect("the bytes");
    assert_eq!(got.data, decoded(PNG));
}

/// Every `input_image` measured on this machine is a `view_image` output rather than a paste
/// (probe #247), and this is that record.
#[test]
fn a_codex_view_image_output_is_named_and_fetches_back() {
    let mut scratch = scratch_codex("att-codex", &codex_view_image(PNG));
    let turns = scratch.turns();

    assert_eq!(md_texts(&turns), vec!["[image · png]"]);
    let att = only(&turns);
    assert_eq!(att.mime.as_deref(), Some("image/png"));
    let got = attach::fetch(&scratch.journals, &att.id, &scratch.transcript).expect("the bytes");
    assert_eq!(got.data, decoded(PNG));
}

/// An id names an attachment by its ordinal within its record, so the walk that mints the headers
/// and the walk that reads the bytes have to agree on the order. Two different images in one
/// record is the cheapest way to catch them disagreeing.
#[test]
fn each_image_in_one_record_fetches_its_own_bytes() {
    let mut scratch = scratch_claude(
        "att-two",
        &[pasted("two", &[("image/png", PNG), ("image/gif", GIF)])],
    );
    let turns = scratch.turns();
    let found = attachments(&turns);

    assert_eq!(found.len(), 2);
    assert_eq!(found[0].mime.as_deref(), Some("image/png"));
    assert_eq!(found[1].mime.as_deref(), Some("image/gif"));
    let first = attach::fetch(&scratch.journals, &found[0].id, &scratch.transcript).expect("the png");
    let second = attach::fetch(&scratch.journals, &found[1].id, &scratch.transcript).expect("the gif");
    assert_eq!(first.data, decoded(PNG));
    assert_eq!(second.data, decoded(GIF));
    assert_ne!(first.data, second.data);
}

fn locator_for(scratch: &mut Scratch, agent: &str) -> Locator {
    let turns = scratch.turns();
    let att = only(&turns);
    let locator = Locator::decode(&att.id).expect("our own id decodes");
    assert_eq!(locator.agent, agent);
    locator
}

fn refuse(scratch: &Scratch, locator: &Locator) -> JournalError {
    attach::fetch(&scratch.journals, &locator.encode(), &scratch.transcript)
        .expect_err("this id must not resolve")
}

#[test]
fn an_id_naming_an_absolute_path_outside_the_root_is_refused() {
    let mut scratch = scratch_claude("att-abs", &[pasted("look", &[("image/png", PNG)])]);
    let mut locator = locator_for(&mut scratch, "claude");
    locator.path = "/etc/passwd".into();

    assert!(matches!(refuse(&scratch, &locator), JournalError::Escape(_)));
}

#[test]
fn an_id_climbing_out_of_the_root_is_refused() {
    let mut scratch = scratch_claude("att-climb", &[pasted("look", &[("image/png", PNG)])]);
    let outside = scratch.root.parent().expect("a parent").join("outside.jsonl");
    std::fs::write(&outside, "{}\n").expect("a file outside the root");
    let mut locator = locator_for(&mut scratch, "claude");
    locator.path = "../outside.jsonl".into();

    assert!(matches!(refuse(&scratch, &locator), JournalError::Escape(_)));
    assert!(
        outside.is_file(),
        "the escape target has to exist or this proves nothing"
    );
}

#[test]
fn an_id_following_a_symlink_out_of_the_root_is_refused() {
    let mut scratch = scratch_claude("att-symlink", &[pasted("look", &[("image/png", PNG)])]);
    let outside = scratch.root.parent().expect("a parent").join("secret.jsonl");
    std::fs::write(&outside, "{}\n").expect("a file outside the root");
    std::os::unix::fs::symlink(&outside, scratch.root.join("link.jsonl")).expect("a symlink");
    let mut locator = locator_for(&mut scratch, "claude");
    locator.path = "link.jsonl".into();

    assert!(matches!(refuse(&scratch, &locator), JournalError::Escape(_)));
}

/// Inside the root, readable, and still not this pane's: containment alone would hand it over.
#[test]
fn an_id_for_another_panes_transcript_is_refused() {
    let mut mine = scratch_claude("att-mine", &[pasted("mine", &[("image/png", PNG)])]);
    let theirs = mine.root.join("projects/-home-u-secret/session.jsonl");
    std::fs::create_dir_all(theirs.parent().expect("a directory")).expect("a directory");
    std::fs::write(
        &theirs,
        pasted("theirs", &[("image/png", GIF)]).to_string() + "\n",
    )
    .expect("a second transcript");
    let mut locator = locator_for(&mut mine, "claude");
    locator.path = "projects/-home-u-secret/session.jsonl".into();

    assert!(matches!(refuse(&mine, &locator), JournalError::Escape(_)));
}

#[test]
fn an_id_for_a_harness_this_node_does_not_serve_is_refused() {
    let mut scratch = scratch_claude("att-agent", &[pasted("look", &[("image/png", PNG)])]);
    let mut locator = locator_for(&mut scratch, "claude");
    locator.agent = "codex".into();

    assert!(matches!(refuse(&scratch, &locator), JournalError::NotFound(_)));
}

#[test]
fn a_forged_id_is_refused_rather_than_read() {
    let scratch = scratch_claude("att-forged", &[pasted("look", &[("image/png", PNG)])]);
    for forged in [
        "",
        "..",
        "/etc/passwd",
        "not base64 at all",
        "AAAA",
        "claude\u{1f}x\u{1f}0\u{1f}0",
    ] {
        assert!(
            attach::fetch(&scratch.journals, forged, &scratch.transcript).is_err(),
            "{forged:?} must not resolve"
        );
    }
}

#[test]
fn an_index_past_the_end_of_its_record_is_refused() {
    let mut scratch = scratch_claude("att-index", &[pasted("look", &[("image/png", PNG)])]);
    let mut locator = locator_for(&mut scratch, "claude");
    locator.index = 7;

    assert!(matches!(refuse(&scratch, &locator), JournalError::NotFound(_)));
}

#[test]
fn an_offset_that_is_not_where_a_record_starts_is_refused() {
    let mut scratch = scratch_claude("att-offset", &[pasted("look", &[("image/png", PNG)])]);
    let mut locator = locator_for(&mut scratch, "claude");
    locator.offset += 40;

    assert!(refuse(&scratch, &locator).to_string().contains("no transcript"));
}

/// The ceiling is read off the record's own base64 and applied *before* anything is allocated, so
/// a record claiming more than the node will serve costs a comparison rather than the memory.
#[test]
fn a_body_past_the_ceiling_is_refused_rather_than_allocated() {
    let over = "A".repeat((MAX_BYTES as usize + 1).div_ceil(3) * 4);
    let att = Att {
        kind: "image",
        mime: Some("image/png"),
        name: None,
        data: &over,
    };

    assert!(matches!(att.fetch(), Err(JournalError::TooLarge(n)) if n > MAX_BYTES));
}

#[test]
fn an_attachment_that_is_exactly_at_the_ceiling_is_still_served() {
    let core = (MAX_BYTES as usize * 4).div_ceil(3);
    let at = "A".repeat(core) + &"=".repeat((4 - core % 4) % 4);
    let att = Att {
        kind: "image",
        mime: Some("image/png"),
        name: None,
        data: &at,
    };

    assert_eq!(att.fetch().expect("the bytes").data.len() as u64, MAX_BYTES);
}

#[test]
fn a_registry_with_no_adapters_resolves_nothing() {
    let scratch = scratch_claude("att-empty", &[pasted("look", &[("image/png", PNG)])]);
    let empty = Registry::new();
    let turns_id = {
        let mut scratch = scratch_claude("att-empty-2", &[pasted("look", &[("image/png", PNG)])]);
        let turns = scratch.turns();
        only(&turns).id.clone()
    };

    assert!(attach::fetch(&empty, &turns_id, &scratch.transcript).is_err());
}

/// Containment is the **second** line, and this is the case where it is the only one.
///
/// Everywhere else the two checks overlap: a path outside the root is also not the transcript the
/// node says the pane is on, so equality alone refuses it. Hand this function a transcript that
/// is itself outside the root — which nothing does today, and which one wrong turn in the node
/// would — and equality is satisfied by the escape. The root is what still says no.
#[test]
fn a_path_outside_the_root_is_refused_even_when_it_is_the_transcript_asked_for() {
    let mut scratch = scratch_claude("att-outside", &[pasted("look", &[("image/png", PNG)])]);
    let locator = locator_for(&mut scratch, "claude");
    let outside = scratch.root.parent().expect("a parent").join("elsewhere.jsonl");
    std::fs::copy(&scratch.transcript, &outside).expect("the same record, outside the root");
    let mut escaped = locator.clone();
    escaped.path = outside.to_string_lossy().into_owned();

    let refusal = attach::fetch(&scratch.journals, &escaped.encode(), &outside)
        .expect_err("a file outside the transcript root is not this node's to serve");
    assert!(matches!(refusal, JournalError::Escape(_)), "{refusal:?}");
    assert!(
        attach::fetch(&scratch.journals, &locator.encode(), &scratch.transcript).is_ok(),
        "the same record inside the root still resolves, so this proves the root and not the record"
    );
}

/// A transcript is a file that grows and, once in a while, is rewritten. An offset into one that
/// has moved under an id points at a record the id was never minted from — and answering that
/// with whatever is there is the shape of the most expensive bug this project has had: a wrong
/// answer that looks exactly like a right one.
#[test]
fn an_id_whose_record_no_longer_holds_what_it_named_is_refused() {
    let mut scratch = scratch_claude("att-moved", &[pasted("look", &[("image/png", PNG)])]);
    let locator = locator_for(&mut scratch, "claude");
    assert_eq!(locator.bytes, 70);

    let mut wrong = locator.clone();
    wrong.bytes = 71;
    assert!(matches!(refuse(&scratch, &wrong), JournalError::NotFound(_)));

    std::fs::write(
        &scratch.transcript,
        pasted("look", &[("image/gif", GIF)]).to_string() + "\n",
    )
    .expect("the transcript rewritten under the id");
    assert!(
        matches!(refuse(&scratch, &locator), JournalError::NotFound(_)),
        "a record that has been replaced must not answer with what took its place"
    );
}
