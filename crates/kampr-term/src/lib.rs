//! Server-side VT emulation.
//!
//! Herdr sends *diff* frames, so applying them requires emulator state. Kampr keeps exactly one
//! emulator per pane, shared by every viewer, and ships clients a cell grid instead of escape
//! bytes — so no client parses ANSI.
//!
//! Scope is deliberately narrow: herdr's frame serialiser emits absolute cursor addressing, SGR,
//! erase, and the synchronised-output and hyperlink markers. It never relies on scroll regions or
//! relative motion, because every frame repaints by position.

mod grid;
mod perform;

#[cfg(test)]
mod tests;

pub use grid::{Cell, CellAttrs, Color, Grid, RowDiff};

use vte::Parser;

pub struct Emulator {
    parser: Parser,
    state: perform::State,
}

impl Emulator {
    pub fn new(cols: u16, rows: u16) -> Self {
        Self { parser: Parser::new(), state: perform::State::new(cols, rows) }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.state, bytes);
    }

    /// A `full: true` frame is a complete repaint; resetting first stops a stale cell surviving
    /// underneath one.
    pub fn reset(&mut self) {
        self.state.grid.clear();
        self.state.cursor = Default::default();
        self.state.pen = Default::default();
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        self.state.grid.resize(cols, rows);
    }

    pub fn grid(&self) -> &Grid {
        &self.state.grid
    }

    pub fn cursor(&self) -> (u16, u16, bool) {
        (self.state.cursor.col, self.state.cursor.row, self.state.cursor_visible)
    }

    /// Rows changed since the last call, cleared as it goes.
    pub fn take_dirty(&mut self) -> Vec<RowDiff> {
        self.state.grid.take_dirty()
    }
}
