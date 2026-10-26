//! The operator's hands on the mosaic: resize mode, spatial focus, and the drags that move a
//! divider or the sidebar's edge. **Kampr's own boxes, never the pane** (ADR 0002, ADR 0015):
//! nothing here sends an op.

use super::App;
use crate::keymap::Dir;
use crate::mosaic::{Axis, Divider, Shape};
use crate::sidebar;

const STEP: i32 = 50;
const SIDEBAR_LEAST: u16 = 16;
const SIDEBAR_MOST: u16 = 80;

impl App {
    pub(super) fn sidebar_width(&self) -> u16 {
        self.prefs
            .sidebar
            .unwrap_or(sidebar::WIDTH)
            .clamp(SIDEBAR_LEAST, SIDEBAR_MOST)
    }

    fn shown(&self) -> (Vec<String>, Shape, Option<usize>) {
        let (mosaic, shape) = self.shape();
        let at = mosaic
            .iter()
            .position(|p| Some(p.as_str()) == self.focus.as_deref());
        (mosaic, shape, at)
    }

    pub(super) fn resize(&mut self, axis: Axis, grow: bool) {
        let (shown, shape, at) = self.shown();
        let Some(at) = at.filter(|_| shown.len() > 1) else {
            self.note("there is one pane on screen — nothing to resize");
            return;
        };
        let by = if grow { STEP } else { -STEP };
        if !self.tiling.grow(&shape, at, axis, by) {
            self.note(match axis {
                Axis::Across => "nothing beside this pane to give it width",
                Axis::Down => "nothing above or below this pane to give it height",
            });
        }
    }

    pub(super) fn arrange(&mut self) {
        if self.shape().1.is_tree() {
            self.note("this tab is laid out the way its own layout has it");
            return;
        }
        self.tiling.arrange = self.tiling.arrange.next();
        self.note(format!("the mosaic is {}", self.tiling.arrange.name()));
    }

    pub(super) fn focus_toward(&mut self, dir: Dir) {
        let (shown, shape, Some(at)) = self.shown() else {
            return;
        };
        if let Some(next) = self.tiling.neighbour(&shape, at, dir) {
            self.focus(shown[next].clone());
        }
    }

    pub(super) fn divide(&mut self, divider: Divider, to: u16) {
        self.tiling.drag(&divider, to);
    }

    pub(super) fn edge(&mut self, width: u16, done: bool) {
        self.prefs.sidebar = Some(width.clamp(SIDEBAR_LEAST, SIDEBAR_MOST));
        if done {
            self.keep_prefs();
        }
    }
}
