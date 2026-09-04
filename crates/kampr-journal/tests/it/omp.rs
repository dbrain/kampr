//! `omp` (oh-my-pi), read off a real run.
//!
//! `tests/fixtures/live/omp-probe.jsonl` and the two transcripts in `omp-probe/` beside it are
//! verbatim: omp 18.1.10 driven through a headless herdr against a local endpoint that answered
//! the Anthropic Messages API, so the harness took every code path it takes against a real
//! provider — a shell command, a failing one, a named `task` spawn, an unnamed one, and the
//! `async-result` notice each spawn's yield came back on. Nothing here is hand-written, because a
//! hand-written transcript would only ever agree with the parser that reads it.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::common::*;
use kampr_journal::{Block, JournalAdapter, OmpAdapter, Registry, ToolState, TranscriptRoot, Turn};

fn root() -> PathBuf {
    fixtures().join("live")
}

fn transcript() -> PathBuf {
    root().join("omp-probe.jsonl")
}

fn adapter() -> OmpAdapter {
    OmpAdapter::new(TranscriptRoot::new(root()).expect("root"))
}

fn registry() -> Registry {
    let mut registry = Registry::new();
    registry.register(Arc::new(adapter()));
    registry
}

fn opened(path: &Path) -> Vec<Turn> {
    let mut journal = adapter().open_path(path.canonicalize().expect("canonical"));
    drain(journal.as_mut())
}

fn launches(turns: &[Turn]) -> Vec<&Block> {
    turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter(|b| matches!(b, Block::Sub { .. }))
        .collect()
}

/// The head of the transcript, up to `records` lines, so a moment part-way through a run can be
/// read without inventing one.
fn upto(records: usize) -> (ScratchDir, PathBuf) {
    let whole = std::fs::read_to_string(transcript()).expect("transcript");
    let head: String = whole.lines().take(records).map(|l| format!("{l}\n")).collect();
    let scratch = scratch_dir("omp");
    let path = scratch.join("omp-probe.jsonl");
    std::fs::write(&path, head).expect("write");
    (scratch, path)
}

#[test]
fn a_command_the_model_ran_carries_its_own_output_and_a_failure_says_so() {
    let turns = opened(&transcript());
    let cards = tool_blocks(&turns);

    let shells: Vec<&&Block> = cards
        .iter()
        .filter(|b| matches!(b, Block::Tool { name, .. } if name == "bash"))
        .collect();
    assert_eq!(shells.len(), 5, "{cards:?}");
    assert_eq!(
        shells[0],
        &&Block::Tool {
            name: "bash".into(),
            summary: Some("echo hello-from-omp".into()),
            lines: Some(4),
            state: ToolState::Done,
        }
    );
    assert!(
        matches!(
            shells[4],
            Block::Tool {
                state: ToolState::Error,
                ..
            }
        ),
        "the `ls /no/such/path` call answered with isError: {:?}",
        shells[4]
    );

    let carried: Vec<&str> = turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Code {
                role: Some(kampr_journal::CodeRole::Output),
                text,
                ..
            } => Some(text.as_str()),
            _ => None,
        })
        .collect();
    assert!(
        carried.iter().any(|t| t.starts_with("hello-from-omp")),
        "{carried:?}"
    );
    assert!(
        carried.iter().any(|t| t.contains("No such file or directory")),
        "{carried:?}"
    );
}

/// omp names a launched agent's transcript after the agent, beside the session that launched it —
/// `<session>/<name>.jsonl`. The name is on the call when the model chose one, and *only* in the
/// acknowledgement when it did not, so both ends mint and neither may mint twice.
#[test]
fn both_a_named_spawn_and_one_omp_named_itself_are_addressable() {
    let turns = opened(&transcript());
    let found = launches(&turns);
    assert_eq!(found.len(), 2, "{:?}", tool_blocks(&turns));

    let Block::Sub { id, kind, title, .. } = found[0] else {
        unreachable!()
    };
    assert_eq!(kind.as_deref(), Some("task"));
    assert_eq!(
        title.as_deref(),
        Some("PROBE-SUBAGENT-TASK: read the README and report what it says")
    );

    let mut sub = registry()
        .open_sub(id, &transcript())
        .expect("the handle opens the transcript it names");
    assert!(sub.path().ends_with("omp-probe/prober.jsonl"), "{:?}", sub.path());
    let said = drain(sub.as_mut());
    assert!(
        md_texts(&said)
            .iter()
            .any(|t| t.contains("Subagent report: the README says hello world.")),
        "{:?}",
        md_texts(&said)
    );

    // The unnamed one: `BizarreWhale` appears nowhere on the call, only in the result that
    // acknowledged it.
    let generated = registry()
        .open_sub(handle(found[1]), &transcript())
        .expect("the generated name resolves too");
    assert!(generated.path().ends_with("omp-probe/BizarreWhale.jsonl"));
}

