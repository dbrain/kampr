use crate::restore::Restore;
use kampr_herdr::{Controller, HOLD_LIMIT};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::oneshot;
use tracing::{debug, warn};

/// A release is a child exiting, which `Controller::release` already bounds at three seconds and
/// then kills — so nothing here can usefully wait longer than that plus room to notice. A hold
/// that is putting a pane's own size back does a second claim-and-release inside that window,
/// which is why this is twice the grace rather than equal to it.
const FREE_TIMEOUT: Duration = Duration::from_secs(12);
const FREE_POLL: Duration = Duration::from_millis(25);

/// Names one hold, so that letting go can be *scoped*: a viewer that hands a pane on to a newer
/// one must not later release the hold that replaced its own.
pub type HoldToken = u64;

/// Why a hold ended, which is the whole of what decides whether anything is put back.
#[derive(Clone, Copy, PartialEq, Eq)]
enum End {
    /// Something else claimed the pane, or the operator asked through the panel. Whoever did it
    /// owns the geometry now, and took the restore with it if it was theirs to take.
    Superseded,
    /// The holder let go — a view closed, a client disconnected, a deadline expired.
    LetGo,
}

struct Entry {
    token: HoldToken,
    stop: Option<oneshot::Sender<End>>,
    /// Taken by whoever supersedes this hold, so the pane's own geometry is carried forward
    /// exactly once and never applied twice.
    restore: Option<Restore>,
}

type Held = HashMap<String, Entry>;

/// The controllers this node is holding on the operator's behalf, one per pane at most.
///
/// A hold exists because a resize on a pane with a desk client attached lasts exactly as long as
/// the claim does — release hands the desk's own geometry back inside a second (#19) — so holding
/// is the only way that pane stays the size it was asked for. It is never implicit: while a
/// controller is held the desk is ignored (#18) and an attached desk TUI renders wrong without
/// being told anything (#298).
///
/// Two kinds of holder, and the difference is where the deadline comes from.
///
/// The **panel's** hold (ADR 0012) is a toggle an operator ticks and an operator unticks, and a
/// client that dies with the panel open never sends the untick — so it is owned by a task that
/// releases it at [`HOLD_LIMIT`] whatever happens. That is the other half of #20: `Controller::
/// release` stops a controller that will not go, and this stops a *holder* that never asks.
///
/// A **matched** hold has a better deadline than a clock: it is owned by a websocket session, and
/// the session's own liveness is what ends it. Dropping the lease releases the hold on every path
/// out of a session — a close, a `break` in the dispatch loop, the keepalive giving up on a peer
/// that froze rather than closing (#284), or the whole task being cancelled — so a wall-clock
/// ceiling would only ever fire on a client that was still there and still looking. See
/// [`ADR 0013`](../../../docs/adr/0013-a-standing-intent-to-match-the-view.md).
#[derive(Default)]
pub struct PaneHolds {
    inner: Arc<Mutex<Held>>,
    next: AtomicU64,
}

impl PaneHolds {
    /// Parks a claimed controller against `pane`, and answers the token that names it.
    ///
    /// The caller is what makes sure the pane was free first — herdr allows one controller at a
    /// time and refuses the second (#21), so it is the *claim* that has to be ordered, not this.
    pub fn park(
        &self,
        pane: &str,
        controller: Controller,
        limit: Option<Duration>,
        restore: Option<Restore>,
        provider: Arc<kampr_core::HerdrProvider>,
    ) -> HoldToken {
        let (tx, rx) = oneshot::channel();
        let token = self.next.fetch_add(1, Ordering::Relaxed) + 1;
        let key = pane.to_string();
        let inner = Arc::clone(&self.inner);
        let displaced = self.inner.lock().expect("holds").insert(
            key.clone(),
            Entry {
                token,
                stop: Some(tx),
                restore,
            },
        );
        if let Some(old) = displaced {
            debug!(pane = %pane, "a second claim replaced an existing hold");
            if let Some(stop) = old.stop {
                let _ = stop.send(End::Superseded);
            }
        }
        tokio::spawn(async move {
            // Either the holder lets go, or — for a panel hold — the deadline does. `timeout`
            // collapses both into the same release, which is why there is no sweeper anywhere:
            // the hold owns its own end.
            let end = match limit {
                Some(limit) => match tokio::time::timeout(limit, rx).await {
                    Ok(Ok(end)) => end,
                    Ok(Err(_)) => End::Superseded,
                    Err(_) => {
                        warn!(pane = %key, "a held pane hit the hold limit and was released for the operator");
                        End::LetGo
                    }
                },
                None => rx.await.unwrap_or(End::Superseded),
            };
            // Taken while the hold still stands, because a held controller *is* the geometry
            // (#18) — after the release the pane may already be the desk's again (#19).
            let restore = match end {
                End::LetGo => take_restore(&inner, &key, token),
                End::Superseded => None,
            };
            let restore = match restore {
                Some(r) if r.still_ours(&key).await => Some(r),
                Some(_) => {
                    debug!(pane = %key, "something else moved this pane while it was held; leaving it alone");
                    None
                }
                None => None,
            };
            if let Err(e) = controller.release().await {
                warn!(pane = %key, error = %e, "releasing a held pane");
            }
            // Before the entry goes, so that `wait_until_free` holds the next claim off until the
            // pane has actually been put back rather than racing it.
            if let Some(restore) = restore {
                restore.apply(&key).await;
            }
            // Cleared here rather than in `let_go`, so `wait_until_free` means "the controller is
            // gone" rather than "somebody asked it to go". Only if it is still ours: a hold that
            // superseded this one owns the entry now.
            let mut held = inner.lock().expect("holds");
            let ours = held.get(&key).is_some_and(|e| e.token == token);
            if ours {
                held.remove(&key);
            }
            drop(held);
            // Only when the entry was still this hold's: a claim that superseded this one has
            // already commanded its own width, and clearing it here would hand the stream back to
            // an inference of rows the *new* hold has since resized away.
            if ours {
                provider.released(&key);
            }
        });
        token
    }

