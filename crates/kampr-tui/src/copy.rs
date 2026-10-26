//! Copy mode's keyboard cursor: a cell of the pane's **surface** — the ring, then the live grid,
//! numbered the way [`crate::mouse::Selection`] numbers it — and the selection it makes, which is
//! handed to [`crate::mouse::Mouse`] so a drag and `v` are one selection and `y` takes the later.

use crate::mouse::Selection;
use crate::render::fit::Placement;
use kampr_term::Cell;
use ratatui::buffer::Buffer;
use ratatui::layout::Position;
use ratatui::style::{Modifier, Style};

/// The right half of a double-width glyph (#210); a cursor never rests on one.
const TAIL: char = '\0';

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Motion {
    Left,
    Right,
    Up,
    Down,
    WordNext,
    WordBack,
    WordEnd,
    LineStart,
    FirstNonBlank,
    LineEnd,
    ParagraphBack,
    ParagraphNext,
    Top,
    Bottom,
    Rows(i32),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Chars,
    Lines,
}

#[derive(Debug, Clone)]
pub struct Cursor {
    pub pane: String,
    pub at: (u16, u16),
    anchor: Option<((u16, u16), Kind)>,
    /// The selection last handed to the mouse. When the mouse holds anything else, a drag came
    /// after it and the anchor is forgotten.
    made: Option<Selection>,
    /// Where the last frame put the cursor on screen, so the borrowed row can get out of its way.
    pub drawn: Option<Position>,
}

impl Cursor {
    pub fn new(pane: &str, at: (u16, u16)) -> Self {
        Self {
            pane: pane.to_string(),
            at,
            anchor: None,
            made: None,
            drawn: None,
        }
    }

    pub fn selecting(&self) -> bool {
        self.anchor.is_some()
    }

    /// The selection this cursor handed over, if it is still the one the mouse holds.
    pub fn made(&self) -> Option<&Selection> {
        self.made.as_ref()
    }

    pub fn superseded(&mut self, holding: Option<&Selection>) {
        if self.made.is_some() && holding != self.made.as_ref() {
            self.anchor = None;
            self.made = None;
        }
    }

    pub fn drop_selection(&mut self) {
        self.anchor = None;
        self.made = None;
    }

    /// `v` and `V`: the same kind again ends it, the other kind converts it, as vi does.
    pub fn toggle(&mut self, kind: Kind) {
        self.anchor = match self.anchor {
            Some((_, held)) if held == kind => None,
            Some((from, _)) => Some((from, kind)),
            None => Some((self.at, kind)),
        };
    }

    pub fn forget(&mut self) -> Option<Selection> {
        self.made.take()
    }

    pub fn selection(&mut self, cols: u16) -> Option<Selection> {
        let (from, kind) = self.anchor?;
        let selection = match kind {
            Kind::Chars => Selection {
                pane: self.pane.clone(),
                from,
                to: self.at,
                block: false,
            },
            Kind::Lines => Selection {
                pane: self.pane.clone(),
                from: (0, from.1.min(self.at.1)),
                to: (cols.saturating_sub(1), from.1.max(self.at.1)),
                block: false,
            },
        };
        self.made = Some(selection.clone());
        Some(selection)
    }

    pub fn walk<R: AsRef<[Cell]>>(&mut self, motion: Motion, rows: &[R], cols: u16) {
        let Some(last) = rows.len().checked_sub(1) else {
            return;
        };
        let last = last.min(u16::MAX as usize) as u16;
        let surface = Surface { rows, cols };
        let (col, row) = (self.at.0, self.at.1.min(last));
        let at = match motion {
            Motion::Left => (col.saturating_sub(1), row),
            Motion::Right => (col.saturating_add(1).min(surface.right(row)), row),
            Motion::Up => (col, row.saturating_sub(1)),
            Motion::Down => (col, row.saturating_add(1).min(last)),
            Motion::Rows(by) => {
                let to = (row as i64 + by as i64).clamp(0, last as i64) as u16;
                (col, to)
            }
            Motion::LineStart => (0, row),
            Motion::FirstNonBlank => (surface.first(row), row),
            Motion::LineEnd => (surface.end(row), row),
            Motion::Top => (0, 0),
            Motion::Bottom => (0, last),
            Motion::ParagraphBack => (0, (0..row).rev().find(|r| surface.blank_row(*r)).unwrap_or(0)),
            Motion::ParagraphNext => (
                0,
                (row + 1..=last).find(|r| surface.blank_row(*r)).unwrap_or(last),
            ),
            Motion::WordNext => surface.word_next((col, row)),
            Motion::WordBack => surface.word_back((col, row)),
            Motion::WordEnd => surface.word_end((col, row)),
        };
        self.at = surface.settle(at, motion == Motion::Right);
    }

    /// Reverse video on the cursor's cell, if this placement shows it.
    pub fn paint(&mut self, buf: &mut Buffer, pane: &str, placement: &Placement, ring: u16) {
        if pane != self.pane {
            return;
        }
        self.drawn = shown(placement, ring, self.at);
        if let Some(at) = self.drawn
            && let Some(cell) = buf.cell_mut(at)
        {
            cell.set_style(Style::default().add_modifier(Modifier::REVERSED));
        }
    }
}

