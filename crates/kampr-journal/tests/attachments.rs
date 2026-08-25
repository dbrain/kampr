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

fn markers(turns: &[Turn]) -> Vec<(&str, bool)> {
    turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Md { text, att } => Some((text.as_str(), att.is_some())),
            _ => None,
        })
        .collect()
}

/// The walk that mints the headers skips an image block with nothing behind it, so the walk that
/// hangs them on markers has to skip it too. A marker that takes a locator it was never minted
/// one for is off by one for the rest of its record — and the picture it then shows is another
/// picture, correctly fetched, which is exactly the shape of a wrong answer that looks right.
#[test]
fn an_image_with_no_data_does_not_take_the_next_ones_locator() {
    let record = json!({
        "type": "user",
        "uuid": "0f2b1a44-1f2f-4a10-9b6a-9a1f0f2b1a44",
        "timestamp": "2026-08-20T02:56:27.681Z",
        "message": { "role": "user", "content": [
            { "type": "image", "source": { "type": "base64", "media_type": "image/png" } },
            { "type": "image", "source": { "type": "base64", "media_type": "image/gif", "data": GIF } }
        ] }
    });
    let mut scratch = scratch_claude("att-dataless", &[record]);
    let turns = scratch.turns();

    assert_eq!(
        markers(&turns),
        vec![("[image · png]", false), ("[image · gif]", true)]
    );
    let att = only(&turns);
    assert_eq!(att.mime.as_deref(), Some("image/gif"));
    let got = attach::fetch(&scratch.journals, &att.id, &scratch.transcript).expect("the gif");
    assert_eq!(got.data, decoded(GIF));
}

/// The same disagreement in the other adapter: an `input_image` whose url is not a base64 data
/// url mints no header, so it must not consume one either.
#[test]
fn a_codex_image_with_no_bytes_behind_it_does_not_take_the_next_ones_locator() {
    let record = json!({
        "type": "response_item",
        "timestamp": "2026-08-18T14:11:36.000Z",
        "payload": { "type": "message", "role": "user", "content": [
            { "type": "input_image", "image_url": "https://example.invalid/shot.png" },
            { "type": "input_image", "image_url": format!("data:image/gif;base64,{GIF}") }
        ] }
    });
    let mut scratch = scratch_codex("att-codex-dataless", &[record]);
    let turns = scratch.turns();

    assert_eq!(markers(&turns), vec![("[image]", false), ("[image · gif]", true)]);
    let att = only(&turns);
    assert_eq!(att.mime.as_deref(), Some("image/gif"));
    let got = attach::fetch(&scratch.journals, &att.id, &scratch.transcript).expect("the gif");
    assert_eq!(got.data, decoded(GIF));
}

/// Two forms of id share one decoder, so the *old* one has to be pinned by something other than
/// the encoder that mints it. This is a literal an installed client is holding right now.
#[test]
fn an_id_minted_before_there_was_a_second_form_still_decodes_as_a_record() {
    const MINTED: &str = "Y2xhdWRlH3Byb2plY3RzLy1ob21lLXUtZGVtby9zZXNzaW9uLmpzb25sHzAfMB83MA";
    let locator = Locator::decode(MINTED).expect("an id this node has already handed out");

    assert_eq!(locator.agent, "claude");
    assert_eq!(locator.path, "projects/-home-u-demo/session.jsonl");
    assert_eq!((locator.offset, locator.index, locator.bytes), (0, 0, 70));
    assert_eq!(locator.encode(), MINTED, "and it still encodes to itself");
    assert_eq!(
        attach::Source::decode(MINTED).expect("the same id through the shared decoder"),
        attach::Source::Record(locator)
    );
}

/// The whole point of the second form: a client that saw a path in a tool call can build the id
/// itself, without the node having minted anything.
#[test]
fn a_file_id_is_something_a_client_can_build_from_a_path_alone() {
    const BUILT: &str = "ZmlsZR8vdmFyL2xpYi9rYW1wci9zaG90LnBuZw";

    assert_eq!(
        attach::Source::decode(BUILT).expect("a client-built id"),
        attach::Source::File(attach::FileRef::new("/var/lib/kampr/shot.png"))
    );
    assert_eq!(attach::FileRef::new("/var/lib/kampr/shot.png").encode(), BUILT);
}

