use kampr_core::HerdrProvider;
use kampr_herdr::{Controller, Herdr};
use serde_json::json;
use std::sync::Arc;
use tracing::{debug, warn};

/// What a *matched* hold puts back when it lets go, and the evidence that putting it back is
/// honest rather than one more write nobody asked for.
///
/// A held controller **is** the pane's geometry (#18): while the hold stands herdr refuses a
/// second controller (#21) and the desk is overridden, so nothing else can have moved the pane.
/// [`Self::still_ours`] is how that is checked rather than assumed — `viewport_rows` is the PTY's
/// own and herdr reports it honestly (#84, #207), so a pane that no longer reads back the rows
/// this hold put on it is a pane something else has taken, and the one thing Kampr must not then
/// do is put a size on it that nobody asked for (#298).
#[derive(Clone)]
pub struct Restore {
    pub binary: String,
    pub herdr: Herdr,
    pub provider: Arc<HerdrProvider>,
    /// The pane's own geometry, read before the *first* claim of a run of matched holds and
    /// carried across every re-claim and every handover to a newer viewer — so that dragging a
    /// window twice does not make the size Kampr set the size Kampr restores.
    pub found: (u16, u16),
    /// The rows the claim standing right now put on the pane.
    pub applied_rows: u32,
}

impl Restore {
    /// Whether the pane still reads back the geometry this hold put on it. Asked *before* the
    /// controller goes, because release hands an attached desk its own geometry back inside a
    /// second (#19) and afterwards the reading is no longer about the hold at all.
    ///
    /// A pane herdr will not answer for is `false`: a check that could not be made must not become
    /// permission to write.
    pub async fn still_ours(&self, pane: &str) -> bool {
        viewport_rows(&self.herdr, pane).await == Some(u64::from(self.applied_rows))
    }

    /// Puts `found` back, through the same claim-resize-release `pane.size` already uses.
    ///
    /// **The 80x24 floor does not apply here.** That floor exists so Kampr can never leave a pane
    /// too small to use (#219, ADR 0012); putting back the size a pane was found at cannot do
    /// that, and refusing would strand it at the viewer's size instead — which is the failure the
    /// restore exists to prevent.
    pub async fn apply(&self, pane: &str) {
        let (cols, rows) = self.found;
        let controller = Controller::claim(
            &self.binary,
            self.herdr.socket(),
            pane,
            u32::from(cols),
            u32::from(rows),
        )
        .await;
        let controller = match controller {
            Ok(c) => c,
            Err(e) => {
                warn!(pane = %pane, error = %e, "could not claim a pane to put its own size back");
                return;
            }
        };
        if let Err(e) = controller.release().await {
            warn!(pane = %pane, error = %e, "releasing the controller that put a pane's size back");
        }
        // The same check `pane.size`'s `once` mode makes, for the same reason: on an attached pane
        // the desk takes the geometry straight back (#19), and adopting a width the PTY does not
        // have crops every client rather than only the one that asked (#233).
        if viewport_rows(&self.herdr, pane).await == Some(u64::from(rows)) {
            // The restore claims, resizes and lets go, so nothing is holding this width: it is the
            // pane's own again and the inference is free to move it.
            self.provider.resized(pane, cols, false);
            debug!(pane = %pane, cols, rows, "a matched hold put the pane's own size back");
        }
    }
}

async fn viewport_rows(herdr: &Herdr, pane: &str) -> Option<u64> {
    let reply: kampr_herdr::model::PaneReply =
        herdr.call("pane.get", json!({ "pane_id": pane })).await.ok()?;
    reply.pane.scroll.map(|s| s.viewport_rows)
}
