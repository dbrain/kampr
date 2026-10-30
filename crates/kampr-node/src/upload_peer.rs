//! The peer's half of an upload a hub relays: the same [`Staged`] file the HTTP route writes,
//! fed from `up.chunk` frames and paced by `up.more` credit.

use std::sync::Arc;

use kampr_auth::Device;
use serde_json::json;
use tokio::sync::mpsc;

use crate::outbox::{Frame, Outbox};
use crate::state::Node;
use crate::upload::{Staged, audit, idle};

pub enum Up {
    Chunk(Vec<u8>),
    Done,
}

pub struct Receiving {
    pub node: Arc<Node>,
    pub device: Device,
    pub peer: String,
    pub outbox: Arc<Outbox>,
    pub rid: u64,
    pub dir: String,
    pub name: String,
    pub chunks: mpsc::Receiver<Up>,
}

/// Ends with the file in place and `up.ok`, or with `up.error` and nothing on disk. The channel
/// closing without an `Up::Done` — the hub's `up.stop`, or the link going — or nothing arriving
/// for the idle limit drops the [`Staged`], which removes the temp file.
pub async fn receive(ctx: Receiving) {
    let Receiving {
        node,
        device,
        peer,
        outbox,
        rid,
        dir,
        name,
        mut chunks,
    } = ctx;
    let say = |message: serde_json::Value| outbox.push(Frame::plain(message.to_string()));
    let error = |code: &str| say(json!({ "t": "up.error", "rid": rid, "code": code }));
    let mut staged = match Staged::open(&node, &dir, &name).await {
        Ok(staged) => staged,
        Err(e) => {
            error(e.code());
            return;
        }
    };
    say(json!({ "t": "up.more", "rid": rid, "n": kampr_mesh::ATT_WINDOW }));
    loop {
        let up = match tokio::time::timeout(idle(&node), chunks.recv()).await {
            Ok(Some(up)) => up,
            Ok(None) => return,
            Err(_) => {
                error("idle");
                return;
            }
        };
        match up {
            Up::Chunk(data) => {
                if let Err(e) = staged.write(&data).await {
                    tracing::debug!(rid, error = %e, "a relayed upload could not be written");
                    error("failed");
                    return;
                }
                say(json!({ "t": "up.more", "rid": rid, "n": 1 }));
            }
            Up::Done => {
                match staged.commit().await {
                    Ok(written) => {
                        audit(&node, &device, &peer, &written);
                        say(json!({
                            "t": "up.ok", "rid": rid,
                            "path": written.path.display().to_string(), "bytes": written.bytes,
                        }));
                    }
                    Err(e) => {
                        tracing::debug!(rid, error = %e, "a relayed upload could not be put in place");
                        error("failed");
                    }
                }
                return;
            }
        }
    }
}
