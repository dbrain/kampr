use serde::{Deserialize, Serialize};

/// Which mouse reports a program asked for: `?1000` is a press and release, `?1002` adds a drag and
/// `?1003` all motion. The widest one wins when several are set.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MouseTracking {
    #[default]
    None,
    Press,
    Drag,
    Any,
}

/// The DEC private modes of a pane that decide what a wheel notch, a drag or a click should send its
/// program (docs/26), as the program itself last set them.
///
/// **Absent on a pane is not the same as all-off.** A herdr pane reports no modes at all (probe #686),
/// so the field is left off its entry and a client keeps choosing by the pane's `agent`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PaneModes {
    #[serde(default)]
    pub mouse: MouseTracking,
    /// `?1006`: reports are SGR, `ESC [ < b ; col ; row M`, rather than the legacy 223-column form.
    #[serde(default)]
    pub mouse_sgr: bool,
    /// `?1007`: a wheel over the alternate screen is cursor keys when no mouse mode is set.
    #[serde(default)]
    pub alt_scroll: bool,
    /// `?1`: cursor keys are the application form, `ESC O A`.
    #[serde(default)]
    pub app_cursor: bool,
    /// `?1049`. A native session's emulator does not track `?47` or `?1047` (probe #687), so a
    /// program that uses only those reads as not being on the alternate screen.
    #[serde(default)]
    pub alt_screen: bool,
}
