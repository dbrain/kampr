use crate::grid::{Cell, CellAttrs, Color, Grid};
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

    fn put(&mut self, c: char) {
        if self.cursor.col >= self.grid.cols() {
            self.cursor.col = 0;
            self.newline();
        }
        let cell = Cell {
            ch: c,
            fg: self.pen.fg,
            bg: self.pen.bg,
            attrs: self.pen.attrs,
            link: self.link,
        };
        self.grid.set(self.cursor.col, self.cursor.row, cell);
        self.cursor.col += 1;
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
                        for c in 0..=self.cursor.col.min(self.grid.cols() - 1) {
                            self.grid.set(c, self.cursor.row, Cell::default());
                        }
                    }
                    _ => self.grid.clear(),
                }
            }
            ('K', false) => {
                let mode = params.iter().next().and_then(|p| p.first().copied()).unwrap_or(0);
                match mode {
                    0 => self.grid.clear_row_from(self.cursor.row, self.cursor.col),
                    1 => {
                        for c in 0..=self.cursor.col.min(self.grid.cols() - 1) {
                            self.grid.set(c, self.cursor.row, Cell::default());
                        }
                    }
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