#[test]
fn a_file_id_is_not_a_record_locator_and_a_record_locator_is_not_a_file() {
    let file = attach::FileRef::new("/etc/hosts").encode();
    let record = Locator {
        agent: "claude".into(),
        path: "projects/x/session.jsonl".into(),
        offset: 0,
        index: 0,
        bytes: 70,
    };

    assert!(Locator::decode(&file).is_err());
    assert!(matches!(
        attach::Source::decode(&record.encode()),
        Ok(attach::Source::Record(_))
    ));
}

fn a_file(tag: &str, name: &str, bytes: &[u8]) -> (ScratchDir, std::path::PathBuf) {
    let dir = scratch_dir(tag);
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("a file");
    (dir, path)
}

#[test]
fn a_path_on_this_machine_reads_back_byte_for_byte_with_no_transcript_at_all() {
    let (_dir, path) = a_file("file-png", "shot.png", &decoded(PNG));

    let got = fetch(&path).expect("the bytes");
    assert_eq!(got.data, decoded(PNG));
    assert_eq!(got.kind, attach::IMAGE);
    assert_eq!(got.mime.as_deref(), Some("image/png"));
    assert_eq!(got.name.as_deref(), Some("shot.png"));
}

/// The extension is the only thing there is to go on, and a file that is not an image must not
/// claim to be one — a client that believes `kind` renders a broken picture instead of offering
/// the download the block is for.
#[test]
fn a_files_kind_and_type_come_off_its_extension_and_nowhere_else() {
    let cases: &[(&str, &str, Option<&str>)] = &[
        ("shot.png", attach::IMAGE, Some("image/png")),
        ("shot.JPEG", attach::IMAGE, Some("image/jpeg")),
        ("shot.gif", attach::IMAGE, Some("image/gif")),
        ("notes.txt", attach::FILE, None),
        ("page.svg", attach::FILE, None),
        ("page.html", attach::FILE, None),
        ("noextension", attach::FILE, None),
    ];
    for (name, kind, mime) in cases {
        let (_dir, path) = a_file("file-kind", name, b"some bytes");
        let got = fetch(&path).expect(name);
        assert_eq!(got.kind, *kind, "{name}");
        assert_eq!(got.mime.as_deref(), *mime, "{name}");
    }
}

/// There is no cwd on the node side of this — a relative path would be resolved against whatever
/// directory the node happens to have been started in, which is not a thing any caller knows.
#[test]
fn a_relative_path_is_refused_rather_than_resolved_against_something() {
    // In the process's own working directory, so a build that dropped the check would find it and
    // this would go green with the defect restored.
    let relative = format!("kampr-cwd-{}.png", std::process::id());
    std::fs::write(&relative, decoded(PNG)).expect("a file in the process's cwd");
    let found_by_the_cwd = std::fs::read(&relative).is_ok();
    let refusal = fetch(&relative);
    std::fs::remove_file(&relative).expect("the file back out of the crate directory");

    assert!(
        found_by_the_cwd,
        "{relative} was not readable, so this proves nothing"
    );
    assert!(matches!(refusal, Err(JournalError::NotFound(_))), "{refusal:?}");
}

#[test]
fn a_directory_a_missing_path_and_an_empty_file_are_all_the_same_refusal() {
    let (dir, path) = a_file("file-empty", "empty", b"");
    for candidate in [dir.to_path_buf(), dir.join("nothing"), path] {
        assert!(
            matches!(fetch(&candidate), Err(JournalError::NotFound(_))),
            "{} must not resolve",
            candidate.display()
        );
    }
}

#[test]
fn a_file_the_nodes_user_cannot_read_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let (_dir, path) = a_file("file-secret", "secret", b"a private key");
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o000)).expect("no permissions");
    // Root ignores the mode, and a node running as root is meant to read it — there is nothing
    // to assert on that machine rather than a weaker thing to assert.
    if std::fs::read(&path).is_ok() {
        eprintln!("skipping: this user reads a 0o000 file, so there is no refusal to measure");
        return;
    }

    assert!(matches!(fetch(&path), Err(JournalError::NotFound(_))));
}

