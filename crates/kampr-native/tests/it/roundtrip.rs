use alacritty_terminal::Term;
use alacritty_terminal::event::VoidListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::{Config, TermMode};
use alacritty_terminal::vte::ansi::{Color as AColor, Processor, StdSyncHandler};
use kampr_native::Screen;
use kampr_term::{Cell, Color, Emulator};

struct Size(u16, u16);

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.1 as usize
    }
    fn screen_lines(&self) -> usize {
        self.1 as usize
    }
    fn columns(&self) -> usize {
        self.0 as usize
    }
}

/// alacritty read directly rather than through anything `kampr-native` derives from it, so a
/// fault in the conversion shows up here instead of being agreed with.
struct Reference {
    term: Term<VoidListener>,
    parser: Processor<StdSyncHandler>,
}

impl Reference {
    fn new(cols: u16, rows: u16, history: usize) -> Self {
        let config = Config {
            scrolling_history: history,
            ..Config::default()
        };
        Self {
            term: Term::new(config, &Size(cols, rows), VoidListener),
            parser: Processor::new(),
        }
    }

    fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
        if self.parser.sync_timeout().sync_timeout().is_some() {
            self.parser.stop_sync(&mut self.term);
        }
    }

    fn resize(&mut self, cols: u16, rows: u16) {
        self.term.resize(Size(cols, rows));
    }

    fn row_text(&self, row: u16) -> String {
        let grid = self.term.grid();
        (0..grid.columns())
            .map(|c| &grid[Line(row as i32)][Column(c)])
            .filter(|c| !c.flags.contains(Flags::WIDE_CHAR_SPACER))
            .map(|c| c.c)
            .collect()
    }
}

fn colour(c: AColor) -> Color {
    match c {
        AColor::Spec(rgb) => Color::Rgb(rgb.r, rgb.g, rgb.b),
        AColor::Indexed(i) => Color::Indexed(i),
        AColor::Named(n) if (n as usize) < 16 => Color::Indexed(n as u8),
        AColor::Named(_) => Color::Default,
    }
}

fn mismatch(got: &Emulator, want: &Reference) -> Option<String> {
    let term = &want.term;
    let g = term.grid();
    let (cols, rows) = (g.columns() as u16, g.screen_lines() as u16);
    let grid = got.grid();
    if (grid.cols(), grid.rows()) != (cols, rows) {
        return Some(format!("size {}x{}", grid.cols(), grid.rows()));
    }
    for r in 0..rows {
        let mut widened = false;
        for c in 0..cols {
            let v = &g[Line(r as i32)][Column(c as usize)];
            if std::mem::take(&mut widened) {
                if !k_tail(grid, c, r) || v.c != ' ' {
                    return Some(format!(
                        "a widened emoji at ({c},{r}) took a column that was not blank"
                    ));
                }
                continue;
            }
            let k: &Cell = grid.cell(c, r).unwrap();
            let f = v.flags;
            let lead = c > 0
                && g[Line(r as i32)][Column(c as usize - 1)]
                    .flags
                    .contains(Flags::WIDE_CHAR);
            let tail = c + 1 < cols
                && g[Line(r as i32)][Column(c as usize + 1)]
                    .flags
                    .contains(Flags::WIDE_CHAR_SPACER);
            let half =
                (f.contains(Flags::WIDE_CHAR) && !tail) || (f.contains(Flags::WIDE_CHAR_SPACER) && !lead);
            let text: String = if half || f.contains(Flags::LEADING_WIDE_CHAR_SPACER) || v.c == '\t' {
                " ".into()
            } else {
                std::iter::once(v.c)
                    .chain(v.zerowidth().unwrap_or_default().iter().copied())
                    .collect()
            };
            // alacritty spends one column on a variation-selector emoji (`❤️`); the frames spend
            // two, as `kampr-term` and herdr do, when the column after it is blank.
            widened = !f.contains(Flags::WIDE_CHAR) && text.contains('\u{fe0f}') && k_tail(grid, c + 1, r);
            let wrong = if f.contains(Flags::WIDE_CHAR_SPACER) && !half {
                (!k.is_tail()).then_some("wanted the tail of a wide glyph")
            } else if k.cluster() != text {
                Some("text")
            } else if (k.fg, k.bg) != (colour(v.fg), colour(v.bg)) {
                Some("colour")
            } else if (
                k.attrs.bold,
                k.attrs.dim,
                k.attrs.italic,
                k.attrs.underline,
                k.attrs.reverse,
                k.attrs.strike,
                k.attrs.hidden,
            ) != (
                f.contains(Flags::BOLD),
                f.contains(Flags::DIM),
                f.contains(Flags::ITALIC),
                f.intersects(Flags::ALL_UNDERLINES),
                f.contains(Flags::INVERSE),
                f.contains(Flags::STRIKEOUT),
                f.contains(Flags::HIDDEN),
            ) {
                Some("attributes")
            } else if k.link.map(|id| grid.links[id as usize].as_str())
                != v.hyperlink().as_ref().map(|h| h.uri())
            {
                Some("link")
            } else if f.contains(Flags::WIDE_CHAR) && !half && !k_tail(grid, c + 1, r) {
                Some("wide glyph lost its tail")
            } else {
                None
            };
            if let Some(wrong) = wrong {
                return Some(format!(
                    "{wrong} at ({c},{r}): got {k:?}, wanted {text:?} {f:?}; row {:?}",
                    grid.row_text(r)
                ));
            }
        }
    }
    let point = g.cursor.point;
    let want_cursor = (
        (point.column.0 as u16).min(cols - 1),
        point.line.0 as u16,
        term.mode().contains(TermMode::SHOW_CURSOR),
    );
    if got.cursor() != want_cursor {
        return Some(format!("cursor {:?}, wanted {want_cursor:?}", got.cursor()));
    }
    None
}

