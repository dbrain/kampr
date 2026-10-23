use super::Screen;
use crate::answer;
use crate::history::{History, Row as Rendered};
use alacritty_terminal::grid::{Cursor, Dimensions, Grid, Row};
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::term::cell::Cell;
use alacritty_terminal::vte::ansi::{CharsetIndex, Color, CursorShape, StandardCharset};
use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use kampr_core::provider::{ProgramState, ProgramStatus};
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use std::io::{Read as _, Write as _};

const VERSION: u32 = 1;
const MAGIC: &[u8] = b"kampr-screen\n";
/// Indices a program can set: the 256 palette entries (`OSC 4`), then foreground, background and
/// cursor (`OSC 10`/`11`/`12`).
const SETTABLE_COLOURS: usize = 259;
const LARGEST: usize = 1 << 22;

/// Everything a later byte can see, alacritty's private fields included: those are read by
/// probing the live terminal and putting back whatever the probe moved, and set again on load by
/// the sequences that set them.
#[derive(Serialize, Deserialize)]
struct Saved {
    version: u32,
    cols: u16,
    rows: u16,
    title: String,
    mode: u32,
    keyboard: (u8, u8),
    region: (i32, i32),
    tabs: Vec<u16>,
    shifted: bool,
    colours: Vec<(u16, [u8; 3])>,
    cursor_style: Option<(u8, bool)>,
    primary: Plane,
    alternate: Option<Plane>,
    history_first: u64,
    history: Vec<(String, String, bool, u16)>,
    known: usize,
    rebase: bool,
    program: Option<(u8, Option<String>, Option<String>)>,
    program_reports: u64,
}

const PROGRAM_STATES: [ProgramState; 5] = [
    ProgramState::Idle,
    ProgramState::Working,
    ProgramState::Blocked,
    ProgramState::Done,
    ProgramState::Error,
];

#[derive(Serialize, Deserialize)]
struct Plane {
    scrolled: usize,
    rows: Vec<Row<Cell>>,
    cursor: Pen,
    saved: Pen,
}

#[derive(Serialize, Deserialize)]
struct Pen {
    line: i32,
    col: usize,
    template: Cell,
    special: [bool; 4],
    wrap: bool,
}

const INDICES: [CharsetIndex; 4] = [
    CharsetIndex::G0,
    CharsetIndex::G1,
    CharsetIndex::G2,
    CharsetIndex::G3,
];

impl Pen {
    fn of(cursor: &Cursor<Cell>) -> Self {
        Self {
            line: cursor.point.line.0,
            col: cursor.point.column.0,
            template: cursor.template.clone(),
            special: INDICES.map(|i| cursor.charsets[i] == StandardCharset::SpecialCharacterAndLineDrawing),
            wrap: cursor.input_needs_wrap,
        }
    }

    fn cursor(&self) -> Cursor<Cell> {
        let mut cursor = Cursor {
            template: self.template.clone(),
            input_needs_wrap: self.wrap,
            ..Cursor::default()
        };
        cursor.point.line = Line(self.line);
        cursor.point.column = Column(self.col);
        for (i, special) in INDICES.into_iter().zip(self.special) {
            if special {
                cursor.charsets[i] = StandardCharset::SpecialCharacterAndLineDrawing;
            }
        }
        cursor
    }

    fn fits(&self, cols: u16, rows: u16) -> bool {
        (0..i32::from(rows)).contains(&self.line) && self.col < usize::from(cols)
    }
}

impl Plane {
    fn of(grid: &Grid<Cell>) -> Self {
        let scrolled = grid.history_size();
        Self {
            scrolled,
            rows: (-(scrolled as i32)..grid.screen_lines() as i32)
                .map(|l| grid[Line(l)].clone())
                .collect(),
            cursor: Pen::of(&grid.cursor),
            saved: Pen::of(&grid.saved_cursor),
        }
    }

