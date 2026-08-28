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
            "a zero-width mark rides on its base rather than taking a column",
            "e\u{301}f".as_bytes(),
            "e\u{301}f",
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

/// Probe #223: a zero-width code point has no column of its own, and dropping it loses the accent
/// for good — herdr keeps it on the base and addresses the next glyph at base + the *cluster's*
/// width, so the emulator has to as well or the mark never reaches the phone.
#[test]
fn a_zero_width_code_point_rides_on_the_cell_before_it() {
    let cases: &[(&str, &str, &[&str])] = &[
        ("a combining mark", "e\u{301}f", &["e\u{301}", "f"]),
        ("two marks stack", "x\u{301}\u{302}y", &["x\u{301}\u{302}", "y"]),
        ("a variation selector", "A\u{FE0F}B", &["A\u{FE0F}", "B"]),
        (
            "a ZWJ sequence is one cell",
            "ZZ\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}XY",
            &[
                "Z",
                "Z",
                "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}",
                "",
                "X",
                "Y",
            ],
        ),
        (
            "a flag is one cell",
            "QQ\u{1F1EC}\u{1F1E7}XY",
            &["Q", "Q", "\u{1F1EC}\u{1F1E7}", "", "X", "Y"],
        ),
        (
            "a third regional indicator starts a new cell",
            "\u{1F1EC}\u{1F1E7}\u{1F1EB}",
            &["\u{1F1EC}\u{1F1E7}", "", "\u{1F1EB}"],
        ),
        (
            "a skin-tone modifier is one cell",
            "\u{1F44D}\u{1F3FD}Z",
            &["\u{1F44D}\u{1F3FD}", "", "Z"],
        ),
        (
            "a keycap widens its base to two columns",
            "1\u{FE0F}\u{20E3}Z",
            &["1\u{FE0F}\u{20E3}", "", "Z"],
        ),
        (
            "an emoji presentation selector widens its base to two columns",
            "\u{2764}\u{FE0F}Z",
            &["\u{2764}\u{FE0F}", "", "Z"],
        ),
        ("a mark with no base is dropped", "\u{301}ab", &["a", "b"]),
        (
            "a mark rides on a blank cell, as herdr lets it",
            "A \u{301}B",
            &["A", " \u{301}", "B"],
        ),
        (
            "a virama joins the consonant after it into one conjunct",
            "\u{915}\u{94D}\u{937}",
            &["\u{915}\u{94D}\u{937}", ""],
        ),
    ];
    for (name, input, want) in cases {
        let mut t = Emulator::new(20, 2);
        t.feed(input.as_bytes());
        let got: Vec<String> = t
            .grid()
            .row(0)
            .iter()
            .take(want.len())
            .map(|c| if c.is_tail() { String::new() } else { c.cluster() })
            .collect();
        assert_eq!(got, *want, "{name}");
    }
}

#[test]
fn a_cluster_of_marks_is_erased_with_the_cell_it_rides_on() {
    let mut t = Emulator::new(8, 1);
    t.feed("e\u{301}f".as_bytes());
    t.feed(b"\x1b[1;1Hx");
    assert_eq!(t.grid().row_text(0), "xf", "the accent goes with its base");

    let mut t = Emulator::new(8, 1);
    t.feed("ab\u{301}c".as_bytes());
    t.feed(b"\x1b[1;2H\x1b[K");
    assert_eq!(t.grid().row_text(0), "a");
}

#[test]
fn a_variation_selector_that_widens_its_base_takes_the_second_column_with_it() {
    let mut t = Emulator::new(8, 1);
    t.feed("\u{2764}\u{FE0F}Z".as_bytes());
    let row = t.grid().row(0);
    assert_eq!(row[0].cluster(), "\u{2764}\u{FE0F}");
    assert!(row[1].is_tail(), "the selector bought a second column");
    assert_eq!(row[2].ch, 'Z', "and the text after it moved over");
    assert_eq!(t.cursor().0, 3);
}

/// Every cell herdr 0.8.2 was measured to spend, base included, against the columns it spent on
/// it: the cluster contents come from `pane.read`, the column count from where herdr wrapped the
/// same string against the right margin of a 93-column pane.
fn cells(input: &str, upto: usize) -> Vec<String> {
    let mut t = Emulator::new(20, 2);
    t.feed(input.as_bytes());
    t.grid()
        .row(0)
        .iter()
        .take(upto)
        .map(|c| if c.is_tail() { String::new() } else { c.cluster() })
        .collect()
}

