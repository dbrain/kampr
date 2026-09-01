//! The grid and the fit ladder.

use kampr_term::{Cell, CellAttrs, Color};
use kampr_tui::render::fit::{self, Chrome, Need, Pan, Refusal, Rung};
use kampr_tui::render::grid::{self, Grid};
use kampr_tui::theme::PHOSPHOR;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::widgets::Widget;
use std::sync::Arc;

fn cell(ch: char) -> Cell {
    Cell {
        ch,
        fg: Color::Default,
        bg: Color::Default,
        attrs: CellAttrs::default(),
        link: None,
        marks: None,
    }
}

fn marked(ch: char, marks: &str) -> Cell {
    Cell {
        marks: Some(Arc::new(marks.to_string())),
        ..cell(ch)
    }
}

fn draw(rows: &[Vec<Cell>], width: u16, height: u16) -> Buffer {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    Grid {
        base: 0,
        selected: None,
        rows,
        pan: Pan::default(),
        theme: &PHOSPHOR,
    }
    .render(area, &mut buf);
    buf
}

fn symbols(buf: &Buffer, y: u16, width: u16) -> Vec<String> {
    (0..width).map(|x| buf[(x, y)].symbol().to_string()).collect()
}

#[test]
fn a_wide_glyphs_tail_is_not_drawn_and_the_next_glyph_lands_at_col_plus_two() {
    // herdr spends two columns on one glyph and addresses the next at col+2 (#210); the tail is
    // a cell whose `ch` is NUL and a client that draws it puts every column after it out by one.
    let row = vec![cell('A'), cell('日'), cell('\0'), cell('B')];
    let buf = draw(&[row], 4, 1);
    assert_eq!(
        symbols(&buf, 0, 4),
        vec!["A", "日", " ", "B"],
        "the tail is blank — ratatui's own well-formed-buffer rule — and B lands at col+2"
    );
}

#[test]
fn a_cells_marks_are_rendered_with_its_base_and_not_as_a_separate_cell() {
    // ré-su-mé: four cells, four columns, and the acute rides on the base it belongs to (#223).
    let row = vec![
        cell('r'),
        marked('e', "\u{301}"),
        cell('s'),
        marked('e', "\u{301}"),
    ];
    let buf = draw(&[row], 4, 1);
    assert_eq!(
        symbols(&buf, 0, 4),
        vec!["r", "e\u{301}", "s", "e\u{301}"],
        "a mark is part of its cell's text, not a cell of its own"
    );
}

#[test]
fn a_pan_that_cuts_a_wide_glyph_in_half_draws_nothing_in_the_column_it_left() {
    let rows = vec![vec![cell('日'), cell('\0'), cell('B')]];
    let area = Rect::new(0, 0, 2, 1);
    let mut buf = Buffer::empty(area);
    Grid {
        base: 0,
        selected: None,
        rows: &rows,
        pan: Pan { col: 1, row: 0 },
        theme: &PHOSPHOR,
    }
    .render(area, &mut buf);
    assert_eq!(symbols(&buf, 0, 2), vec![" ", "B"]);
}

#[test]
fn copied_text_is_the_logical_text_and_not_the_painted_grid() {
    let mut wrapped: Vec<Cell> = "https://herdr.dev/a".chars().map(cell).collect();
    wrapped.extend("bcd".chars().map(cell));
    let second: Vec<Cell> = "efg"
        .chars()
        .map(cell)
        .chain(std::iter::repeat_n(cell(' '), 19))
        .collect();
    let rows = vec![wrapped, second];
    let text = grid::logical(&rows, (0, 0), (21, 1), 22);
    assert_eq!(
        text, "https://herdr.dev/abcdefg",
        "a soft wrap is joined and the padding is stripped — a URL with a newline through it is \
         worse than not copying"
    );
}

struct Fake {
    cells: (u16, u16),
    largest: Option<(u16, u16)>,
    honours: bool,
    asked: Vec<(u16, u16)>,
}

impl fit::Display for Fake {
    fn cells(&mut self) -> Option<(u16, u16)> {
        Some(self.cells)
    }
    fn host(&mut self) -> Option<String> {
        Some("fake 1.0".into())
    }
    fn largest(&mut self) -> Option<(u16, u16)> {
        self.largest
    }
    fn request(&mut self, cols: u16, rows: u16) {
        self.asked.push((cols, rows));
        if self.honours {
            self.cells = (cols, rows);
        }
    }
    fn settle(&mut self, was: (u16, u16)) -> Option<(u16, u16)> {
        (self.cells != was).then_some(self.cells)
    }
}

#[test]
fn the_ladder_stops_at_rung_one_when_the_terminal_is_already_wide_enough() {
    let mut fake = Fake {
        cells: (200, 60),
        largest: Some((300, 80)),
        honours: false,
        asked: Vec::new(),
    };
    let rung = fit::climb(
        &mut fake,
        Need { cols: 93, rows: 40 },
        Chrome { cols: 32, rows: 5 },
        true,
    );
    assert_eq!(rung, Rung::Fits);
    assert!(fake.asked.is_empty(), "rung 1 never writes to the terminal");
}

