use crate::frame;
use crate::history::{self, Row};
use crate::live::Live;
use crate::procs;
use crate::screen::Screen;
use kampr_core::provider::{Found, PaneEvent, PaneStream, RawScrollback, Reflow};
use std::collections::{HashMap, VecDeque};
use std::fmt::Write as _;
use std::hash::{DefaultHasher, Hash, Hasher};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use unicode_segmentation::UnicodeSegmentation;

const PLAIN: &str = "\x1b[0m";
const MIN_COLS: u16 = 10;
const MAX_COLS: u16 = 1000;

/// One width's view of one session, shared by every stream and read of it: the history re-laid
/// so far, and where the view's screen starts. Rows are numbered from the first ever laid, so a
/// row means the same thing to every read; history the session trims is trimmed here with it.
pub(crate) struct View {
    cols: u16,
    settled: VecDeque<Row>,
    first: u64,
    /// Each complete history line laid so far: the physical row after it, and how many rows it
    /// became, so a trim or a taller screen taking rows back can be undone a line at a time.
    lines: VecDeque<(u64, usize)>,
    consumed: u64,
    /// Only ever moves down, as a terminal's screen does: a line that shrinks leaves blank rows
    /// at the bottom rather than pulling back rows a client already holds as history.
    top: u64,
    /// Last frame's screen lines as laid, by content: a scrolling screen is the same lines moved.
    laid: HashMap<u64, Vec<Row>>,
}

pub(crate) struct Shot {
    pub grid: Vec<Row>,
    pub overflow: Vec<Row>,
    pub top: u64,
    pub cursor: (u16, u16, bool),
}

#[derive(Clone, Copy)]
struct Tok<'a> {
    pen: &'a str,
    link: &'a str,
    text: &'a str,
    width: u16,
}

const BLANK: Tok<'static> = Tok {
    pen: PLAIN,
    link: "",
    text: " ",
    width: 1,
};

impl View {
    pub fn new(cols: u16) -> Self {
        Self {
            cols,
            settled: VecDeque::new(),
            first: 0,
            lines: VecDeque::new(),
            consumed: 0,
            top: 0,
            laid: HashMap::new(),
        }
    }

    fn settled_end(&self) -> u64 {
        self.first + self.settled.len() as u64
    }

    fn settle(&mut self, screen: &Screen) {
        let h = screen.history();
        while let Some(&(end, n)) = self.lines.front() {
            if end > h.first() {
                break;
            }
            self.lines.pop_front();
            self.settled.drain(..n.min(self.settled.len()));
            self.first += n as u64;
        }
        if h.end() < self.consumed {
            while let Some(&(end, n)) = self.lines.back() {
                if end <= h.end() {
                    break;
                }
                self.lines.pop_back();
                self.settled.truncate(self.settled.len().saturating_sub(n));
            }
            self.consumed = self.lines.back().map_or(h.first(), |l| l.0);
        }
        if self.consumed < h.first() {
            // Rows the session dropped before they were laid: a gap a ring can see, sized as if
            // each had been one row, rather than history that silently joins across it.
            let lost = h.first() - self.consumed;
            self.first = self.settled_end() + lost;
            self.settled.clear();
            self.lines.clear();
            self.consumed = h.first();
            self.top = self.top.max(self.first);
        }
        let mut line: Vec<&Row> = Vec::new();
        let mut at = self.consumed;
        for row in h.iter_from(self.consumed) {
            at += 1;
            line.push(row);
            if row.wrapped {
                continue;
            }
            let laid = lay(&line, self.cols, None);
            self.lines.push_back((at, laid.rows.len()));
            self.settled.extend(laid.rows);
            self.consumed = at;
            line.clear();
        }
    }