fn k_tail(grid: &kampr_term::Grid, c: u16, r: u16) -> bool {
    grid.cell(c, r).is_some_and(Cell::is_tail)
}

fn assert_matches(what: &str, got: &Emulator, want: &Reference) {
    if let Some(m) = mismatch(got, want) {
        panic!("{what}: {m}");
    }
}

/// Feeds `bytes` in `chunk`-sized pieces. After every piece, a consumer that has followed every
/// diff and a fresh one painted from a full frame must both hold exactly alacritty's screen.
fn round_trip(name: &str, cols: u16, rows: u16, bytes: &[u8], chunk: usize) {
    let mut screen = Screen::new(cols, rows, 500);
    let mut reference = Reference::new(cols, rows, 500);
    let mut follower = Emulator::new(cols, rows);
    for (i, piece) in bytes.chunks(chunk).enumerate() {
        screen.feed(piece);
        reference.process(piece);
        if let Some(diff) = screen.frame_diff() {
            follower.feed(&diff);
        }
        let mut fresh = Emulator::new(cols, rows);
        fresh.feed(&screen.frame_full());
        for (how, consumer) in [("full", &fresh), ("diffs", &follower)] {
            if let Some(m) = mismatch(consumer, &reference) {
                panic!("{name} chunk {i} (by {chunk}), {how}: {m}");
            }
        }
    }
}

fn shell() -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..120 {
        out.extend(format!("\x1b[32m$\x1b[0m ls -la /tmp/{i}\r\n\x1b[1;34mdir{i}\x1b[0m  file-{i}.txt  \x1b[31merr\x1b[0m\r\n").bytes());
    }
    out.extend(b"\x1b[32m$\x1b[0m ");
    out
}

fn scroll_region() -> Vec<u8> {
    let mut out = b"\x1b[H\x1b[2Jheader\x1b[10;1Hstatus line\x1b[2;9r\x1b[2;1H".to_vec();
    for i in 0..30 {
        out.extend(format!("region row {i}\r\n").bytes());
    }
    out.extend(b"\x1b[r\x1b[5;1H\x1b[2L inserted\x1b[7;1H\x1b[1M\x1b[3;3H\x1b[3P\x1b[4;2H\x1b[4@\x1b[6;1H\x1b[5X\x1b[2S\x1b[1T\x1b[HM\x1bM\x1bM");
    out
}

