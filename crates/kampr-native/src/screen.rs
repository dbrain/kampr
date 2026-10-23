mod checkpoint;

use crate::answer::{self, Scan};
use crate::frame::{self, Cursor, Links};
use crate::history::{self, History, Line, Row};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Line as At;
use alacritty_terminal::term::{Config, Term, TermMode};
use alacritty_terminal::vte::Parser;
use alacritty_terminal::vte::ansi::{CursorShape, Processor, Timeout};
use checkpoint::Ground;
use kampr_core::provider::{Found, ProgramStatus, RawScrollback};
use kampr_term::Cell;
use std::fmt::Write as _;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// The most primary history alacritty holds between feeds: what a taller screen can take back.
/// [`History`] holds the rest, as rows already rendered.
const WINDOW: usize = 512;
/// A feed is cut into pieces no longer than this, and alacritty may hold this many rows past the
/// window before a piece ends. Nothing a piece can do pushes more than one row per byte, except
/// the sequences [`Scan`] ends a piece at, which push at most a screenful: so no row ever falls
/// off alacritty's history uncounted.
const PIECE: usize = 4096;

pub struct Screen {
    term: Term<Events>,
    processor: Processor<Unsynced>,
    scan: Parser,
    found: Scan,
    titles: Arc<Mutex<Option<String>>>,
    title: String,
    history: History,
    window: usize,
    /// How many of alacritty's primary history rows [`History`] already holds, its newest ones.
    known: usize,
    /// Set when the primary screen was reflowed while hidden, after which `known` means nothing.
    rebase: bool,
    sent: Vec<Cell>,
    sent_cursor: Cursor,
    program: Option<ProgramStatus>,
    program_reports: u64,
    ground: Ground,
}

#[derive(Debug, Default)]
pub struct Feed {
    pub replies: Vec<u8>,
    pub(crate) asked: Vec<Asked>,
}

/// One question in a feed: where it ended in the bytes fed, and where its answer is in `replies`.
/// `span` is all of its bytes, `None` for a question not wholly in this feed.
#[derive(Debug)]
pub(crate) struct Asked {
    pub span: Option<std::ops::Range<usize>>,
    pub end: usize,
    pub program: bool,
    pub reply: std::ops::Range<usize>,
}

struct Size {
    cols: u16,
    rows: u16,
}

impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.rows.max(1) as usize
    }

    fn screen_lines(&self) -> usize {
        self.rows.max(1) as usize
    }

    fn columns(&self) -> usize {
        self.cols.max(1) as usize
    }
}

struct Events(Arc<Mutex<Option<String>>>);

impl EventListener for Events {
    fn send_event(&self, event: Event) {
        let title = match event {
            Event::Title(title) => title,
            Event::ResetTitle => String::new(),
            _ => return,
        };
        *self.0.lock().unwrap() = Some(title);
    }
}

/// Synchronized output would hold bytes back past the end of a feed, and every frame and answer
/// here is taken at the end of one; a consumer of the frames already sees only whole reads.
#[derive(Default)]
struct Unsynced;

impl Timeout for Unsynced {
    fn set_timeout(&mut self, _: Duration) {}
    fn clear_timeout(&mut self) {}
    fn pending_timeout(&self) -> bool {
        false
    }
}

