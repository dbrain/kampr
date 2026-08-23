use crate::{Color, Emulator};

fn text(t: &Emulator) -> Vec<String> {
    (0..t.grid().rows()).map(|r| t.grid().row_text(r)).collect()
}

#[test]
fn absolute_addressing_places_text() {
    let mut t = Emulator::new(20, 4);
    t.feed(b"\x1b[2J\x1b[2;3Hhello\x1b[4;1Hworld");
    assert_eq!(text(&t), ["", "  hello", "", "world"]);
}

#[test]
fn sgr_colours_and_attributes() {
    let mut t = Emulator::new(20, 1);
    t.feed(b"\x1b[1;3;4;38;2;255;100;0mA\x1b[0m\x1b[38;5;196mB\x1b[0m\x1b[7mC\x1b[0m");
    let row = t.grid().row(0);
    assert_eq!(row[0].fg, Color::Rgb(255, 100, 0));
    assert!(row[0].attrs.bold && row[0].attrs.italic && row[0].attrs.underline);
    assert_eq!(row[1].fg, Color::Indexed(196));
    assert!(!row[1].attrs.bold, "SGR 0 must clear attributes, not just colour");
    assert!(row[2].attrs.reverse);
}

#[test]
fn osc8_hyperlinks_are_interned_per_run() {
    let mut t = Emulator::new(40, 1);
    t.feed(b"\x1b]8;;https://herdr.dev\x1b\\LINK\x1b]8;;\x1b\\ plain");
    assert_eq!(t.grid().links, ["https://herdr.dev"]);
    assert_eq!(t.grid().row(0)[0].link, Some(0));
    assert_eq!(t.grid().row(0)[5].link, None, "the empty-URI form closes the run");
}

#[test]
fn a_hyperlink_longer_than_vtes_osc_buffer_survives_whole() {
    // vte caps OSC payloads at 1024 bytes when built without `std`; with it the buffer grows.
    // A truncated URI is silent — the run still renders, it just points somewhere else.
    let uri = format!("https://herdr.dev/{}", "a".repeat(2000));
    let mut t = Emulator::new(40, 1);
    t.feed(format!("\x1b]8;;{uri}\x1b\\LINK\x1b]8;;\x1b\\").as_bytes());
    assert_eq!(t.grid().links, [uri]);
    assert_eq!(t.grid().row(0)[0].link, Some(0));
}

#[test]
fn erase_in_display_and_line() {
    let mut t = Emulator::new(10, 3);
    t.feed(b"aaaaaaaaaa\r\nbbbbbbbbbb\r\ncccccccccc");
    t.feed(b"\x1b[2;5H\x1b[K");
    assert_eq!(text(&t)[1], "bbbb");
    t.feed(b"\x1b[1;1H\x1b[J");
    assert_eq!(text(&t), ["", "", ""]);
}

#[test]
fn wrap_and_scroll_at_the_bottom() {
    let mut t = Emulator::new(4, 2);
    t.feed(b"abcdefgh");
    assert_eq!(
        text(&t),
        ["abcd", "efgh"],
        "wrap fills the last row without scrolling"
    );
    t.feed(b"ijkl");
    assert_eq!(text(&t), ["efgh", "ijkl"], "wrapping past the last row scrolls");
}

#[test]
fn cursor_visibility_follows_dec_mode_25() {
    let mut t = Emulator::new(4, 1);
    assert!(t.cursor().2);
    t.feed(b"\x1b[?25l");
    assert!(!t.cursor().2);
    t.feed(b"\x1b[?25h");
    assert!(t.cursor().2);
}

#[test]
fn synchronised_output_markers_are_inert() {
    let mut t = Emulator::new(6, 1);
    t.feed(b"\x1b[?2026hhi\x1b[?2026l");
    assert_eq!(text(&t)[0], "hi");
}

