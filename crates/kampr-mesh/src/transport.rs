use serde::Serialize;
use serde::de::DeserializeOwned;
use std::future::Future;
use std::sync::atomic::{AtomicBool, Ordering};
use tokio::sync::mpsc;

/// Whether anything at all has arrived on a link since the last time somebody asked.
///
/// **Any frame counts, including the ones the far end's websocket library answers by itself.**
/// That is the whole of what it is for: a peer whose kernel is alive and whose application has
/// stopped reading answers a ping without running a line of its own code (#284), and a link that
/// has silently stopped delivering answers nothing at all. Held beside the reading half because
/// that is the only place a frame is seen, and read by whatever is doing the pinging.
#[derive(Debug, Default)]
pub struct Heard(AtomicBool);

impl Heard {
    pub fn note(&self) {
        self.0.store(true, Ordering::Relaxed);
    }

    pub fn take(&self) -> bool {
        self.0.swap(false, Ordering::Relaxed)
    }
}

/// The half a mesh link writes to.
///
/// Two traits rather than one duplex object because the two halves are driven by different tasks
/// once the handshake is over — the same shape the node's client sessions already have, which is
/// what lets a peer serve a hub with the very code that serves a browser.
pub trait Outgoing: Send + 'static {
    /// `false` once the far end is gone, which is the signal for a producer to stop.
    fn send(&mut self, text: String) -> impl Future<Output = bool> + Send;
    fn close(&mut self) -> impl Future<Output = ()> + Send;
    /// A frame the far end's websocket library answers on its own, without the application it
    /// belongs to running at all — which is the only question worth asking of a peer that has
    /// frozen rather than closed (#284). Defaulted, because a transport with a liveness check of
    /// its own has nothing to do here: the mesh link keeps its own, and an in-process pair cannot
    /// be lied to.
    fn ping(&mut self) -> impl Future<Output = bool> + Send {
        async { true }
    }
}

/// The half a mesh link reads from. `None` ends the link; non-text frames are skipped rather
/// than reported, because nothing above this layer has an opinion about them.
pub trait Incoming: Send + 'static {
    fn recv(&mut self) -> impl Future<Output = Option<String>> + Send;
}

#[derive(Debug, thiserror::Error)]
pub enum LinkError {
    #[error("the mesh link closed during the handshake")]
    Closed,
    #[error("unreadable mesh message: {0}")]
    Malformed(String),
}

/// Both halves, before they are split. The handshake is strictly request/response, so it runs
/// here; everything after it is concurrent and runs on the halves.
pub struct Link<O: Outgoing, I: Incoming> {
    pub out: O,
    pub incoming: I,
}

impl<O: Outgoing, I: Incoming> Link<O, I> {
    pub fn new(out: O, incoming: I) -> Self {
        Self { out, incoming }
    }

    pub async fn send<T: Serialize>(&mut self, message: &T) -> Result<(), LinkError> {
        let json = serde_json::to_string(message).map_err(|e| LinkError::Malformed(e.to_string()))?;
        match self.out.send(json).await {
            true => Ok(()),
            false => Err(LinkError::Closed),
        }
    }

    pub async fn recv<T: DeserializeOwned>(&mut self) -> Result<T, LinkError> {
        let text = self.incoming.recv().await.ok_or(LinkError::Closed)?;
        serde_json::from_str(&text).map_err(|e| LinkError::Malformed(e.to_string()))
    }

    pub fn split(self) -> (O, I) {
        (self.out, self.incoming)
    }
}

/// One end of an in-process link. Enough to drive a whole handshake and a whole relay in a test
/// without a socket, which is what keeps the mesh tests fast and deterministic.
pub struct Sender(Option<mpsc::Sender<String>>);

pub struct Receiver(mpsc::Receiver<String>);

impl Outgoing for Sender {
    async fn send(&mut self, text: String) -> bool {
        match &self.0 {
            Some(tx) => tx.send(text).await.is_ok(),
            None => false,
        }
    }

    async fn close(&mut self) {
        self.0 = None;
    }
}

impl Incoming for Receiver {
    async fn recv(&mut self) -> Option<String> {
        self.0.recv().await
    }
}

pub fn pair() -> (Link<Sender, Receiver>, Link<Sender, Receiver>) {
    let (atx, arx) = mpsc::channel(256);
    let (btx, brx) = mpsc::channel(256);
    (
        Link::new(Sender(Some(atx)), Receiver(brx)),
        Link::new(Sender(Some(btx)), Receiver(arx)),
    )
}