/// A conjoining jamo sequence is one grapheme however many lead jamo it stacks, and herdr spends
/// **two** columns on it — not the four or six `unicode-width` sums for `L L` and `L L L`.
/// Half-width jamo do not conjoin and stay one column each, and a jamo herdr cannot attach to
/// anything never reaches the node at all.
#[test]
fn a_hangul_jamo_sequence_is_one_cluster_of_two_columns() {
    let cases: &[(&str, &str, &[&str], u16)] = &[
        (
            "precomposed",
            "AB\u{AC01}CD",
            &["A", "B", "\u{AC01}", "", "C", "D"],
            6,
        ),
        (
            "conjoining L V T",
            "AB\u{1100}\u{1161}\u{11A8}CD",
            &["A", "B", "\u{1100}\u{1161}\u{11A8}", "", "C", "D"],
            6,
        ),
        (
            "two lead jamo are still one syllable block",
            "AB\u{1100}\u{1100}CD",
            &["A", "B", "\u{1100}\u{1100}", "", "C", "D"],
            6,
        ),
        (
            "three lead jamo do not buy a third column",
            "AB\u{1100}\u{1100}\u{1100}CD",
            &["A", "B", "\u{1100}\u{1100}\u{1100}", "", "C", "D"],
            6,
        ),
        (
            "a lead jamo swallows the precomposed syllable after it",
            "AB\u{1100}\u{AC01}CD",
            &["A", "B", "\u{1100}\u{AC01}", "", "C", "D"],
            6,
        ),
        (
            "two precomposed syllables are two cells",
            "AB\u{AC01}\u{AC01}CD",
            &["A", "B", "\u{AC01}", "", "\u{AC01}", ""],
            8,
        ),
        (
            "a trailing jamo rides on a precomposed syllable",
            "AB\u{AC00}\u{11A8}CD",
            &["A", "B", "\u{AC00}\u{11A8}", "", "C", "D"],
            6,
        ),
        (
            "old-Korean extended jamo conjoin the same way",
            "AB\u{A960}\u{1161}\u{D7CB}CD",
            &["A", "B", "\u{A960}\u{1161}\u{D7CB}", "", "C", "D"],
            6,
        ),
        (
            "half-width jamo do not conjoin and stay one column each",
            "AB\u{FFA1}\u{FFC2}CD",
            &["A", "B", "\u{FFA1}", "\u{FFC2}", "C", "D"],
            6,
        ),
        (
            "compatibility jamo are ordinary wide characters",
            "AB\u{3131}\u{314F}CD",
            &["A", "B", "\u{3131}", "", "\u{314F}", ""],
            8,
        ),
    ];
    for (name, input, want, cursor) in cases {
        assert_eq!(cells(input, want.len()), *want, "{name}");
        let mut t = Emulator::new(20, 2);
        t.feed(input.as_bytes());
        assert_eq!(t.cursor().0, *cursor, "{name}: columns spent");
    }
}

/// `unicode-width` calls a lone regional indicator one column. Herdr spends **two** — on the
/// first of an odd run, on a third one after a complete flag, and on one standing by itself —
/// and herdr's cell model is the one the grid has to agree with (probes #210, #213).
#[test]
fn an_unpaired_regional_indicator_is_two_columns_because_herdr_spends_two() {
    let cases: &[(&str, &str, &[&str], u16)] = &[
        (
            "on its own",
            "AB\u{1F1EB}CD",
            &["A", "B", "\u{1F1EB}", "", "C", "D"],
            6,
        ),
        (
            "at the left margin",
            "\u{1F1EB}CD",
            &["\u{1F1EB}", "", "C", "D"],
            4,
        ),
        (
            "a third one after a complete flag",
            "AB\u{1F1EC}\u{1F1E7}\u{1F1EB}CD",
            &["A", "B", "\u{1F1EC}\u{1F1E7}", "", "\u{1F1EB}", "", "C", "D"],
            8,
        ),
        (
            "a space between two of them keeps them apart",
            "\u{1F1EB} \u{1F1EB}",
            &["\u{1F1EB}", "", " ", "\u{1F1EB}", ""],
            5,
        ),
    ];
    for (name, input, want, cursor) in cases {
        assert_eq!(cells(input, want.len()), *want, "{name}");
        let mut t = Emulator::new(20, 2);
        t.feed(input.as_bytes());
        assert_eq!(t.cursor().0, *cursor, "{name}: columns spent");
    }
}

