use crate::grid::{Cell, CellAttrs, Color, Grid};
use std::sync::Arc;
use unicode_segmentation::GraphemeCursor;
use unicode_width::UnicodeWidthStr;
use vte::{Params, ParamsIter, Perform};

#[derive(Debug, Clone, Copy, Default)]
pub struct Cursor {
    pub col: u16,
    pub row: u16,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Pen {
    pub fg: Color,
    pub bg: Color,
    pub attrs: CellAttrs,
}

pub struct State {
    pub grid: Grid,
    pub cursor: Cursor,
    pub pen: Pen,
    pub cursor_visible: bool,
    link: Option<u32>,
}

impl State {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self {
            grid: Grid::new(cols, rows),
            cursor: Cursor::default(),
            pen: Pen::default(),
            cursor_visible: true,
            link: None,
        }
    }

    fn last_col(&self) -> u16 {
        self.grid.cols().saturating_sub(1)
    }

    fn last_row(&self) -> u16 {
        self.grid.rows().saturating_sub(1)
    }

    fn newline(&mut self) {
        if self.cursor.row + 1 >= self.grid.rows() {
            self.grid.scroll_up();
        } else {
            self.cursor.row += 1;
        }
    }

    /// Probe #210: herdr spends two columns on a double-width glyph and addresses the next one at
    /// col+2, so advancing one leaves a blank behind every wide character — permanently, because
    /// herdr never repaints a cell it believes already matches.
    ///
    /// Probe #223: herdr's cell is a grapheme, not a code point, so a mark rides on the cell to
    /// its left instead of taking a column of its own — see [`is_boundary`] for where the grapheme
    /// ends and [`cluster_width`] for how many columns herdr spends on it.
    fn put(&mut self, c: char) {
        if self.join(c) {
            return;
        }
        let width = cluster_width(c.encode_utf8(&mut [0u8; 4]));
        if width == 0 {
            return;
        }
        // In u32 because the grid can be 65 535 columns wide — `column_bound` clamps there — and
        // a `saturating_add` reads the pending-wrap column at exactly `u16::MAX` as still inside
        // the margin, which places the next glyph at column 65 535: an add that overflows.
        if self.cursor.col as u32 + width as u32 > self.grid.cols() as u32 {
            self.cursor.col = 0;
            self.newline();
        }
        let cell = Cell {
            ch: c,
            fg: self.pen.fg,
            bg: self.pen.bg,
            attrs: self.pen.attrs,
            link: self.link,
            marks: None,
        };
        self.grid.put(self.cursor.col, self.cursor.row, cell, width);
        self.cursor.col += width;
    }

    /// The cell the cursor is sitting immediately after — the one a mark rides on. Read off the
    /// cursor rather than remembered, so every path that moves the cursor invalidates it for free,
    /// and a mark printed at column 0 has no base, exactly as herdr drops one.
    fn cluster_lead(&self) -> Option<(u16, u16)> {
        let prev = self.cursor.col.min(self.grid.cols()).checked_sub(1)?;
        let lead = if prev > 0 && self.grid.cell(prev, self.cursor.row)?.is_tail() {
            prev - 1
        } else {
            prev
        };
        Some((lead, self.cursor.row))
    }

    /// Appends `c` to the cluster already on screen, and says whether it belonged there.
    fn join(&mut self, c: char) -> bool {
        let Some((col, row)) = self.cluster_lead() else {
            return false;
        };
        let Some(cell) = self.grid.cell(col, row) else {
            return false;
        };
        if cell.is_tail() || (c.is_ascii() && cell.marks.is_none() && cell.ch.is_ascii()) {
            return false;
        }
        if cell.marks().len() >= MAX_CLUSTER_BYTES {
            return false;
        }
        let mut cluster = cell.cluster();
        let boundary = cluster.len();
        cluster.push(c);
        if is_boundary(&cluster, boundary) {
            return false;
        }
        let mut marks = String::with_capacity(cell.marks().len() + c.len_utf8());
        marks.push_str(cell.marks());
        marks.push(c);
        let mut joined = cell.clone();
        joined.marks = Some(Arc::new(marks));

        let was = if self.grid.cell(col + 1, row).is_some_and(Cell::is_tail) {
            2
        } else {
            1
        };
        // A cluster only ever grows into its second column — a variation selector buying an
        // emoji-presentation heart or a keycap its width, a prepend taking the character after
        // it, a lead jamo swallowing the syllable block — and herdr addresses whatever follows
        // past that column, measured.
        let width = if cluster_width(&cluster) == 2 && col + 1 < self.grid.cols() {
            2
        } else {
            1
        };
        self.grid.put(col, row, joined, width);
        if width > was {
            self.cursor.col = (self.cursor.col + width - was).min(self.grid.cols());
        }
        true
    }