impl Screen {
    pub fn new(cols: u16, rows: u16, scrollback_rows: usize) -> Self {
        let window = scrollback_rows.min(WINDOW);
        let config = Config {
            scrolling_history: window + PIECE + u16::MAX as usize,
            kitty_keyboard: true,
            ..Config::default()
        };
        let titles = Arc::new(Mutex::new(None));
        let mut screen = Self {
            term: Term::new(config, &Size { cols, rows }, Events(titles.clone())),
            processor: Processor::new(),
            scan: Parser::new(),
            found: Scan::default(),
            titles,
            title: String::new(),
            history: History::new(scrollback_rows),
            window,
            known: 0,
            rebase: false,
            sent: Vec::new(),
            sent_cursor: Cursor::default(),
            program: None,
            program_reports: 0,
            ground: Ground::default(),
        };
        screen.forget_sent();
        screen
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Feed {
        let mut replies = String::new();
        let mut asked = Vec::new();
        self.ground.advance(bytes);
        let mut rest = bytes;
        while !rest.is_empty() {
            let piece = &rest[..rest.len().min(PIECE)];
            let n = self.scan.advance_until_terminated(&mut self.found, piece);
            self.processor.advance(&mut self.term, &piece[..n]);
            rest = &rest[n..];
            let found = self.found.take();
            let end = bytes.len() - rest.len();
            for ask in &found.asks {
                let start = replies.len();
                replies.push_str(&answer::answer(&self.term, ask));
                asked.push(Asked {
                    span: span(bytes, end),
                    end,
                    program: *ask == answer::Ask::Program,
                    reply: start..replies.len(),
                });
            }
            if let Some(program) = found.program {
                self.program = program;
                self.program_reports += 1;
            }
            self.sync(found.cleared);
        }
        if let Some(title) = self.titles.lock().unwrap().take() {
            self.title = title;
        }
        Feed {
            replies: replies.into_bytes(),
            asked,
        }
    }

    /// Height first, at the old width, so that what a shorter screen pushes into history is read
    /// at the width it was written at; then the width, which alacritty reflows. Its history is
    /// emptied before a reflow, since [`History`] already holds it and a reflowed copy could not
    /// be matched against it.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let (was_cols, was_rows) = self.size();
        if rows != was_rows {
            self.term.resize(Size { cols: was_cols, rows });
            self.sync(false);
        }
        if cols != was_cols {
            if self.alternate_screen() {
                self.rebase = true;
            } else {
                self.term.grid_mut().clear_history();
                self.known = 0;
            }
            self.term.resize(Size { cols, rows });
            self.sync(false);
        }
        self.forget_sent();
    }

    pub fn size(&self) -> (u16, u16) {
        let grid = self.term.grid();
        (grid.columns() as u16, grid.screen_lines() as u16)
    }

