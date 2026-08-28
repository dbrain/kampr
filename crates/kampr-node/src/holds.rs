use kampr_herdr::{Controller, HOLD_LIMIT};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{debug, warn};

/// A release is a child exiting, which `Controller::release` already bounds at three seconds and
/// then kills — so nothing here can usefully wait longer than that plus room to notice.
const FREE_TIMEOUT: Duration = Duration::from_secs(6);
const FREE_POLL: Duration = Duration::from_millis(25);

type Held = HashMap<String, Option<oneshot::Sender<()>>>;

/// The controllers this node is holding on the operator's behalf, one per pane at most.
///
/// A hold exists because a resize on a pane with a desk client attached lasts exactly as long as
/// the claim does — release hands the desk's own geometry back inside a second (#19) — so holding
/// is the only way that pane stays the size it was asked for. It is off by default and never
/// implicit: while a controller is held the desk is ignored (#18) and an attached desk TUI renders
/// wrong without being told anything (#298).
///
/// Every hold is owned by a task that releases it at [`HOLD_LIMIT`] whatever happens, so a client
/// that dies with its panel open cannot leave a pane claimed for ever. That is the other half of
/// #20: `Controller::release` stops a controller that will not go, and this stops a *holder* that
/// never asks.
#[derive(Default)]
pub struct PaneHolds {
    inner: Arc<Mutex<Held>>,
}

impl PaneHolds {
    /// Parks a claimed controller against `pane`.
    ///
    /// The caller is what makes sure the pane was free first — herdr allows one controller at a
    /// time and refuses the second (#21), so it is the *claim* that has to be ordered, not this.
    pub fn park(&self, pane: &str, controller: Controller) {
        let (tx, rx) = oneshot::channel();
        let key = pane.to_string();
        let inner = Arc::clone(&self.inner);
        if self.replace(pane, tx).is_some() {
            debug!(pane = %pane, "a second claim replaced an existing hold");
        }
        tokio::spawn(async move {
            // Either the operator lets go, or the deadline does. `timeout` collapses both into the
            // same release, which is why there is no sweeper anywhere: the hold owns its own end.
            if tokio::time::timeout(HOLD_LIMIT, rx).await.is_err() {
                warn!(pane = %key, "a held pane hit the hold limit and was released for the operator");
            }
            if let Err(e) = controller.release().await {
                warn!(pane = %key, error = %e, "releasing a held pane");
            }
            // Cleared here rather than in `release`, so `wait_until_free` means "the controller is
            // gone" rather than "somebody asked it to go".
            inner.lock().expect("holds").remove(&key);
        });
    }

    /// Asks a hold to let go, and answers whether there was one. The controller's own release runs
    /// on the task that owns it; [`Self::wait_until_free`] is what waits for it.
    pub fn release(&self, pane: &str) -> bool {
        let mut held = self.inner.lock().expect("holds");
        match held.get_mut(pane) {
            Some(slot) => {
                if let Some(tx) = slot.take() {
                    let _ = tx.send(());
                }
                true
            }
            None => false,
        }
    }

    pub fn is_held(&self, pane: &str) -> bool {
        self.inner.lock().expect("holds").contains_key(pane)
    }

    /// Waits for a released hold's controller to actually be gone, because claiming again before it
    /// is puts two controllers on one pane and herdr refuses the second outright (#21).
    pub async fn wait_until_free(&self, pane: &str) {
        let deadline = tokio::time::Instant::now() + FREE_TIMEOUT;
        while self.is_held(pane) {
            if tokio::time::Instant::now() >= deadline {
                warn!(pane = %pane, "a released hold did not let go in time; claiming anyway");
                return;
            }
            tokio::time::sleep(FREE_POLL).await;
        }
    }

    fn replace(&self, pane: &str, tx: oneshot::Sender<()>) -> Option<oneshot::Sender<()>> {
        self.inner
            .lock()
            .expect("holds")
            .insert(pane.to_string(), Some(tx))
            .flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn releasing_a_pane_nobody_holds_says_so_rather_than_pretending() {
        let holds = PaneHolds::default();
        assert!(!holds.release("w1:p1"), "there was no hold to let go of");
        assert!(!holds.is_held("w1:p1"));
    }

    /// A hold reads as held from the moment it is parked until its controller is actually gone —
    /// not from the moment somebody asks it to go. Claiming in the gap between those two is what
    /// herdr refuses (#21), and `wait_until_free` exists to sit in it.
    #[tokio::test]
    async fn a_pane_stays_held_until_the_controller_is_gone_not_until_release_is_asked() {
        let holds = PaneHolds::default();
        let (tx, mut rx) = oneshot::channel();
        holds.replace("w1:p1", tx);
        assert!(holds.is_held("w1:p1"), "a parked pane is held");

        assert!(holds.release("w1:p1"), "releasing it reports that there was one");
        assert!(rx.try_recv().is_ok(), "and the owning task was actually asked");
        assert!(
            holds.is_held("w1:p1"),
            "still held: the controller has not gone, and claiming now would be refused (#21)",
        );

        // What the owning task does once its child has exited.
        holds.inner.lock().expect("holds").remove("w1:p1");
        assert!(!holds.is_held("w1:p1"));
        holds.wait_until_free("w1:p1").await;
    }

    /// The wait is bounded. A controller that never goes must not stall the op for ever: the claim
    /// that follows will fail on its own and say so, which is better than hanging.
    #[tokio::test]
    async fn waiting_for_a_hold_that_never_goes_gives_up_rather_than_hanging() {
        let holds = PaneHolds::default();
        let (tx, _rx) = oneshot::channel();
        holds.replace("w1:p1", tx);
        let started = std::time::Instant::now();
        holds.wait_until_free("w1:p1").await;
        assert!(started.elapsed() >= FREE_TIMEOUT, "it gave up early");
        assert!(started.elapsed() < FREE_TIMEOUT * 2, "it waited far too long");
    }
}
