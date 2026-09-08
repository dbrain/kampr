use kampr_core::provider::RawScrollback;
use kampr_core::scrollback::{Ingest, ScrollbackRing};
use kampr_core::wire::{Encoder, ServerMsg};
use kampr_term::Color;

fn raw(lines: &[&str], cols: u16, viewport_rows: u16, truncated: bool) -> RawScrollback {
    labelled(lines, Some(cols), viewport_rows, truncated)
}

fn labelled(lines: &[&str], cols: Option<u16>, viewport_rows: u16, truncated: bool) -> RawScrollback {
    let mut text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    if lines.is_empty() {
        text.clear();
    }
    RawScrollback {
        text,
        cols,
        viewport_rows,
        truncated,
        first_row: None,
    }
}

fn numbered(from: usize, to: usize) -> Vec<String> {
    (from..=to).map(|i| format!("line-{i}")).collect()
}

/// A read that knows where it sits in the provider's history — which is what lets the ring join
/// two windows by arithmetic instead of by matching their text (probe #522).
fn at(lines: &[&str], viewport_rows: u16, truncated: bool, first_row: Option<u32>) -> RawScrollback {
    RawScrollback {
        first_row,
        ..labelled(lines, Some(80), viewport_rows, truncated)
    }
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn once(raw: &RawScrollback) -> ScrollbackRing {
    let mut ring = ScrollbackRing::default();
    ring.ingest(raw);
    ring
}

fn lines_of(ring: &mut ScrollbackRing) -> Vec<String> {
    ring.render()
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
}

#[test]
fn the_live_viewport_is_stripped_so_history_never_duplicates_the_grid() {
    let mut ring = once(&raw(&["old-1", "old-2", "live-1", "live-2"], 10, 2, false));
    let doc = ring.render();
    assert_eq!(doc.total_rows, 2);
    assert_eq!(doc.from_top, 0);
    assert!(doc.complete);
    assert!(!doc.capped);
    assert_eq!(lines_of(&mut ring), ["old-1", "old-2"]);
}

#[test]
fn colour_survives_the_same_emulator_the_live_grid_uses() {
    let mut ring = once(&raw(
        &[
            "\x1b[38;2;255;120;0mwarm\x1b[0m plain",
            "\x1b[1;31mbold-red\x1b[0m",
            "viewport",
        ],
        20,
        1,
        false,
    ));
    let doc = ring.render();
    assert_eq!(doc.rows.len(), 2);
    assert_eq!(doc.rows[0].cells[0].fg, Color::Rgb(255, 120, 0));
    assert_eq!(
        doc.rows[0].cells[5].fg,
        Color::Default,
        "SGR 0 resets between runs"
    );
    assert_eq!(doc.rows[1].cells[0].fg, Color::Indexed(1));
    assert!(doc.rows[1].cells[0].attrs.bold);
}

#[test]
fn a_bare_newline_still_returns_the_carriage() {
    let mut ring = once(&raw(&["aaa", "b", "viewport"], 10, 1, false));
    assert_eq!(
        lines_of(&mut ring),
        ["aaa", "b"],
        "pane.read separates rows with LF alone"
    );
}

#[test]
fn an_empty_ring_renders_nothing() {
    let mut ring = once(&raw(&["only-viewport"], 20, 1, false));
    let doc = ring.render();
    assert!(doc.rows.is_empty());
    assert_eq!(doc.total_rows, 0);
    assert!(doc.complete);
}

#[test]
fn a_truncated_read_says_history_above_it_is_unreachable() {
    let mut ring = once(&raw(&["a", "b", "viewport"], 10, 1, true));
    let doc = ring.render();
    assert!(
        doc.capped,
        "herdr had more than it returned, and there is no way to ask for it"
    );
    assert_eq!(doc.from_top, 0, "our ring still starts at our own row zero");
    assert_eq!(doc.total_rows, 2);
}

#[test]
fn successive_reads_stitch_into_a_ring_deeper_than_one_read() {
    let mut ring = ScrollbackRing::default();
    let first = numbered(1, 5);
    assert_eq!(
        ring.ingest(&raw(&refs(&first), 10, 1, true)),
        Ingest::Fresh { rows: 4 }
    );

    let second = numbered(3, 9);
    assert_eq!(
        ring.ingest(&raw(&refs(&second), 10, 1, true)),
        Ingest::Stitched { added: 4 }
    );
    assert_eq!(ring.len(), 8, "the overlap joined them without duplicating it");
    assert_eq!(lines_of(&mut ring), numbered(1, 8));

    let doc = ring.render();
    assert_eq!(doc.total_rows, 8);
    assert!(doc.capped, "the first read was already against the cap");
    assert_eq!(doc.rows.last().unwrap().row, 7);
}

#[test]
fn re_reading_unchanged_history_adds_nothing() {
    let mut ring = ScrollbackRing::default();
    let read = numbered(1, 6);
    ring.ingest(&raw(&refs(&read), 10, 1, false));
    assert_eq!(
        ring.ingest(&raw(&refs(&read), 10, 1, false)),
        Ingest::Stitched { added: 0 }
    );
    assert_eq!(ring.len(), 5);
}

#[test]
fn a_read_with_no_overlap_is_a_gap_and_caps_the_ring() {
    let mut ring = ScrollbackRing::default();
    let first = numbered(1, 6);
    ring.ingest(&raw(&refs(&first), 10, 1, false));
    assert_eq!(ring.len(), 5);
    assert!(!ring.capped());

    let far = numbered(900, 906);
    assert_eq!(
        ring.ingest(&raw(&refs(&far), 10, 1, true)),
        Ingest::Gap { dropped: 5 }
    );
    assert!(ring.capped(), "unrelated history must never be spliced together");
    assert_eq!(lines_of(&mut ring), numbered(900, 905));
    let doc = ring.render();
    assert_eq!(doc.from_top, 5, "indices stay monotonic across the gap");
    assert!(!doc.complete);
}

#[test]
fn a_width_change_restarts_the_ring_because_stored_rows_were_wrapped_at_the_old_width() {
    let mut ring = ScrollbackRing::default();
    let read = numbered(1, 4);
    ring.ingest(&raw(&refs(&read), 40, 1, false));
    assert!(matches!(
        ring.ingest(&raw(&refs(&read), 20, 1, false)),
        Ingest::Rewrapped { .. }
    ));
    assert!(ring.capped());
}

#[test]
fn the_ring_is_bounded_and_says_so_when_it_trims() {
    let mut ring = ScrollbackRing::new(4);
    let read = numbered(1, 11);
    ring.ingest(&raw(&refs(&read), 10, 1, false));
    assert_eq!(ring.len(), 4);
    assert!(ring.capped());
    assert_eq!(lines_of(&mut ring), numbered(7, 10));
    assert_eq!(ring.render().from_top, 6);
    assert_eq!(ring.render().total_rows, 4, "a depth, so the ring spans 6..10");
}

/// **The ring caps rows, and a row has no length.** A pane that writes one enormous line per row
/// filled it with 20 000 of them, and 20 000 rows of 144 KB is 2.8 GB of `String` before a grid is
/// built out of them at all — the ring's own row cap is no bound on a hostile pane's memory.
#[test]
fn a_pane_that_writes_enormous_rows_is_bounded_by_bytes_and_not_only_by_rows() {
    let huge: Vec<String> = (0..10).map(|i| format!("{i:2}").repeat(512 * 1024)).collect();
    let mut ring = ScrollbackRing::default();
    ring.ingest(&raw(&refs(&huge), 80, 1, false));

    assert!(
        ring.len() <= 8,
        "{} rows of a megabyte each are still held",
        ring.len()
    );
    assert!(ring.capped(), "and the ring says history above it is gone");
    let mut ring = ScrollbackRing::default();
    ring.ingest(&raw(&[&"z".repeat(9 * 1024 * 1024), "viewport"], 80, 1, false));
    assert_eq!(
        ring.len(),
        1,
        "and a single row past the whole budget is still what the pane is doing now"
    );
}

/// **Width and depth multiply, and only depth was bounded.** `lay_out` sizes one grid by the widest
/// row in the ring, so a single row of 65 535 columns beside twenty thousand ordinary ones is 1.3
/// billion cells — 52 GB at 40 bytes each — handed to the allocator in one piece. That is
/// `handle_alloc_error` and an abort, and every poll rebuilds it.
#[test]
fn one_enormously_wide_row_cannot_take_the_whole_ring_down_with_it() {
    let mut lines = numbered(1, 20_000);
    lines.push("x".repeat(70_000));
    lines.push("viewport".to_string());
    let mut ring = ScrollbackRing::default();
    ring.ingest(&raw(&refs(&lines), 80, 1, false));

    assert!(
        ring.len() <= 64,
        "{} rows to be laid out beside a 65 535-column one",
        ring.len()
    );
    assert!(ring.capped());
}

#[test]
fn absolute_indices_survive_past_sixteen_bits() {
    let mut ring = ScrollbackRing::new(3);
    let read = numbered(1, 70_010);
    ring.ingest(&raw(&refs(&read), 12, 1, false));
    let doc = ring.render();
    assert_eq!(doc.from_top, 70_006);
    assert_eq!(doc.rows.first().unwrap().row, 70_006);
    assert_eq!(doc.total_rows, 3);
}

#[test]
fn scrollback_serialises_to_the_documented_shape() {
    let mut ring = once(&raw(&["\x1b[31mred\x1b[0m", "plain", "viewport"], 10, 1, true));
    let mut enc = Encoder::new();
    let msgs = enc.encode_scrollback("01J/w3:p2", &ring.render());
    let v = serde_json::to_value(msgs.last().unwrap()).unwrap();
    assert_eq!(v["t"], "scrollback");
    assert_eq!(v["pane"], "01J/w3:p2");
    assert_eq!(v["from_top"], 0);
    assert_eq!(v["total_rows"], 2);
    assert_eq!(v["complete"], true);
    assert_eq!(v["capped"], true);
    assert_eq!(v["rows"][0]["row"], 0);
    assert_eq!(v["rows"][0]["runs"][0]["x"], "red");
    assert!(
        matches!(msgs.first(), Some(ServerMsg::Styles(_))),
        "new styles precede the rows that reference them"
    );
}

/// Probe #112. One width change is one restart; every read after it overlaps the rows the restart
/// kept, so the ring has to go back to stitching rather than throwing its history away forever.
#[test]
fn history_accumulates_again_after_a_width_change() {
    let mut ring = ScrollbackRing::new(60);
    ring.ingest(&raw(&refs(&numbered(1, 10)), 40, 1, false));
    assert!(matches!(
        ring.ingest(&raw(&refs(&numbered(1, 10)), 93, 1, false)),
        Ingest::Rewrapped { dropped: 9 }
    ));
    let restarted_at = ring.render().from_top;

    assert_eq!(
        ring.ingest(&raw(&refs(&numbered(3, 14)), 93, 1, false)),
        Ingest::Stitched { added: 4 }
    );
    assert_eq!(
        ring.ingest(&raw(&refs(&numbered(5, 18)), 93, 1, false)),
        Ingest::Stitched { added: 4 }
    );
    assert_eq!(
        ring.render().from_top,
        restarted_at,
        "the ring restarted once, not once per read"
    );
    assert_eq!(lines_of(&mut ring), numbered(1, 17));
}

/// A restart keeps the rows of the read that caused it and nothing of what it replaced.
#[test]
fn a_restarted_ring_holds_the_read_that_restarted_it() {
    let mut ring = ScrollbackRing::new(60);
    ring.ingest(&raw(&["short", "viewport"], 10, 1, false));
    ring.ingest(&raw(&["0123456789abcdefghij", "viewport"], 40, 1, false));
    assert_eq!(lines_of(&mut ring), ["0123456789abcdefghij"]);
}

/// A full-screen program takes the pane and `pane.read recent` comes back as the live viewport
/// and nothing else. A read with no history in it is *no news about history* — not history that
/// disagrees with what the ring holds — and treating it as a gap discarded the operator's whole
/// scrollback and rebased the ring, which every consumer downstream then read as a restart.
#[test]
fn a_read_that_carries_no_history_leaves_the_ring_exactly_where_it_was() {
    let mut ring = ScrollbackRing::default();
    ring.ingest(&raw(&refs(&numbered(1, 6)), 10, 1, false));
    assert_eq!(ring.len(), 5);

    assert_eq!(
        ring.ingest(&raw(&["only-the-viewport"], 10, 1, false)),
        Ingest::Stitched { added: 0 }
    );

    assert_eq!(ring.len(), 5);
    assert_eq!(lines_of(&mut ring), numbered(1, 5));
    assert!(!ring.capped(), "nothing was lost, so nothing is unreachable");
    let doc = ring.render();
    assert_eq!(doc.from_top, 0, "nothing was discarded, so nothing was rebased");
    assert_eq!(doc.total_rows, 5);
}

/// The other half: the pane comes back and the read overlaps what the ring kept, so it stitches
/// rather than starting a second unrelated stretch of history at a new base.
#[test]
fn history_the_alt_screen_hid_is_stitched_back_rather_than_started_again() {
    let mut ring = ScrollbackRing::default();
    ring.ingest(&raw(&refs(&numbered(1, 6)), 10, 1, false));
    ring.ingest(&raw(&["only-the-viewport"], 10, 1, false));

    assert_eq!(
        ring.ingest(&raw(&refs(&numbered(1, 8)), 10, 1, false)),
        Ingest::Stitched { added: 2 }
    );
    assert_eq!(ring.render().from_top, 0);
    assert_eq!(lines_of(&mut ring), numbered(1, 7));
}

/// Probe #68: the rect says 47 while the PTY is 93, so a read labelled with the rect is labelled
/// narrower than the rows in it. Rendering on a grid that narrow wraps every full row onto a
/// second line, pushes the document past the grid's height and drops the top of the ring off it —
/// while `from_top`, `total_rows` and every row index go on describing the whole span. That is
/// ADR 0004's corruption arriving through a door with no `capped` on it.
#[test]
fn a_row_wider_than_its_label_survives_being_rendered() {
    let wide: Vec<String> = (1..=20).map(|i| format!("row-{i:->86}")).collect();
    let mut ring = once(&labelled(&refs(&wide), Some(47), 0, false));
    let doc = ring.render();
    assert_eq!(doc.total_rows, 20);
    assert_eq!(doc.rows.len(), 20, "rows fell off the top of the render grid");
    assert_eq!(lines_of(&mut ring), wide);
}

/// `known_cols` answered with the bare rect whenever nothing had measured the pane yet, and the
/// first history read happens at the same instant the width probe starts — so the label moved
/// 47 → 93 a moment later and the ring flushed itself on every freshly-watched split pane.
#[test]
fn a_width_the_node_has_not_proved_does_not_restart_the_ring() {
    let mut ring = once(&labelled(&refs(&numbered(1, 5)), None, 0, false));
    let stitched = ring.ingest(&labelled(&refs(&numbered(1, 8)), Some(93), 0, false));
    assert_eq!(stitched, Ingest::Stitched { added: 3 });
    assert_eq!(lines_of(&mut ring), numbered(1, 8));
    assert!(!ring.capped());

    let rewrapped = ring.ingest(&labelled(&refs(&numbered(9, 12)), Some(47), 0, false));
    assert!(
        matches!(rewrapped, Ingest::Rewrapped { .. }),
        "a proved width that changes is still a re-wrap: {rewrapped:?}"
    );
}

/// The rendered document is cached so a 3 s poll per client does not rebuild an emulator over
/// twenty thousand rows to answer a question nothing moved. A cache that outlives its ring is
/// worse than no cache.
#[test]
fn a_render_after_an_ingest_is_not_the_document_from_before_it() {
    let mut ring = once(&labelled(&refs(&numbered(1, 3)), Some(20), 0, false));
    assert_eq!(lines_of(&mut ring), numbered(1, 3));
    ring.ingest(&labelled(&refs(&numbered(2, 5)), Some(20), 0, false));
    assert_eq!(lines_of(&mut ring), numbered(1, 5));
    ring.ingest(&labelled(&refs(&numbered(90, 92)), Some(20), 0, false));
    assert_eq!(lines_of(&mut ring), numbered(90, 92));
}

/// The shape a live Claude Code pane actually produces, replayed from a real one.
///
/// Claude Code takes the **alternate screen**, so herdr's ring goes away for as long as the
/// harness holds it (probe #244) and the conversation never enters it — measured end to end: a
/// pane with 363 rows of shell output went to `max_offset_from_bottom: 0` the moment `claude`
/// started, answered every `pane.read` with exactly the viewport for the whole session, and handed
/// the 367-row shell era back untouched on exit with not one conversation row in it. So the ring
/// froze on the shell session and went on calling it complete, which is the pre-harness `git`
/// output an operator found one screen above a Claude conversation.
#[test]
fn the_shell_that_ran_before_a_harness_is_not_the_harness_panes_history() {
    let mut ring = once(&raw(
        &["git-log-1", "git-log-2", "git-log-3", "viewport"],
        20,
        1,
        false,
    ));
    assert!(
        ring.render().complete,
        "the shell era was this pane's whole history"
    );

    assert_eq!(ring.superseded(), 3);

    let doc = ring.render();
    assert!(doc.rows.is_empty(), "a shell session is not a conversation");
    assert_eq!(doc.total_rows, 0);
    assert_eq!(doc.from_top, 3);
    assert!(
        doc.capped,
        "could not read this pane's history, not this pane has none (#233)"
    );
    assert!(!doc.complete);
}

/// It runs on every poll for the harness's whole life, so it has to be free after the first one —
/// and a ring that never held anything has lost nothing to say it is capped about.
#[test]
fn superseding_a_ring_that_holds_nothing_claims_nothing_was_lost() {
    let mut ring = ScrollbackRing::default();
    assert_eq!(ring.superseded(), 0);
    let doc = ring.render();
    assert_eq!(doc.from_top, 0);
    assert!(!doc.capped);
    assert!(doc.complete);
}

/// The harness exits and herdr hands the ring straight back — measured: the shell era verbatim,
/// four rows longer for the command that ran it, and not one conversation row in it. So the pane
/// has a history again, and it joins the ring where the supersede cut it rather than claiming to
/// be the top of it.
///
/// The base has to move exactly once for that. `from_top` is what the client's copy is joined
/// onto, and a supersede that advanced it on every poll would rebase a document that never
/// changed, once every three seconds, for as long as the harness ran.
#[test]
fn a_pane_whose_harness_exited_takes_its_history_up_from_where_it_was_cut() {
    let mut ring = once(&raw(&["shell-1", "shell-2", "viewport"], 20, 1, false));
    assert_eq!(ring.superseded(), 2);
    assert_eq!(
        ring.superseded(),
        0,
        "it runs on every poll for the harness's whole life"
    );
    assert_eq!(ring.superseded(), 0);

    ring.ingest(&raw(
        &["shell-1", "shell-2", "claude", "exit", "viewport"],
        20,
        1,
        false,
    ));
    let doc = ring.render();
    assert_eq!(doc.from_top, 2, "the base moved once, when the rows went");
    assert_eq!(doc.total_rows, 4);
    assert!(doc.capped, "the harness's own era is still unreachable");
    assert_eq!(
        doc.era, 2,
        "the supersede and the refill are two eras, and the second of them is the whole point: \
         the refill lands exactly where a tail would land, so nothing but the era can tell a \
         client that the rows it is being handed are the ones it was already holding (#498)",
    );
}

/// The other direction, and the one that costs something if it is wrong: a ring that is *growing*
/// stays in its era. Every new era makes every consumer downstream throw away what it holds and
/// take the whole document again — so an era that moved on ordinary output would re-send the ring
/// on every poll, and a client that carries a parked reader by what arrives would stop carrying
/// them at all.
///
/// Trimming is growth's other half here. A row dropped off the top keeps every remaining row's
/// index, so what is held is still a run of the same era.
#[test]
fn a_ring_that_is_only_growing_stays_in_the_era_it_started_in() {
    let mut ring = ScrollbackRing::new(4);
    ring.ingest(&raw(&refs(&numbered(1, 2)), 20, 1, false));
    let era = ring.render().era;
    assert_eq!(era, 0, "a pane's first ring is the era it was born in");

    ring.ingest(&raw(&refs(&numbered(1, 4)), 20, 1, false));
    assert_eq!(ring.render().era, era, "stitching a tail on is not a new era");

    ring.ingest(&raw(&refs(&numbered(3, 9)), 20, 1, false));
    let doc = ring.render();
    assert!(
        doc.from_top > 0 && doc.capped,
        "the ring has to have trimmed, or nothing is tested"
    );
    assert_eq!(doc.era, era, "trimming keeps every row it kept, index and all");
}

/// **A ring of repeated lines stitches to the wrong place and says the history is complete.**
///
/// `overlap` finds the longest suffix of what is held that prefixes what arrived, and on output
/// that repeats itself the longest such run is not the true one. A burst that pushes the read
/// window clean past the held rows should be a gap — the honest discard `ScrollbackRing` was built
/// to take (ADR 0004) — but the repetition gives `overlap` something to match, so it splices at an
/// offset nothing chose and the hole simply vanishes.
///
/// What makes it the worst shape of bug this project knows: the document that comes out says
/// `complete: true` and `capped: false`. It is not a read that failed, it is a read that failed and
/// then looked exactly like one that worked — the same shape as the node that answered every
/// question correctly with a dead `observe` (#233).
///
/// Measured against a real herdr 0.9.0 (probe #522): `yes 'SAME LINE' | head -400`, a read, then
/// `head -4000`, a second read. The ring held herdr's absolute rows 0..361 and the incoming window
/// started at absolute 3403 — a 3041-row hole — and `ingest` answered `Stitched { added: 599 }`.
#[test]
fn a_pane_that_repeats_itself_is_not_spliced_over_a_hole_and_called_complete() {
    let same = vec!["SAME LINE"; 400];
    let mut ring = ScrollbackRing::default();
    // The first window starts at the top of herdr's history; it keeps 360 rows, so its last row is
    // herdr's absolute 359.
    assert!(matches!(
        ring.ingest(&at(&same, 40, false, Some(0))),
        Ingest::Fresh { .. }
    ));
    let held = ring.render().total_rows;

    // The next window starts at absolute 3403 — a 3041-row hole — and every row of it is a row the
    // ring already appears to hold. Nothing but the position can tell the two apart.
    let after = vec!["SAME LINE"; 1000];
    let landed = ring.ingest(&at(&after, 40, true, Some(3403)));

    let doc = ring.render();
    assert!(
        matches!(landed, Ingest::Gap { .. }),
        "a window that starts past everything held is a gap, however much the content repeats: \
         {landed:?} — the ring went from {held} rows to {}",
        doc.total_rows
    );
    assert!(
        !doc.complete,
        "and a document over a hole must never claim to be complete: {doc:?}"
    );
}

/// The other half of the same fact: a window that *does* continue the ring is joined at the
/// position it names, and the rows it re-sends are not appended a second time.
#[test]
fn a_window_that_continues_the_ring_is_joined_where_it_says_and_not_where_it_looks() {
    let same = vec!["SAME LINE"; 400];
    let mut ring = ScrollbackRing::default();
    ring.ingest(&at(&same, 40, false, Some(0)));
    let held = ring.render().total_rows;
    assert_eq!(held, 360, "400 read, 40 of them the viewport");

    // Starts at 300 — sixty rows back inside what is held — and runs 200 rows further on.
    let next = vec!["SAME LINE"; 200];
    let landed = ring.ingest(&at(&next, 40, false, Some(300)));
    assert_eq!(
        landed,
        Ingest::Stitched { added: 100 },
        "160 kept rows arrive, 60 of them already held: {landed:?}"
    );
    assert_eq!(ring.render().total_rows, 460);
}

/// **A gap is refilled rather than discarded — and only when the rows demonstrably continue.**
///
/// The ring's answer to a gap was to throw the operator's history away, which was the only honest
/// answer while `pane.read recent` was the sole way in: capped at 1000 rows, no offset (#51). herdr
/// 0.9 can address history by absolute row (#510), so the missing span is fetched and spliced.
///
/// The anchor is the whole of the safety. herdr's row numbers are positions in its *current* ring,
/// not identities — a retention trim renumbers them wholesale and reports nothing about how far
/// (#520) — so rows fetched against a ring that moved underneath would splice somebody else's
/// history in and call it this pane's. Measured refusing on exactly that (#523).
#[test]
fn a_gap_is_refilled_when_the_rows_continue_it_and_refused_when_they_do_not() {
    let first: Vec<String> = numbered(0, 399);
    let mut ring = ScrollbackRing::default();
    ring.ingest(&at(&refs(&first), 40, false, Some(0)));
    // 400 read, 40 of them the viewport: the ring holds herdr's absolute 0..359.
    assert_eq!(ring.render().total_rows, 360);

    // The next window starts at 500 — rows 360..499 never arrived.
    let later: Vec<String> = numbered(500, 699);
    let raw = at(&refs(&later), 40, true, Some(500));
    let gap = ring.gap_before(&raw).expect("a gap");
    assert_eq!(
        gap.hole,
        (360, 499),
        "the hole is named by position, not guessed from the text"
    );
    assert_eq!(
        gap.fetch,
        (352, 499),
        "and the fetch reaches back over the ring's tail, which is what proves it is the same history"
    );

    // Rows that do not continue the ring — herdr trimmed underneath us and renumbered.
    let unrelated: Vec<String> = (0..148).map(|i| format!("someone-elses-{i}")).collect();
    assert!(
        !ring.clone().splice(unrelated, &raw),
        "rows that do not continue what is held are refused, and the discard stands"
    );

    // The real ones. The anchor overlaps the ring's tail, which is what proves they are the same
    // history; the splice keeps only what is past it.
    let missing: Vec<String> = numbered(352, 499);
    let spliced = ring.splice(missing, &raw);
    assert!(spliced, "rows that continue the ring are spliced");
    assert_eq!(ring.render().total_rows, 500, "0..499 held, with the hole closed");

    let landed = ring.ingest(&raw);
    assert_eq!(
        landed,
        Ingest::Stitched { added: 160 },
        "and the read that revealed the gap then joins normally: {landed:?}"
    );
    let doc = ring.render();
    assert_eq!(doc.total_rows, 660, "0..659");
    let rows = lines_of(&mut ring);
    assert_eq!(rows.first().map(String::as_str), Some("line-0"));
    assert_eq!(rows.last().map(String::as_str), Some("line-659"));
    assert!(
        rows.windows(2).all(|w| w[0] != w[1]),
        "no row is repeated across the join"
    );
}