    pub fn shoot(&mut self, screen: &Screen) -> Shot {
        self.settle(screen);
        let (_, height) = screen.size();
        let height = height.max(1);
        let (ccol, crow, visible) = screen.cursor();
        let straddle: Vec<&Row> = screen.history().iter_from(self.consumed).collect();
        let rows = screen.screen_rows();
        let content = rows
            .iter()
            .rposition(|r| !r.plain.is_empty())
            .map_or(0, |i| i + 1)
            .max(crow as usize + 1)
            .min(rows.len());
        let phys: Vec<&Row> = straddle.iter().copied().chain(rows[..content].iter()).collect();
        let cursor_at = straddle.len() + crow as usize;
        let mut laid_rows: Vec<Row> = Vec::new();
        let mut reused = HashMap::new();
        let mut cursor = (0u64, 0u16);
        let mut start = 0;
        while start < phys.len() {
            let mut end = start;
            while end + 1 < phys.len() && phys[end].wrapped {
                end += 1;
            }
            let at = (start..=end)
                .contains(&cursor_at)
                .then(|| (cursor_at - start, ccol));
            let line = &phys[start..=end];
            let key = at.is_none().then(|| line_key(line));
            match key.and_then(|k| self.laid.get(&k).map(|rows| (k, rows))) {
                Some((k, rows)) => {
                    laid_rows.extend(rows.iter().cloned());
                    reused.insert(k, rows.clone());
                }
                None => {
                    let laid = lay(line, self.cols, at);
                    if let Some((r, c)) = laid.cursor {
                        cursor = ((laid_rows.len() + r) as u64, c);
                    }
                    if let Some(k) = key {
                        reused.insert(k, laid.rows.clone());
                    }
                    laid_rows.extend(laid.rows);
                }
            }
            start = end + 1;
        }
        self.laid = reused;
        let base = self.settled_end();
        let end = base + laid_rows.len() as u64;
        let cursor_abs = base + cursor.0;
        let mut top = self.top.max(base).max(end.saturating_sub(u64::from(height)));
        if cursor_abs >= top + u64::from(height) {
            top = cursor_abs + 1 - u64::from(height);
        }
        top = top.min(cursor_abs).max(self.first);
        self.top = top;
        let grid = (top..top + u64::from(height))
            .map(|abs| match abs.checked_sub(base) {
                Some(i) => laid_rows
                    .get(i as usize)
                    .cloned()
                    .unwrap_or_else(|| blank(self.cols)),
                None => self.settled[(abs - self.first) as usize].clone(),
            })
            .collect();
        let overflow = laid_rows
            .into_iter()
            .take(top.saturating_sub(base) as usize)
            .collect();
        Shot {
            grid,
            overflow,
            top,
            cursor: ((cursor_abs - top) as u16, cursor.1, visible),
        }
    }

    /// History is everything above the view's screen, newest `cap` rows of it.
    pub fn scrollback(&mut self, screen: &Screen, cap: usize) -> RawScrollback {
        let shot = self.shoot(screen);
        let history = self.history(&shot);
        let skip = history.len().saturating_sub(cap);
        let mut text = String::new();
        for row in history.iter().copied().skip(skip).chain(shot.grid.iter()) {
            text.push_str(&row.styled);
            text.push('\n');
        }
        let start = shot.top - history.len() as u64 + skip as u64;
        RawScrollback {
            text,
            cols: Some(self.cols),
            viewport_rows: shot.grid.len() as u16,
            truncated: start > 0,
            first_row: u32::try_from(start).ok(),
        }
    }

    pub fn read_rows(&mut self, screen: &Screen, from: u32, to: u32) -> Option<Vec<String>> {
        let shot = self.shoot(screen);
        let history = self.history(&shot);
        let start = shot.top - history.len() as u64;
        let all: Vec<&Row> = history.into_iter().chain(shot.grid.iter()).collect();
        let (from, to) = (u64::from(from), u64::from(to));
        if to < from || from < start || to >= start + all.len() as u64 {
            return None;
        }
        Some(
            (from..=to)
                .map(|abs| all[(abs - start) as usize].plain.clone())
                .collect(),
        )
    }

    pub fn find(&mut self, screen: &Screen, query: &str, backward: bool, from: Option<u32>) -> Found {
        let shot = self.shoot(screen);
        let history = self.history(&shot);
        let all: Vec<&Row> = history.into_iter().chain(shot.grid.iter()).collect();
        let lines: Vec<history::Line<'_>> = all
            .iter()
            .enumerate()
            .map(|(i, r)| history::Line {
                plain: &r.plain,
                wrapped: r.wrapped && i + 1 < all.len(),
                cols: r.cols,
            })
            .collect();
        let (hits, current) = history::find(&lines, query, backward, from);
        Found {
            total: hits.len() as u32,
            hits,
            current,
        }
    }

    fn history<'a>(&'a self, shot: &'a Shot) -> Vec<&'a Row> {
        let held = (shot.top.saturating_sub(self.first) as usize).min(self.settled.len());
        self.settled
            .iter()
            .take(held)
            .chain(shot.overflow.iter())
            .collect()
    }
}

