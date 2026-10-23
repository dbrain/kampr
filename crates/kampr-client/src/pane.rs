use kampr_core::provider::Reflow;
use kampr_core::wire::Cursor;
use kampr_mesh::shadow::{History, Shadow};
use kampr_term::Cell;

/// Everything this client holds for one pane.
///
/// The grid survives a reconnect. A full grid is about 3 KB and herdr coalesces bursts to end
/// state, so there is never a backlog to drain — a client renders what it last held, marked
/// [`stale`](Self::stale), and swaps on the `grid.reset` that follows the next greeting. No
/// spinner.
#[derive(Debug, Default)]
pub struct PaneState {
    shadow: Shadow,
    history: History,
    stale: bool,
    reflow: Option<Reflow>,
}

impl PaneState {
    pub fn shadow(&self) -> &Shadow {
        &self.shadow
    }

    pub fn history(&self) -> &History {
        &self.history
    }

    /// Whether what is on screen is the last thing that was true rather than what is true.
    pub fn stale(&self) -> bool {
        self.stale
    }

    /// Whether a grid has ever arrived. A pane the node cannot stream gets no `grid.reset` at all
    /// — the geometry is a promise that rows are coming — so this stays false and the herd
    /// entry's `detail` is what says why.
    pub fn painted(&self) -> bool {
        self.shadow.is_ready()
    }

    /// Which grid a watch that asked for a reflowed view is being sent, as its last `grid.reset`
    /// said; `None` on any other watch, and from a node older than the field.
    pub fn reflow(&self) -> Option<Reflow> {
        self.reflow
    }

    pub fn geometry(&self) -> (u16, u16) {
        self.shadow.geometry()
    }

    pub fn rows(&self) -> &[Vec<Cell>] {
        self.shadow.rows()
    }

    pub fn cursor(&self) -> Cursor {
        self.shadow.cursor()
    }

    pub fn link(&self, id: u32) -> Option<&str> {
        self.shadow.link(id)
    }

    pub fn links(&self) -> &[String] {
        self.shadow.links()
    }

    pub(crate) fn shadow_mut(&mut self) -> &mut Shadow {
        &mut self.shadow
    }

    pub(crate) fn history_mut(&mut self) -> &mut History {
        &mut self.history
    }

    pub(crate) fn set_reflow(&mut self, reflow: Option<Reflow>) {
        self.reflow = reflow;
    }

    pub(crate) fn set_stale(&mut self, stale: bool) {
        self.stale = stale;
    }
}
