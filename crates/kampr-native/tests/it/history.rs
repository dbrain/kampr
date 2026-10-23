use kampr_core::scrollback::ScrollbackRing;
use kampr_native::Screen;
use kampr_term::Color;

fn lines(screen: &mut Screen, from: usize, to: usize) {
    for i in from..to {
        screen.feed(format!("line {i}\r\n").as_bytes());
    }
}

fn rows(screen: &Screen) -> Vec<String> {
    let first = screen.first_row();
    let (_, rows) = screen.size();
    screen
        .read_rows(first, first + screen.history_rows() + u32::from(rows) - 1)
        .unwrap()
}

#[test]
fn rows_that_scroll_off_enter_history_and_keep_their_number() {
    let mut screen = Screen::new(20, 5, 1000);
    lines(&mut screen, 0, 50);
    assert_eq!((screen.first_row(), screen.history_rows()), (0, 46));
    assert_eq!(screen.read_rows(0, 1).unwrap(), ["line 0", "line 1"]);
    assert_eq!(
        screen.read_rows(46, 47).unwrap(),
        ["line 46", "line 47"],
        "the screen follows history"
    );
    assert_eq!(screen.read_rows(49, 50).unwrap(), ["line 49", ""]);
    assert_eq!(screen.read_rows(50, 51), None, "past the screen's last row");
    assert_eq!(screen.read_rows(3, 2), None);
}

#[test]
fn a_full_history_drops_its_oldest_rows_without_renumbering_the_rest() {
    let mut screen = Screen::new(20, 5, 10);
    lines(&mut screen, 0, 50);
    assert_eq!((screen.first_row(), screen.history_rows()), (36, 10));
    assert_eq!(screen.read_rows(35, 36), None, "row 35 is gone");
    assert_eq!(screen.read_rows(36, 36).unwrap(), ["line 36"]);
    lines(&mut screen, 50, 53);
    assert_eq!(screen.first_row(), 39);
    assert_eq!(screen.read_rows(39, 39).unwrap(), ["line 39"]);
}

#[test]
fn one_feed_far_longer_than_any_history_still_numbers_every_row() {
    let mut screen = Screen::new(20, 5, 100);
    let bytes: String = (0..100_000).map(|i| format!("line {i}\r\n")).collect();
    screen.feed(bytes.as_bytes());
    assert_eq!((screen.first_row(), screen.history_rows()), (99_896, 100));
    assert_eq!(screen.read_rows(99_896, 99_896).unwrap(), ["line 99896"]);
    assert_eq!(screen.read_rows(99_999, 99_999).unwrap(), ["line 99999"]);
}

#[test]
fn a_scroll_of_a_whole_screen_at_a_time_is_counted_too() {
    let mut screen = Screen::new(20, 100, 100);
    screen.feed(b"\x1b[100;1Hmark");
    screen.feed("\x1b[99S".repeat(819).as_bytes());
    assert_eq!(screen.first_row() + screen.history_rows(), 819 * 99);
}

#[test]
fn every_way_history_is_emptied_still_never_reuses_a_number() {
    let cases: &[(&str, &[u8], &str)] = &[
        ("CSI 3 J, which leaves the screen", b"\x1b[3J", "line 26"),
        ("a full reset, which clears it too", b"\x1bc", "line 100"),
    ];
    for (name, erase, oldest) in cases {
        let mut screen = Screen::new(20, 5, 1000);
        lines(&mut screen, 0, 30);
        let end = screen.first_row() + screen.history_rows();
        screen.feed(erase);
        assert_eq!(screen.history_rows(), 0, "{name}");
        lines(&mut screen, 100, 110);
        assert!(
            screen.first_row() >= end,
            "{name}: first row {} reuses a number below {end}",
            screen.first_row()
        );
        let first = screen.first_row();
        assert_eq!(screen.read_rows(first, first).unwrap(), [*oldest], "{name}");
    }
}

#[test]
fn history_survives_the_alternate_screen_and_none_of_it_enters() {
    let mut screen = Screen::new(20, 5, 1000);
    lines(&mut screen, 0, 20);
    let before = rows(&screen);
    let held = screen.history_rows();
    screen.feed(b"\x1b[?1049h");
    assert!(screen.alternate_screen());
    for i in 0..30 {
        screen.feed(format!("alt {i}\r\n").as_bytes());
    }
    assert_eq!(screen.history_rows(), held, "the alternate screen has no history");
    assert_eq!(screen.read_rows(0, held - 1).unwrap(), before[..held as usize]);
    assert_eq!(
        screen.find("alt 3", false, None).hits.len(),
        0,
        "only the visible alternate rows are searched"
    );
    screen.feed(b"\x1b[?1049l");
    assert!(!screen.alternate_screen());
    lines(&mut screen, 20, 25);
    assert_eq!(screen.read_rows(held, held).unwrap(), ["line 16"]);
}

#[test]
fn a_taller_screen_takes_rows_back_from_history_as_terminals_do() {
    let mut screen = Screen::new(20, 5, 1000);
    lines(&mut screen, 0, 20);
    assert_eq!(screen.history_rows(), 16);
    screen.resize(20, 9);
    assert_eq!((screen.first_row(), screen.history_rows()), (0, 12));
    assert_eq!(
        screen.read_rows(12, 12).unwrap(),
        ["line 12"],
        "the first screen row is the newest pulled back"
    );
    screen.resize(20, 5);
    assert_eq!(screen.history_rows(), 16);
    assert_eq!(screen.read_rows(15, 15).unwrap(), ["line 15"]);
}