/// The cases the four hand-written rules got wrong in the other direction: a ZWJ only joins two
/// pictographs, so `X\u{200D}Y` is two cells and so is `\u{65E5}\u{200D}\u{672C}`; a prepend and
/// a spacing mark do join; and a zero-width space is not a mark, so it rides on nothing and is
/// dropped. All measured against herdr's wrap column on a 93-column pane.
#[test]
fn a_cluster_ends_where_herdr_ends_it_and_no_earlier() {
    let cases: &[(&str, &str, &[&str], u16)] = &[
        (
            "a ZWJ between two letters joins nothing",
            "X\u{200D}Y",
            &["X\u{200D}", "Y"],
            2,
        ),
        (
            "a ZWJ between two CJK glyphs joins nothing",
            "\u{65E5}\u{200D}\u{672C}",
            &["\u{65E5}\u{200D}", "", "\u{672C}", ""],
            4,
        ),
        (
            "a ZWJ between two pictographs joins them",
            "\u{1F468}\u{200D}\u{1F469}Z",
            &["\u{1F468}\u{200D}\u{1F469}", "", "Z"],
            3,
        ),
        (
            "a prepend takes the character after it",
            "AB\u{600}\u{661}CD",
            &["A", "B", "\u{600}\u{661}", "", "C", "D"],
            6,
        ),
        (
            "three prepends do not buy a third column",
            "\u{600}\u{600}\u{600}\u{661}Z",
            &["\u{600}\u{600}\u{600}\u{661}", "", "Z"],
            3,
        ),
        (
            "a spacing mark joins its base",
            "AB\u{915}\u{93E}CD",
            &["A", "B", "\u{915}\u{93E}", "", "C", "D"],
            6,
        ),
        (
            "a conjunct is one cell, a Tamil one is not",
            "\u{BA4}\u{BCD}\u{BA4}Z",
            &["\u{BA4}\u{BCD}", "\u{BA4}", "Z"],
            3,
        ),
        (
            "a zero-width space rides on nothing",
            "A\u{200B}B",
            &["A", "B"],
            2,
        ),
    ];
    for (name, input, want, cursor) in cases {
        assert_eq!(cells(input, want.len()), *want, "{name}");
        let mut t = Emulator::new(20, 2);
        t.feed(input.as_bytes());
        assert_eq!(t.cursor().0, *cursor, "{name}: columns spent");
    }
}

/// Every one of these panics in a debug build and wraps silently in a release one, and `feed`
/// runs inside the `Mutex<PaneState>` a pane's whole surface is served from — so a poisoned lock
/// takes attach, screen and the lag path with it.
#[test]
fn a_hostile_cursor_address_cannot_take_the_emulator_arithmetic_past_the_grid() {
    let cases: &[(&str, &[u8])] = &[
        ("absolute column then forward", b"\x1b[65535G\x1b[65535C"),
        ("absolute row then down", b"\x1b[65535d\x1b[65535B"),
        (
            "absolute column then a wide glyph",
            "\x1b[65535G\u{65E5}".as_bytes(),
        ),
        ("absolute column then a tab", b"\x1b[65535G\t"),
        ("absolute address past both edges", b"\x1b[65535;65535Hx"),
    ];
    for (name, bytes) in cases {
        let mut t = Emulator::new(80, 24);
        t.feed(bytes);
        let (col, row, _) = t.cursor();
        // A column *equal* to the width is the pending-wrap position a glyph in the last cell
        // leaves behind, and every path that reads the cursor already expects it.
        assert!(col <= 80, "{name}: cursor at column {col} of an 80-column grid");
        assert!(row < 24, "{name}: cursor at row {row} of a 24-row grid");
    }

    let mut t = Emulator::new(80, 24);
    t.feed(b"\x1b[65535;65535Hx");
    assert_eq!(t.grid().row_text(23), " ".repeat(79) + "x");
}

