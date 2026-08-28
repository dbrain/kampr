//! Shaping herdr's own agents sidebar, at somebody else's desk.
//!
//! This is a sibling of [`crate::reporter`] rather than part of it because almost nothing about it
//! is the same problem. A report is per pane and is **confirmed by reading `pane.get` back**
//! (probe #295); a view is one per herdr session, cannot be read back at all — there is no
//! `agent.view.get` and `agent.list` is untouched by it (probe #296) — and has a teardown, because
//! a sort that outlives the node that set it is somebody's sidebar left wrong for ever.
//!
//! It writes into a screen this node is only looking at, so it is off unless an operator turned it
//! on. Probe #298 is the same class with the volume up: a viewer reshaped somebody's PTY and their
//! screen went wrong with no error anywhere.

use kampr_herdr::{Herdr, SortOrder};
use tracing::{debug, info, warn};

/// The word that replaces the sort-mode in the sidebar's section header (probe #296). herdr
/// refuses an empty label or one past 32 characters.
pub const LABEL: &str = "kampr name";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct View {
    pub source: String,
    pub token: String,
    pub order: SortOrder,
    pub label: String,
}

impl View {
    /// Sorted by the name this node computed, which reaches herdr as a token on
    /// [`crate::reporter::TOKEN`] and exists only while reporting is on: the sortable builtins are
    /// `agent` and `status` and nothing else, so there is no way to sort a desk by a Kampr name
    /// that does not go through a reported token.
    pub fn by_name() -> Self {
        Self {
            source: crate::reporter::SOURCE.to_string(),
            token: crate::reporter::TOKEN.to_string(),
            order: SortOrder::Asc,
            label: LABEL.to_string(),
        }
    }
}

#[derive(Debug, Default)]
pub struct DeskAgents {
    /// An async lock, held across the round trip, because the check and the write have to be one
    /// step: two sweeps overlap whenever a watcher speeds the poll up while the slow one is in
    /// flight, and a check-then-act would have both of them decide the view had never been sent.
    sent: tokio::sync::Mutex<Option<View>>,
}

impl DeskAgents {
    pub fn new() -> Self {
        Self::default()
    }

    /// Brings the desk to `want`, and is a no-op when it is already there.
    ///
    /// **What this compares against is what it last sent, not what herdr holds**, because herdr
    /// will not say what it holds (probe #296). That record is the only thing standing between one
    /// setting and a write into somebody's sidebar on every sweep for ever.
    ///
    /// A node that never sorted this desk sends no clear either: `agent.view.clear` carries no
    /// source and wipes whatever view is active, whoever set it, so an unconditional clear on the
    /// way out would throw away a view the operator set for themselves.
    pub async fn sweep(&self, herdr: &Herdr, want: Option<&View>) {
        let mut sent = self.sent.lock().await;
        if sent.as_ref() == want {
            return;
        }
        match want {
            Some(view) => match herdr
                .set_agent_view(&view.source, &view.token, view.order, &view.label)
                .await
            {
                Ok(reply) => {
                    *sent = Some(view.clone());
                    debug!(
                        active = reply.active,
                        label = %view.label,
                        token = %view.token,
                        "asked this desk's agents sidebar to sort by Kampr's name; herdr does not \
                         say back what it sorted on, so this is what was sent and not what it did"
                    );
                }
                Err(e) => warn!(error = %e, "could not sort this desk's agents sidebar"),
            },
            None => match herdr.clear_agent_view().await {
                Ok(_) => {
                    *sent = None;
                    info!("put this desk's own agent order back");
                }
                Err(e) => warn!(
                    error = %e,
                    "could not put this desk's own agent order back; the sidebar is still sorted \
                     by Kampr's name and nothing else will clear it"
                ),
            },
        }
    }

    /// On the way out. Identical to a sweep that wants nothing, deliberately: a setting turned off
    /// and a node shutting down owe the desk the same thing, and one path is one path to get right.
    pub async fn restore(&self, herdr: &Herdr) {
        self.sweep(herdr, None).await;
    }
}