    /// Asks a hold to let go whoever put it there, and answers whether there was one. This is the
    /// panel's release and the claim path's make-way: it takes any restore with it, so nothing is
    /// put back behind an operator who asked for a size by name.
    pub fn release(&self, pane: &str) -> bool {
        let mut held = self.inner.lock().expect("holds");
        match held.get_mut(pane) {
            Some(entry) => {
                entry.restore = None;
                if let Some(tx) = entry.stop.take() {
                    let _ = tx.send(End::Superseded);
                }
                true
            }
            None => false,
        }
    }

    /// Lets go of *this* hold and nothing else, and answers whether it was still the one standing.
    ///
    /// A viewer that was displaced by a newer one calls this on its way out and it does nothing,
    /// which is the whole of "newest holder wins": the earlier viewer never fights back and never
    /// takes the later one's hold down with it.
    pub fn let_go(&self, pane: &str, token: HoldToken) -> bool {
        let mut held = self.inner.lock().expect("holds");
        match held.get_mut(pane) {
            Some(entry) if entry.token == token => {
                if let Some(tx) = entry.stop.take() {
                    let _ = tx.send(End::LetGo);
                }
                true
            }
            _ => false,
        }
    }

    /// Supersedes whatever is holding `pane` and takes the geometry it was going to put back.
    ///
    /// Carrying rather than re-reading is what stops a window drag from turning the size Kampr set
    /// into the size Kampr restores, and it is what makes a handover between two viewers restore
    /// the pane's *own* geometry rather than the first viewer's.
    pub fn carry_for_match(&self, pane: &str) -> Option<(u16, u16)> {
        let mut held = self.inner.lock().expect("holds");
        let entry = held.get_mut(pane)?;
        let carried = entry.restore.take().map(|r| r.found);
        if let Some(tx) = entry.stop.take() {
            let _ = tx.send(End::Superseded);
        }
        carried
    }

    pub fn is_held(&self, pane: &str) -> bool {
        self.inner.lock().expect("holds").contains_key(pane)
    }

    /// Waits for a released hold's controller to actually be gone, because claiming again before
    /// it is puts two controllers on one pane and herdr refuses the second outright (#21).
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
}

/// The ceiling a panel hold is parked under. A matched hold passes `None` — its ceiling is the
/// websocket session that owns it.
pub const PANEL_LIMIT: Option<Duration> = Some(HOLD_LIMIT);

fn take_restore(inner: &Mutex<Held>, pane: &str, token: HoldToken) -> Option<Restore> {
    let mut held = inner.lock().expect("holds");
    held.get_mut(pane)
        .filter(|e| e.token == token)
        .and_then(|e| e.restore.take())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn park_bare(holds: &PaneHolds, pane: &str) -> (HoldToken, oneshot::Receiver<End>) {
        let (tx, rx) = oneshot::channel();
        let token = holds.next.fetch_add(1, Ordering::Relaxed) + 1;
        holds.inner.lock().expect("holds").insert(
            pane.to_string(),
            Entry {
                token,
                stop: Some(tx),
                restore: None,
            },
        );
        (token, rx)
    }

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
        let (_token, mut rx) = park_bare(&holds, "w1:p1");
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
        park_bare(&holds, "w1:p1");
        let started = std::time::Instant::now();
        holds.wait_until_free("w1:p1").await;
        assert!(started.elapsed() >= FREE_TIMEOUT, "it gave up early");
        assert!(started.elapsed() < FREE_TIMEOUT * 2, "it waited far too long");
    }

    /// Newest holder wins, and the earlier one does not take the later one down with it. A viewer
    /// that was displaced still runs its own release on the way out — the whole point of scoping
    /// it to a token is that the release lands on nothing.
    #[tokio::test]
    async fn a_displaced_holder_letting_go_does_not_release_the_hold_that_replaced_it() {
        let holds = PaneHolds::default();
        let (first, mut first_rx) = park_bare(&holds, "w1:p1");
        let (second, mut second_rx) = park_bare(&holds, "w1:p1");

        assert!(
            !holds.let_go("w1:p1", first),
            "the first holder was displaced and has nothing to let go of",
        );
        assert!(
            second_rx.try_recv().is_err(),
            "and the hold that replaced it was not asked to go",
        );
        let _ = first_rx.try_recv();

        assert!(holds.let_go("w1:p1", second), "the standing hold does let go");
        assert!(matches!(second_rx.try_recv(), Ok(End::LetGo)));
    }
}