    fn fits(&self, cols: u16, rows: u16) -> bool {
        self.rows.len() == self.scrolled + usize::from(rows)
            && self.rows.iter().all(|r| r.len() == usize::from(cols))
            && self.cursor.fits(cols, rows)
            && self.saved.fits(cols, rows)
    }

    fn put(&self, grid: &mut Grid<Cell>) -> bool {
        let lines = grid.screen_lines();
        let mut left = self.scrolled.saturating_sub(grid.history_size());
        while left > 0 {
            let n = left.min(lines);
            grid.scroll_up::<Color>(&(Line(0)..Line(lines as i32)), n);
            left -= n;
        }
        if grid.history_size() != self.scrolled {
            return false;
        }
        for (i, row) in self.rows.iter().enumerate() {
            grid[Line(i as i32 - self.scrolled as i32)] = row.clone();
        }
        grid.cursor = self.cursor.cursor();
        grid.saved_cursor = self.saved.cursor();
        true
    }
}

const MODES: [(TermMode, &str); 10] = [
    (TermMode::SHOW_CURSOR, "?25"),
    (TermMode::APP_CURSOR, "?1"),
    (TermMode::BRACKETED_PASTE, "?2004"),
    (TermMode::LINE_WRAP, "?7"),
    (TermMode::LINE_FEED_NEW_LINE, "20"),
    (TermMode::ORIGIN, "?6"),
    (TermMode::INSERT, "4"),
    (TermMode::FOCUS_IN_OUT, "?1004"),
    (TermMode::ALTERNATE_SCROLL, "?1007"),
    (TermMode::URGENCY_HINTS, "?1042"),
];
const MOUSE: [(TermMode, &str); 3] = [
    (TermMode::MOUSE_REPORT_CLICK, "?1000"),
    (TermMode::MOUSE_DRAG, "?1002"),
    (TermMode::MOUSE_MOTION, "?1003"),
];
const ENCODING: [(TermMode, &str); 2] = [(TermMode::UTF8_MOUSE, "?1005"), (TermMode::SGR_MOUSE, "?1006")];

fn modes(mode: TermMode) -> String {
    let mut out = String::new();
    for (bit, n) in MODES {
        let _ = write!(out, "\x1b[{n}{}", if mode.contains(bit) { 'h' } else { 'l' });
    }
    for group in [&MOUSE[..], &ENCODING[..]] {
        for (_, n) in group {
            let _ = write!(out, "\x1b[{n}l");
        }
        for (bit, n) in group {
            if mode.contains(*bit) {
                let _ = write!(out, "\x1b[{n}h");
            }
        }
    }
    out.push_str(if mode.contains(TermMode::APP_KEYPAD) {
        "\x1b="
    } else {
        "\x1b>"
    });
    out
}

fn default_tabs(cols: u16) -> Vec<u16> {
    (8..cols.saturating_sub(1)).step_by(8).collect()
}

impl Screen {
    /// The state as bytes [`Self::from_checkpoint`] rebuilds exactly, or `None` while the parser
    /// is inside a sequence, whose half could not be carried.
    pub fn checkpoint(&mut self) -> Option<Vec<u8>> {
        if !self.ground.idle() {
            return None;
        }
        let saved = self.save();
        let body = postcard::to_stdvec(&saved).ok()?;
        let mut out = ZlibEncoder::new(MAGIC.to_vec(), Compression::fast());
        out.write_all(&body).ok()?;
        out.finish().ok()
    }

