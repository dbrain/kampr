use super::Screen;
use alacritty_terminal::term::TermMode;
use kampr_core::{MouseTracking, PaneModes};

impl Screen {
    /// What the program has asked of the mouse and the cursor keys, for a client to choose its wheel
    /// bytes by (docs/26). alacritty starts with `?1007` on, as xterm-likes do, so a program that
    /// never mentions it still takes cursor keys on the alternate screen (#390).
    pub fn modes(&self) -> PaneModes {
        let mode = *self.term.mode();
        PaneModes {
            mouse: if mode.contains(TermMode::MOUSE_MOTION) {
                MouseTracking::Any
            } else if mode.contains(TermMode::MOUSE_DRAG) {
                MouseTracking::Drag
            } else if mode.contains(TermMode::MOUSE_REPORT_CLICK) {
                MouseTracking::Press
            } else {
                MouseTracking::None
            },
            mouse_sgr: mode.contains(TermMode::SGR_MOUSE),
            alt_scroll: mode.contains(TermMode::ALTERNATE_SCROLL),
            app_cursor: mode.contains(TermMode::APP_CURSOR),
            alt_screen: mode.contains(TermMode::ALT_SCREEN),
        }
    }
}