/// The ceiling is decided from what `stat` says, before a byte is read — a 9 MiB file costs a
/// comparison here exactly as a 9 MiB record does.
#[test]
fn a_file_past_the_ceiling_is_refused_on_its_size_rather_than_read() {
    let dir = scratch_dir("file-huge");
    let path = dir.join("huge.png");
    let file = std::fs::File::create(&path).expect("a file");
    file.set_len(MAX_BYTES + 1)
        .expect("a sparse file past the ceiling");
    drop(file);

    assert!(matches!(
        fetch(&path),
        Err(JournalError::TooLarge(n)) if n == MAX_BYTES + 1
    ));
}

#[test]
fn a_file_exactly_at_the_ceiling_is_still_served() {
    let dir = scratch_dir("file-exact");
    let path = dir.join("exact.png");
    let file = std::fs::File::create(&path).expect("a file");
    file.set_len(MAX_BYTES).expect("a sparse file at the ceiling");
    drop(file);

    assert_eq!(fetch(&path).expect("the bytes").data.len() as u64, MAX_BYTES);
}

/// A fifo is why the size is read with `stat` rather than by opening the path first: opening one
/// with no writer on the other end blocks until there is one, and a request that never comes back
/// is worse than a refusal. Timed, because a build that got this wrong would hang the suite rather
/// than fail it.
#[test]
fn a_fifo_is_refused_without_waiting_for_a_writer() {
    let dir = scratch_dir("file-fifo");
    let path = dir.join("pipe");
    let made = std::process::Command::new("mkfifo")
        .arg(&path)
        .status()
        .expect("mkfifo");
    assert!(made.success(), "mkfifo failed");

    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(fetch(&path).is_err());
    });

    match rx.recv_timeout(std::time::Duration::from_secs(5)) {
        Ok(refused) => assert!(refused, "a fifo must not be served"),
        Err(_) => panic!("the fetch blocked on a fifo with no writer"),
    }
}

/// A home no test has anything under, so nothing below resolves a `~` by accident.
const NO_HOME: &str = "/kampr-no-such-home";

fn fetch(path: impl Into<std::path::PathBuf>) -> Result<kampr_journal::Fetched, JournalError> {
    attach::FileRef::new(path).fetch(std::path::Path::new(NO_HOME))
}

fn fetch_from(home: &std::path::Path, path: &str) -> Result<kampr_journal::Fetched, JournalError> {
    attach::FileRef::new(path).fetch(home)
}

fn a_home(tag: &str, files: &[(&str, &[u8])]) -> ScratchDir {
    let home = scratch_dir(tag);
    for (name, bytes) in files {
        let path = home.join(name);
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("a directory");
        std::fs::write(&path, bytes).expect("a file under the home");
    }
    home
}

/// The whole of why this form is usable. Agents write `~/screenshot.png` and `~/dev/x/plot.png`
/// constantly, and those are the paths a person taps.
#[test]
fn a_leading_tilde_resolves_against_the_nodes_own_home() {
    let home = a_home(
        "tilde",
        &[("shot.png", &decoded(PNG)), ("dev/deep/plot.png", &decoded(PNG))],
    );

    for asked in ["~/shot.png", "~/dev/deep/plot.png"] {
        let got = fetch_from(&home, asked).unwrap_or_else(|e| panic!("{asked}: {e}"));
        assert_eq!(got.data, decoded(PNG), "{asked}");
        assert_eq!(got.mime.as_deref(), Some("image/png"), "{asked}");
    }
    assert!(
        fetch(std::path::PathBuf::from("~/shot.png")).is_err(),
        "the same id against a home with nothing in it must not resolve"
    );
}

/// A home that is itself a file, which is the only way to watch a **bare** `~` expand without the
/// answer being the directory refusal either way.
#[test]
fn a_bare_tilde_is_the_home_itself() {
    let dir = scratch_dir("tilde-bare");
    let home = dir.join("home.png");
    std::fs::write(&home, decoded(PNG)).expect("a home that is a file");

    assert_eq!(fetch_from(&home, "~").expect("the bytes").data, decoded(PNG));
}