/// A pane herdr reported before it had a size at all. Nothing may divide, subtract or index its
/// way off a grid with no cells in it — and a grid has no cells in it whenever *either* dimension
/// is zero, which is what this test drove only one of. An 80x0 grid held the same nothing and
/// `scroll_up` rotated 80 cells of it: a slice assertion, so it panicked in release too.
#[test]
fn an_empty_grid_survives_being_driven() {
    for (cols, rows) in [(0, 0), (80, 0), (0, 24)] {
        let mut t = Emulator::new(cols, rows);
        t.feed(b"\x1b[5B\x1b[5C\x1b[2Jhello\r\n\t\x1b[1;1H\n\n");
        assert_eq!(t.grid().rows(), rows, "{cols}x{rows}");
        assert!(t.take_dirty().iter().all(|d| d.cells.is_empty()), "{cols}x{rows}");
    }
}

/// The widest grid there is, filled to its last cell. The cursor parks one column past the glyph
/// it wrote, so at `u16::MAX` the margin test was comparing 65 536 against 65 535 through a
/// `saturating_add` that had already clamped it — no wrap, a `col + width` that overflows, and a
/// glyph dropped without one wherever overflow checks are off.
#[test]
fn a_glyph_at_the_last_column_of_the_widest_grid_wraps_rather_than_overflowing() {
    let mut t = Emulator::new(u16::MAX, 2);
    t.feed("x".repeat(u16::MAX as usize + 4).as_bytes());

    assert_eq!(t.cursor(), (4, 1, true));
    assert_eq!(
        t.grid().row(1)[0].ch,
        'x',
        "the glyph past the margin is on the next row"
    );
}

/// `CSI 4:3 m` is undercurl. Flattened, its subparameter reads as a top-level `3` and turns the
/// run italic; `38:2::r:g:b` reads the colourspace id as red.
#[test]
fn a_colon_separated_subparameter_is_not_a_parameter_of_its_own() {
    let mut t = Emulator::new(20, 1);
    t.feed(b"\x1b[4:3mA\x1b[0m\x1b[38:2::10:20:30mB\x1b[0m\x1b[38:5:196mC\x1b[0m\x1b[4:0mD");
    let row = t.grid().row(0);
    assert!(row[0].attrs.underline, "4:3 is an underline style");
    assert!(!row[0].attrs.italic, "4:3 must not read its 3 as SGR 3");
    assert_eq!(row[1].fg, Color::Rgb(10, 20, 30));
    assert_eq!(row[2].fg, Color::Indexed(196));
    assert!(!row[3].attrs.underline, "4:0 turns it off");
}

/// What a scrollback ring sizes its grid from. Under by one column wraps a row and loses one off
/// the top of the document, with every row index still claiming otherwise.
#[test]
fn a_column_bound_is_never_under_what_the_emulator_actually_spends() {
    let cases = [
        "plain ascii",
        "\x1b[1;31mcoloured\x1b[0m and \x1b[38;2;1;2;3mtrue\x1b[m",
        "\u{65E5}\u{672C}\u{8A9E} wide",
        "e\u{301}f a combining mark",
        "\u{1F1EC}\u{1F1E7} a flag and \u{1F1EC} half of one",
        "\u{1100}\u{1161}\u{11A8} jamo",
        "\x1b]8;;https://herdr.dev\x1b\\LINK\x1b]8;;\x1b\\ after",
        "\ta\tb",
        "",
    ];
    // The shapes the walk over clusters has to get right, which the list above does not reach: a
    // cluster past the ceiling, one that grows into its second column with an SGR standing between
    // it and the mark that buys it, and a payload the emulator never sees.
    let hostile = [
        format!("a{}", "\u{301}".repeat(200)),
        format!("\u{1F468}{}", "\u{200D}\u{1F469}".repeat(20)),
        "\u{2764}\x1b[0m\u{FE0F} widened across an SGR".to_string(),
        format!("x\x1bP{}\x1b\\y", "q".repeat(64)),
    ];
    for case in cases.iter().copied().chain(hostile.iter().map(String::as_str)) {
        let bound = crate::column_bound(case);
        let mut t = Emulator::new(bound.max(1), 1);
        t.feed(case.as_bytes());
        assert!(
            t.cursor().0 <= bound,
            "{case:?}: bound {bound}, emulator reached {}",
            t.cursor().0
        );
    }
}

