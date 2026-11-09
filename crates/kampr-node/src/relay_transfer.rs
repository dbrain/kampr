//! `att.*` and `up.*` from a hub, for a node further down the chain: this node is a hub to that
//! one, so it pulls or pushes the bytes over its own link and passes them on a chunk at a time,
//! granting credit upstream only as the far end grants it here.

use crate::outbox::{Frame, Outbox};
use crate::served::STREAM_CEILING;
use crate::upload_peer::Up;
use base64::Engine;
use kampr_mesh::{FetchError, Peers, Span};
use serde_json::{Value, json};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::debug;

pub struct Fetch {
    pub peers: Arc<Peers>,
    pub outbox: Arc<Outbox>,
    pub rid: u64,
    pub pane: String,
    pub id: String,
    pub span: Option<Span>,
    pub window: u32,
    pub granted: mpsc::Receiver<u32>,
}

pub async fn fetch(ctx: Fetch) {
    let Fetch {
        peers,
        outbox,
        rid,
        pane,
        id,
        span,
        window,
        mut granted,
    } = ctx;
    let say = |frame: Value| outbox.push(Frame::plain(frame.to_string()));
    let mut transfer = match peers.fetch_attachment(&pane, &id, STREAM_CEILING, span).await {
        Ok(transfer) => transfer,
        Err(e) => {
            debug!(rid, %pane, error = %e, "a relayed attachment was refused further down");
            say(match e {
                FetchError::TooLarge(_) => json!({ "t": "att.error", "rid": rid, "code": "too_large" }),
                FetchError::Unsatisfiable(total) => {
                    json!({ "t": "att.error", "rid": rid, "code": "unsatisfiable", "total": total })
                }
                _ => json!({ "t": "att.error", "rid": rid, "code": "not_found" }),
            });
            return;
        }
    };
    let header = transfer.header().clone();
    let mut open = json!({ "t": "att.open", "rid": rid, "bytes": header.bytes, "kind": header.kind });
    for (field, value) in [("mime", header.mime), ("name", header.name)] {
        if let Some(value) = value {
            open[field] = json!(value);
        }
    }
    if let Some(total) = header.total {
        open["total"] = json!(total);
    }
    if !say(open) {
        return;
    }
    let mut credit = window;
    let mut seq = 0u64;
    loop {
        while credit == 0 {
            let Some(more) = granted.recv().await else { return };
            credit += more;
        }
        let frame = match transfer.next_chunk().await {
            None => json!({ "t": "att.end", "rid": rid }),
            Some(Err(e)) => {
                debug!(rid, error = %e, "a relayed attachment stopped part way further down");
                json!({ "t": "att.error", "rid": rid, "code": "not_found" })
            }
            Some(Ok(chunk)) => {
                credit -= 1;
                seq += 1;
                json!({
                    "t": "att.chunk", "rid": rid, "seq": seq - 1,
                    "b64": base64::engine::general_purpose::STANDARD.encode(&chunk),
                })
            }
        };
        let last = frame["t"] != "att.chunk";
        if !outbox.push_bulk(Frame::plain(frame.to_string())) || last {
            return;
        }
    }
}

pub struct Push {
    pub peers: Arc<Peers>,
    pub outbox: Arc<Outbox>,
    pub rid: u64,
    pub node: String,
    pub dir: String,
    pub name: String,
    pub chunks: mpsc::Receiver<Up>,
}

pub async fn push(ctx: Push) {
    let Push {
        peers,
        outbox,
        rid,
        node,
        dir,
        name,
        mut chunks,
    } = ctx;
    let say = |frame: Value| outbox.push(Frame::plain(frame.to_string()));
    let refused = |e: FetchError| {
        let code = match e {
            FetchError::Refused(code) => code,
            e => {
                debug!(rid, %node, error = %e, "a relayed upload failed further down");
                crate::upload::UploadError::Missing.code().to_string()
            }
        };
        json!({ "t": "up.error", "rid": rid, "code": code })
    };
    let mut upload = match peers.upload(&node, &dir, &name).await {
        Ok(upload) => upload,
        Err(e) => {
            say(refused(e));
            return;
        }
    };
    say(json!({ "t": "up.more", "rid": rid, "n": kampr_mesh::ATT_WINDOW }));
    while let Some(up) = chunks.recv().await {
        match up {
            Up::Chunk(data) => {
                if let Err(e) = upload.send(&data).await {
                    say(refused(e));
                    return;
                }
                say(json!({ "t": "up.more", "rid": rid, "n": 1 }));
            }
            Up::Done => {
                let mut reply = match upload.finish().await {
                    Ok(written) => written,
                    Err(e) => refused(e),
                };
                reply["rid"] = json!(rid);
                say(reply);
                return;
            }
        }
    }
}
