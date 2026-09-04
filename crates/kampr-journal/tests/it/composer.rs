//! The line the operator has half-typed at the desk, read off screens captured from real
//! harnesses.
//!
//! Every fixture under `tests/fixtures/composer` is a verbatim grid of a live `claude` 2.1.250,
//! `codex` 0.149.1 or `agy` 1.1.22 in a headless herdr, folded from that pane's own
//! `terminal session observe` stream so the caret on its first line is the caret herdr reported
//! — not one a test chose. `research/probe/composer-line.py` is what captured them.

use crate::common;

use kampr_journal::{
    AgyAdapter, Caret, ClaudeAdapter, CodexAdapter, Composed, ComposerFeed, JournalAdapter, OmpAdapter,
    TranscriptRoot,
};

/// A fixture's caret header and its grid. The caret has to travel with the screen: it is the only
/// thing that separates an empty composer from one painting a placeholder.
fn capture(name: &str) -> (String, Caret) {
    let path = common::fixtures().join("composer").join(format!("{name}.txt"));
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let (head, body) = text.split_once('\n').expect("caret header");
    let mut parts = head.split_whitespace();
    assert_eq!(parts.next(), Some("caret"), "{name}: no caret header");
    let col = parts.next().expect("caret col").parse().expect("caret col");
    let row = parts.next().expect("caret row").parse().expect("caret row");
    (body.to_string(), Caret { col, row })
}

