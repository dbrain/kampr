//! W5 — the client's own hit testing, a selection, and a per-pane passthrough toggle.
//!
//! Nothing here reaches the wire and nothing reaches `kampr-term`: herdr's observe frames carry
//! no mouse mode and no other surface on its socket does either (#292), so *asked* can only ever
//! mean the operator asked.

use crate::app::{Layout, Placed};
use crate::render::grid;
use crossterm::event::{KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use kampr_client::{PendingOption, Role};
use kampr_term::Cell;
use ratatui::layout::Rect;
use std::collections::HashSet;

/// The right half of a double-width glyph (#210). Hit testing and selection ends resolve to the
/// lead or they are a glyph out.
const TAIL: char = '\0';

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Click {
    None,
    Focus(String),
    Tab(String),
    OpenHerd,
    Answer {
        pane: String,
        key: String,
    },
    /// An attachment this terminal will not draw inline, asked for as a file.
    Save {
        pane: String,
        id: String,
    },
    /// SGR 1006 with 1002 drag reporting, as `text`, and only for a pane the operator armed.
    Passthrough {
        pane: String,
        text: String,
    },
}

/// Surface coordinates, `(col, row)`, of the two ends the operator dragged between.
///
/// The **surface** is the pane's ring followed by its live grid — one continuous thing, the way
/// [`crate::render::fit::place`] lays it out — so a selection that starts in scrollback and ends
/// on the live viewport is one range rather than two.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Selection {
    pub pane: String,
    pub from: (u16, u16),
    pub to: (u16, u16),
    pub block: bool,
}

/// An OSC 8 URI is a harness-declared one; a bare URL found in cell text is not, and the two are
/// separate variants so a client can offer the second rather than follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Link {
    Declared(String),
    Detected(String),
}

#[derive(Debug, Default)]
pub struct Mouse {
    armed: HashSet<String>,
    focus: Option<String>,
    drag: Option<Selection>,
    done: Option<Selection>,
    /// A finished drag stays here to be painted after its text has been taken, because the
    /// highlight is what says what was copied.
    copied: bool,
    at: Option<(String, u16, u16)>,
    reported: Option<(u16, u16)>,
}

impl Mouse {
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the terminal's own mouse reporting should be turned on at all. Kampr's chrome is
    /// clickable whatever any pane is doing, so this is unconditional.
    pub fn capture(&self) -> bool {
        true
    }

    pub fn hit(&mut self, event: MouseEvent, layout: &Layout, role: Role) -> Click {
        let at = (event.column, event.row);
        match event.kind {
            MouseEventKind::Down(MouseButton::Left) => {
                self.press(at, event.modifiers.contains(KeyModifiers::ALT), layout, role)
            }
            MouseEventKind::Drag(MouseButton::Left) => self.extend(at, layout, role),
            MouseEventKind::Up(MouseButton::Left) => self.release(at, layout, role),
            MouseEventKind::ScrollUp => self.wheel(at, 64, layout, role),
            MouseEventKind::ScrollDown => self.wheel(at, 65, layout, role),
            _ => Click::None,
        }
    }

    fn press(&mut self, at: (u16, u16), block: bool, layout: &Layout, role: Role) -> Click {
        self.drag = None;
        self.done = None;
        self.reported = None;
        self.at = None;
        if let Some(tab) = pick(&layout.tabs, at) {
            return Click::Tab(tab);
        }
        // The chips sit inside a pane's own rectangle, so they are asked before it is.
        for chips in &layout.chips {
            match self.answer(&chips.pane, &chips.options, &chips.rects, at, role) {
                Click::None => {}
                answered => return answered,
            }
        }
        if let Some((pane, id)) = pick_attachment(&layout.attachments, at) {
            return Click::Save { pane, id };
        }
        if let Some(placed) = layout.panes.iter().find(|p| inside(p.rect, at)) {
            let pane = placed.pane.clone();
            let arriving = self.focus.as_deref() != Some(pane.as_str());
            if arriving {
                self.focus = Some(pane.clone());
            }
            if self.sends(&pane, role) {
                let Some(cell) = live_cell(placed, at) else {
                    return focus(arriving, pane);
                };
                // The click that focuses a pane is not the click that drives it: a pane the
                // mouse was not already on gets the focus and nothing else, so arriving from
                // another pane can never put a byte into this one.
                if arriving {
                    return Click::Focus(pane);
                }
                self.reported = Some(cell);
                return Click::Passthrough {
                    pane,
                    text: sgr(0, cell, true),
                };
            }
            let Some(cell) = surface_cell(placed, at) else {
                return focus(arriving, pane);
            };
            self.at = Some((pane.clone(), cell.0, cell.1));
            self.drag = Some(Selection {
                pane: pane.clone(),
                from: cell,
                to: cell,
                block,
            });
            return focus(arriving, pane);
        }
        if let Some(pane) = pick(&layout.herd, at) {
            return Click::Focus(pane);
        }
        if let Some(pane) = pick_row(&layout.rows, at) {
            return Click::Focus(pane);
        }
        match inside(layout.sidebar, at) {
            true => Click::OpenHerd,
            false => Click::None,
        }
    }

