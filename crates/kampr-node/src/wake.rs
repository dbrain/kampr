use crate::sessions::SessionNode;
use kampr_core::Provider;
use kampr_core::push::{PaneChanges, PulseRx};
use tokio::time::Instant;

/// Only a native pane's: herdr's `screen` and `status` are its whole snapshot's revision, which
/// would wake a pane's readers on every other pane's news.
pub fn native_changes(session: &SessionNode, local: &str) -> PaneChanges {
    session
        .native_of(local)
        .map(|native| native.pane_changes(local))
        .unwrap_or_default()
}

/// Until the pulse moves. Never resolves when it cannot.
pub async fn pulsed(rx: &mut Option<PulseRx>) {
    match rx {
        Some(rx) => rx.changed().await,
        None => std::future::pending().await,
    }
}

pub async fn either(a: &mut Option<PulseRx>, b: &mut Option<PulseRx>) {
    tokio::select! {
        _ = pulsed(a) => {}
        _ = pulsed(b) => {}
    }
}

/// True when either pulse moved, false at `until`.
pub async fn either_before(a: &mut Option<PulseRx>, b: &mut Option<PulseRx>, until: Instant) -> bool {
    tokio::select! {
        biased;
        _ = either(a, b) => true,
        _ = tokio::time::sleep_until(until) => false,
    }
}
