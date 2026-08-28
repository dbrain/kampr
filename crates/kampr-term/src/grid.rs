use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    #[serde(flatten)]
    pub attrs: CellAttrs,
    /// Index into the grid's hyperlink table; OSC 8 survives here where `pane.read` drops it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub link: Option<u32>,
    /// The zero-width code points riding on [`Self::ch`] — combining marks, ZWJ, variation
    /// selectors (probe #223). Boxed rather than interned like `link`, because nothing outside
    /// this grid needs an id for them and a table would have to travel beside every `RowDiff`;
    /// `Arc<String>` is one thin pointer, so a cell that wears nothing costs 8 bytes and a row
    /// clone stays a refcount bump.
    #[serde(skip_serializing_if = "Option::is_none", serialize_with = "marks_as_str")]
    pub marks: Option<Arc<String>>,
}

/// The right half of a double-width glyph. Herdr's own cell model spends two columns on one
/// (probe #210), so the grid does too; `'\0'` is a character no `print` can produce, and a
/// consumer that forgets to skip it renders something obviously wrong rather than a plausible
/// blank in the middle of a CJK line.
const TAIL: char = '\0';

/// `serde` only reaches through an `Arc` under its `rc` feature, and one field of one struct is
/// not worth turning that on across the workspace.
fn marks_as_str<S: serde::Serializer>(marks: &Option<Arc<String>>, s: S) -> Result<S::Ok, S::Error> {
    match marks {
        Some(m) => s.serialize_str(m),
        None => s.serialize_none(),
    }
}

impl Cell {
    pub fn is_tail(&self) -> bool {
        self.ch == TAIL
    }

    pub fn marks(&self) -> &str {
        self.marks.as_deref().map_or("", String::as_str)
    }

    /// The whole grapheme this cell holds: its base and whatever rides on it.
    pub fn cluster(&self) -> String {
        let mut out = String::with_capacity(1 + self.marks().len());
        self.push_cluster(&mut out);
        out
    }

    pub fn push_cluster(&self, out: &mut String) {
        out.push(self.ch);
        out.push_str(self.marks());
    }

    /// The right half of a wide cell. It never carries the marks: they belong to the lead, and a
    /// consumer that reads both halves would otherwise render the accent twice.
    pub fn tail(&self) -> Self {
        Self {
            ch: TAIL,
            marks: None,
            ..self.clone()
        }
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
            marks: None,
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

/// How many hyperlinks one pane's grid will hold.
///
/// The table is emptied only by [`Grid::clear`], which is a `full: true` frame — the first frame
/// of an `observe` stream and no other (#53) — so on a stream of diffs it grows for the life of
/// the pane, and every entry is serialised to every viewer. A 93x40 pane is 3 720 cells, so this
/// is past a screenful of entirely distinct links and far past what real content emits; it is the
/// same ceiling, for the same reason, that `wire::MAX_STYLES` puts on a pen table a connection can
/// never evict from either.
pub(crate) const MAX_LINKS: usize = 4096;

/// How long a URI may be to be interned. Apache refuses a request line past 8 190 bytes and nginx
/// past 8 KB, so nothing a browser will follow is longer, and the longest hyperlink this crate is
/// driven with is 2 018 (`a_hyperlink_longer_than_vtes_osc_buffer_survives_whole`).
pub(crate) const MAX_LINK_BYTES: usize = 8192;

#[derive(Debug, Clone)]
pub struct Grid {
    cols: u16,
    rows: u16,
    cells: Vec<Cell>,
    dirty: Vec<bool>,
    pub links: Vec<String>,
    link_ids: HashMap<String, u32>,
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
            link_ids: HashMap::new(),
        }
    }

    pub fn cols(&self) -> u16 {
        self.cols
    }

    pub fn rows(&self) -> u16 {
        self.rows
    }

    pub fn cell(&self, col: u16, row: u16) -> Option<&Cell> {
        if col >= self.cols || row >= self.rows {
            return None;
        }
        self.cells.get(row as usize * self.cols as usize + col as usize)
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
        let mut text = String::with_capacity(self.cols as usize);
        for cell in self.row(row).iter().filter(|c| !c.is_tail()) {
            cell.push_cluster(&mut text);
        }
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
        self.link_ids.clear();
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
        if width == 2 {
            self.set(col + 1, row, cell.tail());
        }
        self.set(col, row, cell);
    }

    pub fn scroll_up(&mut self) {
        // A grid with no cells has no rows to rotate, and `rotate_left` off the end of one is a
        // slice assertion rather than an overflow check — so it panics in release too.
        if self.cells.is_empty() {
            return;
        }
        let w = self.cols as usize;
        self.cells.rotate_left(w);
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

    /// Nothing past a ceiling is truncated: half a URI points somewhere else, so a run that cannot
    /// be interned renders as the text it is with no link on it at all.
    pub fn intern_link(&mut self, uri: &str) -> Option<u32> {
        if let Some(id) = self.link_ids.get(uri) {
            return Some(*id);
        }
        if uri.len() > MAX_LINK_BYTES || self.links.len() >= MAX_LINKS {
            return None;
        }
        let id = self.links.len() as u32;
        self.links.push(uri.to_string());
        self.link_ids.insert(uri.to_string(), id);
        Some(id)
    }
}