fn relative() -> Vec<u8> {
    b"\x1b[H\x1b[2Jfirst\r\x1b[1Bsecond\x1b[12Gcol twelve\r\x1b[1B\x1b[3Cthree in\x1b[2A\x1b[Dback\x1b[5;20Hfar\x1b[E next line\x1b[F prev\x1b[3dthird row\x1b[7`col7\tTAB\x08\x08bs\x1b7saved\x1b[10;10H\x1b8restored".to_vec()
}

fn alternate() -> Vec<u8> {
    let mut out = b"primary text\r\nmore primary\r\n".to_vec();
    out.extend(b"\x1b[?1049h\x1b[H\x1b[2J\x1b[44m\x1b[2;5Hin the alternate\x1b[0m\x1b[?25l");
    out.extend(b"\x1b[8;1H\x1b[7mbar\x1b[K\x1b[0m");
    out.extend(b"\x1b[?25h\x1b[?1049lback on primary\r\n\x1b[?47hshort alt\x1b[?47l");
    out
}

fn wide() -> Vec<u8> {
    let mut out = b"\x1b[H\x1b[2J".to_vec();
    out.extend("中文字符 mixed ascii 漢字かなカナ 한국어\r\n".bytes());
    out.extend("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLM中".bytes());
    out.extend("\r\n\x1b[3;1H字字字\x1b[3;2Hx\x1b[3;5Hy".bytes());
    out.extend("\r\ne\u{301}t\u{e9} cafe\u{301} \u{2764}\u{fe0f} \u{1f600} \u{2705}\r\n".bytes());
    out.extend("\x1b[6;1H\x1b[45m 背景 \x1b[0m\x1b[6;3H\x1b[K".bytes());
    out
}

fn colours() -> Vec<u8> {
    let mut out = b"\x1b[H\x1b[2J".to_vec();
    for i in 0..48u16 {
        out.extend(format!("\x1b[38;5;{}m\x1b[48;5;{}m{:02}", i * 5, 255 - i, i % 100).bytes());
    }
    out.extend(b"\x1b[0m\r\n");
    for i in 0..20u16 {
        out.extend(format!("\x1b[38;2;{};{};{}mT\x1b[48;2;1;2;{}mB", i * 12, 255 - i, i, i).bytes());
    }
    out.extend(b"\x1b[0m\r\n\x1b[1mbold\x1b[22m \x1b[2mdim\x1b[22m \x1b[3mitalic\x1b[23m \x1b[4munder\x1b[24m \x1b[7mrev\x1b[27m");
    out.extend(b" \x1b[1;2;3;4;7mall\x1b[0m \x1b[91;102mbright\x1b[39;49m\r\n");
    out.extend(b"\x1b[41m\x1b[K\x1b[0m\r\n\x1b[43mpart\x1b[0m\x1b[1K\r\n\x1b[46m\x1b[2K\x1b[0m");
    out.extend(b"\x1b[10;1H\x1b[44m\x1b[J\x1b[0m\x1b[12;5H\x1b[42m\x1b[1J\x1b[0m");
    out
}

/// What herdr's stream carried and vt100 dropped: strikethrough, concealed text, every underline
/// style (which `kampr-term` holds as one), and OSC 8 links in both terminators.
fn styles() -> Vec<u8> {
    let mut out =
        b"\x1b[H\x1b[2J\x1b[9mstruck\x1b[29m \x1b[8mhidden\x1b[28m \x1b[4:3mcurly\x1b[4:0m ".to_vec();
    out.extend(b"\x1b[21mdouble\x1b[24m \x1b[1;9;31mall\x1b[0m\r\n");
    out.extend(b"\x1b]8;;https://a.example/x\x1b\\a link\x1b]8;;\x1b\\ plain ");
    out.extend(b"\x1b]8;id=7;https://b.example/?q=1\x07\x1b[32mgreen link\x1b[0m\x1b]8;;\x07\r\n");
    out.extend(b"\x1b]8;;https://a.example/x\x1b\\again\x1b]8;;\x1b\\\x1b[3;3H\x1b[9mover\x1b[0m");
    out.extend(b"\x1b[4;1H\x1b]8;;https://wrap.example\x1b\\");
    out.extend("a link long enough to wrap onto the next row ".repeat(2).bytes());
    out.extend(b"\x1b]8;;\x1b\\\x1b[2;3H\x1b[K");
    out
}

