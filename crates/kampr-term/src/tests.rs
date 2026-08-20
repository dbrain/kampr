use crate::{Color, Emulator};

fn text(t: &Emulator) -> Vec<String> {
    (0..t.grid().rows())
        .map(|r| {
            t.grid()
                .row(r)
                .iter()
                .map(|c| c.ch)
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect()
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
