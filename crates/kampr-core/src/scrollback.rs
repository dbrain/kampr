use crate::provider::RawScrollback;
use kampr_term::{Emulator, RowDiff};

#[derive(Debug, Clone)]
pub struct ScrollbackDoc {
    /// Absolute index of the first delivered row, counted from the top of herdr's ring.
    pub from_top: u32,
    pub rows: Vec<RowDiff>,
    pub total_rows: u32,
    pub complete: bool,
}

/// Renders `pane.read recent format=ansi` through the same emulator the live grid uses, so a
/// client's history is styled identically to its viewport.
pub fn render(raw: &RawScrollback) -> ScrollbackDoc {
    let mut lines: Vec<&str> = raw.text.split('\n').collect();
    if lines.last().is_some_and(|l| l.is_empty()) {
        lines.pop();
    }
    // `recent` returns history *and* the live viewport; the viewport already travels as the grid.
    let keep = lines.len().saturating_sub(raw.viewport_rows as usize);
    let total_rows = raw.scrollback_rows.max(keep as u32);
    let from_top = total_rows - keep as u32;
    if keep == 0 {
        return ScrollbackDoc {
            from_top: 0,
            rows: Vec::new(),
            total_rows,
            complete: !raw.truncated,
        };
    }

    let mut term = Emulator::new(raw.cols.max(1), keep as u16);
    // herdr separates rows with LF alone, which moves down without returning the carriage.
    term.feed(lines[..keep].join("\r\n").as_bytes());

    let grid = term.grid();
    let rows = (0..keep as u16)
        .map(|r| RowDiff {
            row: (from_top + r as u32).min(u16::MAX as u32) as u16,
            cells: grid.row(r).to_vec(),
        })
        .collect();

    ScrollbackDoc {
        from_top,
        rows,
        total_rows,
        complete: from_top == 0 && !raw.truncated,
    }
}
