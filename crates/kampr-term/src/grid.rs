use serde::Serialize;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
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

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
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

impl Default for Cell {
    fn default() -> Self {
        Self { ch: ' ', fg: Color::Default, bg: Color::Default, attrs: CellAttrs::default(), link: None }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct RowDiff {
    pub row: u16,
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
        Self { cols, rows, cells: vec![Cell::default(); n], dirty: vec![true; rows as usize], links: Vec::new() }
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

    pub fn clear_row_from(&mut self, row: u16, from_col: u16) {
        if row >= self.rows {
            return;
        }
        for c in from_col..self.cols {
            self.set(c, row, Cell::default());
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
                out.push(RowDiff { row: r, cells: self.row(r).to_vec() });
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