fn handle(block: &Block) -> &str {
    match block {
        Block::Sub { id, .. } => id.as_str(),
        other => panic!("expected a launch, got {other:?}"),
    }
}

/// A subagent's words are the subagent's: the parent carries a handle, and the reply arrives only
/// when the handle is opened.
#[test]
fn a_launched_agents_words_are_not_spoken_in_the_parents_voice() {
    let turns = opened(&transcript());
    assert!(
        !md_texts(&turns)
            .iter()
            .any(|t| t.contains("Subagent report: the README")),
        "{:?}",
        md_texts(&turns)
    );
}

/// **A spawn is detached by default and its call is answered at once**, so an outstanding tool
/// call is not how a running agent is found. The launch stands until the `async-result` notice
/// names the job.
#[test]
fn a_spawn_is_running_from_its_acknowledgement_until_its_yield_comes_back() {
    // Sixteen records in: `prober` has been spawned and acknowledged, and nothing has come back.
    let (_scratch, head) = upto(16);
    let mut fold = adapter().fold().expect("a fold");
    let facets = fold.facets(&head, None);
    assert_eq!(facets.running.len(), 1, "{facets:?}");
    assert_eq!(facets.running[0].kind, "agent");
    assert_eq!(facets.running[0].name.as_deref(), Some("task"));
    assert_eq!(
        facets.running[0].title.as_deref(),
        Some("PROBE-SUBAGENT-TASK: read the README and report what it says")
    );

    // The whole run: both spawns yielded, and the same fold is asked again rather than a fresh one.
    let settled = fold.facets(&transcript(), None);
    assert!(
        settled.running.is_empty(),
        "both `async-result` notices landed: {:?}",
        settled.running
    );
}

/// The title omp gave the session itself, off the fixed-width slot on line 1.
#[test]
fn the_title_a_session_generated_for_itself_is_published() {
    let title = adapter()
        .fold()
        .expect("a fold")
        .facets(&transcript(), None)
        .title
        .expect("a title");
    assert_eq!(title.text, "I will run one command first");
    assert_eq!(title.source, kampr_journal::TitleSource::Generated);
}

/// **The slot is rewritten in place**, so a fold reading only the bytes a transcript has grown by
/// never sees it move. The `title_change` entry omp appends beside that rewrite is what carries a
/// rename, and it is read here out of a transcript growing under the fold — the shape a watched
/// pane has. The slot is left off, which is the harness's own legacy shape (`docs/session.md`:
/// *"Legacy files may begin directly with the header"*), so the only title on offer is the
/// appended one.
#[test]
fn a_rename_after_the_fold_started_arrives_on_the_appended_entry() {
    let whole = std::fs::read_to_string(transcript()).expect("transcript");
    let lines: Vec<&str> = whole.lines().collect();
    let scratch = scratch_dir("omp-rename");
    let path = scratch.join("growing.jsonl");
    let write = |upto: usize| {
        let body: String = lines[1..upto].iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&path, body).expect("write");
    };

    let mut fold = adapter().fold().expect("a fold");
    write(8);
    assert_eq!(fold.facets(&path, None).title, None, "nothing has named it yet");

    write(lines.len());
    let title = fold.facets(&path, None).title.expect("the rename");
    assert_eq!(title.text, "I will run one command first");
}