    fn extend(&mut self, at: (u16, u16), layout: &Layout, role: Role) -> Click {
        if let Some(drag) = self.drag.as_mut() {
            if let Some(placed) = layout.panes.iter().find(|p| inside(p.rect, at))
                && placed.pane == drag.pane
                && let Some(cell) = surface_cell(placed, at)
            {
                drag.to = cell;
            }
            return Click::None;
        }
        let Some((pane, cell)) = self.driving(at, layout, role) else {
            return Click::None;
        };
        // 1002, not 1003: motion is reported when it crosses into another cell and while a
        // button is down, which is what every mouse-aware program of the last decade asked for.
        if self.reported == Some(cell) {
            return Click::None;
        }
        self.reported = Some(cell);
        Click::Passthrough {
            pane,
            text: sgr(32, cell, true),
        }
    }

    fn release(&mut self, at: (u16, u16), layout: &Layout, role: Role) -> Click {
        if let Some(mut drag) = self.drag.take() {
            if let Some(placed) = layout.panes.iter().find(|p| inside(p.rect, at))
                && placed.pane == drag.pane
                && let Some(cell) = surface_cell(placed, at)
            {
                drag.to = cell;
            }
            if drag.from != drag.to {
                // A drag is not a click: the cell it started on must not also resolve a link.
                self.at = None;
                self.copied = false;
                self.done = Some(drag);
            }
            return Click::None;
        }
        let Some((pane, cell)) = self.driving(at, layout, role) else {
            return Click::None;
        };
        self.reported = None;
        Click::Passthrough {
            pane,
            text: sgr(0, cell, false),
        }
    }

    fn wheel(&mut self, at: (u16, u16), button: u8, layout: &Layout, role: Role) -> Click {
        let Some((pane, cell)) = self.driving(at, layout, role) else {
            return Click::None;
        };
        Click::Passthrough {
            pane,
            text: sgr(button, cell, true),
        }
    }

    fn driving(&self, at: (u16, u16), layout: &Layout, role: Role) -> Option<(String, (u16, u16))> {
        let placed = layout.panes.iter().find(|p| inside(p.rect, at))?;
        let pane = placed.pane.clone();
        let on = self.focus.as_deref() == Some(pane.as_str());
        (on && self.sends(&pane, role)).then(|| live_cell(placed, at).map(|cell| (pane, cell)))?
    }

    fn sends(&self, pane: &str, role: Role) -> bool {
        role.writes() && self.passes_through(pane)
    }

    /// Off by default, remembered per pane in `prefs`, and **offered, never flipped**: under
    /// ble.sh `pane.process_info` names only `bash` (#297), so a heuristic that failed open there
    /// would be typing into a shell.
    pub fn passes_through(&self, pane: &str) -> bool {
        self.armed.contains(pane)
    }

    pub fn set_passthrough(&mut self, pane: &str, on: bool) {
        match on {
            true => self.armed.insert(pane.to_string()),
            false => self.armed.remove(pane),
        };
    }

    pub fn footer(&self, pane: &str) -> Option<String> {
        self.passes_through(pane).then(|| "mouse → pane".to_string())
    }

    /// What is under the pointer right now, for a renderer to paint.
    pub fn selection(&self) -> Option<&Selection> {
        self.drag.as_ref().or(self.done.as_ref())
    }

    /// Whether anything is waiting on this pane's surface. Assembling one costs a row of fat
    /// pointers per line of scrollback, so the draw path asks this before it builds one.
    pub fn wants(&self, pane: &str) -> bool {
        (!self.copied && self.done.as_ref().is_some_and(|s| s.pane == pane))
            || self.at.as_ref().is_some_and(|(held, _, _)| held == pane)
    }