fn read(adapter: &dyn JournalAdapter, name: &str) -> Option<String> {
    let (body, caret) = capture(name);
    let rows: Vec<&str> = body.lines().collect();
    adapter.composer()?(&rows, caret).map(|c| c.text)
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

/// The root is only a containment check here — a composer read touches no transcript — so the
/// fixture directory stands in for one.
fn omp() -> OmpAdapter {
    OmpAdapter::new(TranscriptRoot::new(common::fixtures()).expect("root"))
}

const TYPED: &str = "push the branch when the tests go green";

#[test]
fn what_the_operator_has_typed_at_the_desk_is_read_off_every_harness_probed() {
    assert_eq!(read(&claude(), "claude-typed").as_deref(), Some(TYPED));
    assert_eq!(read(&codex(), "codex-typed").as_deref(), Some(TYPED));
    assert_eq!(read(&agy(), "agy-typed").as_deref(), Some(TYPED));
    assert_eq!(read(&omp(), "omp-typed").as_deref(), Some(TYPED));
}

/// Claude paints `Try "refactor <filepath>"` into an empty composer and Codex paints `Ask Codex to
/// do anything`, in the same cells the operator's own words would occupy — so a reader that took
/// the text after the marker would publish the harness's hint as the operator's sentence, and the
/// strip would claim a line nobody typed. The caret is the only thing that separates them.
#[test]
fn a_harnesss_own_placeholder_is_not_the_operators_line() {
    assert_eq!(read(&claude(), "claude-empty"), None);
    assert_eq!(read(&codex(), "codex-empty"), None);
    assert_eq!(read(&agy(), "agy-empty"), None);
    // omp paints no placeholder at all, and the caret says the same thing about it.
    assert_eq!(read(&omp(), "omp-empty"), None);
}

/// A line longer than the box wraps onto rows indented by two, and the whole of it is the
/// operator's sentence. Reading only the marked row would hand back a third of what is there and
/// then offer to clear the rest.
#[test]
fn a_line_too_long_for_the_box_is_read_whole_and_not_just_its_first_row() {
    for (name, text) in [
        ("claude-wrapped", read(&claude(), "claude-wrapped")),
        ("codex-wrapped", read(&codex(), "codex-wrapped")),
        ("agy-wrapped", read(&agy(), "agy-wrapped")),
        ("omp-wrapped", read(&omp(), "omp-wrapped")),
    ] {
        let text = text.unwrap_or_else(|| panic!("{name}: nothing read"));
        assert!(text.starts_with(TYPED), "{name}: {text:?}");
        assert!(
            text.len() > TYPED.len() + 40,
            "{name}: only the first row came back: {text:?}"
        );
        assert!(
            !text.contains('\n'),
            "{name}: a wrapped row is not a new line: {text:?}"
        );
    }
}

/// **A measured limitation, kept deliberately.** `ctrl+a` puts the caret back at the input column
/// on all three harnesses with the operator's text still on the line, and nothing else on the
/// screen tells the two apart — so the strip goes away rather than reporting a line it cannot be
/// sure of. Absent is the failure this is allowed to have; wrong is not.
#[test]
fn a_caret_sent_home_reads_as_empty_rather_than_as_a_guess() {
    assert_eq!(read(&claude(), "claude-caret-at-home"), None);
    assert_eq!(read(&codex(), "codex-caret-at-home"), None);
    assert_eq!(read(&agy(), "agy-caret-at-home"), None);
    assert_eq!(read(&omp(), "omp-caret-at-home"), None);
}

/// The clearing keystroke is a per-harness measurement, and the three disagree: one `ctrl+u` takes
/// the whole of Codex's and agy's buffer but only one *visual row* of Claude's, and `ctrl+c` takes
/// the whole of Claude's and Codex's while agy answers it by arming an exit. A single key for all
/// three would delete part of somebody's sentence on one harness and quit the agent on another.
#[test]
fn each_harness_carries_the_keystroke_measured_to_clear_its_own_composer() {
    let (body, caret) = capture("claude-typed");
    let rows: Vec<&str> = body.lines().collect();
    assert_eq!(
        claude().composer().unwrap()(&rows, caret).unwrap().clear,
        Some("\u{3}")
    );

    let (body, caret) = capture("codex-typed");
    let rows: Vec<&str> = body.lines().collect();
    assert_eq!(
        codex().composer().unwrap()(&rows, caret).unwrap().clear,
        Some("\u{15}")
    );

    let (body, caret) = capture("agy-typed");
    let rows: Vec<&str> = body.lines().collect();
    assert_eq!(
        agy().composer().unwrap()(&rows, caret).unwrap().clear,
        Some("\u{15}")
    );
}

/// **A menu is opened by the same marker the composer is**, and its second option is indented
/// exactly like a wrapped continuation: Codex asks whether to trust a directory as `› 1. Yes,
/// continue` over `  2. No, quit`, so a walk that gathered rows without asking where the caret was
/// would publish the harness's own dialog as a sentence the operator had typed — and then offer to
/// clear it. The caret is nowhere near either row, and that is the whole of the answer.
#[test]
fn a_menu_the_harness_is_asking_about_is_never_read_as_the_operators_line() {
    assert_eq!(read(&codex(), "codex-trust-menu"), None);
    assert_eq!(read(&agy(), "agy-trust-menu"), None);
}

/// A pane whose screen has nothing that opens a composer — a shell, a pager, a harness nobody has
/// probed — says nothing rather than offering the last line it happened to find.
#[test]
fn a_screen_with_no_composer_on_it_reports_nothing() {
    let rows = [
        "$ ls -la",
        "total 4",
        "drwxr-xr-x 2 dbrain dbrain 4096 Aug 29 00:00 .",
    ];
    let caret = Caret { col: 8, row: 0 };
    assert!(claude().composer().unwrap()(&rows, caret).is_none());
    assert!(codex().composer().unwrap()(&rows, caret).is_none());
    assert!(agy().composer().unwrap()(&rows, caret).is_none());
}

/// The same rule `FacetFeed` follows: a conversation is polled several times a second and a desk
/// line moves when somebody types, so an unchanged composer is not a frame. The first look at an
/// empty one is silence too — it is the same message as never having sent anything.
#[test]
fn an_unchanged_composer_is_not_a_frame_and_an_empty_one_opens_in_silence() {
    let mut feed = ComposerFeed::default();
    assert_eq!(feed.moved(None), None);
    assert_eq!(feed.moved(Some(said(TYPED))), Some(Some(said(TYPED))));
    assert_eq!(feed.moved(Some(said(TYPED))), None);
    assert_eq!(
        feed.moved(Some(said("push the branch"))),
        Some(Some(said("push the branch")))
    );
    assert_eq!(feed.moved(None), Some(None));
    assert_eq!(feed.moved(None), None);
}

/// A pane whose agent is quit and a different one started in its place can be left holding the
/// same half-sentence — and the key that empties the box is not the same key. `ctrl+u` clears
/// Codex's whole buffer and only one visual row of Claude's, and `ctrl+c` clears Claude's and arms
/// an **exit** on agy, so a client left holding the previous harness's key would either mangle the
/// line or quit the session. The words being unchanged is not the composer being unchanged.
#[test]
fn the_same_words_under_a_different_harness_is_a_different_key_and_is_published() {
    let mut feed = ComposerFeed::default();
    assert_eq!(feed.moved(Some(said(TYPED))), Some(Some(said(TYPED))));
    let elsewhere = Composed {
        text: TYPED.to_string(),
        clear: Some("\u{15}"),
    };
    assert_eq!(feed.moved(Some(elsewhere.clone())), Some(Some(elsewhere)));
}

fn said(text: &str) -> Composed {
    Composed {
        text: text.to_string(),
        clear: Some("\u{3}"),
    }
}
