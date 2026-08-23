use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(tag = "k", content = "v")]
pub enum Color {
    #[default]
    #[serde(rename = "d")]
    Default,
    #[serde(rename = "i")]
    Indexed(u8),
    #[serde(rename = "r")]
    Rgb(u8, u8, u8),
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(default)]
pub struct CellAttrs {
    #[serde(skip_serializing_if = "is_false")]
    pub bold: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub dim: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub italic: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub underline: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub blink: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub reverse: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub strike: bool,
    #[serde(skip_serializing_if = "is_false")]
    pub hidden: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    #[serde(flatten)]
    pub attrs: CellAttrs,
    /// Index into the grid's hyperlink table; OSC 8 survives here where `pane.read` drops it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<u32>,
}

/// The right half of a double-width glyph. Herdr's own cell model spends two columns on one
/// (probe #210), so the grid does too; `'\0'` is a character no `print` can produce, and a
/// consumer that forgets to skip it renders something obviously wrong rather than a plausible
/// blank in the middle of a CJK line.
const TAIL: char = '\0';

impl Cell {
    pub fn is_tail(&self) -> bool {
        self.ch == TAIL
    }

    fn tail(&self) -> Self {
        Self { ch: TAIL, ..*self }
    }
}

impl Default for Cell {
    fn default() -> Self {
        Self {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            attrs: CellAttrs::default(),
            link: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RowDiff {
    /// Wide enough for an absolute scrollback index: a ring outgrows 16 bits long before it
    /// outgrows memory.
    pub row: u32,
    pub cells: Vec<Cell>,
}

#[derive(Debug, Clone)]
pub struct Grid {
    cols: u16,
    rows: u16,
    cells: Vec<Cell>,
    dirty: Vec<bool>,
    pub links: Vec<String>,
}

impl Grid {
    pub fn new(cols: u16, rows: u16) -> Self {
        let n = cols as usize * rows as usize;
        Self {
            cols,
            rows,
            cells: vec![Cell::default(); n],
            dirty: vec![true; rows as usize],
            links: Vec::new(),
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn row(&self, row: u16) -> &[Cell] {
        let start = row as usize * self.cols as usize;
        &self.cells[start..start + self.cols as usize]
    }

    /// The row as text: one character per glyph, so a double-width glyph contributes itself once
    /// and not a trailing NUL. Column and string index part company here — that is the point.
    pub fn row_text(&self, row: u16) -> String {
        if row >= self.rows {
            return String::new();
        }
        let text: String = self
            .row(row)
            .iter()
            .filter(|c| !c.is_tail())
            .map(|c| c.ch)
            .collect();
        text.trim_end().to_string()
    }

    pub fn set(&mut self, col: u16, row: u16, cell: Cell) {
        if col >= self.cols || row >= self.rows {
            return;
        }
        let i = row as usize * self.cols as usize + col as usize;
        if self.cells[i] != cell {
            self.cells[i] = cell;
            self.dirty[row as usize] = true;
        }
    }

    pub fn clear(&mut self) {
        self.cells.fill(Cell::default());
        self.dirty.fill(true);
        self.links.clear();
    }

    /// Blanks both halves of any double-width glyph straddling the boundary to the left of `col`,
    /// so nothing is ever left holding half a character.
    fn split_wide(&mut self, row: u16, col: u16) {
        if row >= self.rows || col == 0 || col >= self.cols {
            return;
        }
        if self.cells[row as usize * self.cols as usize + col as usize].is_tail() {
            self.set(col - 1, row, Cell::default());
            self.set(col, row, Cell::default());
        }
    }

    pub fn clear_span(&mut self, row: u16, from: u16, to: u16) {
        if row >= self.rows || from > to || from >= self.cols {
            return;
        }
        let to = to.min(self.cols - 1);
        self.split_wide(row, from);
        self.split_wide(row, to + 1);
        for c in from..=to {
            self.set(c, row, Cell::default());
        }
    }

    pub fn clear_row_from(&mut self, row: u16, from_col: u16) {
        if self.cols == 0 {
            return;
        }
        self.clear_span(row, from_col, self.cols - 1);
    }

    /// Places a glyph of `width` columns, clearing whatever double-width glyph it lands on top of.
    pub fn put(&mut self, col: u16, row: u16, cell: Cell, width: u16) {
        self.split_wide(row, col);
        self.split_wide(row, col + width);
        self.set(col, row, cell);
        if width == 2 {
            self.set(col + 1, row, cell.tail());
        }
    }

    pub fn scroll_up(&mut self) {
        let w = self.cols as usize;
        self.cells.copy_within(w.., 0);
        let last = self.cells.len() - w;
        self.cells[last..].fill(Cell::default());
        self.dirty.fill(true);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        if cols == self.cols && rows == self.rows {
            return;
        }
        *self = Grid::new(cols, rows);
    }

    pub fn take_dirty(&mut self) -> Vec<RowDiff> {
        let mut out = Vec::new();
        for r in 0..self.rows {
            if std::mem::replace(&mut self.dirty[r as usize], false) {
                out.push(RowDiff {
                    row: r as u32,
                    cells: self.row(r).to_vec(),
                });
            }
        }
        out
    }

    pub fn intern_link(&mut self, uri: &str) -> u32 {
        if let Some(i) = self.links.iter().position(|u| u == uri) {
            return i as u32;
        }
        self.links.push(uri.to_string());
        (self.links.len() - 1) as u32
    }
}