    pub fn from_checkpoint(bytes: &[u8], scrollback_rows: usize) -> Option<Self> {
        let packed = bytes.strip_prefix(MAGIC)?;
        let mut body = Vec::new();
        ZlibDecoder::new(packed)
            .take(LARGEST as u64 * 64)
            .read_to_end(&mut body)
            .ok()?;
        let saved: Saved = postcard::from_bytes(&body).ok()?;
        if !valid(&saved) {
            return None;
        }
        std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| load(saved, scrollback_rows)))
            .ok()
            .flatten()
    }

    fn probe(&mut self, bytes: &[u8]) {
        self.processor.advance(&mut self.term, bytes);
    }

    fn save(&mut self) -> Saved {
        let (cols, rows) = self.size();
        let mode = *self.term.mode();
        let region = self.probe_region();
        let tabs = self.probe_tabs();
        let shifted = self.probe_shift();
        let colours = (0..SETTABLE_COLOURS)
            .filter_map(|i| self.term.colors()[i].map(|c| (i as u16, [c.r, c.g, c.b])))
            .collect();
        let style = self.term.cursor_style();
        let cursor_style = match style.shape {
            CursorShape::Block => Some(1),
            CursorShape::Underline => Some(3),
            CursorShape::Beam => Some(5),
            _ => None,
        }
        .filter(|_| style != alacritty_terminal::term::Config::default().default_cursor_style)
        .map(|shape| (shape, style.blinking));
        let (primary, alternate, keyboard) = if mode.contains(TermMode::ALT_SCREEN) {
            let alternate = Plane::of(self.term.grid());
            let on_alternate = answer::keyboard_flags(*self.term.mode());
            self.term.swap_alt();
            // Entering the alternate screen sets the primary's saved cursor to its cursor, so
            // the way back in leaves them equal; the saved one is what leaving restores.
            let grid = self.term.grid_mut();
            grid.cursor = grid.saved_cursor.clone();
            let primary = Plane::of(self.term.grid());
            let on_primary = answer::keyboard_flags(*self.term.mode());
            self.term.swap_alt();
            alternate.put(self.term.grid_mut());
            (primary, Some(alternate), (on_primary, on_alternate))
        } else {
            let primary = Plane::of(self.term.grid());
            let on_primary = answer::keyboard_flags(mode);
            let saved = self.term.grid().saved_cursor.clone();
            self.term.swap_alt();
            let on_alternate = answer::keyboard_flags(*self.term.mode());
            self.term.swap_alt();
            self.term.grid_mut().saved_cursor = saved;
            (primary, None, (on_primary, on_alternate))
        };
        Saved {
            version: VERSION,
            cols,
            rows,
            title: self.title.clone(),
            mode: mode.bits(),
            keyboard,
            region,
            tabs,
            shifted,
            colours,
            cursor_style,
            primary,
            alternate,
            history_first: self.history.first(),
            history: self
                .history
                .iter_from(self.history.first())
                .map(|r| (r.styled.clone(), r.plain.clone(), r.wrapped, r.cols))
                .collect(),
            known: self.known,
            rebase: self.rebase,
            program: self.program.as_ref().map(|p| {
                let state = PROGRAM_STATES.iter().position(|s| *s == p.state).unwrap_or(0) as u8;
                (state, p.kind.clone(), p.message.clone())
            }),
            program_reports: self.program_reports,
        }
    }

    /// With origin mode on, addressing is clamped to the scroll region, which alacritty keeps
    /// private.
    fn probe_region(&mut self) -> (i32, i32) {
        let cursor = self.term.grid().cursor.clone();
        let origin = self.term.mode().contains(TermMode::ORIGIN);
        self.probe(b"\x1b[?6h\x1b[1;1H");
        let start = self.term.grid().cursor.point.line.0;
        self.probe(b"\x1b[32767;1H");
        let end = self.term.grid().cursor.point.line.0 + 1;
        if !origin {
            self.probe(b"\x1b[?6l");
        }
        self.term.grid_mut().cursor = cursor;
        (start, end)
    }

    /// `CHT` walks the private tab stops without writing a cell. A stop in the first or last
    /// column changes nothing any sequence does, so neither is carried.
    fn probe_tabs(&mut self) -> Vec<u16> {
        let cursor = self.term.grid().cursor.clone();
        let last = self.term.grid().columns() - 1;
        let mut tabs = Vec::new();
        self.probe(b"\x1b[1G");
        loop {
            self.probe(b"\x1b[I");
            let col = self.term.grid().cursor.point.column.0;
            if col >= last || tabs.last().is_some_and(|&t| usize::from(t) >= col) {
                break;
            }
            tabs.push(col as u16);
        }
        self.term.grid_mut().cursor = cursor;
        tabs
    }

    /// Which of G0 and G1 is invoked is private too; one character drawn through a known
    /// designation tells, and the row and cursor it touched are put back.
    fn probe_shift(&mut self) -> bool {
        let cursor = self.term.grid().cursor.clone();
        // Under origin mode `CHA` also moves the cursor into the scroll region.
        self.probe(b"\x1b[1G");
        let line = self.term.grid().cursor.point.line;
        let row = self.term.grid()[line].clone();
        self.probe(b"\x1b(B\x1b)0q");
        let shifted = self.term.grid()[line][Column(0)].c != 'q';
        let grid = self.term.grid_mut();
        grid[line] = row;
        grid.cursor = cursor;
        shifted
    }
}

