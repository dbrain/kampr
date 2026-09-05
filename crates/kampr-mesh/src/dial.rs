use crate::handshake::{HandshakeError, HubIdentity, Presence, greet};
use crate::transport::{Heard, Incoming, Link, Outgoing};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use kampr_auth::NodeIdentity;
use kampr_core::Backoff;
use std::future::Future;
use std::sync::Arc;
use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::Message;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};
use tracing::{info, warn};

type Ws = WebSocketStream<MaybeTlsStream<TcpStream>>;

pub struct WsOut(SplitSink<Ws, Message>);

pub struct WsIn(SplitStream<Ws>, Arc<Heard>);

impl WsIn {
    /// **The half of #284 the dialling side was missing.** A hub is served by the same code that
    /// serves a browser, and that code asks a socket whether it is still there — but only when it
    /// is given somewhere to record the answer. Without this the ping arm never ran on an outbound
    /// link at all: a hub that stopped delivering was indistinguishable from a hub with nothing to
    /// say, and the node went on serving a socket the hub had already dropped it from, until a
    /// write happened to fail. Measured on a real herd at **three hours**, ended by the operator
    /// typing into a pane.
    pub fn heard(&self) -> Arc<Heard> {
        self.1.clone()
    }
}

impl Outgoing for WsOut {
    async fn send(&mut self, text: String) -> bool {
        self.0.send(Message::text(text)).await.is_ok()
    }

    async fn close(&mut self) {
        let _ = self.0.close().await;
    }

    async fn ping(&mut self) -> bool {
        self.0.send(Message::Ping(Default::default())).await.is_ok()
    }
}

impl Incoming for WsIn {
    async fn recv(&mut self) -> Option<String> {
        loop {
            match self.0.next().await? {
                Ok(Message::Text(text)) => {
                    self.1.note();
                    return Some(text.to_string());
                }
                Ok(Message::Close(_)) | Err(_) => return None,
                Ok(_) => self.1.note(),
            }
        }
    }
}

/// A hub this node dials out to.
#[derive(Debug, Clone)]
pub struct Hub {
    pub url: String,
    pub name: String,
    /// The hub's public key, pinned at join. `None` only on the very first connection, when the
    /// join code is the credential and the operator is looking at the fingerprint.
    pub key: Option<String>,
    pub join: Option<String>,
}

#[derive(Debug, Clone)]
pub struct DialPolicy {
    pub backoff: Backoff,
    pub connect_timeout: Duration,
    /// How long a link must stand before it counts as a success worth resetting the backoff for.
    pub settled_after: Duration,
}

impl Default for DialPolicy {
    fn default() -> Self {
        Self {
            backoff: Backoff {
                initial: Duration::from_secs(1),
                max: Duration::from_secs(30),
            },
            connect_timeout: Duration::from_secs(15),
            settled_after: Duration::from_secs(10),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum DialError {
    #[error("connecting to {0}: {1}")]
    Connect(String, String),
    #[error(transparent)]
    Handshake(#[from] HandshakeError),
}

/// `wss://host/mesh` from whatever the operator typed. An `https://` origin is the same host with
/// a different scheme name, and a bare host is the common case; both are accepted so a join
/// command can be copied straight out of `kampr status`.
pub fn mesh_url(input: &str) -> String {
    let trimmed = input.trim().trim_end_matches('/');
    let (scheme, rest) = match trimmed.split_once("://") {
        Some(("https", rest)) | Some(("wss", rest)) => ("wss", rest),
        Some(("http", rest)) | Some(("ws", rest)) => ("ws", rest),
        Some((_, rest)) => ("wss", rest),
        None => ("wss", trimmed),
    };
    match rest.ends_with("/mesh") {
        true => format!("{scheme}://{rest}"),
        false => format!("{scheme}://{rest}/mesh"),
    }
}

/// Dials a hub and completes the mutual handshake. Everything after this point is the ordinary
/// client protocol, with the hub as the client.
pub async fn dial(
    hub: &Hub,
    identity: &NodeIdentity,
    me: &Presence,
    timeout: Duration,
) -> Result<(HubIdentity, WsOut, WsIn), DialError> {
    // tokio-tungstenite builds its rustls client config from the process default provider, and
    // this tree carries exactly one — ring. Installing twice is not an error worth reporting.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let url = mesh_url(&hub.url);
    let (socket, _) = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(&url))
        .await
        .map_err(|_| DialError::Connect(url.clone(), "timed out".into()))?
        .map_err(|e| DialError::Connect(url.clone(), e.to_string()))?;
    let (sink, stream) = socket.split();
    let mut link = Link::new(WsOut(sink), WsIn(stream, Arc::default()));
    let hub_identity = greet(&mut link, identity, me, hub.join.as_deref(), hub.key.as_deref()).await?;
    let (out, incoming) = link.split();
    Ok((hub_identity, out, incoming))
}

/// Keeps one outbound link up for as long as the process lives.
///
/// A hub that is down, or that refuses this node, costs one task and a growing sleep — never the
/// rest of the herd, and never this node's own clients. Reconnecting is unattended by design:
/// the credential is a key on disk, so there is nothing for anybody to re-type.
pub async fn supervise<F, Fut>(hub: Hub, identity: NodeIdentity, me: Presence, policy: DialPolicy, serve: F)
where
    F: Fn(HubIdentity, WsOut, WsIn) -> Fut,
    Fut: Future<Output = ()>,
{
    let mut backoff = policy.backoff.start();
    loop {
        match dial(&hub, &identity, &me, policy.connect_timeout).await {
            Ok((hub_identity, out, incoming)) => {
                info!(
                    hub = %hub.name,
                    url = %hub.url,
                    fingerprint = %hub_identity.fingerprint(),
                    "joined a hub"
                );
                let at = std::time::Instant::now();
                serve(hub_identity, out, incoming).await;
                // A link that closed as soon as it opened is a refusal wearing a connection's
                // clothes — a revoked hub device, say — so the backoff keeps growing rather than
                // resetting into a tight reconnect loop.
                if at.elapsed() >= policy.settled_after {
                    backoff.reset();
                }
                warn!(hub = %hub.name, "the hub link ended; reconnecting");
            }
            Err(e) => warn!(hub = %hub.name, url = %hub.url, error = %e, "could not reach the hub"),
        }
        backoff.sleep().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_a_hub_address_becomes_one_mesh_url() {
        for input in [
            "https://kampr.example.com",
            "https://kampr.example.com/",
            "wss://kampr.example.com/mesh",
            "kampr.example.com",
        ] {
            assert_eq!(mesh_url(input), "wss://kampr.example.com/mesh", "{input}");
        }
        assert_eq!(
            mesh_url("http://127.0.0.1:8790"),
            "ws://127.0.0.1:8790/mesh",
            "a Tier 0 hub on the LAN is still a hub"
        );
    }
}