/// A `~` anywhere but the front is an ordinary character in a filename, and a build that treated
/// it as anything else would refuse a file that is right there.
#[test]
fn a_tilde_that_is_not_the_first_character_is_an_ordinary_one() {
    let (dir, path) = a_file("tilde-mid", "a~b.png", &decoded(PNG));
    let leading = dir.join("~leading.png");
    std::fs::write(&leading, decoded(PNG)).expect("a file whose name starts with a tilde");
    // A `~/` in the *middle* of the path, which is the case a prefix check passes and a search
    // does not: a directory whose name ends in a tilde.
    let mid = dir.join("a~/shot.png");
    std::fs::create_dir_all(mid.parent().expect("a directory")).expect("a directory");
    std::fs::write(&mid, decoded(PNG)).expect("a file under a directory named with a tilde");

    for absolute in [&path, &leading, &mid] {
        assert_eq!(
            fetch(absolute)
                .unwrap_or_else(|e| panic!("{}: {e}", absolute.display()))
                .data,
            decoded(PNG),
            "{}",
            absolute.display()
        );
    }
}

/// `~user/x` is another account's home, and guessing at one would hand over a different user's
/// files under a gate that reasoned about this one. It is refused, not expanded.
#[test]
fn another_users_home_is_refused_rather_than_guessed_at() {
    // Both of the shapes a wrong expansion would produce are real files here, so a refusal below
    // is the rule and not the filesystem.
    let home = a_home(
        "tilde-user",
        &[("root/shot.png", &decoded(PNG)), ("shot.png", &decoded(PNG))],
    );

    for asked in ["~root/shot.png", "~someone/shot.png", "~root", "~/../shot.png~x"] {
        assert!(
            fetch_from(&home, asked).is_err(),
            "{asked} must not resolve to another account's home"
        );
    }
    assert!(
        fetch_from(&home, "~/root/shot.png").is_ok(),
        "the same file under this home still resolves, so the refusals above are the rule"
    );
}

/// The separators after `~` belong to the prefix, not to a new root. `Path::join` with an
/// absolute argument *replaces*, so without this `~//etc/hosts` would be `/etc/hosts`.
#[test]
fn the_slashes_after_a_tilde_do_not_start_a_new_root() {
    let home = a_home("tilde-slash", &[("etc/hosts", b"a home's own hosts file")]);

    let got = fetch_from(&home, "~//etc/hosts").expect("the home's file");
    assert_eq!(got.data, b"a home's own hosts file");
}

/// `$HOME` unset — the node's `journal_home()` answers with an empty path — must fail closed
/// rather than resolve `~/x` to the relative `x`.
#[test]
fn a_tilde_with_no_home_behind_it_resolves_to_nothing() {
    // An empty home makes `~/x` the relative `x`, so the file goes where a relative `x` would be
    // found — the process's own directory — and a build that let one through would serve it.
    let name = format!("kampr-nohome-{}.png", std::process::id());
    std::fs::write(&name, decoded(PNG)).expect("a file in the process's cwd");
    let found_by_the_cwd = std::fs::read(&name).is_ok();
    let refusals = [
        fetch_from(std::path::Path::new(""), &format!("~/{name}")),
        fetch_from(std::path::Path::new(""), "~"),
    ];
    std::fs::remove_file(&name).expect("the file back out of the crate directory");

    assert!(
        found_by_the_cwd,
        "{name} was not readable, so this proves nothing"
    );
    for refusal in refusals {
        assert!(matches!(refusal, Err(JournalError::NotFound(_))), "{refusal:?}");
    }
}

/// Expansion must not be the thing that turns a relative path absolute.
#[test]
fn nothing_but_a_leading_tilde_is_expanded() {
    let home = a_home("tilde-relative", &[("shot.png", &decoded(PNG))]);

    for asked in ["shot.png", "./shot.png", "../shot.png", "dev/shot.png", ""] {
        assert!(
            fetch_from(&home, asked).is_err(),
            "{asked:?} must not be resolved against the home"
        );
    }
}