    /// **A parameter is a slice, and flattening it is wrong twice over.** `CSI 4:3 m` is undercurl
    /// and reads as italic once its subparameter becomes a parameter of its own; `38:2::r:g:b`
    /// reads the colourspace id as red. The walk is over `ParamsIter` rather than a collected
    /// `Vec` because this is the emulator's hot path and the `Vec` was the one allocation on it
    /// (#58-#62).
    fn sgr(&mut self, params: &Params) {
        if params.is_empty() {
            self.pen = Pen::default();
            return;
        }
        let mut rest = params.iter();
        while let Some(param) = rest.next() {
            let code = param.first().copied().unwrap_or(0);
            match (code, param.len()) {
                (4, 2..) => self.pen.attrs.underline = param[1] != 0,
                (38, 1) => self.pen.fg = colour(&mut rest).unwrap_or(self.pen.fg),
                (48, 1) => self.pen.bg = colour(&mut rest).unwrap_or(self.pen.bg),
                (38, _) => self.pen.fg = sub_colour(param).unwrap_or(self.pen.fg),
                (48, _) => self.pen.bg = sub_colour(param).unwrap_or(self.pen.bg),
                _ => self.attribute(code),
            }
        }
    }

    fn attribute(&mut self, code: u16) {
        match code {
            0 => self.pen = Pen::default(),
            1 => self.pen.attrs.bold = true,
            2 => self.pen.attrs.dim = true,
            3 => self.pen.attrs.italic = true,
            4 => self.pen.attrs.underline = true,
            5 | 6 => self.pen.attrs.blink = true,
            7 => self.pen.attrs.reverse = true,
            8 => self.pen.attrs.hidden = true,
            9 => self.pen.attrs.strike = true,
            21 | 22 => {
                self.pen.attrs.bold = false;
                self.pen.attrs.dim = false;
            }
            23 => self.pen.attrs.italic = false,
            24 => self.pen.attrs.underline = false,
            25 => self.pen.attrs.blink = false,
            27 => self.pen.attrs.reverse = false,
            28 => self.pen.attrs.hidden = false,
            29 => self.pen.attrs.strike = false,
            30..=37 => self.pen.fg = Color::Indexed((code - 30) as u8),
            39 => self.pen.fg = Color::Default,
            40..=47 => self.pen.bg = Color::Indexed((code - 40) as u8),
            49 => self.pen.bg = Color::Default,
            90..=97 => self.pen.fg = Color::Indexed((code - 90 + 8) as u8),
            100..=107 => self.pen.bg = Color::Indexed((code - 100 + 8) as u8),
            _ => {}
        }
    }
}

/// How many bytes of marks one cell will carry.
///
/// A cluster is rebuilt on every code point that joins it, so an unbounded one is quadratic: on
/// this crate 4 KB of marks on a single cell took 5.9 ms, 16 KB 89 ms, 64 KB 1.43 s and 128 KB
/// 5.72 s, all of it under the pane's mutex. Real content is nowhere near this — a four-person ZWJ
/// family sequence is 25 bytes, a flag 8, the Devanagari and Bengali clusters of #225 under 10 —
/// so this is five times the longest cluster measured anywhere and still a ceiling. Past it a code
/// point is put as if it opened a cluster of its own, which is where a mark with no base already
/// goes: nowhere.
pub(crate) const MAX_CLUSTER_BYTES: usize = 128;

/// Herdr 0.8.2 breaks cells exactly where UAX #29 breaks extended grapheme clusters — measured
/// against where it wraps a string at the right margin of a 93-column pane (#225 — herdr's cell
/// boundary is UAX #29). Devanagari `\u{915}\u{94D}\u{937}` wraps whole and the same shape in
/// Tamil does not, which is GB9c to the letter and not something four hand-written rules reach.
fn is_boundary(cluster: &str, at: usize) -> bool {
    GraphemeCursor::new(at, cluster.len(), true)
        .is_boundary(cluster, 0)
        .unwrap_or(true)
}

/// The most columns the emulator can spend laying `text` out, escape sequences and all.
///
/// A bound rather than a count, because what it sizes is a grid the rows are about to be laid out
/// on: over-estimating costs cells, and under-estimating wraps a row and loses one off the top.
/// A code point either opens a cluster, costing [`cluster_width`], or joins the one before it,
/// costing at most the single column that cluster can grow into.
///
/// **Which is why it walks clusters and not code points.** Charging every code point a column
/// charged one for each of the marks the emulator spends nothing on: 80 columns of text wearing
/// combining marks bounded at 65 535, and the grid that sizes is `65535 * rows` cells of 40 bytes.
/// The walk mirrors [`State::join`] — the same boundary rule and the same [`MAX_CLUSTER_BYTES`] —
/// because the loop it is bounding is that one.
pub fn column_bound(text: &str) -> u16 {
    let mut cols: usize = 0;
    let mut cell = ClusterBound::default();
    let mut rest = text.chars();
    while let Some(c) = rest.next() {
        match c {
            '\u{1b}' => {
                if skip_sequence(&mut rest) {
                    cell.detach();
                }
            }
            '\t' => {
                cols = (cols / 8 + 1) * 8;
                cell.detach();
            }
            c if c.is_control() => cell.detach(),
            c => cols += cell.charge(c) as usize,
        }
    }
    cols.min(u16::MAX as usize) as u16
}