/// The working directory is a hint that has to be checked: the bucket omp files a session in is
/// derived from the cwd, and the session's own header is what settles it.
#[test]
fn a_session_is_found_by_the_directory_its_own_header_declares() {
    let declared = Path::new(
        "/tmp/claude-1000/-home-dbrain-dev-kampr/22464faf-f7be-45de-8ae4-330f7280a839/scratchpad/probe/project",
    );
    let scratch = scratch_dir("omp-root");
    let bucket = scratch.join("sessions/-tmp-komp");
    std::fs::create_dir_all(&bucket).expect("bucket");
    let filed = bucket.join("2026-09-04T12-29-19-072Z_01a06c65-1560-710f-9ae4-8bb687cce92e.jsonl");
    std::fs::copy(transcript(), &filed).expect("copy");
    let adapter = OmpAdapter::new(TranscriptRoot::new(&scratch).expect("root"));

    assert_eq!(
        adapter
            .locate_by_cwd(declared, None)
            .expect("the declared directory"),
        filed.canonicalize().expect("canonical")
    );
    assert!(
        adapter
            .locate_by_cwd(Path::new("/tmp/somewhere-else"), None)
            .is_err(),
        "a directory no transcript names has no conversation, not the newest one"
    );
    assert_eq!(
        adapter
            .locate(&kampr_journal::SessionRef::id(
                "omp",
                "01a06c65-1560-710f-9ae4-8bb687cce92e"
            ))
            .expect("the id"),
        filed.canonicalize().expect("canonical")
    );
}

/// A title is read only for the harness that writes it: read by the wrong one it is a status
/// invented for a pane nobody measured.
#[test]
fn a_terminal_title_is_read_only_by_the_harness_that_writes_it() {
    use kampr_journal::title_status;
    assert_eq!(title_status(Some("omp"), Some("π ⠹ project")), Some("busy"));
    // **`pi` is not read for one**, though the same adapter serves its transcripts: [#490](#)
    // measured its title as `π - <session> - <dir>`, with no run state in it at all.
    assert_eq!(title_status(Some("pi"), Some("π ! project")), None);
    assert_eq!(title_status(Some("claude"), Some("π ⠹ project")), None);
    assert_eq!(title_status(None, Some("π ⠹ project")), None);
    assert_eq!(title_status(Some("omp"), None), None);
}

/// **omp stamps every message it writes, so a turn's duration is a span between two of its own
/// recorded instants** rather than an inference: the prompt's `timestamp` to the `completedAt` of
/// the message that ended the turn. It is the same quantity Claude's `turn_duration` carries —
/// tool time included — and pointedly not omp's own per-message `duration`, which excludes it.
#[test]
fn every_message_carries_the_time_the_model_spent_writing_it() {
    let facets = adapter().fold().expect("a fold").facets(&transcript(), None);
    assert!(!facets.timings.is_empty(), "{:?}", facets.timings);

    let turns = opened(&transcript());
    for timing in &facets.timings {
        assert!(
            turns.iter().any(|t| t.id == timing.turn),
            "a timing hung off no turn: {timing:?} against {:?}",
            turns.iter().map(|t| &t.id).collect::<Vec<_>>()
        );
        assert!(timing.duration_ms > 0, "{timing:?}");
    }
    // The run's first turn ran a 25 s command inside it, and the `stop` that ended it is what
    // closes the span — an `error` the harness retried and every `toolUse` in between do not.
    let longest = facets
        .timings
        .iter()
        .max_by_key(|t| t.duration_ms)
        .expect("a timing");
    assert!(longest.duration_ms > 25_000, "{longest:?}");
    assert!(
        longest.messages.is_some_and(|n| n > 1),
        "a turn is several messages and says how many: {longest:?}"
    );
}

/// An insertion, where omp's two sides of numbering visibly disagree: the added rows carry their
/// **new** numbers and the context rows below them their **old** ones, so ` 3|line three` follows
/// `+3|` and `+4|` and is old line 3 at new line 5 ([#497](#)). The new side's start is therefore
/// arithmetic over what the hunks above have done to the file's length, not a number read off a
/// row — a rule that only shows itself on a patch with two hunks and an insertion in the first.
#[test]
fn an_inserted_line_does_not_move_the_hunk_it_was_inserted_into() {
    let turns = opened(&fixtures().join("live/omp-insert.jsonl"));
    let diffs: Vec<&String> = turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Diff { text, .. } => Some(text),
            _ => None,
        })
        .collect();
    assert_eq!(diffs.len(), 1, "{:?}", tool_blocks(&turns));
    assert_eq!(
        diffs[0],
        "@@ -1,5 +1,7 @@\n line one\n line two\n+an inserted line\n+and another\n line three\n line four\n line five\n"
    );
}