/// Deterministic noise over everything a program might emit, so the property is exercised on
/// shapes nobody wrote down.
fn noise(seed: u64, len: usize) -> Vec<u8> {
    let mut state = seed;
    let mut next = move |n: u64| {
        state = state
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (state >> 33) % n
    };
    let words = [
        "hello",
        "中文",
        "wörld",
        "e\u{301}",
        "──┼──",
        "⏺",
        "✻",
        "  ",
        "x",
        "かな",
        "🙂",
    ];
    let mut out = Vec::new();
    while out.len() < len {
        let s = match next(16) {
            0..=4 => words[next(words.len() as u64) as usize].to_string(),
            5 => format!("\x1b[{};{}H", next(14) + 1, next(42) + 1),
            6 => format!(
                "\x1b[{}{}",
                next(4),
                ["A", "B", "C", "D", "G", "d", "E", "F"][next(8) as usize]
            ),
            7 => ["\r\n", "\n", "\r", "\t", "\x08"][next(5) as usize].to_string(),
            8 => format!(
                "\x1b[{}m",
                [0, 1, 2, 3, 4, 7, 22, 31, 42, 93, 104, 39, 49][next(13) as usize]
            ),
            9 => format!(
                "\x1b[38;5;{}m\x1b[48;2;{};{};{}m",
                next(256),
                next(256),
                next(256),
                next(256)
            ),
            10 => format!(
                "\x1b[{}{}",
                next(3),
                ["J", "K", "X", "P", "@", "L", "M", "S", "T"][next(9) as usize]
            ),
            11 => match next(4) {
                0 => format!("\x1b[{};{}r", next(5) + 1, next(8) + 6),
                1 => "\x1b[r".into(),
                2 => "\x1bM".into(),
                _ => "\x1b7\x1b[3;3Hsaved\x1b8".into(),
            },
            12 => ["\x1b[?1049h", "\x1b[?1049l", "\x1b[?25l", "\x1b[?25h"][next(4) as usize].to_string(),
            _ => "the quick brown fox jumps over the lazy dog ".to_string(),
        };
        out.extend(s.bytes());
    }
    out
}

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn table(cases: &[(&str, u16, u16, Vec<u8>)], chunks: &[usize]) {
    for (name, cols, rows, bytes) in cases {
        for &chunk in chunks {
            round_trip(name, *cols, *rows, bytes, chunk);
        }
    }
}

#[test]
fn every_frame_rebuilds_alacrittys_screen_in_kampr_term() {
    table(
        &[
            ("shell output that scrolls", 40, 10, shell()),
            ("scroll regions and line edits", 30, 10, scroll_region()),
            ("relative motion", 40, 10, relative()),
            ("the alternate screen", 30, 10, alternate()),
            ("wide glyphs and marks", 40, 8, wide()),
            ("256 and truecolour, erases on a colour", 50, 14, colours()),
            (
                "strikethrough, concealed, underline styles and links",
                40,
                8,
                styles(),
            ),
        ],
        &[1, 13, usize::MAX],
    );
}

#[test]
fn noise_round_trips_too() {
    table(
        &[
            ("noise 1", 42, 14, noise(1, 4000)),
            ("noise 2", 42, 14, noise(2, 4000)),
            ("noise 3", 17, 5, noise(3, 3000)),
        ],
        &[7, 61, usize::MAX],
    );
}

#[test]
fn claude_round_trips_in_both_renderers() {
    table(
        &[
            (
                "Claude, fullscreen renderer (#587)",
                100,
                30,
                fixture("claude-fullscreen-turn.bin"),
            ),
            (
                "Claude, classic renderer (#588)",
                100,
                30,
                fixture("claude-default-turn.bin"),
            ),
        ],
        &[97, usize::MAX],
    );
}