#[test]
fn the_scrollback_read_lays_out_on_the_registrys_own_ring() {
    let mut screen = Screen::new(30, 4, 1000);
    screen.feed(b"\x1b[31mred\x1b[0m plain \x1b[1;48;5;202mbold\x1b[0m\r\n");
    screen.feed("中文 wide\r\n".as_bytes());
    lines(&mut screen, 0, 6);
    let raw = screen.scrollback(None);
    assert_eq!(raw.first_row, Some(0));
    assert_eq!(raw.viewport_rows, 4);
    assert_eq!(raw.cols, Some(30));
    assert!(!raw.truncated);
    let mut ring = ScrollbackRing::default();
    ring.ingest(&raw);
    let doc = ring.render();
    assert_eq!(doc.total_rows, screen.history_rows());
    let text: Vec<String> = doc
        .rows
        .iter()
        .map(|r| {
            r.cells
                .iter()
                .filter(|c| !c.is_tail())
                .map(|c| c.cluster())
                .collect::<String>()
                .trim_end()
                .to_string()
        })
        .collect();
    assert_eq!(
        text,
        ["red plain bold", "中文 wide", "line 0", "line 1", "line 2"]
    );
    let first = &doc.rows[0].cells;
    assert_eq!(first[0].fg, Color::Indexed(1));
    assert_eq!(first[4].fg, Color::Default);
    assert!(first[10].attrs.bold && first[10].bg == Color::Indexed(202));
    let next = &doc.rows[1].cells[0];
    assert!(
        !next.attrs.bold && next.bg == Color::Default,
        "the pen does not leak past a row"
    );
    assert!(doc.rows[1].cells[1].is_tail());

    lines(&mut screen, 6, 9);
    let later = screen.scrollback(Some(4));
    assert_eq!(later.first_row, Some(4));
    assert!(later.truncated, "rows above the window exist");
    ring.ingest(&later);
    assert_eq!(
        ring.render().total_rows,
        screen.history_rows(),
        "a window from the ring's end stitches by position"
    );
}

#[test]
fn find_reports_rows_counted_up_from_the_live_row() {
    let mut screen = Screen::new(20, 4, 1000);
    screen.feed(b"Needle one\r\nhay\r\nneedle two\r\n");
    lines(&mut screen, 0, 5);
    screen.feed(b"a needle on screen");
    let found = screen.find("needle", false, None);
    let at: Vec<(u32, u16, u32, u16)> = found
        .hits
        .iter()
        .map(|h| (h.from_bottom, h.col, h.end_from_bottom, h.end_col))
        .collect();
    assert_eq!(at, [(8, 0, 8, 5), (6, 0, 6, 5), (0, 2, 0, 7)]);
    assert_eq!(found.total, 3);
    assert_eq!(found.hits[1].text, "needle two");
    assert_eq!(found.current, Some(2), "forward from the live row lands on it");
    assert_eq!(
        screen.find("needle", true, None).current,
        Some(1),
        "backward from the live row"
    );
    assert_eq!(screen.find("needle", true, Some(6)).current, Some(0));
    assert_eq!(screen.find("needle", false, Some(6)).current, Some(1));
    assert_eq!(
        screen.find("Needle", false, None).total,
        1,
        "a capital makes it exact"
    );
    assert_eq!(screen.find("absent", false, None).current, None);
}

#[test]
fn find_matches_across_a_soft_wrap() {
    let mut screen = Screen::new(10, 4, 1000);
    screen.feed(b"0123456 wrapped word\r\n");
    lines(&mut screen, 0, 4);
    let found = screen.find("wrapped", false, None);
    assert_eq!(found.total, 1);
    let hit = &found.hits[0];
    assert_eq!(
        (hit.from_bottom, hit.col, hit.end_from_bottom, hit.end_col),
        (6, 8, 5, 4)
    );
}

#[test]
fn a_narrower_screen_reflows_its_rows_and_history_keeps_the_width_it_was_written_at() {
    let mut screen = Screen::new(20, 5, 1000);
    for i in 0..4 {
        screen.feed(format!("{i}bcdefghijklmnopqrs\r\n").as_bytes());
    }
    screen.resize(10, 2);
    assert_eq!(screen.read_rows(0, 0).unwrap(), ["0bcdefghijklmnopqrs"]);
    assert_eq!(
        screen.history_rows(),
        4,
        "the top of the rewrapped screen went into history"
    );
    assert_eq!(screen.read_rows(3, 5).unwrap(), ["3bcdefghij", "klmnopqrs", ""]);
}

#[test]
fn a_wider_screen_rejoins_what_wrapped() {
    let mut screen = Screen::new(10, 4, 1000);
    screen.feed(b"0123456789abcdef\r\n$ ");
    assert_eq!(screen.read_rows(0, 2).unwrap(), ["0123456789", "abcdef", "$"]);
    screen.resize(20, 4);
    assert_eq!(screen.read_rows(0, 1).unwrap(), ["0123456789abcdef", "$"]);
    assert_eq!(screen.cursor(), (2, 1, true));
}
