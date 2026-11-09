use crate::handshake::{HandshakeError, HubIdentity, Presence, greet};
use crate::ws::{self, ClientSocket, HttpRequestBuilder};
use kampr_auth::NodeIdentity;
use kampr_core::Backoff;
use std::future::Future;
use std::time::Duration;
use tokio::net::TcpStream;
use tracing::{info, warn};

pub type WsOut = ws::WsOut<yawc::MaybeTlsStream<TcpStream>>;
pub type WsIn = ws::WsIn<yawc::MaybeTlsStream<TcpStream>>;

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
    let url = mesh_url(&hub.url);
    let (mut link, _) = ws::split(open(&url, timeout).await?);
    let hub_identity = greet(&mut link, identity, me, hub.join.as_deref(), hub.key.as_deref()).await?;
    let (out, incoming) = link.split();
    Ok((hub_identity, out, incoming))
}

async fn open(url: &str, timeout: Duration) -> Result<ClientSocket, DialError> {
    ws::connect(url, HttpRequestBuilder::new(), crate::MAX_MESSAGE_BYTES, timeout)
        .await
        .map_err(|e| DialError::Connect(url.to_string(), e.to_string()))
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

    #[tokio::test]
    async fn the_link_to_a_hub_offers_compression() {
        use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.expect("bind");
        let addr = listener.local_addr().expect("addr");
        let hub = tokio::spawn(async move {
            let (tcp, _) = listener.accept().await.expect("accept");
            let mut offered = None;
            #[allow(clippy::result_large_err)]
            let read = |request: &Request, response: Response| {
                offered = request
                    .headers()
                    .get("sec-websocket-extensions")
                    .and_then(|v| v.to_str().ok())
                    .map(str::to_string);
                Ok(response)
            };
            let socket = tokio_tungstenite::accept_hdr_async(tcp, read)
                .await
                .expect("upgrade");
            (offered, socket)
        });
        let _socket = open(&format!("ws://{addr}/mesh"), Duration::from_secs(5))
            .await
            .expect("a hub that declines still links");
        let (offered, _) = hub.await.expect("the hub");
        let offered = offered.unwrap_or_default();
        assert!(offered.starts_with("permessage-deflate"), "offered {offered:?}");
    }

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
