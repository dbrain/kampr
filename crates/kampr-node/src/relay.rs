use crate::wire::Wire;
use kampr_core::wire::ErrorCode;
use kampr_mesh::{Peers, RemoteEvent, RemoteWatcher};
use serde_json::Value;
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
    pub conversation: bool,
}

pub async fn pump_peer_pane(ctx: PeerPaneCtx) {
    let PeerPaneCtx {
        peers,
        wire,
        global,
        conversation,
    } = ctx;
    let mut watcher = match peers.watch(&global, conversation) {
        Ok(watcher) => watcher,
        Err(e) => {
            wire.error(e.code(), &e.to_string(), Some(&global));
            return;
        }
    };
    for event in watcher.initial() {
        if !emit(&peers, &wire, &global, event) {
            return;
        }
    }
    loop {
        let Some(event) = watcher.recv().await else {
            // The link went away under a live watcher. Say so on the pane rather than going
            // quiet: a client that is told nothing shows a frozen grid forever.
            wire.error(
                ErrorCode::NodeOffline,
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
        if !emit(&peers, &wire, &global, event) {
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

fn emit(peers: &Peers, wire: &Wire, global: &str, event: RemoteEvent) -> bool {
    match event {
        RemoteEvent::Update(update) => wire.send_update(global, &update),
        RemoteEvent::Scrollback(doc) => wire.send_scrollback(global, &doc),
        RemoteEvent::Passthrough(value) => match peers.can_serve_attachments(global) {
            true => wire.send_json(&value),
            false => wire.send_json(&without_attachment_promises(value, global)),
        },
        // A peer's code is forwarded verbatim rather than narrowed to this build's vocabulary:
        // a newer peer may name one this hub has no variant for, and dropping it is the same
        // forward-compatibility failure as refusing an unknown `t`.
        RemoteEvent::Error { code, message } => wire.send_json(&serde_json::json!({
            "t": "error", "code": code, "message": message, "pane": global
        })),
    }
}

/// **The promise is this hop's to make, and it is only made when this hub can keep it.**
///
/// `GET /api/attachment/{pane}/{id}` pulls a relayed pane's bytes off the peer over the link the
/// peer dialled, so the button works — but only while there *is* a link, and only if the build at
/// the far end of it answers `att.fetch`. For a peer that is offline, one this hub has never met,
/// and one running a build from before that verb, the id resolves to nothing here and comes back
/// as the one 404 every refusal wears — reaching the operator as a dead button on a picture that
/// is intact one hop away.
///
/// A client renders a button for exactly as long as an `att` is present, so relaying one in those
/// three cases is offering a control that cannot work, which is the #233 shape. The marker text is
/// the peer's own and stays either way: the image was there, this hub just cannot fetch it.
fn without_attachment_promises(mut message: Value, global: &str) -> Value {
    let mut dropped = 0;
    let turns = message.get_mut("turns").and_then(Value::as_array_mut);
    for turn in turns.into_iter().flatten() {
        let blocks = turn.get_mut("blocks").and_then(Value::as_array_mut);
        for block in blocks.into_iter().flatten() {
            if block.as_object_mut().is_some_and(|b| b.remove("att").is_some()) {
                dropped += 1;
            }
        }
    }
    if dropped > 0 {
        debug!(pane = %global, dropped, "a relayed pane's attachments are not carried by this hub");
    }
    message
}