    pub fn alternate_screen(&self) -> bool {
        self.term.mode().contains(TermMode::ALT_SCREEN)
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    /// The harness's last `OSC 7501` report (#600), `None` until it makes one and after it clears.
    pub fn program(&self) -> Option<&ProgramStatus> {
        self.program.as_ref()
    }

    /// How many reports have been read, so a caller can tell a new `done` from the one it saw.
    pub(crate) fn program_reports(&self) -> u64 {
        self.program_reports
    }

    pub fn cursor(&self) -> (u16, u16, bool) {
        let c = self.cursor_now();
        (c.col, c.row, c.visible)
    }

    /// The kitty keyboard flags the program has pushed on the screen it is on; Claude Code pushes 5
    /// (#594) and still takes legacy bytes under them (#596).
    pub fn keyboard_flags(&self) -> u8 {
        answer::keyboard_flags(*self.term.mode())
    }

    /// The input modes a terminal painted from [`Self::frame_full`] needs before live bytes pass
    /// through to it. Every mode is stated, the resets first, because the terminal may still hold
    /// ones an earlier paint set; xterm's mouse modes replace each other.
    pub fn input_modes(&self) -> Vec<u8> {
        let mode = *self.term.mode();
        let mut on = String::new();
        let mut off = String::new();
        for (bit, n) in [
            (TermMode::APP_CURSOR, 1),
            (TermMode::MOUSE_REPORT_CLICK, 1000),
            (TermMode::MOUSE_DRAG, 1002),
            (TermMode::MOUSE_MOTION, 1003),
            (TermMode::FOCUS_IN_OUT, 1004),
            (TermMode::UTF8_MOUSE, 1005),
            (TermMode::SGR_MOUSE, 1006),
            (TermMode::BRACKETED_PASTE, 2004),
        ] {
            let _ = match mode.contains(bit) {
                true => write!(on, "\x1b[?{n}h"),
                false => write!(off, "\x1b[?{n}l"),
            };
        }
        off.push_str(if mode.contains(TermMode::APP_KEYPAD) {
            "\x1b="
        } else {
            "\x1b>"
        });
        let _ = write!(on, "\x1b[={};1u", self.keyboard_flags());
        let style = self.term.cursor_style();
        let shape = match style.shape {
            CursorShape::Underline => 3,
            CursorShape::Beam => 5,
            _ => 1,
        };
        let _ = write!(on, "\x1b[{} q", shape + u8::from(!style.blinking));
        off.push_str(&on);
        off.into_bytes()
    }

    /// A complete repaint. A consumer joining a stream that others are already following on
    /// [`Self::frame_diff`] must be sent this only after a `frame_diff` has been taken, or the
    /// next diff is computed against a state older than the one this painted.
    pub fn frame_full(&self) -> Vec<u8> {
        let (cols, _) = self.size();
        let (grid, links) = self.grid();
        frame::full(&grid, &links, cols, self.cursor_now())
    }

    /// What changed since the last diff (or since construction or a resize, both of which a
    /// consumer meets with a blank grid of the new size).
    pub fn frame_diff(&mut self) -> Option<Vec<u8>> {
        let (cols, _) = self.size();
        let (grid, links) = self.grid();
        let cursor = self.cursor_now();
        let out = frame::diff(&self.sent, &grid, &links, cols, self.sent_cursor, cursor);
        self.sent = grid;
        self.sent_cursor = cursor;
        out
    }

    pub fn history_rows(&self) -> u32 {
        self.history.len() as u32
    }

    pub fn first_row(&self) -> u32 {
        u32::try_from(self.history.first()).unwrap_or(u32::MAX)
    }

    /// History from absolute row `from` (all of it when `None`) and then the visible screen, in
    /// the shape `ScrollbackRing::ingest` reads: one styled row per line, the screen last.
    pub fn scrollback(&self, from: Option<u32>) -> RawScrollback {
        let (cols, rows) = self.size();
        let start = from.map_or(self.history.first(), |f| u64::from(f).max(self.history.first()));
        let mut text = String::new();
        for row in self.history.iter_from(start) {
            text.push_str(&row.styled);
            text.push('\n');
        }
        for row in self.screen_rows() {
            text.push_str(&row.styled);
            text.push('\n');
        }
        RawScrollback {
            text,
            cols: Some(cols),
            viewport_rows: rows,
            truncated: start > 0,
            first_row: u32::try_from(start.min(self.history.end())).ok(),
        }
    }

    /// Plain rows `from ..= to` by absolute index, history then screen; `None` unless every one
    /// of them is still held.
    pub fn read_rows(&self, from: u32, to: u32) -> Option<Vec<String>> {
        let (from, to) = (u64::from(from), u64::from(to));
        let (_, rows) = self.size();
        let end = self.history.end();
        if to < from || from < self.history.first() || to >= end + u64::from(rows) {
            return None;
        }
        let screen = (to >= end).then(|| self.screen_rows());
        Some(
            (from..=to)
                .map(|abs| match self.history.row(abs) {
                    Some(row) => row.plain.clone(),
                    None => screen
                        .as_ref()
                        .map_or_else(String::new, |s| s[(abs - end) as usize].plain.clone()),
                })
                .collect(),
        )
    }

    pub fn find(&self, query: &str, backward: bool, from: Option<u32>) -> Found {
        let (_, rows) = self.size();
        let screen = self.screen_rows();
        let mut lines: Vec<Line<'_>> = self
            .history
            .iter_from(0)
            .map(|r| Line {
                plain: &r.plain,
                wrapped: r.wrapped,
                cols: r.cols,
            })
            .collect();
        lines.extend(screen.iter().enumerate().map(|(r, row)| Line {
            plain: &row.plain,
            wrapped: r + 1 < rows as usize && row.wrapped,
            cols: row.cols,
        }));
        let (hits, current) = history::find(&lines, query, backward, from);
        Found {
            total: hits.len() as u32,
            hits,
            current,
        }
    }

    fn row(&self, at: i32, cols: u16) -> Row {
        let mut links = Links::default();
        let row = &self.term.grid()[At(at)];
        let cells = frame::row_cells(row, cols, &mut links);
        Row {
            styled: frame::styled_row(&cells, &links),
            plain: frame::plain_row(&cells),
            wrapped: frame::wrapped(row),
            cols,
        }
    }

    pub(crate) fn history(&self) -> &History {
        &self.history
    }

    pub(crate) fn screen_rows(&self) -> Vec<Row> {
        let (cols, rows) = self.size();
        (0..rows as i32).map(|r| self.row(r, cols)).collect()
    }

    fn grid(&self) -> (Vec<Cell>, Links) {
        let grid = self.term.grid();
        let (cols, rows) = self.size();
        let mut links = Links::default();
        let mut cells = Vec::with_capacity(rows as usize * cols as usize);
        for r in 0..rows as i32 {
            cells.extend(frame::row_cells(&grid[At(r)], cols, &mut links));
        }
        (cells, links)
    }

    fn cursor_now(&self) -> Cursor {
        let grid = self.term.grid();
        let point = grid.cursor.point;
        Cursor {
            row: point.line.0.max(0) as u16,
            col: point.column.0.min(grid.columns() - 1) as u16,
            visible: self.term.mode().contains(TermMode::SHOW_CURSOR),
        }
    }

    fn forget_sent(&mut self) {
        let (cols, rows) = self.size();
        self.sent = vec![Cell::default(); cols as usize * rows as usize];
        self.sent_cursor = Cursor {
            row: 0,
            col: 0,
            visible: true,
        };
    }

    /// Copies whatever alacritty has newly scrolled into its primary history, then trims it back
    /// to the window. Only possible while the primary screen is up; rows that arrive under the
    /// alternate one are copied when it leaves.
    fn sync(&mut self, cleared: bool) {
        if self.alternate_screen() {
            return;
        }
        let held = self.term.grid().history_size();
        if cleared {
            self.history.clear();
            self.known = 0;
        }
        if std::mem::take(&mut self.rebase) {
            self.known = held;
        }
        if held < self.known {
            self.history.drop_newest(self.known - held);
        }
        let (cols, _) = self.size();
        for back in (1..=held.saturating_sub(self.known)).rev() {
            let row = self.row(-(back as i32), cols);
            self.history.push(row);
        }
        self.known = held;
        if held > self.window {
            let grid = self.term.grid_mut();
            grid.update_history(self.window);
            grid.update_history(self.window + PIECE + u16::MAX as usize);
            self.known = self.window;
        }
    }
}

/// A query has no ESC inside it but its own introducer and an `ST` terminator, since an ESC
/// anywhere else would have ended it before it was dispatched. `vte` dispatches an OSC at the
/// `ST`'s ESC, so its `\\` is still to come.
fn span(bytes: &[u8], end: usize) -> Option<std::ops::Range<usize>> {
    let (body, stop) = match bytes[..end].strip_suffix(b"\x1b") {
        Some(body) if bytes.get(end) == Some(&b'\\') => (body, end + 1),
        Some(_) => return None,
        None => (&bytes[..end], end),
    };
    body.iter().rposition(|&b| b == 0x1b).map(|start| start..stop)
}

#[cfg(test)]
mod tests {
    use super::Screen;

    #[test]
    fn a_question_spans_its_own_bytes_unless_one_feed_does_not_hold_them_all() {
        for (feeds, want) in [
            (&[&b"ab\x1b[0mcd\x1b[c"[..]][..], vec![Some(8..11)]),
            (
                &[&b"x\x1b]11;?\x07y\x1b[?2026$p"[..]][..],
                vec![Some(1..8), Some(9..18)],
            ),
            (&[&b"\x1b]10;?;?\x1b\\z"[..]][..], vec![Some(0..10), Some(0..10)]),
            (&[&b"ok\x1b["[..], &b"5n."[..]][..], vec![None]),
            (&[&b"\x1b]11;?\x1b"[..], &b"\\"[..]][..], vec![None]),
        ] {
            let mut screen = Screen::new(80, 24, 0);
            let mut got = Vec::new();
            for feed in feeds {
                got.extend(screen.feed(feed).asked.iter().map(|a| a.span.clone()));
            }
            assert_eq!(got, want, "{feeds:?}");
        }
    }
}