/// **A pane's content decides how much memory the node hands `Grid::new`.** `column_bound` sizes
/// the grid a whole scrollback ring is laid out on, and charging a column per *code point* charges
/// one for a mark the emulator spends nothing on: 80 columns of text wearing combining marks is
/// 288 KB of one row and was bounded at 65 535 columns. A `Cell` is 40 bytes and the ring holds
/// 20 000 rows, so that one row asked for 52 GB — `handle_alloc_error`, which aborts the process
/// rather than unwinding anywhere a `catch_unwind` could see it.
#[test]
fn a_row_of_combining_marks_is_bounded_by_the_columns_the_emulator_can_spend() {
    let mut row = String::new();
    for _ in 0..80 {
        row.push('a');
        for _ in 0..1800 {
            row.push('\u{301}');
        }
    }
    let bound = crate::column_bound(&row);
    let mut t = Emulator::new(bound.max(1), 1);
    t.feed(row.as_bytes());

    assert!(
        t.cursor().0 <= bound,
        "bound {bound}, emulator reached {}",
        t.cursor().0
    );
    assert!(
        bound <= 160,
        "{} bytes of 80 columns bounded at {bound}",
        row.len()
    );
}

/// A DCS, SOS, PM or APC payload is swallowed whole by `vte` and never reaches `print`, so the
/// emulator spends nothing on it — measured, cursor at column 0 for all four. `skip_sequence` knew
/// only `ESC [` and `ESC ]`, so 76 KB of sixel counted as 76 000 printable columns.
#[test]
fn a_string_sequence_the_emulator_never_sees_costs_no_columns() {
    for (name, intro) in [
        ("DCS", "\x1bP"),
        ("SOS", "\x1bX"),
        ("PM", "\x1b^"),
        ("APC", "\x1b_"),
    ] {
        for end in ["\x1b\\", "\x07"] {
            let payload = format!("x{intro}q{}{end}y", "q".repeat(500));
            let bound = crate::column_bound(&payload);
            let mut t = Emulator::new(bound.max(1), 1);
            t.feed(payload.as_bytes());
            assert!(t.cursor().0 <= bound, "{name}: bound {bound}");
            assert!(bound <= 8, "{name} terminated by {end:?}: bounded at {bound}");
        }
    }
}

/// **A cluster is rebuilt from scratch on every code point that joins it**, so an unbounded one is
/// quadratic: 4 KB of marks on one cell measured 5.9 ms, 16 KB 89 ms, 64 KB 1.43 s and 128 KB
/// 5.72 s — clean 4x per doubling, and all of it under the `Mutex<PaneState>` that `screen`,
/// `attach` and lag recovery take. Past the cap a code point is put as if it opened a cluster of
/// its own, which drops a mark exactly as one printed at column 0 is dropped.
#[test]
fn a_cluster_stops_growing_at_a_ceiling_no_real_content_reaches() {
    let mut hostile = String::from("a");
    for _ in 0..20_000 {
        hostile.push('\u{301}');
    }
    let mut t = Emulator::new(80, 1);
    t.feed(hostile.as_bytes());

    let row = t.grid().row(0);
    assert!(
        row[0].marks().len() <= crate::perform::MAX_CLUSTER_BYTES,
        "one cell wearing {} bytes of marks",
        row[0].marks().len()
    );
    assert_eq!(
        t.cursor().0,
        1,
        "a mark it cannot attach takes no column of its own"
    );

    let family = "\u{1F468}\u{200D}\u{1F469}\u{200D}\u{1F467}\u{200D}\u{1F466}";
    let mut t = Emulator::new(80, 1);
    t.feed(family.as_bytes());
    assert_eq!(
        t.grid().row(0)[0].cluster(),
        family,
        "real content is nowhere near it"
    );
}

