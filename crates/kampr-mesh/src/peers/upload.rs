//! A file pushed from the hub to a peer, the `att.*` lane run backwards.
//!
//! The hub sends `up.open`; the peer answers `up.more` with the window it will take (or
//! `up.error`), then one `up.more` per chunk it has written. The hub sends `up.chunk` only against
//! credit, so what the peer buffers is the window whatever the file's size, and `up.done` asks it
//! to put the file in place, which it confirms with `up.ok`.

use super::{ATT_CHUNK_BYTES, ATT_WINDOW, FetchError, MANAGE_TIMEOUT, PeerLink, Peers, RelayError};
use base64::Engine;
use serde_json::{Value, json};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use tokio::sync::mpsc;
use tracing::debug;

#[derive(Debug)]
pub(super) enum UpEvent {
    More(u32),
    Ok(Value),
    Refused(String),
}

/// One upload in flight towards a peer. Dropping it before [`Self::finish`] is the cancellation:
/// the peer is told to stop and removes what it had written.
pub struct Upload {
    link: Arc<PeerLink>,
    rid: u64,
    rx: mpsc::Receiver<UpEvent>,
    credit: u32,
    seq: u64,
    done: bool,
}

impl Peers {
    /// Opens an upload into `dir` on the node behind `node`, which may be a node id or a pane id.
    /// The peer decides everything — the gate against the hub's own role, the name, the directory
    /// — and a refusal comes back as its code.
    pub async fn upload(&self, node: &str, dir: &str, name: &str) -> Result<Upload, FetchError> {
        let link = self.resolve(node)?;
        if !link.state.lock().unwrap().uploads {
            return Err(FetchError::Refused("that node takes no uploads".into()));
        }
        let rid = link.next_request.fetch_add(1, Ordering::Relaxed);
        let (tx, rx) = mpsc::channel(ATT_WINDOW as usize + 2);
        link.uploads.lock().unwrap().insert(rid, tx);
        let mut upload = Upload {
            link: link.clone(),
            rid,
            rx,
            credit: 0,
            seq: 0,
            done: false,
        };
        link.request(json!({ "t": "up.open", "rid": rid, "dir": dir, "name": name }))?;
        match upload.event().await? {
            UpEvent::More(n) => {
                upload.credit = n;
                Ok(upload)
            }
            UpEvent::Refused(code) => {
                upload.done = true;
                Err(FetchError::Refused(code))
            }
            UpEvent::Ok(_) => Err(FetchError::Refused("out of order".into())),
        }
    }
}

impl Upload {
    async fn event(&mut self) -> Result<UpEvent, FetchError> {
        match tokio::time::timeout(MANAGE_TIMEOUT, self.rx.recv()).await {
            Ok(Some(event)) => Ok(event),
            Ok(None) => Err(RelayError::Offline("that node left the herd".into()).into()),
            Err(_) => Err(RelayError::NoAnswer.into()),
        }
    }

    /// Sends `data`, in pieces no larger than an attachment chunk, each one only once the peer has
    /// room for it.
    pub async fn send(&mut self, data: &[u8]) -> Result<(), FetchError> {
        for piece in data.chunks(ATT_CHUNK_BYTES) {
            while self.credit == 0 {
                match self.event().await? {
                    UpEvent::More(n) => self.credit += n,
                    UpEvent::Refused(code) => {
                        self.done = true;
                        return Err(FetchError::Refused(code));
                    }
                    UpEvent::Ok(_) => return Err(FetchError::Refused("out of order".into())),
                }
            }
            self.credit -= 1;
            self.link.request(json!({
                "t": "up.chunk", "rid": self.rid, "seq": self.seq,
                "b64": base64::engine::general_purpose::STANDARD.encode(piece),
            }))?;
            self.seq += 1;
        }
        Ok(())
    }

    /// Asks the peer to put the file in place, and returns what it said it wrote.
    pub async fn finish(mut self) -> Result<Value, FetchError> {
        self.link.request(json!({ "t": "up.done", "rid": self.rid }))?;
        loop {
            match self.event().await? {
                UpEvent::More(_) => {}
                UpEvent::Ok(written) => {
                    self.done = true;
                    return Ok(written);
                }
                UpEvent::Refused(code) => {
                    self.done = true;
                    return Err(FetchError::Refused(code));
                }
            }
        }
    }
}

impl Drop for Upload {
    fn drop(&mut self) {
        self.link.uploads.lock().unwrap().remove(&self.rid);
        if !self.done {
            let _ = self.link.request(json!({ "t": "up.stop", "rid": self.rid }));
        }
    }
}

impl PeerLink {
    pub(super) fn upload_event(&self, message: &Value) {
        let Some(rid) = message["rid"].as_u64() else {
            return;
        };
        let event = match message["t"].as_str().unwrap_or_default() {
            "up.more" => UpEvent::More(message["n"].as_u64().unwrap_or(1).min(u64::from(ATT_WINDOW)) as u32),
            "up.ok" => UpEvent::Ok(message.clone()),
            _ => UpEvent::Refused(message["code"].as_str().unwrap_or("refused").to_string()),
        };
        let sink = self.uploads.lock().unwrap().get(&rid).cloned();
        let Some(sink) = sink else { return };
        if sink.try_send(event).is_err() {
            debug!(node = %self.node_id, rid, "a peer overran its upload window");
            self.uploads.lock().unwrap().remove(&rid);
        }
    }
}
