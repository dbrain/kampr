use crate::render::fit::Pan;
use crate::theme::Theme;
use kampr_core::wire::Cursor;
use kampr_term::Cell;
use std::borrow::Cow;

use ratatui::buffer::Buffer;
use ratatui::layout::{Position, Rect};
use ratatui::widgets::Widget;

/// The right half of a double-width glyph. Herdr spends two columns on one and addresses the
/// next glyph at col+2 (#210), so a consumer that draws this renders a glyph's second column
/// twice and puts every column after it out by one.
const TAIL: char = '\0';

/// A cell's text is its base followed by its marks — a combining mark, a ZWJ or a variation
/// selector rides on the base it belongs to rather than occupying a cell of its own (#223).
/// A cell's text: its base followed by whatever it is wearing.
///
/// Borrowed for a cell wearing nothing, which is nearly every cell — `char::to_string` heap-
/// allocated for each one, 6000 per frame on a 150x40 grid, and text shaping is already the whole
/// cost of a frame (#58-#62). The marked path owns, and is rare by construction (#223).
pub fn text<'a>(cell: &'a Cell, scratch: &'a mut [u8; 4]) -> Cow<'a, str> {
    match &cell.marks {
        Some(marks) => {
            let mut s = String::with_capacity(cell.ch.len_utf8() + marks.len());
            s.push(cell.ch);
            s.push_str(marks);
            Cow::Owned(s)
        }
        None => Cow::Borrowed(cell.ch.encode_utf8(scratch)),
    }
}

/// A range of the pane's surface the operator has dragged over, in surface coordinates.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Highlight {
    pub from: (u16, u16),
    pub to: (u16, u16),
    pub block: bool,
}

impl Highlight {
    fn covers(&self, col: u16, row: u16) -> bool {
        let (a, b) = match (self.from.1, self.from.0) <= (self.to.1, self.to.0) {
            true => (self.from, self.to),
            false => (self.to, self.from),
        };
        if self.block {
            return (a.1..=b.1).contains(&row) && (a.0.min(b.0)..=a.0.max(b.0)).contains(&col);
        }
        (a.1..=b.1).contains(&row) && !(row == a.1 && col < a.0) && !(row == b.1 && col > b.0)
    }
}

pub struct Grid<'a> {
    pub rows: &'a [Vec<Cell>],
    pub pan: Pan,
    pub theme: &'a Theme,
    /// The surface row `rows[pan.row]` is. The ring and the live grid are drawn by two of these
    /// over one continuous surface, so a selection that crosses the join lands on both.
    pub base: u16,
    pub selected: Option<Highlight>,
}

impl Widget for Grid<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        for y in 0..area.height {
            let Some(row) = self.rows.get((self.pan.row + y) as usize) else {
                continue;
            };
            let surface_row = self.base + self.pan.row + y;
            let mut src = self.pan.col as usize;
            let mut x = 0u16;
            while x < area.width {
                let Some(cell) = row.get(src) else { break };
                let Some(slot) = buf.cell_mut(Position::new(area.x + x, area.y + y)) else {
                    break;
                };
                let mut style = self.theme.cell(cell);
                if self.selected.is_some_and(|s| s.covers(src as u16, surface_row)) {
                    style = style.bg(self.theme.accent_soft);
                }
                if cell.ch == TAIL {
                    // A pan that lands on a tail has cut a wide glyph in half; its lead is off
                    // screen, so there is nothing honest to draw in the column it left behind.
                    slot.set_char(' ').set_style(style);
                    src += 1;
                    x += 1;
                    continue;
                }
                slot.set_symbol(&text(cell, &mut [0u8; 4])).set_style(style);
                let wide = matches!(row.get(src + 1), Some(next) if next.ch == TAIL);
                if wide {
                    // ratatui's own convention: the cell after a double-width symbol is blank,
                    // and its buffer diff skips it rather than painting over the glyph's second
                    // column.
                    if x + 1 < area.width
                        && let Some(tail) = buf.cell_mut(Position::new(area.x + x + 1, area.y + y))
                    {
                        tail.reset();
                        tail.set_style(style);
                    }
                    src += 2;
                    x += 2;
                } else {
                    src += 1;
                    x += 1;
                }
            }
        }
    }
}

/// Where the caret lands on screen, or `None` when it is hidden or panned out of view.
pub fn caret(cursor: Cursor, area: Rect, pan: Pan) -> Option<Position> {
    if !cursor.visible || cursor.col < pan.col || cursor.row < pan.row {
        return None;
    }
    let x = area.x + (cursor.col - pan.col);
    let y = area.y + (cursor.row - pan.row);
    (x < area.right() && y < area.bottom()).then_some(Position::new(x, y))
}

/// The **logical** text of a span of rows, not the painted grid: trailing padding stripped and a
/// row that is a soft wrap of the one above joined to it. A path or a URL copied with a newline
/// through the middle of it is worse than not copying.
pub fn logical<R: AsRef<[Cell]>>(rows: &[R], from: (u16, u16), to: (u16, u16), cols: u16) -> String {
    let (start, end) = if (from.1, from.0) <= (to.1, to.0) {
        (from, to)
    } else {
        (to, from)
    };
    let mut out = String::new();
    for y in start.1..=end.1 {
        let Some(row) = rows.get(y as usize).map(AsRef::as_ref) else {
            break;
        };
        let first = if y == start.1 { start.0 as usize } else { 0 };
        let last = if y == end.1 {
            (end.0 as usize + 1).min(row.len())
        } else {
            row.len()
        };
        let mut line = String::new();
        for cell in row.iter().take(last).skip(first) {
            if cell.ch != TAIL {
                line.push_str(&text(cell, &mut [0u8; 4]));
            }
        }
        let wrapped =
            row.len() >= cols as usize && cols > 0 && row.get(cols as usize - 1).is_some_and(|c| c.ch != ' ');
        out.push_str(line.trim_end());
        if y < end.1 && !wrapped {
            out.push('\n');
        }
    }
    out
}