/// The cell [`column_bound`] is charging for, standing in for the one [`State::put`] would be
/// writing. An SGR leaves it alone, exactly as an SGR leaves the cell the cursor is sitting after
/// alone — a mark separated from its base by one still rides on that base and still buys it a
/// second column — while anything that moves the cursor or erases under it detaches.
#[derive(Default)]
struct ClusterBound {
    cluster: String,
    marks: usize,
    charged: u16,
    detached: bool,
}

impl ClusterBound {
    fn charge(&mut self, c: char) -> u16 {
        // Two, because the cell under a moved cursor is unknown: this either opens one of its own,
        // which is two columns at most, or joins one already there and grows it by one.
        if self.detached {
            self.detached = false;
            self.cluster.clear();
            self.cluster.push(c);
            self.marks = 0;
            self.charged = 2;
            return 2;
        }
        if self.joins(c) {
            self.cluster.push(c);
            self.marks += c.len_utf8();
            // A cluster only ever grows into its second column. A base that spent no column has no
            // cell of its own, so what a mark after it joins is the cell before *that* one — which
            // has the same one column to grow into, and is charged it here.
            let growth = 2 - self.charged.max(1);
            self.charged = 2;
            return growth;
        }
        let width = cluster_width(c.encode_utf8(&mut [0u8; 4]));
        // `put` drops a code point of no width, leaving the cell it could not join exactly as it
        // was — including, past the ceiling, still full.
        if width == 0 {
            return 0;
        }
        self.cluster.clear();
        self.cluster.push(c);
        self.marks = 0;
        self.charged = width;
        width
    }

    /// The cursor moved, so a mark arriving next rides on a cell this walk has not charged for —
    /// a blank one, which it can still grow into a second column, and which carries no marks of
    /// its own however full the cell before the move was. Measured: `\u{2764}` `HT` `\u{1F3FD}`
    /// spends nine columns and was bounded at eight.
    fn detach(&mut self) {
        self.detached = true;
    }

    fn joins(&mut self, c: char) -> bool {
        if self.cluster.is_empty() || self.marks >= MAX_CLUSTER_BYTES {
            return false;
        }
        if c.is_ascii() && self.marks == 0 && self.cluster.starts_with(|b: char| b.is_ascii()) {
            return false;
        }
        let at = self.cluster.len();
        self.cluster.push(c);
        let joins = !is_boundary(&self.cluster, at);
        self.cluster.truncate(at);
        joins
    }
}

/// Consumes one escape sequence, and says whether it can have moved the cursor or erased what it
/// is sitting after — either of which puts a different cell under the mark that arrives next. The
/// actions listed are the ones [`State::csi_dispatch`] implements that do either; an action added
/// there and not here is a bound that reads a column short.
fn skip_sequence(rest: &mut std::str::Chars<'_>) -> bool {
    match rest.next() {
        Some('[') => {
            for c in rest {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    return matches!(c, 'H' | 'f' | 'A' | 'B' | 'C' | 'D' | 'G' | 'd' | 'J' | 'K');
                }
            }
            false
        }
        // OSC, and the four `vte` swallows whole without one `print` between them: DCS, SOS, PM
        // and APC. A sixel image is tens of kilobytes of payload the emulator spends nothing on,
        // and every byte of it was a column here.
        Some(']' | 'P' | 'X' | '^' | '_') => {
            let mut after_escape = false;
            for c in rest {
                if c == '\u{7}' || (after_escape && c == '\\') {
                    break;
                }
                after_escape = c == '\u{1b}';
            }
            false
        }
        _ => false,
    }
}

