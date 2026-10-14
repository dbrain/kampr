//! A frame of a pane's grid is drawn on every tick and every patch, and text is the whole cost of
//! one (#58–#62). Drawing a steady grid allocates nothing; `render::grid::text` borrows for every
//! unmarked cell precisely so that this holds.

use kampr_term::{Cell, Color};
use kampr_testkit::alloc::count;
use kampr_tui::render::fit::Pan;
use kampr_tui::render::grid::Grid;
use kampr_tui::theme::PHOSPHOR;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;

const COLS: u16 = 150;
const ROWS: u16 = 40;

fn rows() -> Vec<Vec<Cell>> {
    (0..ROWS)
        .map(|row| {
            (0..COLS)
                .map(|col| Cell {
                    ch: char::from(b'a' + ((row + col) % 26) as u8),
                    fg: Color::Indexed((col % 16) as u8),
                    ..Cell::default()
                })
                .collect()
        })
        .collect()
}

fn draw(rows: &[Vec<Cell>], buffer: &mut Buffer) {
    Grid {
        rows,
        pan: Pan::default(),
        theme: &PHOSPHOR,
        base: 0,
        selected: None,
    }
    .render(buffer.area, buffer);
}

#[test]
fn drawing_a_steady_grid_allocates_nothing() {
    let rows = rows();
    let mut buffer = Buffer::empty(Rect::new(0, 0, COLS, ROWS));
    draw(&rows, &mut buffer);
    for _ in 0..3 {
        assert_eq!(count(|| draw(&rows, &mut buffer)), 0, "a {COLS}x{ROWS} frame");
    }
}
