use kampr_core::provider::RawScrollback;
use kampr_core::scrollback::{Ingest, ScrollbackRing};
use kampr_core::wire::{Encoder, ServerMsg};
use kampr_term::Color;

fn raw(lines: &[&str], cols: u16, viewport_rows: u16, truncated: bool) -> RawScrollback {
    let mut text: String = lines.iter().map(|l| format!("{l}\n")).collect();
    if lines.is_empty() {
        text.clear();
    }
    RawScrollback {
        text,
        cols,
        viewport_rows,
        truncated,
    }
}

fn numbered(from: usize, to: usize) -> Vec<String> {
    (from..=to).map(|i| format!("line-{i}")).collect()
}

fn refs(v: &[String]) -> Vec<&str> {
    v.iter().map(String::as_str).collect()
}

fn once(raw: &RawScrollback) -> ScrollbackRing {
    let mut ring = ScrollbackRing::default();
    ring.ingest(raw);
    ring
}

fn lines_of(ring: &ScrollbackRing) -> Vec<String> {
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
    let ring = once(&raw(&["old-1", "old-2", "live-1", "live-2"], 10, 2, false));
    let doc = ring.render();
    assert_eq!(doc.total_rows, 2);
    assert_eq!(doc.from_top, 0);
    assert!(doc.complete);
    assert!(!doc.capped);
    assert_eq!(lines_of(&ring), ["old-1", "old-2"]);
}

#[test]
fn colour_survives_the_same_emulator_the_live_grid_uses() {
    let ring = once(&raw(
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
    let ring = once(&raw(&["aaa", "b", "viewport"], 10, 1, false));
    assert_eq!(
        lines_of(&ring),
        ["aaa", "b"],
        "pane.read separates rows with LF alone"
    );
}

#[test]
fn an_empty_ring_renders_nothing() {
    let ring = once(&raw(&["only-viewport"], 20, 1, false));
    let doc = ring.render();
    assert!(doc.rows.is_empty());
    assert_eq!(doc.total_rows, 0);
    assert!(doc.complete);
}

#[test]
fn a_truncated_read_says_history_above_it_is_unreachable() {
    let ring = once(&raw(&["a", "b", "viewport"], 10, 1, true));
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
    assert_eq!(lines_of(&ring), numbered(1, 8));

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
    assert_eq!(lines_of(&ring), numbered(900, 905));
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
    assert_eq!(lines_of(&ring), numbered(7, 10));
    assert_eq!(ring.render().from_top, 6);
    assert_eq!(ring.render().total_rows, 4, "a depth, so the ring spans 6..10");
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
    let ring = once(&raw(&["\x1b[31mred\x1b[0m", "plain", "viewport"], 10, 1, true));
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
    assert_eq!(lines_of(&ring), numbered(1, 17));
}

/// The rows a restart keeps were read at the *new* width, so rendering them at the old one wraps
/// history that never wrapped.
#[test]
fn a_restarted_ring_renders_at_the_width_it_restarted_at() {
    let mut ring = ScrollbackRing::new(60);
    ring.ingest(&raw(&["short", "viewport"], 10, 1, false));
    ring.ingest(&raw(&["0123456789abcdefghij", "viewport"], 40, 1, false));
    assert_eq!(lines_of(&ring), ["0123456789abcdefghij"]);
}