fn line_key(line: &[&Row]) -> u64 {
    let mut hasher = DefaultHasher::new();
    for row in line {
        (&row.styled, row.cols, row.wrapped).hash(&mut hasher);
    }
    hasher.finish()
}

fn blank(cols: u16) -> Row {
    Row {
        styled: String::new(),
        plain: String::new(),
        wrapped: false,
        cols,
    }
}

struct Laid {
    rows: Vec<Row>,
    cursor: Option<(usize, u16)>,
}

/// One logical line (its physical rows, every one but the last wrapped) re-laid at `cols`.
/// `cursor` is a physical row within the line and a column on it.
fn lay(phys: &[&Row], cols: u16, cursor: Option<(usize, u16)>) -> Laid {
    let cols = cols.max(1);
    let mut cells: Vec<Tok<'_>> = Vec::new();
    let mut offset = None;
    for (i, row) in phys.iter().enumerate() {
        let at = cells.iter().map(|c| u32::from(c.width)).sum::<u32>();
        if let Some((r, c)) = cursor
            && r == i
        {
            offset = Some(at + u32::from(c));
        }
        let used = tokens(&row.styled, &mut cells);
        if i + 1 < phys.len() {
            let deficit = row.cols.saturating_sub(used);
            // alacritty leaves the last column blank when a wide glyph did not fit in it, and the
            // glyph opens the next row: that blank is not in the line the program wrote.
            let pushed_wide = deficit == 1 && first_width(&phys[i + 1].styled) == 2;
            if !pushed_wide {
                cells.extend(std::iter::repeat_n(BLANK, deficit as usize));
            }
        }
    }
    let mut breaks = vec![(0usize, 0u32)];
    let (mut col, mut logical) = (0u16, 0u32);
    for (i, cell) in cells.iter().enumerate() {
        if col > 0 && col + cell.width > cols {
            breaks.push((i, logical));
            col = 0;
        }
        col += cell.width;
        logical += u32::from(cell.width);
    }
    let mut rows: Vec<Row> = breaks
        .iter()
        .enumerate()
        .map(|(n, &(from, _))| {
            let to = breaks.get(n + 1).map_or(cells.len(), |b| b.0);
            render(&cells[from..to], cols, n + 1 < breaks.len())
        })
        .collect();
    let cursor = offset.map(|offset| {
        let n = breaks.iter().rposition(|b| b.1 <= offset).unwrap_or(0);
        let col = offset - breaks[n].1;
        let row = n + (col / u32::from(cols)) as usize;
        (row, (col % u32::from(cols)) as u16)
    });
    if let Some((row, _)) = cursor {
        while rows.len() <= row {
            if let Some(last) = rows.last_mut() {
                last.wrapped = true;
            }
            rows.push(blank(cols));
        }
    }
    Laid { rows, cursor }
}

fn tokens<'a>(styled: &'a str, out: &mut Vec<Tok<'a>>) -> u16 {
    let (mut pen, mut link, mut used) = (PLAIN, "", 0u16);
    let mut rest = styled;
    while !rest.is_empty() {
        if let Some(after) = rest.strip_prefix("\x1b[") {
            let end = after.find('m').map_or(after.len(), |i| i + 1);
            pen = &rest[..2 + end];
            rest = &after[end..];
            continue;
        }
        if let Some(after) = rest.strip_prefix("\x1b]8;;") {
            let end = after.find("\x1b\\").unwrap_or(after.len());
            link = &after[..end];
            rest = after.get(end + 2..).unwrap_or("");
            continue;
        }
        let end = rest
            .char_indices()
            .skip(1)
            .find(|&(_, c)| c == '\x1b')
            .map_or(rest.len(), |(i, _)| i);
        for text in rest[..end].graphemes(true) {
            let width = frame::term_width(text).max(1);
            used = used.saturating_add(width);
            out.push(Tok {
                pen,
                link,
                text,
                width,
            });
        }
        rest = &rest[end..];
    }
    used
}

fn first_width(styled: &str) -> u16 {
    let mut cells = Vec::new();
    tokens(styled, &mut cells);
    cells.first().map_or(0, |c| c.width)
}

