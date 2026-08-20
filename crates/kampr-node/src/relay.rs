use crate::wire::Wire;
use kampr_mesh::{Peers, RemoteEvent, RemoteWatcher};
use std::sync::Arc;
use tracing::debug;

/// One peer pane, relayed to one client.
///
/// This is [`crate::session::pump_pane`]'s twin for a pane the hub does not own, and it applies
/// the *same* backpressure rule at this hop: before enqueuing a grid frame it asks whether the
/// client is keeping up, and if it is not, that pane's queued frames are dropped and it rejoins at
/// one `grid.reset` — taken from the hub's shadow of the pane, so the reset costs no round trip to
/// the peer at all.
///
/// Nothing else is ever purged. A `scrollback` message is append-only history that no later reset
/// repairs, and a `pending` prompt is a fact about the pane rather than a repaintable frame.
pub struct PeerPaneCtx {
    pub peers: Arc<Peers>,
    pub wire: Arc<Wire>,
    pub global: String,
}

pub async fn pump_peer_pane(ctx: PeerPaneCtx) {
    let PeerPaneCtx { peers, wire, global } = ctx;
    let mut watcher = match peers.watch(&global) {
        Ok(watcher) => watcher,
        Err(e) => {
            wire.error(e.code(), &e.to_string(), Some(&global));
            return;
        }
    };
    for event in watcher.initial() {
        if !emit(&wire, &global, event) {
            return;
        }
    }
    loop {
        let Some(event) = watcher.recv().await else {
            // The link went away under a live watcher. Say so on the pane rather than going
            // quiet: a client that is told nothing shows a frozen grid forever.
            wire.error(
                "node_offline",
                "the node serving this pane left the herd",
                Some(&global),
            );
            return;
        };
        if matches!(event, RemoteEvent::Update(_)) && wire.outbox().congested() {
            let dropped = wire.outbox().purge_pane(&global);
            debug!(pane = %global, dropped, "client is behind on a relayed pane; resetting instead of buffering");
            match resync(&wire, &global, &watcher) {
                true => continue,
                false => return,
            }
        }
        if !emit(&wire, &global, event) {
            return;
        }
    }
}

fn resync(wire: &Wire, global: &str, watcher: &RemoteWatcher) -> bool {
    match watcher.resync() {
        Some(full) => wire.send_update(global, &full),
        None => true,
    }
}

fn emit(wire: &Wire, global: &str, event: RemoteEvent) -> bool {
    match event {
        RemoteEvent::Update(update) => wire.send_update(global, &update),
        RemoteEvent::Scrollback(doc) => wire.send_scrollback(global, &doc),
        RemoteEvent::Passthrough(value) => wire.send_json(&value),
        RemoteEvent::Error { code, message } => wire.error(&code, &message, Some(global)),
    }
}
