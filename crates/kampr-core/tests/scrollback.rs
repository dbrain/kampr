use kampr_core::provider::RawScrollback;
use kampr_core::scrollback;
use kampr_core::wire::{Encoder, ServerMsg};
use kampr_term::Color;

fn raw(text: &str, cols: u16, viewport_rows: u16, scrollback_rows: u32) -> RawScrollback {
    RawScrollback {
        text: text.into(),
        cols,
        viewport_rows,
        scrollback_rows,
        truncated: false,
    }
}

#[test]
fn the_live_viewport_is_stripped_so_history_never_duplicates_the_grid() {
    let text = "old-1\nold-2\nlive-1\nlive-2\n";
    let doc = scrollback::render(&raw(text, 10, 2, 2));
    assert_eq!(doc.total_rows, 2);
    assert_eq!(doc.from_top, 0);
    assert!(doc.complete);
    let lines: Vec<String> = doc
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
        .collect();
    assert_eq!(lines, ["old-1", "old-2"]);
}

#[test]
fn rows_are_absolute_indices_from_the_top_of_the_ring() {
    let text = (1..=6).map(|i| format!("line-{i}\n")).collect::<String>();
    let doc = scrollback::render(&raw(&text, 10, 2, 10));
    assert_eq!(
        doc.total_rows, 10,
        "the ring is deeper than what herdr handed back"
    );
    assert_eq!(
        doc.from_top, 6,
        "10 rows of ring, 4 delivered -> they start at index 6"
    );
    assert_eq!(doc.rows.first().unwrap().row, 6);
    assert_eq!(doc.rows.last().unwrap().row, 9);
    assert!(!doc.complete);
}

#[test]
fn colour_survives_the_same_emulator_the_live_grid_uses() {
    let text = "\x1b[38;2;255;120;0mwarm\x1b[0m plain\n\x1b[1;31mbold-red\x1b[0m\nviewport\n";
    let doc = scrollback::render(&raw(text, 20, 1, 2));
    assert_eq!(doc.rows.len(), 2);
    let first = &doc.rows[0].cells;
    assert_eq!(first[0].fg, Color::Rgb(255, 120, 0));
    assert_eq!(first[5].fg, Color::Default, "SGR 0 resets between runs");
    let second = &doc.rows[1].cells;
    assert_eq!(second[0].fg, Color::Indexed(1));
    assert!(second[0].attrs.bold);
}

#[test]
fn a_bare_newline_still_returns_the_carriage() {
    let doc = scrollback::render(&raw("aaa\nb\nviewport\n", 10, 1, 2));
    let lines: Vec<String> = doc
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
        .collect();
    assert_eq!(lines, ["aaa", "b"], "pane.read separates rows with LF alone");
}

#[test]
fn an_empty_ring_renders_nothing() {
    let doc = scrollback::render(&raw("only-viewport\n", 20, 1, 0));
    assert!(doc.rows.is_empty());
    assert_eq!(doc.total_rows, 0);
    assert!(doc.complete);
}

#[test]
fn scrollback_serialises_to_the_documented_shape() {
    let text = "\x1b[31mred\x1b[0m\nplain\nviewport\n";
    let doc = scrollback::render(&raw(text, 10, 1, 2));
    let mut enc = Encoder::new();
    let msgs = enc.encode_scrollback("01J/w3:p2", &doc);
    let v = serde_json::to_value(msgs.last().unwrap()).unwrap();
    assert_eq!(v["t"], "scrollback");
    assert_eq!(v["pane"], "01J/w3:p2");
    assert_eq!(v["from_top"], 0);
    assert_eq!(v["total_rows"], 2);
    assert_eq!(v["complete"], true);
    assert_eq!(v["rows"][0]["row"], 0);
    assert_eq!(v["rows"][0]["runs"][0]["x"], "red");
    assert!(
        matches!(msgs.first(), Some(ServerMsg::Styles(_))),
        "new styles precede the rows that reference them"
    );
}