#[test]
fn a_resized_screen_is_rebuilt_from_a_blank_grid_of_the_new_size() {
    for (cols, rows) in [(20, 6), (60, 20), (40, 3)] {
        let mut screen = Screen::new(40, 10, 100);
        let mut reference = Reference::new(40, 10, 100);
        let bytes = shell();
        screen.feed(&bytes);
        reference.process(&bytes);
        screen.frame_diff();
        screen.resize(cols, rows);
        reference.resize(cols, rows);
        assert_eq!(screen.size(), (cols, rows));
        let mut follower = Emulator::new(cols, rows);
        follower.feed(&screen.frame_diff().expect("a resize owes a repaint"));
        assert_matches(&format!("resize to {cols}x{rows}"), &follower, &reference);
        let more = b"after the resize\r\n\x1b[31mred\x1b[0m";
        screen.feed(more);
        reference.process(more);
        follower.feed(&screen.frame_diff().unwrap());
        assert_matches(
            &format!("resize to {cols}x{rows}, then output"),
            &follower,
            &reference,
        );
    }
}

#[test]
fn nothing_changed_is_no_frame_and_a_moved_cursor_is_one() {
    let mut screen = Screen::new(20, 4, 0);
    screen.feed(b"hi");
    assert!(screen.frame_diff().is_some());
    assert_eq!(screen.frame_diff(), None);
    screen.feed(b"\x1b[3;4H");
    let frame = screen.frame_diff().expect("a cursor move is a frame");
    assert!(
        frame.ends_with(b"\x1b[3;4H\x1b[?25h"),
        "{:?}",
        String::from_utf8_lossy(&frame)
    );
    screen.feed(b"\x1b[?25l");
    assert_eq!(screen.frame_diff().as_deref(), Some(&b"\x1b[?25l\x1b[3;4H"[..]));
}

#[test]
fn frames_use_only_what_kampr_term_parses() {
    let mut screen = Screen::new(40, 10, 100);
    screen.feed(&relative());
    screen.feed(&scroll_region());
    screen.feed(&styles());
    let mut frames = vec![screen.frame_full()];
    frames.extend(screen.frame_diff());
    for frame in frames {
        let text = String::from_utf8(frame).unwrap();
        let mut rest = text.as_str();
        while let Some(i) = rest.find('\x1b') {
            rest = &rest[i + 1..];
            if let Some(link) = rest.strip_prefix("]8;;") {
                let end = link.find("\x1b\\").expect("a link ends in ST");
                assert!(!link[..end].contains('\x1b'), "{link:?}");
                rest = &link[end + 2..];
                continue;
            }
            let end = rest.find(|c: char| c.is_ascii_alphabetic()).unwrap();
            let seq = &rest[..=end];
            let ok = seq.starts_with('[')
                && (seq.ends_with('H')
                    || seq.ends_with('m')
                    || seq == "[K"
                    || seq == "[2J"
                    || seq == "[?25h"
                    || seq == "[?25l");
            assert!(ok, "a frame carried ESC{seq}");
            rest = &rest[end + 1..];
        }
    }
}

#[test]
fn a_glyph_the_two_emulators_disagree_on_costs_one_cell_and_not_the_rest_of_the_row() {
    let cases: &[(&str, &str)] = &[
        (
            "a ZWJ family, one cluster to kampr-term and two to alacritty",
            "\u{1f468}\u{200d}\u{1f469}",
        ),
        ("a flag, two regional indicators", "\u{1f1e6}\u{1f1fa}"),
        ("a spacing mark", "\u{915}\u{93e}"),
    ];
    for (name, glyph) in cases {
        let mut screen = Screen::new(30, 2, 0);
        let mut reference = Reference::new(30, 2, 0);
        let bytes = format!("a{glyph}bc after");
        screen.feed(bytes.as_bytes());
        reference.process(bytes.as_bytes());
        let mut follower = Emulator::new(30, 2);
        follower.feed(&screen.frame_diff().unwrap());
        let mut fresh = Emulator::new(30, 2);
        fresh.feed(&screen.frame_full());
        let want = reference.row_text(0);
        let at = want.find("bc after").unwrap();
        let col = want[..at]
            .chars()
            .map(|c| unicode_width::UnicodeWidthChar::width(c).unwrap_or(0))
            .sum::<usize>();
        for (how, got) in [("diffs", &follower), ("full", &fresh)] {
            let grid = got.grid();
            assert_eq!(
                grid.cell(col as u16, 0).unwrap().ch,
                'b',
                "{name}, {how}: {:?}",
                grid.row_text(0)
            );
            assert_eq!(got.cursor(), screen.cursor(), "{name}, {how}");
        }
    }
}