/// **The link table is emptied only by `Grid::clear`** — a `full: true` frame, which is the first
/// frame of a stream and no other (#53). On a stream of diffs it grows for the life of the pane,
/// it was interned by a linear scan (5 000 URIs 18 ms, 20 000 227 ms, 80 000 5.2 s), and
/// `registry.rs` serialises every new entry to every viewer.
#[test]
fn a_pane_cannot_mint_hyperlinks_without_end() {
    let mut hostile = String::new();
    for i in 0..6_000 {
        hostile.push_str(&format!("\x1b]8;;https://herdr.dev/{i}\x1b\\x\x1b]8;;\x1b\\"));
    }
    let mut t = Emulator::new(80, 24);
    t.feed(hostile.as_bytes());

    assert_eq!(
        t.grid().links.len(),
        crate::grid::MAX_LINKS,
        "a table of {}",
        t.grid().links.len()
    );
    assert_eq!(
        t.grid().links[0],
        "https://herdr.dev/0",
        "what was interned before the ceiling still points where it pointed"
    );

    t.feed(b"\x1b[1;1H\x1b]8;;https://herdr.dev/late\x1b\\z");
    assert_eq!(
        t.grid().row(0)[0].link,
        None,
        "and a run past it renders unlinked rather than under somebody else's URI"
    );
    assert_eq!(t.grid().row(0)[0].ch, 'z');
}

/// A URI is content too. Nothing is truncated: half a URI points somewhere else, so one past the
/// ceiling renders as text with no link at all.
#[test]
fn a_uri_past_the_length_ceiling_is_refused_rather_than_cut_short() {
    let uri = format!("https://herdr.dev/{}", "a".repeat(crate::grid::MAX_LINK_BYTES));
    let mut t = Emulator::new(40, 1);
    t.feed(format!("\x1b]8;;{uri}\x1b\\LINK").as_bytes());

    assert!(t.grid().links.is_empty(), "nothing interned");
    assert_eq!(t.grid().row(0)[0].link, None);
    assert_eq!(t.grid().row_text(0), "LINK", "and the text still renders");
}

/// The contract of [`crate::column_bound`], driven over shapes a hand-written list does not reach:
/// deterministic strings built from the code points that make a cluster behave unlike its parts,
/// the sequences the emulator swallows whole, and the controls and erases that put a cell the walk
/// has not charged for under the next mark. Two of those — a `HT` before an emoji modifier, and an
/// erase under a cluster that had already bought its second column — were real holes this found.
///
/// Cursor addressing is deliberately absent: `CSI 5;40H` moves into whatever grid it is given, and
/// the grid `lay_out` gives is the bound itself, so the emulator clamps rather than needing room.
#[test]
fn no_string_of_escapes_marks_and_glyphs_finds_a_column_the_bound_missed() {
    let pool = [
        "a",
        "Z",
        " ",
        "\u{301}",
        "\u{FE0F}",
        "\u{FE0E}",
        "\u{200D}",
        "\u{20E3}",
        "\u{1F468}",
        "\u{1F469}",
        "\u{2764}",
        "\u{65E5}",
        "\u{1F1EC}",
        "\u{1F1E7}",
        "\u{1100}",
        "\u{1161}",
        "\u{11A8}",
        "\u{915}",
        "\u{94D}",
        "\u{937}",
        "\u{BA4}",
        "\u{BCD}",
        "\u{600}",
        "\u{0D4E}",
        "\u{1F3FD}",
        "\x1b[0m",
        "\x1b[1;31m",
        "\x1b[K",
        "\x1b[1K",
        "\x1b[2J",
        "\x1bPqqq\x1b\\",
        "\x1b_apc\x1b\\",
        "\x1b]8;;https://x/\x1b\\",
        "\t",
        "\u{0}",
        "\u{7}",
        "\u{8}",
    ];
    let mut seed: u64 = 0x2545F4914F6CDD1D;
    let mut next = move || {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize
    };
    for _ in 0..2_000 {
        let mut case = String::new();
        for _ in 0..1 + next() % 12 {
            case.push_str(pool[next() % pool.len()]);
        }
        let bound = crate::column_bound(&case);
        let mut t = Emulator::new(bound.max(1), 1);
        t.feed(case.as_bytes());
        assert!(
            t.cursor().0 <= bound,
            "{case:?}: bound {bound}, emulator reached {}",
            t.cursor().0
        );
    }
}