    /// The text of whatever is selected right now, without taking it — what `prefix [ y` copies.
    pub fn selected_text<R: AsRef<[Cell]>>(&self, rows: &[R], cols: u16) -> Option<String> {
        self.selection().map(|s| selected_text(s, rows, cols))
    }

    /// The **logical** text of a finished drag — trailing padding stripped, soft-wrapped rows
    /// joined — taken once. `rows` is the pane's surface: its ring, then its live grid.
    pub fn copy<R: AsRef<[Cell]>>(&mut self, rows: &[R], cols: u16) -> Option<String> {
        if self.copied {
            return None;
        }
        let selected = self.done.as_ref()?;
        let text = selected_text(selected, rows, cols);
        self.copied = true;
        Some(text)
    }

    /// The link under the last cell clicked, taken once. Detection runs over the *logical* line
    /// so a URL wrapped at the grid edge is not missed, and a [`Link::Detected`] is something to
    /// offer — pane output is attacker-influenceable and nothing here navigates.
    pub fn link<R: AsRef<[Cell]>>(&mut self, rows: &[R], links: &[String], cols: u16) -> Option<Link> {
        let (_, col, row) = self.at.take()?;
        let col = lead(rows, row, col);
        let cell = rows.get(row as usize)?.as_ref().get(col as usize)?;
        if let Some(id) = cell.link
            && let Some(url) = links.get(id as usize)
        {
            return Some(Link::Declared(url.clone()));
        }
        detected(rows, cols, col, row).map(Link::Detected)
    }

    /// A pending prompt's options, hit-tested against the rectangles they were drawn in. **Only
    /// a key that was offered** — the node decides whether a submit key follows, per harness, and
    /// an Enter is never synthesised (#43).
    pub fn answer(
        &self,
        pane: &str,
        options: &[PendingOption],
        rects: &[Rect],
        at: (u16, u16),
        role: Role,
    ) -> Click {
        if !role.writes() {
            return Click::None;
        }
        for (option, rect) in options.iter().zip(rects) {
            if inside(*rect, at) {
                return Click::Answer {
                    pane: pane.to_string(),
                    key: option.key.clone(),
                };
            }
        }
        Click::None
    }
}

fn focus(arriving: bool, pane: String) -> Click {
    match arriving {
        true => Click::Focus(pane),
        false => Click::None,
    }
}

fn sgr(button: u8, cell: (u16, u16), press: bool) -> String {
    let end = match press {
        true => 'M',
        false => 'm',
    };
    format!("\u{1b}[<{button};{};{}{end}", cell.0 + 1, cell.1 + 1)
}

fn inside(rect: Rect, at: (u16, u16)) -> bool {
    rect.width > 0
        && rect.height > 0
        && at.0 >= rect.x
        && at.0 < rect.right()
        && at.1 >= rect.y
        && at.1 < rect.bottom()
}

fn pick(rects: &[(String, Rect)], at: (u16, u16)) -> Option<String> {
    rects
        .iter()
        .find(|(_, rect)| inside(*rect, at))
        .map(|(id, _)| id.clone())
}

fn pick_row(rows: &[(Option<String>, Rect)], at: (u16, u16)) -> Option<String> {
    rows.iter()
        .find(|(_, rect)| inside(*rect, at))
        .and_then(|(pane, _)| pane.clone())
}

fn pick_attachment(rects: &[(String, String, Rect)], at: (u16, u16)) -> Option<(String, String)> {
    rects
        .iter()
        .find(|(_, _, rect)| inside(*rect, at))
        .map(|(pane, id, _)| (pane.clone(), id.clone()))
}

/// The cell an SGR report names: the **live viewport's** own coordinates, which is the grid the
/// program in the pane is drawing on. A pan or a scroll moves where that grid is on screen and
/// which of its columns is at the left edge, so a report worked out of the rectangle alone names
/// the wrong cell. A click in the ring is not a cell of the viewport at all.
fn live_cell(placed: &Placed, at: (u16, u16)) -> Option<(u16, u16)> {
    let placement = placed.placement?;
    inside(placement.grid, at).then(|| {
        (
            placement.pan.col + (at.0 - placement.grid.x),
            placement.skip_grid + (at.1 - placement.grid.y),
        )
    })
}