fn is_regional(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

/// The columns herdr spends on one cluster, which is **not** `unicode-width`'s sum over it.
/// A conjoining jamo block sums to 4 or 6 and herdr spends 2; a prepend chain sums to 3 or 4 and
/// herdr spends 2; an unpaired regional indicator sums to 1 and herdr spends 2, the same as the
/// flag it half is (#226, #227).
fn cluster_width(cluster: &str) -> u16 {
    if cluster.starts_with(is_regional) {
        return 2;
    }
    cluster.width().min(2) as u16
}

/// `38;5;n` and `38;2;r;g;b`, whose parts arrive as parameters of their own.
fn colour(rest: &mut ParamsIter<'_>) -> Option<Color> {
    let mut next = || rest.next().and_then(|p| p.first().copied());
    match next()? {
        5 => Some(Color::Indexed(next()? as u8)),
        2 => Some(Color::Rgb(next()? as u8, next()? as u8, next()? as u8)),
        _ => None,
    }
}

/// `38:5:n` and `38:2:<colourspace>:r:g:b`, whose parts are subparameters of one parameter. The
/// colourspace id is all but always empty and is skipped by length rather than by value, because
/// the four-part form without it is what several emitters actually send.
fn sub_colour(param: &[u16]) -> Option<Color> {
    match param.get(1)? {
        5 => Some(Color::Indexed(*param.get(2)? as u8)),
        2 if param.len() >= 6 => Some(Color::Rgb(param[3] as u8, param[4] as u8, param[5] as u8)),
        2 => Some(Color::Rgb(
            *param.get(2)? as u8,
            *param.get(3)? as u8,
            *param.get(4)? as u8,
        )),
        _ => None,
    }
}

fn arg(params: &Params, idx: usize, default: u16) -> u16 {
    params
        .iter()
        .nth(idx)
        .and_then(|p| p.first().copied())
        .filter(|v| *v != 0)
        .unwrap_or(default)
}

impl Perform for State {
    fn print(&mut self, c: char) {
        self.put(c);
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            b'\n' => self.newline(),
            b'\r' => self.cursor.col = 0,
            b'\t' => {
                self.cursor.col = (self.cursor.col / 8)
                    .saturating_add(1)
                    .saturating_mul(8)
                    .min(self.last_col())
            }
            0x08 => self.cursor.col = self.cursor.col.saturating_sub(1),
            _ => {}
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell: bool) {
        // OSC 8 ;; URI  — an empty URI closes the run. Herdr emits the closing form on every
        // frame, which is why hyperlinks survive here but not through `pane.read`.
        if params.first().map(|p| p == b"8").unwrap_or(false) {
            let uri = params.get(2).copied().unwrap_or(b"");
            self.link = if uri.is_empty() {
                None
            } else {
                self.grid.intern_link(&String::from_utf8_lossy(uri))
            };
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        let private = intermediates.first() == Some(&b'?');
        match (action, private) {
            // Every one of these is clamped into the grid it is addressing, and none of them
            // reshapes it: herdr's serialiser only ever addresses inside the geometry the stream
            // was opened at, so a parameter that lands outside is hostile input rather than a
            // pane that grew (ADR 0002).
            ('H', false) | ('f', false) => {
                self.cursor.row = arg(params, 0, 1).saturating_sub(1).min(self.last_row());
                self.cursor.col = arg(params, 1, 1).saturating_sub(1).min(self.last_col());
            }
            ('A', false) => self.cursor.row = self.cursor.row.saturating_sub(arg(params, 0, 1)),
            ('B', false) => {
                self.cursor.row = self
                    .cursor
                    .row
                    .saturating_add(arg(params, 0, 1))
                    .min(self.last_row())
            }
            ('C', false) => {
                self.cursor.col = self
                    .cursor
                    .col
                    .saturating_add(arg(params, 0, 1))
                    .min(self.last_col())
            }
            ('D', false) => self.cursor.col = self.cursor.col.saturating_sub(arg(params, 0, 1)),
            ('G', false) => self.cursor.col = arg(params, 0, 1).saturating_sub(1).min(self.last_col()),
            ('d', false) => self.cursor.row = arg(params, 0, 1).saturating_sub(1).min(self.last_row()),
            ('J', false) => {
                let mode = params.iter().next().and_then(|p| p.first().copied()).unwrap_or(0);
                match mode {
                    0 => {
                        self.grid.clear_row_from(self.cursor.row, self.cursor.col);
                        for r in self.cursor.row + 1..self.grid.rows() {
                            self.grid.clear_row_from(r, 0);
                        }
                    }
                    1 => {
                        for r in 0..self.cursor.row {
                            self.grid.clear_row_from(r, 0);
                        }
                        self.grid.clear_span(self.cursor.row, 0, self.cursor.col);
                    }
                    _ => self.grid.clear(),
                }
            }
            ('K', false) => {
                let mode = params.iter().next().and_then(|p| p.first().copied()).unwrap_or(0);
                match mode {
                    0 => self.grid.clear_row_from(self.cursor.row, self.cursor.col),
                    1 => self.grid.clear_span(self.cursor.row, 0, self.cursor.col),
                    _ => self.grid.clear_row_from(self.cursor.row, 0),
                }
            }
            ('m', false) => self.sgr(params),
            ('h', true) | ('l', true) => {
                let set = action == 'h';
                for p in params.iter().flat_map(|p| p.iter().copied()) {
                    if p == 25 {
                        self.cursor_visible = set;
                    }
                }
            }
            _ => {}
        }
    }
}
