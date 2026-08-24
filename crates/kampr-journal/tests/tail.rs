//! What the byte offsets a transcript is read at are worth.
//!
//! `line_start` is not bookkeeping: it is the offset half of every attachment id, so a loop that
//! miscounts one line's terminator hands the next marker a record that no longer parses — or,
//! worse, one that does.

mod common;

use std::io::Write;

use common::*;
use kampr_journal::{Attachment, Block, Locator, Turn, attach};
use serde_json::json;

/// A 1×1 PNG, and a 1×1 GIF, so two records' attachments cannot be told apart by luck.
const PNG: &str =
    "iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAYAAAAfFcSJAAAADUlEQVR42mP8z8BQDwAEhQGAhKmMIQAAAABJRU5ErkJggg==";
const GIF: &str = "R0lGODlhAQABAIAAAAAAAP///yH5BAEAAAAALAAAAAABAAEAAAIBRAA7";

fn pasted(uuid: &str, mime: &str, data: &str) -> String {
    json!({
        "type": "user",
        "uuid": uuid,
        "timestamp": "2026-08-20T02:56:27.681Z",
        "message": { "role": "user", "content": [
            { "type": "image", "source": { "type": "base64", "media_type": mime, "data": data } }
        ] }
    })
    .to_string()
}

fn said(uuid: &str, text: &str) -> String {
    json!({
        "type": "assistant",
        "uuid": uuid,
        "timestamp": "2026-08-20T02:56:29.000Z",
        "message": { "role": "assistant", "content": [ { "type": "text", "text": text } ] }
    })
    .to_string()
}

fn skipped() -> String {
    json!({ "type": "summary", "summary": "a record this parser drops" }).to_string()
}

fn ids(turns: &[Turn]) -> Vec<&str> {
    turns.iter().map(|t| t.id.as_str()).collect()
}

fn only(turns: &[Turn]) -> &Attachment {
    let found: Vec<&Attachment> = turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Md { att: Some(att), .. } => Some(att),
            _ => None,
        })
        .collect();
    assert_eq!(found.len(), 1, "expected exactly one attachment in {turns:?}");
    found[0]
}

fn decoded(b64: &str) -> Vec<u8> {
    use base64::Engine;
    base64::engine::general_purpose::STANDARD
        .decode(b64)
        .expect("base64")
}

fn append(path: &std::path::Path, bytes: &[u8]) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(bytes).unwrap();
    f.flush().unwrap();
}

/// Every terminator a transcript has been seen to carry, in one file, with an attachment on the
/// record after each of them. A locator resolves by seeking to its offset and reading a line, so
/// a fetch that comes back with the right bytes is the offset proving itself against the file.
#[test]
fn a_transcript_of_mixed_line_endings_locates_every_record_at_its_own_byte() {
    let (skip, png, text, gif) = (
        skipped(),
        pasted("u1", "image/png", PNG),
        said("a1", "on its way"),
        pasted("u2", "image/gif", GIF),
    );
    let body = format!("{skip}\r\n\n{png}\r\n{text}\n{gif}");
    let png_at = (skip.len() + 2 + 1) as u64;
    let gif_at = png_at + (png.len() + 2 + text.len() + 1) as u64;

    let mut scratch = scratch_claude_body("tail-endings", &body);
    let turns = scratch.turns();

    assert_eq!(
        ids(&turns),
        ["u1", "a1"],
        "the unterminated tail is not a record yet"
    );
    assert_eq!(md_texts(&turns), ["[image · png]", "on its way"]);
    let att = only(&turns);
    assert_eq!(Locator::decode(&att.id).expect("our own id").offset, png_at);
    let got = attach::fetch(&scratch.journals, &att.id, &scratch.transcript).expect("the png");
    assert_eq!(got.data, decoded(PNG));

    append(&scratch.transcript, b"\n");
    let after = scratch.turns();

    assert_eq!(ids(&after), ["u2"]);
    let att = only(&after);
    assert_eq!(
        Locator::decode(&att.id).expect("our own id").offset,
        gif_at,
        "a record that arrived torn still starts where it started"
    );
    let got = attach::fetch(&scratch.journals, &att.id, &scratch.transcript).expect("the gif");
    assert_eq!(got.data, decoded(GIF));
}

/// How long one poll of a 40 MB transcript is allowed to take.
///
/// The loop this replaces drained its buffer one line at a time, which memmoves the whole
/// remainder down per line: 40 MB across 20 000 lines measured **16.8 s**, against 15.7 ms for a
/// single pass, and the largest rollout on a real machine is 88.7 MB (#247). The budget is two
/// orders of magnitude above the single pass and two below the drain, so machine load decides
/// nothing here.
const BUDGET: std::time::Duration = std::time::Duration::from_secs(3);

const LINES: usize = 20_000;
const WIDTH: usize = 2_000;