/// **omp files the whole of an edit beside the result** — `op`, `path`, `oldText`, `newText` and
/// a line-numbered `diff` — and the line numbers are what make a unified hunk out of it without
/// inventing anything: a hunk runs while the old side stays contiguous, and omp leaves a gap in
/// the numbering exactly where one ends. Two edits in one call, four lines apart, is what says so.
#[test]
fn an_edit_is_published_as_the_hunks_its_own_line_numbers_describe() {
    let turns = opened(&fixtures().join("live/omp-edit.jsonl"));
    let diffs: Vec<(&Option<String>, &String)> = turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Diff { path, text } => Some((path, text)),
            _ => None,
        })
        .collect();
    assert_eq!(diffs.len(), 1, "{:?}", tool_blocks(&turns));
    let (path, text) = diffs[0];
    assert!(
        path.as_deref().is_some_and(|p| p.ends_with("README.md")),
        "{path:?}"
    );
    assert_eq!(
        text,
        "@@ -1,4 +1,4 @@\n line one\n-line two\n+second line, changed\n line three\n line four\n\
         @@ -6,5 +6,5 @@\n line six\n line seven\n-line eight\n+eighth line, changed\n line nine\n line ten\n"
    );
}

/// **omp's steering queue is on the screen and nowhere else.** It writes a queued prompt down when
/// it delivers it and not before ([#489](#)), so a session with two waiting is byte-identical on
/// disk to one with none — and the screen is what the operator can see, which is what is
/// published.
#[test]
fn the_prompts_waiting_behind_a_turn_are_read_off_the_screen() {
    let grid = screen("omp-queued");
    let rows: Vec<&str> = grid.lines().collect();
    let waiting = adapter().queued().expect("a queue reader")(&rows);
    assert_eq!(
        waiting.iter().map(|q| q.text.as_str()).collect::<Vec<_>>(),
        ["and then push it", "and tag the release too"]
    );
    assert!(
        waiting.iter().all(|q| q.at.is_none()),
        "the screen carries no stamp"
    );

    // The hint row omp draws under the last prompt sits at the same indent and is not a third
    // thing anybody is waiting on.
    assert_eq!(waiting.len(), 2, "{waiting:?}");

    // A prompt too long for a row is truncated by the harness rather than wrapped, and what is
    // published is what the operator sees — ellipsis included, not a sentence reassembled out of
    // rows omp never drew.
    let clipped = screen("omp-queued-clipped");
    let rows: Vec<&str> = clipped.lines().collect();
    let waiting = adapter().queued().expect("a queue reader")(&rows);
    assert_eq!(waiting.len(), 2, "{waiting:?}");
    assert!(waiting[1].text.ends_with('…'), "{:?}", waiting[1].text);

    // An idle pane has no queue, and that is an empty list rather than a stale one.
    let idle = screen("omp-idle");
    let rows: Vec<&str> = idle.lines().collect();
    assert!(adapter().queued().expect("a queue reader")(&rows).is_empty());
}

