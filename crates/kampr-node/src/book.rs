//! The fleet book: the commands this node remembers, and the two ops that curate them.
//!
//! **The book belongs to the node, not to the device and not to the herd.**
//!
//! Not the device, because a device row *is* the identity in this schema — there is no account
//! above it — so a book kept the way `pane_prefs` is kept would be empty on the second device the
//! operator picked up, and following them between their phone and their desktop is the whole of
//! what was asked for.
//!
//! Not the herd, because a peer is dialled *outbound* to a hub and has no inbound path
//! ([ADR 0007](../../../docs/adr/0007-peers-dial-outbound-to-a-hub.md)): every device the operator
//! pairs is paired against the one reachable node, and a device id minted there means nothing on a
//! peer. So the book lives on the node the client dialled, which is structurally the hub, and
//! [`is_book_op`] is checked *before* a manage op is routed — a `fleet.save` naming a peer must
//! not be relayed to it, or the operator's list would be stored on whichever machine their last
//! pane happened to live on.
//!
//! What an entry is: **the argv and the working directory, and nothing about which hosts it
//! reached.** A run is fanned out to every host reachable when it starts, that set is different
//! next week, and an entry pinning last week's would offer to run somewhere that no longer exists.
//! The host count the operator is about to reach is resolved fresh and shown before the run goes.

use crate::manage::{ManageError, ManageOp};
use kampr_auth::Store;
use serde_json::{Value, json};

/// Curating the book, as ordinary manage ops so they get the role gate, the audit line and the
/// `managed` ack that every other write on this socket gets.
///
/// There is no `fleet.book` *read* op: the node pushes the book unasked, as `prefs` is pushed, so
/// no client has to ask for a memory it is about to render.
pub const OPS: [&str; 2] = ["fleet.save", "fleet.drop"];

pub fn is_book_op(op: &str) -> bool {
    OPS.contains(&op.trim())
}

pub async fn frame(store: &Store) -> Value {
    let book = store.fleet_book().await.unwrap_or_default();
    json!({ "t": "fleet.book", "recent": book.recent, "saved": book.saved })
}

/// A run the operator issued, written down — unless it looks like it is carrying a credential.
///
/// **Recorded when it is issued, not when it finishes.** Outcome is knowable only per host and
/// only on the host that ran it, so a "did it work" gate on the node the client dialled would be
/// deciding for five machines having seen one — [#233](../../../docs/03-probe-log.md) in
/// miniature. And it would be wrong even where it could see: answering `n` to `sudo pacman -Syu`
/// exits non-zero everywhere and is a perfectly good command. What bounds the typos instead is
/// that the list holds five, deduplicates, and every entry can be deleted.
///
/// Returns whether the book changed, so a caller knows whether to publish it.
pub async fn record_run(store: &Store, op: &ManageOp, now: i64) -> bool {
    let Some(job) = crate::manage::fleet_job(op) else {
        return false;
    };
    // **One element, and that element is the whole line.** An entry is rendered by joining `args`
    // with spaces, on this node's own clients and on every older one still installed on a phone, so
    // a shell line stored as its single argument reads back byte for byte as the operator typed it
    // — quotes, pipes and all. Splitting it into words to look argv-shaped would render
    // `echo "hello world"` as `echo hello world` and lose the thing that made it one argument.
    let args = job.words();
    if kampr_fleet::secretish(&args).is_some() {
        return false;
    }
    store
        .record_fleet_run(&args, op.cwd.as_deref(), now)
        .await
        .unwrap_or(false)
}

/// `fleet.save` and `fleet.drop`. Returns the ack body; the caller publishes the book after it.
pub async fn apply(store: &Store, op: &ManageOp, now: i64) -> Result<Value, ManageError> {
    match op.op.as_str() {
        "fleet.save" => {
            let label = op.label.as_deref();
            // By entry id when the operator pressed one they can already see, because re-deriving
            // the key from an argv the client re-typed is a second chance to disagree about what
            // "the same command" is — and a disagreement here means the command in both lists.
            if let Some(entry) = op.entry.as_deref() {
                return match store.keep_fleet_command(entry, label, now).await {
                    Ok(true) => Ok(json!({ "ok": true, "entry": entry })),
                    Ok(false) => Err(ManageError::BadRequest(format!(
                        "no saved room, or nothing in the book with id {entry}"
                    ))),
                    Err(e) => Err(ManageError::BadRequest(e.to_string())),
                };
            }
            let args = op.args.clone().unwrap_or_default();
            if args.is_empty() {
                return Err(ManageError::BadRequest(
                    "fleet.save needs `args`, the command to keep, or `entry`, one already in the book"
                        .into(),
                ));
            }
            // A secret-shaped command is refused the *automatic* half of the book and allowed the
            // deliberate one. Nothing pressed anything to ask for the history; a save is the
            // operator saying they mean it, and the client warns them with the same rule before
            // they press. A refusal they cannot get past on their own machine is a rule that gets
            // ripped out rather than one that holds.
            match store
                .save_fleet_command(&args, op.cwd.as_deref(), label, now)
                .await
            {
                Ok(Some(entry)) => Ok(json!({ "ok": true, "entry": entry.id })),
                Ok(None) => Err(ManageError::BadRequest(format!(
                    "the saved list holds {}, and this command did not fit",
                    kampr_auth::book::MAX_FLEET_SAVED
                ))),
                Err(e) => Err(ManageError::BadRequest(e.to_string())),
            }
        }
        "fleet.drop" => {
            let entry = op.entry.as_deref().ok_or_else(|| {
                ManageError::BadRequest("fleet.drop needs `entry`, the book entry to remove".into())
            })?;
            match store.drop_fleet_command(entry).await {
                Ok(true) => Ok(json!({ "ok": true })),
                Ok(false) => Err(ManageError::BadRequest(format!(
                    "nothing in the book with id {entry}"
                ))),
                Err(e) => Err(ManageError::BadRequest(e.to_string())),
            }
        }
        other => Err(ManageError::BadRequest(format!("{other} is not a book op"))),
    }
}
