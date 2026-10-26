//! Copy mode's keyboard half: where the cursor starts, how the window follows it, and the
//! selection it hands to the mouse. The motions themselves are [`crate::copy`].

use super::{App, View};
use crate::copy::{Cursor, Kind, Motion};
use crate::keymap::{Action, Dir, Mode};
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kampr_term::Cell;
use ratatui::layout::Rect;

impl App {
    /// What the last copy put on the clipboard, by whichever gesture.
    pub fn copied(&self) -> Option<&str> {
        self.clipboard.as_deref()
    }

    pub(super) fn clip(&mut self, text: &str) {
        crate::osc52(text);
        self.clipboard = Some(text.to_string());
    }

    /// Opens or closes the cursor with the mode, however the mode was left.
    pub(super) fn copy_follows(&mut self, before: Mode) {
        match (before == Mode::Copy, self.router.mode() == Mode::Copy) {
            (false, true) => self.copying = self.place_cursor(),
            (true, false) => {
                if let Some(cursor) = self.copying.take()
                    && cursor.made().is_some()
                    && self.mouse.selection() == cursor.made()
                {
                    self.mouse.deselect();
                }
            }
            _ => {}
        }
    }

    /// Every key copy mode takes comes here first: a drag since the last one has replaced the
    /// cursor's selection, and `esc` and `q` put a selection down before they put the mode down.
    pub(super) fn copy_key(&mut self, key: KeyEvent) -> bool {
        let leaving = matches!(key.code, KeyCode::Esc | KeyCode::Char('q'))
            && !key.modifiers.contains(KeyModifiers::CONTROL);
        let Some(cursor) = self.copying.as_mut() else {
            return false;
        };
        cursor.superseded(self.mouse.selection());
        if !leaving || !cursor.selecting() {
            return false;
        }
        cursor.drop_selection();
        self.mouse.deselect();
        true
    }

    /// The pane's own cursor when it is on screen, and the bottom-left of the window otherwise.
    fn place_cursor(&self) -> Option<Cursor> {
        let pane = self.focus.clone()?;
        if self.view(&pane) != View::Terminal {
            return None;
        }
        let state = self.client.state();
        let held = state.pane(&pane)?;
        let ring = self.rings.get(&pane).map_or(0, Vec::len) as u16;
        let total = ring.saturating_add(held.rows().len() as u16);
        let scroll = self.scrolls.get(&pane).copied().unwrap_or_default();
        let cursor = held.cursor();
        if cursor.visible && scroll == 0 {
            return Some(Cursor::new(&pane, (cursor.col, ring + cursor.row)));
        }
        let pan = self.pans.get(&pane).map_or(0, |pan| pan.col);
        let bottom = total.saturating_sub(1).saturating_sub(scroll);
        Some(Cursor::new(&pane, (pan, bottom)))
    }

    pub(super) fn copy_select(&mut self, kind: Kind) -> bool {
        let Some(cursor) = self.copying.as_mut() else {
            return false;
        };
        cursor.toggle(kind);
        self.hand_over();
        true
    }