/// In the shape `frame::styled_row` writes: starting and ending on the default pen, no link open.
fn render(cells: &[Tok<'_>], cols: u16, wrapped: bool) -> Row {
    let end = cells
        .iter()
        .rposition(|c| !(c.text == " " && c.pen == PLAIN && c.link.is_empty()))
        .map_or(0, |i| i + 1);
    let (mut styled, mut plain) = (String::new(), String::new());
    let (mut pen, mut link) = (PLAIN, "");
    for cell in &cells[..end] {
        if cell.pen != pen {
            styled.push_str(cell.pen);
            pen = cell.pen;
        }
        if cell.link != link {
            let _ = write!(styled, "\x1b]8;;{}\x1b\\", cell.link);
            link = cell.link;
        }
        styled.push_str(cell.text);
        plain.push_str(cell.text);
    }
    if pen != PLAIN {
        styled.push_str(PLAIN);
    }
    if !link.is_empty() {
        styled.push_str("\x1b]8;;\x1b\\");
    }
    Row {
        styled,
        plain: plain.trim_end().to_string(),
        wrapped,
        cols,
    }
}

/// What one stream of a view last sent, so the next frame is a diff against it.
#[derive(Default)]
struct Painter {
    sent: Vec<String>,
    cursor: (u16, u16, bool),
}

impl Painter {
    fn full(&mut self, shot: &Shot) -> Vec<u8> {
        let mut out = String::from("\x1b[?25l\x1b[0m\x1b[H\x1b[2J");
        for (r, row) in shot.grid.iter().enumerate() {
            if !row.styled.is_empty() {
                let _ = write!(out, "\x1b[{};1H{}", r + 1, row.styled);
            }
        }
        self.sent = shot.grid.iter().map(|r| r.styled.clone()).collect();
        self.cursor = shot.cursor;
        cursor(&mut out, shot.cursor);
        out.into_bytes()
    }

    /// Whole rows, each erased first: a row ends on the default pen, so `K` blanks the rest of it,
    /// and nothing is written after a row's last column that autowrap could move.
    fn diff(&mut self, shot: &Shot) -> Option<Vec<u8>> {
        let changed: Vec<usize> = (0..shot.grid.len())
            .filter(|&r| self.sent.get(r) != Some(&shot.grid[r].styled))
            .collect();
        if changed.is_empty() && self.cursor == shot.cursor {
            return None;
        }
        let mut out = String::from("\x1b[?25l");
        for r in changed {
            let _ = write!(out, "\x1b[{};1H\x1b[0m\x1b[K{}", r + 1, shot.grid[r].styled);
        }
        self.sent = shot.grid.iter().map(|r| r.styled.clone()).collect();
        self.cursor = shot.cursor;
        cursor(&mut out, shot.cursor);
        Some(out.into_bytes())
    }
}

fn cursor(out: &mut String, (row, col, visible): (u16, u16, bool)) {
    let _ = write!(out, "\x1b[{};{}H", row + 1, col + 1);
    if visible {
        out.push_str("\x1b[?25h");
    }
}

/// How long a pane's harness answer stands while its foreground job does not change: it is a walk
/// of `/proc`, and the node asks on every frame of a reflowed watch. A wrapper that starts its
/// harness after it has become the job is the change this still catches.
const HARNESS_TTL: Duration = Duration::from_secs(1);

pub(crate) type Shared = Arc<Mutex<View>>;
type Held = HashMap<(String, u16), Weak<Mutex<View>>>;

#[derive(Default)]
pub(crate) struct Views {
    views: Mutex<Held>,
    harness: Mutex<HashMap<String, (Instant, u64, bool)>>,
}

impl Views {
    pub fn mode(&self, live: &Live) -> Reflow {
        if live.with_screen(Screen::alternate_screen) {
            return Reflow::Alternate;
        }
        let mut cache = self.harness.lock().unwrap();
        let id = live.dir.id();
        let job = live.job_revision();
        let harness = match cache.get(id) {
            Some(&(at, seen, harness)) if seen == job && at.elapsed() < HARNESS_TTL => harness,
            _ => {
                let harness = live
                    .dir
                    .keeper()
                    .filter(|_| live.dir.exit().is_none())
                    .and_then(|k| procs::foreground(k.child))
                    .is_some_and(|fg| procs::harness(&fg).is_some());
                cache.insert(id.to_string(), (Instant::now(), job, harness));
                harness
            }
        };
        match harness {
            true => Reflow::Harness,
            false => Reflow::On,
        }
    }

    pub fn get(&self, session: &str, cols: u16) -> Arc<Mutex<View>> {
        let cols = cols.clamp(MIN_COLS, MAX_COLS);
        let mut views = self.views.lock().unwrap();
        views.retain(|_, v| v.strong_count() > 0);
        let key = (session.to_string(), cols);
        if let Some(view) = views.get(&key).and_then(Weak::upgrade) {
            return view;
        }
        let view = Arc::new(Mutex::new(View::new(cols)));
        views.insert(key, Arc::downgrade(&view));
        view
    }
}

/// A view's stream rides the session's own: every frame the session publishes is a cue to re-lay
/// the screen, and the cues queued behind one are taken with it, so a busy log costs one lay-out
/// per batch rather than one per read. The session's emulator is not touched.
pub(crate) fn stream(live: Arc<Live>, view: Arc<Mutex<View>>) -> PaneStream {
    let (tx, rx) = tokio::sync::mpsc::channel(256);
    let task = tokio::spawn(async move {
        loop {
            let mut events = live.join().events;
            let mut painter = Painter::default();
            let (cols, rows, full) = live.with_screen(|s| {
                let mut view = view.lock().unwrap();
                let shot = view.shoot(s);
                (view.cols, shot.grid.len() as u16, painter.full(&shot))
            });
            if tx.send(PaneEvent::Reset { cols, rows }).await.is_err()
                || tx
                    .send(PaneEvent::Bytes {
                        full: true,
                        bytes: full,
                    })
                    .await
                    .is_err()
            {
                return;
            }
            loop {
                match events.recv().await {
                    Ok(_) | Err(broadcast::error::RecvError::Lagged(_)) => {}
                    Err(broadcast::error::RecvError::Closed) => return,
                }
                while matches!(
                    events.try_recv(),
                    Ok(_) | Err(broadcast::error::TryRecvError::Lagged(_))
                ) {}
                let out = live.with_screen(|s| {
                    let shot = view.lock().unwrap().shoot(s);
                    (shot.grid.len() as u16 == rows).then(|| painter.diff(&shot))
                });
                match out {
                    Some(Some(bytes)) => {
                        if tx.send(PaneEvent::Bytes { full: false, bytes }).await.is_err() {
                            return;
                        }
                    }
                    Some(None) => {}
                    None => break,
                }
            }
        }
    });
    PaneStream::supervised(rx, task)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(cols: u16, rows: u16, bytes: &str) -> Screen {
        let mut s = Screen::new(cols, rows, 1000);
        s.feed(bytes.as_bytes());
        s
    }

    fn plain(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.plain.clone()).collect()
    }

    #[test]
    fn long_lines_are_re_laid_and_the_cursor_follows_them() {
        let line = "a".repeat(50) + &"b".repeat(50);
        let s = feed(100, 5, &format!("{line}\r\n$ "));
        let shot = View::new(40).shoot(&s);
        assert_eq!(
            plain(&shot.grid),
            vec![
                "a".repeat(40),
                "a".repeat(10) + &"b".repeat(30),
                "b".repeat(20),
                "$".to_string(),
                String::new()
            ]
        );
        assert_eq!(shot.cursor, (3, 2, true));
        assert!(shot.grid[0].wrapped && shot.grid[1].wrapped && !shot.grid[2].wrapped);
    }

    #[test]
    fn styles_and_wide_glyphs_survive_the_cut() {
        let s = feed(20, 3, "\x1b[31mredredred\x1b[0m日本語日本語");
        let shot = View::new(10).shoot(&s);
        assert_eq!(
            plain(&shot.grid)[..2],
            ["redredred".to_string(), "日本語日本".to_string()]
        );
        assert_eq!(shot.grid[0].styled, "\x1b[0;38;5;1mredredred\x1b[0m");
        assert_eq!(plain(&shot.grid)[2], "語");
    }

    #[test]
    fn what_does_not_fit_the_view_is_history_numbered_once() {
        let mut text = String::new();
        for n in 0..12 {
            text.push_str(&format!("{n:02}{}\r\n", "x".repeat(58)));
        }
        let s = feed(60, 6, &text);
        let mut view = View::new(30);
        let raw = view.scrollback(&s, 1000);
        let rows: Vec<&str> = raw.text.lines().collect();
        assert_eq!(raw.viewport_rows, 6);
        assert_eq!(raw.first_row, Some(0));
        assert_eq!(rows.len(), 24 + 1);
        assert!(rows[0].starts_with("00") && rows[2].starts_with("01"), "{rows:?}");
        assert!(rows[22].starts_with("11"), "{rows:?}");
        let again = view.scrollback(&s, 4);
        assert_eq!(again.first_row, Some(15));
    }
}
