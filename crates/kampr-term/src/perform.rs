use crate::grid::{Cell, CellAttrs, Color, Grid};
use std::sync::Arc;
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};
use vte::{Params, Perform};

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
    /// Probe #215: herdr's cell is a grapheme, not a code point. A zero-width character rides on
    /// the cell to its left instead of taking a column of its own, and dropping it — which is what
    /// this used to do — loses the accent for the same reason.
    fn put(&mut self, c: char) {
        if self.join(c) {
            return;
        }
        let width = match c.width() {
            Some(0) | None => return,
            Some(2) => 2,
            Some(_) => 1,
        };
        if self.cursor.col + width > self.grid.cols() {
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
        if cell.is_tail() || !continues(cell, c) {
            return false;
        }
        let mut marks = String::with_capacity(cell.marks().len() + c.len_utf8());
        marks.push_str(cell.marks());
        marks.push(c);
        let mut cluster = String::with_capacity(marks.len() + cell.ch.len_utf8());
        cluster.push(cell.ch);
        cluster.push_str(&marks);
        let mut joined = cell.clone();
        joined.marks = Some(Arc::new(marks));

        let was = if self.grid.cell(col + 1, row).is_some_and(Cell::is_tail) {
            2
        } else {
            1
        };
        // A variation selector can buy its base a second column — an emoji-presentation heart, a
        // keycap — and herdr addresses whatever follows past that column, measured.
        let width = if cluster.width() >= 2 && col + 1 < self.grid.cols() {
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

    fn sgr(&mut self, params: &Params) {
        let flat: Vec<u16> = params.iter().flat_map(|p| p.iter().copied()).collect();
        if flat.is_empty() {
            self.pen = Pen::default();
            return;
        }
        let mut i = 0;
        while i < flat.len() {
            match flat[i] {
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
                30..=37 => self.pen.fg = Color::Indexed((flat[i] - 30) as u8),
                38 => {
                    if let Some((c, used)) = extended(&flat[i..]) {
                        self.pen.fg = c;
                        i += used - 1;
                    }
                }
                39 => self.pen.fg = Color::Default,
                40..=47 => self.pen.bg = Color::Indexed((flat[i] - 40) as u8),
                48 => {
                    if let Some((c, used)) = extended(&flat[i..]) {
                        self.pen.bg = c;
                        i += used - 1;
                    }
                }
                49 => self.pen.bg = Color::Default,
                90..=97 => self.pen.fg = Color::Indexed((flat[i] - 90 + 8) as u8),
                100..=107 => self.pen.bg = Color::Indexed((flat[i] - 100 + 8) as u8),
                _ => {}
            }
            i += 1;
        }
    }
}

const ZWJ: char = '\u{200D}';

fn is_regional(c: char) -> bool {
    ('\u{1F1E6}'..='\u{1F1FF}').contains(&c)
}

fn is_emoji_modifier(c: char) -> bool {
    ('\u{1F3FB}'..='\u{1F3FF}').contains(&c)
}

/// Grapheme clustering cut down to what a terminal has to get right (probe #215): anything of
/// width zero, anything after a ZWJ, a skin-tone modifier, and the second of a flag's two regional
/// indicators. Herdr's own cell model was measured to agree on all four. Everything else — Hangul
/// jamo, Indic conjuncts — starts a new cell, which is what herdr does with them too.
fn continues(cell: &Cell, c: char) -> bool {
    if matches!(c.width(), Some(0) | None) || is_emoji_modifier(c) {
        return true;
    }
    let marks = cell.marks();
    marks.ends_with(ZWJ) || (marks.is_empty() && is_regional(c) && is_regional(cell.ch))
}

fn extended(rest: &[u16]) -> Option<(Color, usize)> {
    match rest.get(1)? {
        5 => Some((Color::Indexed(*rest.get(2)? as u8), 3)),
        2 => Some((
            Color::Rgb(*rest.get(2)? as u8, *rest.get(3)? as u8, *rest.get(4)? as u8),
            5,
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
            b'\t' => self.cursor.col = (self.cursor.col / 8 + 1) * 8,
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
            ('H', false) | ('f', false) => {
                self.cursor.row = arg(params, 0, 1).saturating_sub(1);
                self.cursor.col = arg(params, 1, 1).saturating_sub(1);
            }
            ('A', false) => self.cursor.row = self.cursor.row.saturating_sub(arg(params, 0, 1)),
            ('B', false) => self.cursor.row = (self.cursor.row + arg(params, 0, 1)).min(self.grid.rows() - 1),
            ('C', false) => self.cursor.col = (self.cursor.col + arg(params, 0, 1)).min(self.grid.cols() - 1),
            ('D', false) => self.cursor.col = self.cursor.col.saturating_sub(arg(params, 0, 1)),
            ('G', false) => self.cursor.col = arg(params, 0, 1).saturating_sub(1),
            ('d', false) => self.cursor.row = arg(params, 0, 1).saturating_sub(1),
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