/// The cell of the pane's whole **surface** — the ring, and then the live grid under it — which
/// is what a selection and a link resolve against.
fn surface_cell(placed: &Placed, at: (u16, u16)) -> Option<(u16, u16)> {
    let placement = placed.placement?;
    if inside(placement.grid, at) {
        return Some((
            placement.pan.col + (at.0 - placement.grid.x),
            placed.ring + placement.skip_grid + (at.1 - placement.grid.y),
        ));
    }
    inside(placement.history, at).then(|| {
        (
            placement.pan.col + (at.0 - placement.history.x),
            placement.skip_history + (at.1 - placement.history.y),
        )
    })
}

fn lead<R: AsRef<[Cell]>>(rows: &[R], row: u16, col: u16) -> u16 {
    match rows.get(row as usize).and_then(|r| r.as_ref().get(col as usize)) {
        Some(cell) if cell.ch == TAIL => col.saturating_sub(1),
        _ => col,
    }
}

fn selected_text<R: AsRef<[Cell]>>(selected: &Selection, rows: &[R], cols: u16) -> String {
    let (from, to) = (selected.from, selected.to);
    if !selected.block {
        let a = (lead(rows, from.1, from.0), from.1);
        let b = (lead(rows, to.1, to.0), to.1);
        return grid::logical(rows, a, b, cols);
    }
    let (top, bottom) = (from.1.min(to.1), from.1.max(to.1));
    let (left, right) = (from.0.min(to.0), from.0.max(to.0));
    (top..=bottom)
        .filter_map(|y| {
            let row = rows.get(y as usize)?.as_ref();
            let first = lead(rows, y, left) as usize;
            let last = lead(rows, y, right) as usize;
            let mut line = String::new();
            for cell in row.iter().take(last + 1).skip(first) {
                if cell.ch != TAIL {
                    line.push_str(&grid::text(cell, &mut [0u8; 4]));
                }
            }
            Some(line.trim_end().to_string())
        })
        .collect::<Vec<_>>()
        .join("\n")
}

/// The rule [`grid::logical`] joins on: a row that filled its last column is the first half of
/// one logical line, not a line of its own.
fn wrapped<R: AsRef<[Cell]>>(rows: &[R], y: u16, cols: u16) -> bool {
    cols > 0
        && rows.get(y as usize).is_some_and(|row| {
            let row = row.as_ref();
            row.len() >= cols as usize && row.get(cols as usize - 1).is_some_and(|c| c.ch != ' ')
        })
}

fn width<R: AsRef<[Cell]>>(rows: &[R], y: u16, until: usize) -> usize {
    rows.get(y as usize)
        .map(|row| {
            row.as_ref()
                .iter()
                .take(until)
                .filter(|c| c.ch != TAIL)
                .map(|c| grid::text(c, &mut [0u8; 4]).chars().count())
                .sum()
        })
        .unwrap_or(0)
}

fn detected<R: AsRef<[Cell]>>(rows: &[R], cols: u16, col: u16, row: u16) -> Option<String> {
    let mut start = row;
    while start > 0 && wrapped(rows, start - 1, cols) {
        start -= 1;
    }
    let mut end = row;
    while wrapped(rows, end, cols) && (end as usize) + 1 < rows.len() {
        end += 1;
    }
    let line: Vec<char> = grid::logical(rows, (0, start), (cols.saturating_sub(1), end), cols)
        .chars()
        .collect();
    let mut offset = 0;
    for y in start..row {
        offset += width(rows, y, usize::MAX);
    }
    offset += width(rows, row, col as usize);
    url_at(&line, offset)
}

/// A strict scheme, conservatively: a bare URL is *detected*, not declared, so anything looser
/// hands the operator a link a pane's output invented.
const SCHEMES: [&str; 2] = ["https://", "http://"];

fn url_at(line: &[char], offset: usize) -> Option<String> {
    for scheme in SCHEMES {
        let mark: Vec<char> = scheme.chars().collect();
        for start in 0..line.len() {
            if !line[start..].starts_with(&mark[..]) {
                continue;
            }
            let mut end = start + mark.len();
            while end < line.len() && !line[end].is_whitespace() && !line[end].is_control() {
                end += 1;
            }
            while end > start + mark.len()
                && matches!(
                    line[end - 1],
                    '.' | ',' | ';' | ':' | '!' | '?' | ')' | ']' | '}' | '>' | '\'' | '"'
                )
            {
                end -= 1;
            }
            if end == start + mark.len() || !line[start + mark.len()].is_alphanumeric() {
                continue;
            }
            if (start..end).contains(&offset) {
                return Some(line[start..end].iter().collect());
            }
        }
    }
    None
}