fn valid(saved: &Saved) -> bool {
    let (cols, rows) = (saved.cols, saved.rows);
    saved.version == VERSION
        && cols > 0
        && rows > 0
        && usize::from(cols) * usize::from(rows) <= LARGEST
        && saved.primary.fits(cols, rows)
        && saved
            .alternate
            .as_ref()
            .is_none_or(|a| a.scrolled == 0 && a.fits(cols, rows))
        && saved.known <= saved.primary.scrolled
        && 0 <= saved.region.0
        && saved.region.0 < saved.region.1
        && saved.region.1 <= i32::from(rows)
        && saved.tabs.iter().all(|&t| t < cols)
        && saved
            .colours
            .iter()
            .all(|&(i, _)| usize::from(i) < SETTABLE_COLOURS)
}

fn load(saved: Saved, scrollback_rows: usize) -> Option<Screen> {
    let (cols, rows) = (saved.cols, saved.rows);
    let mut screen = Screen::new(cols, rows, scrollback_rows);
    let mode = TermMode::from_bits_truncate(saved.mode);
    let mut setup = modes(mode);
    for (i, [r, g, b]) in &saved.colours {
        let colour = format!("rgb:{r:02x}/{g:02x}/{b:02x}");
        let _ = match i {
            256 => write!(setup, "\x1b]10;{colour}\x07"),
            257 => write!(setup, "\x1b]11;{colour}\x07"),
            258 => write!(setup, "\x1b]12;{colour}\x07"),
            i => write!(setup, "\x1b]4;{i};{colour}\x07"),
        };
    }
    if let Some((shape, blinking)) = saved.cursor_style {
        let _ = write!(setup, "\x1b[{} q", shape + u8::from(!blinking));
    }
    if saved.region != (0, i32::from(rows)) {
        let _ = write!(setup, "\x1b[{};{}r", saved.region.0 + 1, saved.region.1);
    }
    if saved.tabs != default_tabs(cols) {
        setup.push_str("\x1b[3g");
        for tab in &saved.tabs {
            let _ = write!(setup, "\x1b[{}G\x1bH", tab + 1);
        }
    }
    if saved.shifted {
        setup.push('\x0e');
    }
    if !saved.title.is_empty() {
        let _ = write!(setup, "\x1b]2;{}\x07", saved.title);
    }
    let (on_primary, on_alternate) = saved.keyboard;
    if on_primary != 0 {
        let _ = write!(setup, "\x1b[>{on_primary}u");
    }
    screen.probe(setup.as_bytes());
    screen.term.swap_alt();
    if on_alternate != 0 {
        screen.probe(format!("\x1b[>{on_alternate}u").as_bytes());
    }
    screen.term.swap_alt();
    if !saved.primary.put(screen.term.grid_mut()) {
        return None;
    }
    if let Some(alternate) = &saved.alternate {
        screen.term.swap_alt();
        if !alternate.put(screen.term.grid_mut()) {
            return None;
        }
    }
    if *screen.term.mode() != mode {
        return None;
    }
    *screen.titles.lock().unwrap() = None;
    screen.title = saved.title;
    screen.history = History::resume(
        scrollback_rows,
        saved.history_first,
        saved
            .history
            .into_iter()
            .map(|(styled, plain, wrapped, cols)| Rendered {
                styled,
                plain,
                wrapped,
                cols,
            }),
    );
    screen.known = saved.known;
    screen.rebase = saved.rebase;
    screen.program = match saved.program {
        Some((state, kind, message)) => Some(ProgramStatus {
            state: *PROGRAM_STATES.get(usize::from(state))?,
            kind,
            message,
        }),
        None => None,
    };
    screen.program_reports = saved.program_reports;
    screen.forget_sent();
    Some(screen)
}

