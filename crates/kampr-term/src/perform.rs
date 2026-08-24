use crate::grid::{Cell, CellAttrs, Color, Grid};
use std::sync::Arc;
use unicode_segmentation::GraphemeCursor;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
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
        if self.cursor.col.saturating_add(width) > self.grid.cols() {
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
pub fn column_bound(text: &str) -> u16 {
    let mut cols: usize = 0;
    let mut rest = text.chars();
    while let Some(c) = rest.next() {
        match c {
            '\u{1b}' => skip_sequence(&mut rest),
            '\t' => cols = (cols / 8 + 1) * 8,
            c if c.is_control() => {}
            c if is_regional(c) => cols += 2,
            c => cols += c.width().unwrap_or(0).max(1),
        }
    }
    cols.min(u16::MAX as usize) as u16
}

fn skip_sequence(rest: &mut std::str::Chars<'_>) {
    match rest.next() {
        Some('[') => {
            for c in rest {
                if ('\u{40}'..='\u{7e}').contains(&c) {
                    return;
                }
            }
        }
        Some(']') => {
            let mut after_escape = false;
            for c in rest {
                if c == '\u{7}' || (after_escape && c == '\\') {
                    return;
                }
                after_escape = c == '\u{1b}';
            }
        }
        _ => {}
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
                Some(self.grid.intern_link(&String::from_utf8_lossy(uri)))
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