#[test]
fn dirty_rows_are_reported_once() {
    let mut t = Emulator::new(6, 3);
    t.take_dirty();
    t.feed(b"\x1b[2;1Hxx");
    let d = t.take_dirty();
    assert_eq!(d.len(), 1);
    assert_eq!(d[0].row, 1);
    assert!(t.take_dirty().is_empty());
}

#[test]
fn full_frame_reset_clears_stale_cells() {
    let mut t = Emulator::new(8, 2);
    t.feed(b"stale\r\ndata");
    t.reset();
    t.feed(b"\x1b[1;1Hnew");
    assert_eq!(text(&t), ["new", ""]);
}

// Probe #210: herdr advances two columns per double-width glyph and addresses the next one at
// col+2, so an emulator that advances one leaves a blank behind every wide character — and herdr
// never repaints a cell it believes already matches, so the gap is permanent.
#[test]
fn double_width_glyphs_occupy_two_columns() {
    let cases: &[(&str, &[u8], &str)] = &[
        (
            "herdr's own addressing",
            b"\x1b[1;1HAB\xe6\x97\xa5\x1b[1;5H\xe6\x9c\xac\x1b[1;7H\xe8\xaa\x9e\x1b[1;9HCD",
            "AB日本語CD",
        ),
        ("printed straight through", "AB日本語CD".as_bytes(), "AB日本語CD"),
        (
            "an astral emoji is one glyph, not two cells",
            b"\x1b[1;1HXY\xf0\x9f\x9a\x80\x1b[1;5HZW",
            "XY🚀ZW",
        ),
        (
            "a zero-width mark does not consume a column",
            "e\u{301}f".as_bytes(),
            "ef",
        ),
    ];
    for (name, bytes, want) in cases {
        let mut t = Emulator::new(20, 2);
        t.feed(bytes);
        assert_eq!(t.grid().row_text(0), *want, "{name}");
    }
}

#[test]
fn a_wide_glyph_is_addressable_at_the_column_herdr_puts_it_in() {
    let mut t = Emulator::new(20, 1);
    t.feed("AB日本語CD".as_bytes());
    let row = t.grid().row(0);
    assert_eq!(row[2].ch, '日');
    assert!(row[3].is_tail(), "column 3 is the other half of 日, not a blank");
    assert_eq!(row[4].ch, '本');
    assert_eq!(row[6].ch, '語');
    assert_eq!(row[8].ch, 'C');
    assert_eq!(t.cursor().0, 10, "the cursor advanced two columns per wide glyph");
}

#[test]
fn overwriting_either_half_of_a_wide_glyph_clears_the_other() {
    let mut t = Emulator::new(8, 1);
    t.feed("日本".as_bytes());
    t.feed(b"\x1b[1;1Hx");
    assert_eq!(
        t.grid().row_text(0),
        "x 本",
        "the orphaned right half goes with it"
    );

    let mut t = Emulator::new(8, 1);
    t.feed("日本".as_bytes());
    t.feed(b"\x1b[1;2Hx");
    assert_eq!(t.grid().row_text(0), " x本");

    let mut t = Emulator::new(8, 1);
    t.feed("ab".as_bytes());
    t.feed(b"\x1b[1;2H\xe6\x97\xa5");
    assert_eq!(t.grid().row_text(0), "a日");
}

#[test]
fn erasing_a_line_takes_a_wide_glyph_whole() {
    let mut t = Emulator::new(8, 1);
    t.feed("a日bc".as_bytes());
    t.feed(b"\x1b[1;3H\x1b[K");
    assert_eq!(
        t.grid().row_text(0),
        "a",
        "the lead at column 1 cannot survive its tail"
    );
}

#[test]
fn a_wide_glyph_that_does_not_fit_wraps_whole() {
    let mut t = Emulator::new(4, 2);
    t.feed("abc日".as_bytes());
    assert_eq!(t.grid().row_text(0), "abc");
    assert_eq!(t.grid().row_text(1), "日");
}