fn screen(name: &str) -> String {
    let path = fixtures().join("screens").join(format!("{name}.txt"));
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

/// A picture the operator handed omp, and the bytes behind the marker.
///
/// **Measured inline**: omp normalises an image on the way in — a 7.8 KB PNG was stored as 904
/// characters of base64 — so it lands under the 1 024-character threshold its own `session.md`
/// says a payload is content-addressed into `blobs/<sha256>` at. A record carrying
/// `blob:sha256:<hash>` instead yields no attachment here rather than a header pointing at bytes
/// the record does not hold, and nothing has been measured to produce one.
#[test]
fn a_picture_in_a_prompt_is_offered_with_the_bytes_behind_it() {
    let transcript = fixtures().join("live/omp-image.jsonl");
    let turns = opened(&transcript);
    let marked: Vec<&kampr_journal::Attachment> = turns
        .iter()
        .flat_map(|t| &t.blocks)
        .filter_map(|b| match b {
            Block::Md { att: Some(att), .. } => Some(att),
            _ => None,
        })
        .collect();
    assert_eq!(marked.len(), 1, "{turns:?}");
    assert_eq!(marked[0].kind, "image");
    assert_eq!(marked[0].mime.as_deref(), Some("image/png"));
    assert!(marked[0].bytes.is_some_and(|n| n > 300), "{:?}", marked[0].bytes);

    // The marker beside it is what a client that has never heard of attachments draws.
    let markers = md_texts(&turns);
    assert!(markers.contains(&"[image · png]"), "{markers:?}");

    // And the id resolves to the picture itself, off the record it was minted from.
    let record = std::fs::read_to_string(&transcript)
        .expect("transcript")
        .lines()
        .find(|l| l.contains("\"type\":\"image\""))
        .expect("the image record")
        .to_string();
    let fetched = adapter().attachment(&record, 0).expect("the bytes");
    assert!(fetched.data.starts_with(b"\x89PNG"), "a PNG, not a marker");
}

/// **An omp session is a tree, and a rewind leaves the abandoned branch in the file.**
///
/// Driven, not reasoned: two prompts answered, `/tree` searched for the first and selected, then a
/// third prompt ([#495](#)). omp keeps `SAY: bravo` and its answer in the file and gives the new
/// prompt the *alpha answer* as its parent, so a reader that takes the file in order publishes a
/// turn the operator took back — which is the one way an omp conversation can be wrong rather than
/// merely thin.
#[test]
fn a_turn_the_operator_rewound_past_is_not_published_as_something_the_agent_said() {
    let turns = opened(&fixtures().join("live/omp-rewound.jsonl"));
    let said = md_texts(&turns);
    assert!(said.contains(&"SAY: alpha"), "{said:?}");
    assert!(said.contains(&"Answering alpha."), "{said:?}");
    assert!(said.contains(&"SAY: charlie"), "{said:?}");
    assert!(said.contains(&"Answering charlie."), "{said:?}");
    assert!(
        !said.iter().any(|t| t.contains("bravo")),
        "the abandoned branch is still in the file and must not be spoken: {said:?}"
    );
}

/// And a client that was already holding the abandoned turns has to be told to drop them: a page
/// merges by id, so a turn that simply stops being sent stays on the screen for ever. The
/// retirement is the same one the live preview uses — the turn's own id carrying no blocks.
#[test]
fn a_rewind_retires_the_turns_it_took_back() {
    let whole = std::fs::read_to_string(fixtures().join("live/omp-rewound.jsonl")).expect("fixture");
    let lines: Vec<&str> = whole.lines().collect();
    let scratch = scratch_dir("omp-rewind");
    let path = scratch.join("growing.jsonl");
    let write = |upto: usize| {
        let body: String = lines[..upto].iter().map(|l| format!("{l}\n")).collect();
        std::fs::write(&path, body).expect("write");
    };

    // Nine records in: alpha and bravo have both been said and nothing has been rewound.
    write(9);
    let mut journal = adapter().open_path(path.canonicalize().expect("canonical"));
    let before = drain(journal.as_mut());
    let bravo: Vec<String> = before
        .iter()
        .filter(|t| {
            t.blocks
                .iter()
                .any(|b| matches!(b, Block::Md { text, .. } if text.contains("bravo")))
        })
        .map(|t| t.id.clone())
        .collect();
    assert_eq!(bravo.len(), 2, "the prompt and the answer: {before:?}");

    // The rest of the file, which is the rewind and the turn that replaced it.
    write(lines.len());
    let after = drain(journal.as_mut());
    for id in &bravo {
        let retired = after
            .iter()
            .find(|t| &t.id == id)
            .unwrap_or_else(|| panic!("nothing withdrew {id}: {after:?}"));
        assert!(
            retired.blocks.is_empty(),
            "a retirement is a turn with no blocks: {retired:?}"
        );
    }
    assert!(
        md_texts(&after).contains(&"Answering charlie."),
        "{:?}",
        md_texts(&after)
    );
}