/// Where `vte` would be after the bytes so far, to the precision a checkpoint needs: in ground
/// with no half a character held, or not. It follows `vte` 0.15's state table, merging the states
/// that leave by the same bytes; only `ESC` leaves ground.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct Ground {
    state: State,
    pending: u8,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Escape,
    EscapeIntermediate,
    Csi,
    DcsEntry,
    DcsParam,
    DcsIntermediate,
    DcsPassthrough,
    Osc,
    Ignore,
}

impl Ground {
    pub(super) fn idle(&self) -> bool {
        self.state == State::Ground && self.pending == 0
    }

    pub(super) fn advance(&mut self, mut bytes: &[u8]) {
        while !bytes.is_empty() {
            if self.state == State::Ground {
                let run = memchr::memchr(0x1b, bytes).unwrap_or(bytes.len());
                let tail = &bytes[run.saturating_sub(4)..run];
                if run > 4 {
                    self.pending = 0;
                }
                for &b in tail {
                    self.pending = utf8(self.pending, b);
                }
                if run == bytes.len() {
                    return;
                }
                self.pending = 0;
                self.state = State::Escape;
                bytes = &bytes[run + 1..];
                continue;
            }
            self.state = next(self.state, bytes[0]);
            bytes = &bytes[1..];
        }
    }
}

fn utf8(pending: u8, b: u8) -> u8 {
    match b {
        0x80..=0xbf => pending.saturating_sub(1),
        0xc2..=0xdf => 1,
        0xe0..=0xef => 2,
        0xf0..=0xf4 => 3,
        _ => 0,
    }
}

fn next(state: State, b: u8) -> State {
    use State::*;
    let anywhere = |stay: State| match b {
        0x18 | 0x1a => Ground,
        0x1b => Escape,
        _ => stay,
    };
    match state {
        Ground => Ground,
        Escape => match b {
            0x18 | 0x1a => Ground,
            0x20..=0x2f => EscapeIntermediate,
            0x30..=0x4f | 0x51..=0x57 | 0x59..=0x5a | 0x5c | 0x60..=0x7e => Ground,
            0x50 => DcsEntry,
            0x58 | 0x5e | 0x5f => Ignore,
            0x5b => Csi,
            0x5d => Osc,
            _ => Escape,
        },
        EscapeIntermediate => match b {
            0x30..=0x7e => Ground,
            _ => anywhere(EscapeIntermediate),
        },
        Csi => match b {
            0x40..=0x7e => Ground,
            _ => anywhere(Csi),
        },
        DcsEntry => match b {
            0x20..=0x2f => DcsIntermediate,
            0x30..=0x3f => DcsParam,
            0x40..=0x7e => DcsPassthrough,
            _ => anywhere(DcsEntry),
        },
        DcsParam => match b {
            0x20..=0x2f => DcsIntermediate,
            0x30..=0x3b => DcsParam,
            0x3c..=0x3f => Ignore,
            0x40..=0x7e => DcsPassthrough,
            _ => anywhere(DcsParam),
        },
        DcsIntermediate => match b {
            0x20..=0x2f => DcsIntermediate,
            0x30..=0x3f => Ignore,
            0x40..=0x7e => DcsPassthrough,
            _ => anywhere(DcsIntermediate),
        },
        DcsPassthrough => match b {
            0x9c => Ground,
            _ => anywhere(DcsPassthrough),
        },
        Osc => match b {
            0x07 => Ground,
            _ => anywhere(Osc),
        },
        Ignore => anywhere(Ignore),
    }
}