#[test]
fn the_ladder_reports_rung_three_when_the_resize_request_is_refused() {
    // ghostty 1.3.1 and kitty 0.48.2 ignore `CSI 8;rows;cols t` outright (#291), so on this desk
    // rung 3 is the path and the ladder has to say so rather than silently cropping.
    let mut fake = Fake {
        cells: (80, 24),
        largest: Some((300, 80)),
        honours: false,
        asked: Vec::new(),
    };
    let rung = fit::climb(
        &mut fake,
        Need { cols: 171, rows: 40 },
        Chrome { cols: 32, rows: 5 },
        true,
    );
    assert_eq!(
        rung,
        Rung::CropAndPan {
            host: Some("fake 1.0".into()),
            refusal: Refusal::Ignored
        }
    );
    assert_eq!(fake.asked, vec![(203, 45)], "it did ask before it gave up");
    assert_eq!(rung.number(), 3);
    assert!(rung.report().contains("rung 2 was refused by fake 1.0"));
}

#[test]
fn the_ladder_climbs_to_rung_two_when_the_terminal_honours_it() {
    let mut fake = Fake {
        cells: (80, 24),
        largest: Some((300, 80)),
        honours: true,
        asked: Vec::new(),
    };
    let rung = fit::climb(
        &mut fake,
        Need { cols: 171, rows: 40 },
        Chrome { cols: 32, rows: 5 },
        true,
    );
    assert_eq!(rung.number(), 2);
    assert!(rung.report().contains("grew to 203×45"));
}

#[test]
fn the_ladder_refuses_a_window_the_display_cannot_show() {
    // konsole honoured 400x900 on a 2560x1440 display and handed back a window the operator can
    // see a slice of (#291). The clamp is this client's, because the emulator has none.
    let mut fake = Fake {
        cells: (80, 24),
        largest: Some((120, 40)),
        honours: true,
        asked: Vec::new(),
    };
    let rung = fit::climb(&mut fake, Need { cols: 400, rows: 90 }, Chrome::default(), true);
    assert_eq!(
        rung,
        Rung::CropAndPan {
            host: Some("fake 1.0".into()),
            refusal: Refusal::LargerThanDisplay
        }
    );
    assert!(fake.asked.is_empty(), "it refused before it wrote the request");
}

#[test]
fn default_zoom_fills_an_axis_rather_than_letterboxing() {
    // A pane shorter than its box does not sit in the middle of one. The live viewport is pinned
    // to the bottom and the history above it fills everything else: scrollback and the live grid
    // are one continuous surface, and blank space below the last row is a bug, not a layout.
    let area = Rect::new(3, 2, 60, 30);
    // With a ring above it, the two rectangles tile the whole box and nothing is blank.
    let placed = fit::place(area, Need { cols: 93, rows: 12 }, 40, 0, Pan::default());
    assert_eq!(placed.grid.height, 12);
    assert_eq!(
        placed.grid.bottom(),
        area.bottom(),
        "the last row settles on the bottom edge"
    );
    assert_eq!(placed.history.y, area.y);
    assert_eq!(
        placed.history.height + placed.grid.height,
        area.height,
        "history and the live grid are one continuous surface — no band is left over"
    );
    assert_eq!(placed.grid.width, area.width, "and the surface fills the width");

    // With no ring there is nothing above to fill with, and the live rows are still pinned to
    // the bottom rather than floating in the middle of a letterbox.
    let bare = fit::place(area, Need { cols: 93, rows: 12 }, 0, 0, Pan::default());
    assert_eq!(bare.grid.bottom(), area.bottom());
    assert_eq!(bare.history.height, 0);
}

#[test]
fn a_pane_taller_than_its_box_takes_the_whole_box() {
    let area = Rect::new(0, 0, 60, 20);
    let placed = fit::place(area, Need { cols: 93, rows: 40 }, 0, 0, Pan::default());
    assert_eq!(placed.grid, area);
    assert_eq!(placed.history.height, 0);
    assert_eq!(placed.skip_grid, 20, "the live viewport shows its own last rows");
}

#[test]
fn a_pan_is_clamped_to_what_there_is_to_pan_over() {
    let area = Rect::new(0, 0, 60, 20);
    let placed = fit::place(area, Need { cols: 93, rows: 40 }, 0, 0, Pan { col: 900, row: 0 });
    assert_eq!(placed.pan.col, 33);
}

#[test]
fn scrolling_moves_one_window_over_history_and_the_live_grid() {
    // Scrollback and the live grid are one continuous surface, not two panels: history scrolls
    // up out of the top and the live viewport sits at the bottom until the reader leaves it.
    let area = Rect::new(0, 0, 60, 10);
    let need = Need { cols: 93, rows: 6 };

    let bottom = fit::place(area, need, 20, 0, Pan::default());
    assert_eq!(bottom.grid.height, 6, "the whole live viewport is on screen");
    assert_eq!(bottom.history.height, 4);
    assert_eq!(bottom.skip_history, 16, "showing the ring's last four rows");

    let up = fit::place(area, need, 20, 6, Pan::default());
    assert_eq!(up.grid.height, 0, "scrolled clear of the live grid");
    assert_eq!(up.history.height, 10);
    assert_eq!(
        up.history.bottom(),
        area.bottom(),
        "and still no blank band below"
    );
    assert_eq!(up.skip_history, 10);

    let past = fit::place(area, need, 20, 999, Pan::default());
    assert_eq!(past.scroll, 16, "clamped to the top of what there is");
    assert_eq!(past.skip_history, 0);
}