fn shown(placement: &Placement, ring: u16, (col, row): (u16, u16)) -> Option<Position> {
    let pan = placement.pan.col;
    let (rect, first) = match row >= ring {
        true => (placement.grid, ring + placement.skip_grid),
        false => (placement.history, placement.skip_history),
    };
    if col < pan || row < first {
        return None;
    }
    let (x, y) = (rect.x + (col - pan), rect.y + (row - first));
    (x < rect.right() && y < rect.bottom()).then_some(Position::new(x, y))
}

struct Surface<'a, R> {
    rows: &'a [R],
    cols: u16,
}

#[derive(PartialEq, Eq, Clone, Copy)]
enum Class {
    Blank,
    Word,
    Punct,
}

impl<R: AsRef<[Cell]>> Surface<'_, R> {
    fn row(&self, row: u16) -> &[Cell] {
        self.rows.get(row as usize).map(AsRef::as_ref).unwrap_or_default()
    }

    fn right(&self, row: u16) -> u16 {
        (self.row(row).len().max(self.cols as usize).max(1) - 1).min(u16::MAX as usize) as u16
    }

    fn ch(&self, (col, row): (u16, u16)) -> char {
        let cells = self.row(row);
        match cells.get(col as usize) {
            Some(cell) if cell.ch == TAIL && col > 0 => cells.get(col as usize - 1).map_or(' ', |c| c.ch),
            Some(cell) => cell.ch,
            None => ' ',
        }
    }

    fn class(&self, at: (u16, u16)) -> Class {
        match self.ch(at) {
            c if c.is_whitespace() || c == TAIL => Class::Blank,
            c if c.is_alphanumeric() || c == '_' => Class::Word,
            _ => Class::Punct,
        }
    }

    fn blank_row(&self, row: u16) -> bool {
        self.row(row).iter().all(|c| c.ch == ' ' || c.ch == TAIL)
    }

    fn first(&self, row: u16) -> u16 {
        let cells = self.row(row);
        cells
            .iter()
            .position(|c| c.ch != ' ' && c.ch != TAIL)
            .unwrap_or(0) as u16
    }

    fn end(&self, row: u16) -> u16 {
        let cells = self.row(row);
        let last = cells.iter().rposition(|c| c.ch != ' ').unwrap_or(0) as u16;
        self.lead(last, row)
    }

    fn lead(&self, col: u16, row: u16) -> u16 {
        match self.row(row).get(col as usize) {
            Some(cell) if cell.ch == TAIL => col.saturating_sub(1),
            _ => col,
        }
    }

    /// A row that filled its last column runs on into the next one, the rule
    /// [`crate::render::grid::logical`] joins on; any other row ends in a blank, so a word never
    /// runs from one line into the next.
    fn wrapped(&self, row: u16) -> bool {
        let cells = self.row(row);
        self.cols > 0
            && cells.len() >= self.cols as usize
            && cells.get(self.cols as usize - 1).is_some_and(|c| c.ch != ' ')
    }

    /// The column one past a row's cells is its end of line, a blank, unless the row wraps.
    fn width(&self, row: u16) -> u16 {
        let len = self.row(row).len().min(u16::MAX as usize - 1) as u16;
        match self.wrapped(row) {
            true => len.max(1),
            false => len + 1,
        }
    }

    fn next(&self, (col, row): (u16, u16)) -> Option<(u16, u16)> {
        if col + 1 < self.width(row) {
            return Some((col + 1, row));
        }
        ((row as usize) + 1 < self.rows.len()).then_some((0, row + 1))
    }

    fn prev(&self, (col, row): (u16, u16)) -> Option<(u16, u16)> {
        if col > 0 {
            return Some((col - 1, row));
        }
        let up = row.checked_sub(1)?;
        Some((self.width(up) - 1, up))
    }

    fn word_next(&self, from: (u16, u16)) -> (u16, u16) {
        let class = self.class(from);
        let mut at = from;
        if class != Class::Blank {
            while self.class(at) == class {
                match self.next(at) {
                    Some(next) => at = next,
                    None => return at,
                }
            }
        }
        while self.class(at) == Class::Blank {
            match self.next(at) {
                Some(next) => at = next,
                None => return at,
            }
        }
        at
    }

    fn word_end(&self, from: (u16, u16)) -> (u16, u16) {
        let Some(mut at) = self.next(from) else {
            return from;
        };
        while self.class(at) == Class::Blank {
            match self.next(at) {
                Some(next) => at = next,
                None => return at,
            }
        }
        let class = self.class(at);
        while let Some(next) = self.next(at).filter(|n| self.class(*n) == class) {
            at = next;
        }
        at
    }

    fn word_back(&self, from: (u16, u16)) -> (u16, u16) {
        let Some(mut at) = self.prev(from) else {
            return from;
        };
        while self.class(at) == Class::Blank {
            match self.prev(at) {
                Some(prev) => at = prev,
                None => return at,
            }
        }
        let class = self.class(at);
        while let Some(prev) = self.prev(at).filter(|p| self.class(*p) == class) {
            at = prev;
        }
        at
    }

    /// Off the end of a row and off the tail of a wide glyph, onto a cell that is there.
    fn settle(&self, (col, row): (u16, u16), rightward: bool) -> (u16, u16) {
        let col = col.min(self.right(row));
        match self.row(row).get(col as usize) {
            Some(cell) if cell.ch == TAIL && rightward && col < self.right(row) => (col + 1, row),
            _ => (self.lead(col, row), row),
        }
    }
}