/// A poll runs on a tokio worker holding the lock `convo.load` also takes, so its cost is a stall
/// on every socket that worker is carrying — and the first poll after a transcript opens reads
/// all of it.
#[test]
fn a_transcript_of_forty_megabytes_is_read_in_one_pass_rather_than_once_per_line() {
    let mut body = String::with_capacity(LINES * (WIDTH + 1));
    for _ in 0..LINES {
        body.push_str(&"x".repeat(WIDTH));
        body.push('\n');
    }
    let at = body.len() as u64;
    body.push_str(&pasted("u1", "image/gif", GIF));
    body.push('\n');

    let mut scratch = scratch_claude_body("tail-large", &body);
    let start = std::time::Instant::now();
    let turns = scratch.turns();
    let took = start.elapsed();

    assert_eq!(ids(&turns), ["u1"]);
    let att = only(&turns);
    assert_eq!(
        Locator::decode(&att.id).expect("our own id").offset,
        at,
        "40 MB of lines the parser drops still move the offset of the one it keeps"
    );
    let got = attach::fetch(&scratch.journals, &att.id, &scratch.transcript).expect("the gif");
    assert_eq!(got.data, decoded(GIF));
    assert!(took < BUDGET, "one poll of {at} bytes took {took:?}");
}

/// The tail is a torn line until its newline arrives, however many polls that takes, and the
/// bytes it is holding must not be read twice.
#[test]
fn a_record_torn_across_three_polls_is_parsed_once_and_at_its_own_offset() {
    let head = said("a1", "first");
    let tail = pasted("u1", "image/gif", GIF);
    let at = (head.len() + 1) as u64;
    let (a, rest) = tail.split_at(tail.len() / 3);
    let (b, c) = rest.split_at(rest.len() / 2);

    let mut scratch = scratch_claude_body("tail-torn", &format!("{head}\n"));
    assert_eq!(ids(&scratch.turns()), ["a1"]);

    for piece in [a, b, c] {
        append(&scratch.transcript, piece.as_bytes());
        assert!(scratch.turns().is_empty(), "a torn line must not be parsed");
    }

    append(&scratch.transcript, b"\n");
    let turns = scratch.turns();

    assert_eq!(ids(&turns), ["u1"]);
    assert_eq!(Locator::decode(&only(&turns).id).expect("our own id").offset, at);
    assert!(scratch.turns().is_empty(), "and it is not read a second time");
}

/// A parser has no `Value` to fall back on, so an id is minted at the offset of the record it was
/// read from and nothing else. Two identical records prove the second is not handed the first's.
#[test]
fn two_identical_records_are_located_at_two_different_offsets() {
    let record = pasted("u1", "image/gif", GIF);
    let second = pasted("u2", "image/gif", GIF);
    let mut scratch = scratch_claude_body("tail-twice", &format!("{record}\n{second}\n"));
    let turns = scratch.turns();

    let found: Vec<u64> = turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Md { att: Some(att), .. } => Some(Locator::decode(&att.id).expect("our own id").offset),
            _ => None,
        })
        .collect();

    assert_eq!(found, [0, (record.len() + 1) as u64]);
}

/// Nothing above is worth anything if the fixture the rest of the suite reads parses differently
/// through this loop than it did through the last one.
#[test]
fn the_fixture_transcript_parses_the_same_records_whole_as_it_does_a_byte_at_a_time() {
    let source = std::fs::read_to_string(claude_transcript()).expect("the fixture");
    let mut whole = scratch_claude_body("tail-whole", &source);
    let expected = whole.turns();
    assert_eq!(expected.len(), 5);

    let mut piecemeal = scratch_claude_body("tail-piecemeal", "");
    let mut seen: Vec<Turn> = Vec::new();
    for byte in source.as_bytes() {
        append(&piecemeal.transcript, &[*byte]);
        for turn in piecemeal.turns() {
            match seen.iter_mut().find(|t| t.id == turn.id) {
                Some(held) => *held = turn,
                None => seen.push(turn),
            }
        }
    }

    assert_eq!(seen, expected);
}

/// A JSON record is one line of a `.jsonl`, but nothing stops the bytes inside it from being a
/// value the parser cannot use — and the empty line between two records must not become one.
#[test]
fn blank_lines_are_not_records_and_do_not_move_what_follows_them() {
    let record = pasted("u1", "image/gif", GIF);
    let body = format!("\n\n\r\n{record}\n\n");
    let mut scratch = scratch_claude_body("tail-blank", &body);
    let turns = scratch.turns();

    assert_eq!(ids(&turns), ["u1"]);
    assert_eq!(Locator::decode(&only(&turns).id).expect("our own id").offset, 4);
    let got = attach::fetch(&scratch.journals, &only(&turns).id, &scratch.transcript).expect("the gif");
    assert_eq!(got.data, decoded(GIF));
}
