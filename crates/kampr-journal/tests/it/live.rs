//! The live turn, driven from screens captured off real harnesses.
//!
//! Every fixture under `tests/fixtures/screens` is a verbatim `pane.read visible` of a live
//! `claude` 2.1.239 or `codex` 0.149 in a headless herdr, and the two transcripts under
//! `tests/fixtures/live` are what those same runs wrote. Nothing here is hand-written, because a
//! hand-written screen would only ever agree with the parser that reads it.

use crate::common;

use kampr_journal::{AgyAdapter, ClaudeAdapter, CodexAdapter, JournalAdapter, LIVE_ID, TranscriptRoot};
use kampr_journal::{Block, Journal, Turn};
use std::path::PathBuf;

fn screen(name: &str) -> String {
    let path = common::fixtures().join("screens").join(format!("{name}.txt"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

fn lines(text: &str) -> Vec<&str> {
    text.lines().collect()
}

fn transcript(name: &str) -> PathBuf {
    common::fixtures().join("live").join(format!("{name}.jsonl"))
}

/// A journal open on a real transcript, read up to `records` lines of it — which is how a turn
/// half-way through a conversation is reproduced without inventing one.
fn upto(adapter: &dyn JournalAdapter, name: &str, records: usize) -> Box<dyn Journal> {
    upto_path(adapter, &transcript(name), records)
}

fn upto_path(adapter: &dyn JournalAdapter, from: &std::path::Path, records: usize) -> Box<dyn Journal> {
    let whole = std::fs::read_to_string(from).expect("transcript");
    let head: String = whole.lines().take(records).map(|l| format!("{l}\n")).collect();
    let scratch = common::scratch_dir("live");
    let path = scratch.join("t.jsonl");
    std::fs::write(&path, head).expect("write");
    let mut journal = adapter.open_path(path);
    let _ = journal.poll().expect("poll");
    journal
}

fn claude() -> ClaudeAdapter {
    ClaudeAdapter::new(TranscriptRoot::new(common::claude_root()).expect("root"))
}

fn codex() -> CodexAdapter {
    CodexAdapter::new(TranscriptRoot::new(common::codex_root()).expect("root"))
}

fn agy() -> AgyAdapter {
    AgyAdapter::new(TranscriptRoot::new(common::agy_root()).expect("root"))
}

/// `agy` keeps its transcript under the conversation rather than in a directory of its own, so
/// the same run's screens and records are reached from the adapter's own fixture root.
fn agy_upto(records: usize) -> Box<dyn Journal> {
    upto_path(&agy(), &common::agy_transcript(), records)
}

fn md(turn: &Turn) -> &str {
    match turn.blocks.first() {
        Some(Block::Md { text, .. }) => text,
        other => panic!("expected one md block, got {other:?}"),
    }
}

/// The whole point: text that is on the screen and *not yet* in the transcript is published.
#[test]
fn a_message_claude_is_still_writing_is_previewed() {
    let text = screen("claude-streaming");
    // Twelve records in: the operator's prompt has landed and no assistant record has.
    let journal = upto(&claude(), "claude-notes", 12);
    let turn = journal.preview(&lines(&text)).expect("a preview");
    assert_eq!(turn.id, LIVE_ID);
    assert_eq!(turn.role, kampr_journal::Role::Assistant);
    let body = md(&turn);
    assert!(
        body.starts_with("A terminal emulator sits between a program writing bytes"),
        "the message starts at its own first word, not mid-wrap: {body:?}"
    );
    assert!(
        body.contains("Printable text goes through width resolution before it lands."),
        "the whole visible message is carried, not just its first line: {body:?}"
    );
    assert!(
        !body.contains("Without using any tools"),
        "the operator's own prompt is a different turn: {body:?}"
    );
    assert!(
        !body.contains("accept edits on"),
        "the harness's footer is not part of the answer: {body:?}"
    );
    assert!(
        !body.contains('❯') && !body.contains('●'),
        "the markers are layout, not text: {body:?}"
    );
    assert!(
        body.lines().all(|l| !l.starts_with("  ")),
        "the wrap indent is stripped so the client can re-wrap it: {body:?}"
    );
}

/// The yield. The same screen against a transcript that has caught up publishes nothing, so a
/// finished message is never rendered twice — once as a preview and once as its own record.
#[test]
fn a_message_the_transcript_has_caught_up_with_is_not_previewed() {
    let text = screen("claude-recorded");
    let lines = lines(&text);
    let adapter = claude();

    let before = upto(&adapter, "claude-notes", 12);
    let published = before.preview(&lines).expect("a preview before the record lands");
    assert_eq!(
        md(&published),
        "I'll write the file, then explain it.",
        "the screen carries the message a whole beat before the record does"
    );

    // Record 13 is that message, in markdown, unwrapped.
    let after = upto(&adapter, "claude-notes", 13);
    assert_eq!(
        after.preview(&lines),
        None,
        "the record is authoritative and the preview stands down"
    );
}

/// Rendered markdown is not markdown source, and a wrapped line is not an unwrapped one. This is
/// the pair the comparison actually has to survive: backticks, bold and an em dash on one side,
/// none of them on the other, and the whole thing re-wrapped at 93 columns.
#[test]
fn the_yield_survives_the_screen_having_rendered_the_markdown_away() {
    let text = screen("claude-streaming-after-tool");
    let lines = lines(&text);
    let adapter = claude();

    let before = upto(&adapter, "claude-notes", 16);
    let published = before.preview(&lines).expect("a preview");
    assert!(
        md(&published).starts_with("notes.md is written."),
        "the screen has already eaten the backticks around `notes.md`: {:?}",
        md(&published)
    );

    let after = upto(&adapter, "claude-notes", 17);
    let recorded = after
        .page_before(None, 40)
        .turns
        .last()
        .map(|t| md(t).to_string())
        .expect("the record");
    assert!(
        recorded.starts_with("`notes.md` is written."),
        "the transcript keeps the source: {recorded:?}"
    );
    assert!(
        recorded.contains("**Bullet one — structure.**"),
        "with its emphasis markers intact: {recorded:?}"
    );
    assert_eq!(
        after.preview(&lines),
        None,
        "and the preview still recognises its own text through all of that"
    );
}

/// A message longer than the pane loses its own header off the top of the screen. It is still
/// worth previewing — and it must still stand down, which needs a different test than the one a
/// message with a visible header gets.
#[test]
fn a_message_whose_header_has_scrolled_off_previews_and_still_stands_down() {
    let text = screen("claude-clipped");
    let lines = lines(&text);
    let adapter = claude();

    let before = upto(&adapter, "claude-notes", 16);
    let published = before.preview(&lines).expect("a clipped preview");
    assert!(
        md(&published).starts_with("Bullet one — structure."),
        "what is left of the message is what gets shown: {:?}",
        md(&published)
    );

    let after = upto(&adapter, "claude-notes", 17);
    assert_eq!(
        after.preview(&lines),
        None,
        "a record that merely contains the visible fragment is enough to retire it"
    );
}

/// A tool card is already a turn in the transcript, under its own id, with a state a preview
/// cannot know. Publishing one as prose would double it.
#[test]
fn a_tool_card_is_never_previewed() {
    let journal = upto(&claude(), "claude-notes", 12);
    assert_eq!(journal.preview(&lines(&screen("claude-tool"))), None);
}

/// The marker is painted before the first token arrives.
#[test]
fn a_message_header_with_nothing_under_it_is_not_previewed() {
    let journal = upto(&claude(), "claude-notes", 12);
    assert_eq!(journal.preview(&lines(&screen("claude-header-only"))), None);
}

#[test]
fn codex_streams_the_same_way() {
    let text = screen("codex-streaming");
    let adapter = codex();

    // Nine records in: the operator's prompt is recorded, the answer is not.
    let before = upto(&adapter, "codex-notes", 9);
    let turn = before.preview(&lines(&text)).expect("a preview");
    let body = md(&turn);
    assert_eq!(turn.id, LIVE_ID);
    assert!(
        body.starts_with("A terminal emulator receives a byte stream"),
        "{body:?}"
    );
    assert!(body.contains("- Input: Read bytes from the PTY."), "{body:?}");
    assert!(
        !body.contains("Ask Codex to do anything") && !body.contains("gpt-5.6-sol default"),
        "the composer and the footer are not the answer: {body:?}"
    );

    let after = upto(&adapter, "codex-notes", 14);
    assert_eq!(
        after.preview(&lines(&text)),
        None,
        "codex writes its message record at the end of the message, and the preview yields to it"
    );
}

/// Codex opens its own status line with the same glyph it opens a message with, which is the one
/// thing its reader has to get right.
#[test]
fn the_codex_spinner_is_not_a_message() {
    let journal = upto(&codex(), "codex-notes", 9);
    assert_eq!(journal.preview(&lines(&screen("codex-working"))), None);
}

/// While a codex tool runs, the spinner sits below its card — so the block at the foot of the
/// screen is the spinner, and nothing above it is reached.
#[test]
fn a_codex_tool_card_is_never_previewed() {
    let journal = upto(&codex(), "codex-notes", 20);
    assert_eq!(journal.preview(&lines(&screen("codex-tool"))), None);
}

#[test]
fn codex_previews_a_message_that_follows_a_tool_card() {
    let text = screen("codex-streaming-after-tool");
    let adapter = codex();
    let before = upto(&adapter, "codex-notes", 25);
    let body = before
        .preview(&lines(&text))
        .map(|t| md(&t).to_string())
        .expect("a preview");
    assert!(body.starts_with("Created notes.md."), "{body:?}");
    assert!(
        !body.contains("+- ANSI escape sequences can move the cursor"),
        "the tool card's diff preview sits above this message, not inside it: {body:?}"
    );
    let after = upto(&adapter, "codex-notes", 33);
    assert_eq!(after.preview(&lines(&text)), None);
}

/// Nineteen of herdr's twenty-two agent kinds have no adapter at all, and the three that do are
/// the three whose screens have been probed. A harness nobody has probed must serve its transcript
/// exactly as it did before live turns existed rather than guess at a layout.
#[test]
fn a_harness_with_no_probed_screen_publishes_no_preview() {
    struct Unprobed(kampr_journal::ClaudeAdapter);
    impl JournalAdapter for Unprobed {
        fn agent(&self) -> &str {
            "gemini"
        }
        fn locate(
            &self,
            session: &kampr_journal::SessionRef,
        ) -> Result<PathBuf, kampr_journal::JournalError> {
            self.0.locate(session)
        }
        fn locate_by_cwd(
            &self,
            cwd: &std::path::Path,
            since: Option<std::time::SystemTime>,
        ) -> Result<PathBuf, kampr_journal::JournalError> {
            self.0.locate_by_cwd(cwd, since)
        }
        fn parser(&self) -> Box<dyn kampr_journal::TranscriptParser> {
            self.0.parser()
        }
        fn root(&self) -> &kampr_journal::TranscriptRoot {
            self.0.root()
        }
    }
    let journal = upto(&Unprobed(claude()), "claude-notes", 12);
    assert_eq!(
        journal.preview(&lines(&screen("claude-streaming"))),
        None,
        "the default is silence, not a guess"
    );
}

/// Claude 2.1.239 opens an in-flight tool card, a background-command notice and a real answer
/// with the same glyph, and two of the three are one line that never becomes two. Growing is what
/// separates them.
#[test]
fn only_a_block_that_grows_earns_a_preview() {
    let adapter = claude();
    let journal = upto(&adapter, "claude-notes", 12);
    let mut watch = kampr_journal::Watch::default();

    for name in ["claude-inflight-tool", "claude-notice"] {
        let text = screen(name);
        let seen = journal.preview(&lines(&text));
        assert!(
            seen.is_some(),
            "{name}: the reader does find a block here — the growth rule is what refuses it"
        );
        assert_eq!(watch.observe(seen.clone()), kampr_journal::Change::Held);
        assert_eq!(
            watch.observe(seen),
            kampr_journal::Change::Held,
            "{name}: still the same line a poll later, so still not an answer"
        );
        assert!(!watch.showing());
    }

    let early = screen("claude-streaming-earlier");
    let late = screen("claude-streaming");
    let mut watch = kampr_journal::Watch::default();
    assert_eq!(
        watch.observe(journal.preview(&lines(&early))),
        kampr_journal::Change::Held,
        "the first sighting is not yet proof of a message"
    );
    let grown = watch.observe(journal.preview(&lines(&late)));
    let kampr_journal::Change::Show(turn) = grown else {
        panic!("the same message, longer, is a message: {grown:?}");
    };
    assert!(md(&turn).contains("Printable text goes through width resolution"));
    assert!(watch.showing());
}

/// The preview is withdrawn under its own id, so a client that matches by id and replaces has a
/// way to be rid of it.
#[test]
fn the_preview_is_withdrawn_when_the_transcript_takes_over() {
    let adapter = claude();
    let text = screen("claude-streaming-after-tool");
    let mut watch = kampr_journal::Watch::default();
    let before = upto(&adapter, "claude-notes", 16);
    let earlier = screen("claude-streaming-after-tool-earlier");
    watch.observe(before.preview(&lines(&earlier)));
    let shown = watch.observe(before.preview(&lines(&text)));
    assert!(matches!(shown, kampr_journal::Change::Show(_)));

    let after = upto(&adapter, "claude-notes", 17);
    assert_eq!(
        watch.observe(after.preview(&lines(&text))),
        kampr_journal::Change::Retire
    );
    assert!(!watch.showing());
    assert_eq!(
        watch.observe(after.preview(&lines(&text))),
        kampr_journal::Change::Held,
        "withdrawn once, not once per poll"
    );
    let empty = kampr_journal::retired();
    assert_eq!(empty.id, LIVE_ID);
    assert!(empty.blocks.is_empty(), "no blocks is how a client drops it");
}

/// An in-flight tool card is painted with no marker at all, indented like a wrapped line. The
/// walk must not mistake it — or the operator's own prompt above it — for an answer.
#[test]
fn an_unmarked_tool_line_is_not_read_as_a_message() {
    let journal = upto(&claude(), "claude-notes", 12);
    assert_eq!(journal.preview(&lines(&screen("claude-unmarked-tool"))), None);
}

/// A message longer than the pane does not grow, it *slides*: the header scrolls off the top
/// while new lines arrive at the bottom, so two successive views share a middle rather than a
/// prefix. Seen live at 2203 characters into a 2975-character answer, where treating the slide as
/// a new block withdrew the preview and left three seconds with nothing on screen at all.
#[test]
fn a_message_that_outgrows_the_pane_keeps_streaming_while_it_slides() {
    let adapter = claude();
    let journal = upto(&adapter, "claude-vt320", 1);
    let first = screen("claude-sliding-1");
    let second = screen("claude-sliding-2");
    let mut watch = kampr_journal::Watch::default();

    // The pair is genuinely two views of one message, and neither carries its own header.
    let a = journal.preview(&lines(&first)).expect("a clipped preview");
    let b = journal.preview(&lines(&second)).expect("a clipped preview");
    assert_ne!(md(&a), md(&b), "the screen moved between the two captures");
    assert!(
        !md(&b).starts_with(md(&a)),
        "and it moved by sliding, not by extending — which is the whole point"
    );

    assert_eq!(watch.observe(Some(a)), kampr_journal::Change::Held);
    let moved = watch.observe(Some(b));
    assert!(
        matches!(moved, kampr_journal::Change::Show(_)),
        "a slide is the same message still being written: {moved:?}"
    );

    let after = upto(&adapter, "claude-vt320", 2);
    assert_eq!(
        watch.observe(after.preview(&lines(&second))),
        kampr_journal::Change::Retire,
        "and it still yields the moment the record lands"
    );
}

/// `agy` 1.1.18 paints its answer *inside* the block it opened for its own reasoning, so the two
/// lines above the message — `▸ Thought for 4s` and the reasoning's one-line title — are the
/// harness, not the answer, and they are the whole per-harness difference here.
#[test]
fn agy_streams_from_inside_its_own_thought_block() {
    let text = screen("agy-streaming");
    // Fourteen records in: the operator's prompt has landed and the answer has not.
    let before = agy_upto(14);
    let turn = before.preview(&lines(&text)).expect("a preview");
    assert_eq!(turn.id, LIVE_ID);
    let body = md(&turn);
    assert!(
        body.starts_with("1. A terminal multiplexer allows multiple terminal sessions"),
        "the message starts at its own first word: {body:?}"
    );
    assert!(
        !body.contains("Thought for") && !body.contains("Begin Considering Parameters"),
        "the thought header and its title belong to no turn — the record keeps the reasoning \
         in a field of its own: {body:?}"
    );
    assert!(
        !body.contains("Without using any tools"),
        "the operator's own prompt is a different turn: {body:?}"
    );
    assert!(
        !body.contains("Tip:") && !body.contains("esc to cancel"),
        "the harness's footer is not part of the answer: {body:?}"
    );
    assert!(
        body.lines().all(|l| !l.starts_with("  ")),
        "the wrap indent is stripped so the client can re-wrap it: {body:?}"
    );

    let after = agy_upto(15);
    assert_eq!(
        after.preview(&lines(&text)),
        None,
        "and the record it is a prefix of retires it"
    );
}

/// The same message once it is finished and still on screen, with its header intact. This is the
/// pair the stripping has to survive in both directions: publish before the record, stand down
/// after it.
#[test]
fn an_agy_message_the_transcript_has_caught_up_with_is_not_previewed() {
    let text = screen("agy-recorded");
    let lines = lines(&text);

    let before = agy_upto(12);
    let published = before.preview(&lines).expect("a preview before the record lands");
    assert!(
        md(&published).starts_with("Append-only logs allow consumers to stream updates"),
        "{:?}",
        md(&published)
    );
    assert!(
        md(&published).ends_with("for real-time monitoring and log aggregation."),
        "the whole visible message, not its first line: {:?}",
        md(&published)
    );

    let after = agy_upto(13);
    assert_eq!(after.preview(&lines), None);
}

/// Past thirty-odd lines the thought header scrolls off the top and the message is all that is
/// left on screen — so there is nothing to strip, and the comparison has to be a containment
/// rather than a prefix.
#[test]
fn an_agy_message_whose_header_has_scrolled_off_previews_and_still_stands_down() {
    let text = screen("agy-clipped");
    let lines = lines(&text);

    let before = agy_upto(14);
    let published = before.preview(&lines).expect("a clipped preview");
    assert!(
        md(&published).ends_with("boosts productivity in command-line\nenvironments."),
        "what is left of the message is what gets shown: {:?}",
        md(&published)
    );
    assert!(
        !md(&published).contains("1. A terminal multiplexer allows"),
        "the first ten sentences have gone off the top with the header: {:?}",
        md(&published)
    );
    assert!(
        !md(&published).contains("Thought for"),
        "and the header is not on screen to be stripped: {:?}",
        md(&published)
    );

    let after = agy_upto(15);
    assert_eq!(after.preview(&lines), None);
}

/// A thought block that has painted its title and no message yet is the harness clearing its
/// throat. Stripping the two lines it owns leaves nothing, which is the answer.
#[test]
fn an_agy_tool_card_and_a_bare_thought_title_are_never_previewed() {
    for name in ["agy-tool", "agy-working"] {
        assert_eq!(agy_upto(2).preview(&lines(&screen(name))), None, "{name}");
    }
}

/// The growth rule, on `agy`: two captures of one message a poll apart, where the second extends
/// a line the first cut mid-word.
#[test]
fn an_agy_block_earns_its_preview_by_growing() {
    let journal = agy_upto(14);
    let mut watch = kampr_journal::Watch::default();

    let early = journal
        .preview(&lines(&screen("agy-streaming-earlier")))
        .expect("a preview");
    let late = journal
        .preview(&lines(&screen("agy-streaming")))
        .expect("a preview");
    assert_ne!(md(&early), md(&late), "the screen moved between the two captures");
    assert!(
        md(&early).ends_with("10. Navigation between pane") && md(&late).starts_with(md(&early)),
        "the later capture finishes the word the earlier one was cut mid-way through, so this \
         message extends rather than slides"
    );

    assert_eq!(
        watch.observe(Some(early)),
        kampr_journal::Change::Held,
        "the first sighting is not yet proof of a message"
    );
    let grown = watch.observe(Some(late));
    let kampr_journal::Change::Show(turn) = grown else {
        panic!("the same message, longer, is a message: {grown:?}");
    };
    assert!(md(&turn).contains("11. Most multiplexers use a customizable prefix key"));
    assert!(watch.showing());
}

/// What `agy`'s reader is allowed to assume, checked against every frame captured off it: the
/// marker glyph opens a thought header and nothing else. The reader refuses any other head
/// rather than stripping two lines it cannot account for, and this is the evidence for that
/// being a refusal of nothing.
#[test]
fn every_agy_head_in_every_capture_is_a_thought_header() {
    let dir = common::fixtures().join("screens");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        if !name.starts_with("agy-") {
            continue;
        }
        for line in std::fs::read_to_string(entry.path()).unwrap().lines() {
            if let Some(head) = line.strip_prefix('▸') {
                seen += 1;
                assert!(head.trim_start().starts_with("Thought for "), "{name}: {line:?}");
            }
        }
    }
    assert!(seen >= 6, "the corpus has heads to check: {seen}");
}