    /// One motion of the cursor, and the window brought along to keep it in sight. `false` when
    /// there is no cursor — a conversation, or no mode — and the caller's own meaning stands.
    pub(super) fn walk(&mut self, action: Action) -> bool {
        if self.router.mode() != Mode::Copy {
            return false;
        }
        let Some(pane) = self.copying.as_ref().map(|c| c.pane.clone()) else {
            return false;
        };
        if self.focus.as_deref() != Some(pane.as_str()) {
            self.copying = self.place_cursor();
            return self.copying.is_some();
        }
        let window = self.layout.pane(&pane).map(|placed| placed.inner);
        let height = window.map_or(10, |w| w.height.max(1)) as i32;
        let motion = match action {
            Action::Move(Dir::Left) => Motion::Left,
            Action::Move(Dir::Right) => Motion::Right,
            Action::Move(Dir::Up) => Motion::Up,
            Action::Move(Dir::Down) => Motion::Down,
            Action::WordNext => Motion::WordNext,
            Action::WordBack => Motion::WordBack,
            Action::WordEnd => Motion::WordEnd,
            Action::LineStart => Motion::LineStart,
            Action::FirstNonBlank => Motion::FirstNonBlank,
            Action::LineEnd => Motion::LineEnd,
            Action::ParagraphBack => Motion::ParagraphBack,
            Action::ParagraphNext => Motion::ParagraphNext,
            Action::HistoryTop => Motion::Top,
            Action::HistoryBottom => Motion::Bottom,
            Action::HalfPageUp => Motion::Rows(-height / 2),
            Action::HalfPageDown => Motion::Rows(height / 2),
            Action::Scroll(Dir::Up) => Motion::Rows(-height),
            Action::Scroll(Dir::Down) => Motion::Rows(height),
            _ => return false,
        };
        let (total, cols) = {
            let state = self.client.state();
            let Some(held) = state.pane(&pane) else {
                return false;
            };
            let ring = self.rings.get(&pane).map(Vec::as_slice).unwrap_or_default();
            let surface: Vec<&[Cell]> = ring
                .iter()
                .map(Vec::as_slice)
                .chain(held.rows().iter().map(Vec::as_slice))
                .collect();
            let (cols, _) = held.geometry();
            let Some(cursor) = self.copying.as_mut() else {
                return false;
            };
            cursor.walk(motion, &surface, cols);
            (surface.len().min(u16::MAX as usize) as u16, cols)
        };
        if let Motion::Rows(by) = motion {
            let at = self.scrolls.entry(pane.clone()).or_default();
            *at = (*at as i32 - by).clamp(0, u16::MAX as i32) as u16;
        }
        if let Some(window) = window {
            self.follow(&pane, window, total, cols);
        }
        self.hand_over();
        true
    }

    fn follow(&mut self, pane: &str, window: Rect, total: u16, cols: u16) {
        let Some(at) = self.copying.as_ref().map(|c| c.at) else {
            return;
        };
        let scroll = self.scrolls.entry(pane.to_string()).or_default();
        let held = (*scroll).min(total.saturating_sub(window.height));
        let bottom = total.saturating_sub(1).saturating_sub(held);
        let top = total.saturating_sub(held).saturating_sub(window.height);
        *scroll = match at.1 {
            row if row < top => held + (top - row),
            row if row > bottom => held - (row - bottom),
            _ => held,
        };
        if cols > window.width {
            let pan = self.pans.entry(pane.to_string()).or_default();
            if at.0 < pan.col {
                pan.col = at.0;
            } else if at.0 >= pan.col + window.width {
                pan.col = at.0 + 1 - window.width;
            }
        }
    }

    fn hand_over(&mut self) {
        let cols = {
            let Some(pane) = self.copying.as_ref().map(|c| c.pane.clone()) else {
                return;
            };
            let state = self.client.state();
            state.pane(&pane).map_or(0, |held| held.geometry().0)
        };
        let Some(cursor) = self.copying.as_mut() else {
            return;
        };
        match cursor.selection(cols) {
            Some(selection) => self.mouse.select(selection),
            None => {
                if let Some(made) = cursor.forget()
                    && self.mouse.selection() == Some(&made)
                {
                    self.mouse.deselect();
                }
            }
        }
    }

    /// The borrowed row moves to the top of the panes while the cursor is on it: herdr's footer
    /// paints over the pane's last row (#374), which is where a shell's prompt — and so the
    /// cursor — usually is.
    pub(super) fn copy_dodge(&self, borrowed: Rect, panes: Rect) -> Rect {
        match self.copying.as_ref().and_then(|c| c.drawn) {
            Some(at) if at.y == borrowed.y && panes.height > 1 => Rect {
                y: panes.y,
                ..borrowed
            },
            _ => borrowed,
        }
    }
}
