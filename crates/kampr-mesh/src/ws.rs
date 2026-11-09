use crate::transport::{Heard, Incoming, Link, Outgoing};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::{SinkExt, StreamExt};
use std::sync::{Arc, OnceLock};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite};
use yawc::{Frame, OpCode, WebSocket};

pub use yawc::{
    HttpRequestBuilder, HttpWebSocket as ServerSocket, IncomingUpgrade, TcpWebSocket as ClientSocket,
};

/// The settings for every socket Kampr opens or accepts, browser, phone or peer.
///
/// One deflate context for the life of the connection, because consecutive frames repeat each
/// other: a busy pane's `grid.patch` stream measured 183 KB as text, 133 KB deflated a message at
/// a time and 7 KB through one context. Level 6 because inflating costs a client the same at any
/// level and fewer bytes is less of it, so the extra CPU is the node's and not the phone's.
///
/// `max` bounds what a message inflates to, not its bytes on the wire: thirty-two MiB of one
/// character deflates to 32 KB.
pub fn options(max: usize) -> yawc::Options {
    yawc::Options::default()
        .with_balanced_compression()
        .with_max_payload_read(max)
        .with_max_read_buffer(max)
        .with_utf8()
}

pub struct WsOut<S>(SplitSink<WebSocket<S>, Frame>);

pub struct WsIn<S>(SplitStream<WebSocket<S>>, Arc<Heard>);

impl<S> WsIn<S> {
    /// **The half of #284 the dialling side was missing.** A hub is served by the same code that
    /// serves a browser, and that code asks a socket whether it is still there only when it is
    /// given somewhere to record the answer. Without this a hub that stopped delivering was
    /// indistinguishable from a hub with nothing to say, and the node went on serving a socket
    /// the hub had already dropped, measured on a real herd at **three hours**.
    pub fn heard(&self) -> Arc<Heard> {
        self.1.clone()
    }
}

impl<S> Outgoing for WsOut<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    async fn send(&mut self, text: String) -> bool {
        self.0.send(Frame::text(text)).await.is_ok()
    }

    async fn close(&mut self) {
        let _ = self.0.close().await;
    }

    async fn ping(&mut self) -> bool {
        self.0.send(Frame::ping(Vec::new())).await.is_ok()
    }
}

impl<S> Incoming for WsIn<S>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    async fn recv(&mut self) -> Option<String> {
        loop {
            let frame = self.0.next().await?;
            self.1.note();
            match frame.opcode() {
                OpCode::Text => return String::from_utf8(Vec::from(frame.into_payload())).ok(),
                OpCode::Close => return None,
                _ => {}
            }
        }
    }
}

pub fn split<S>(socket: WebSocket<S>) -> (Link<WsOut<S>, WsIn<S>>, Arc<Heard>)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let (sink, stream) = socket.split();
    let heard = Arc::new(Heard::default());
    (Link::new(WsOut(sink), WsIn(stream, heard.clone())), heard)
}

#[derive(Debug, thiserror::Error)]
pub enum ConnectError {
    #[error("timed out")]
    Timeout,
    #[error("not a URL: {0}")]
    Url(String),
    #[error("the server answered {0}")]
    Status(u16),
    #[error("{0}")]
    Failed(String),
}

/// Nagle is off because a socket carries every keystroke and its echo, a few dozen bytes each way,
/// and its wait for an ACK is latency on every one of them.
fn dialling(max: usize) -> yawc::Options {
    options(max).with_no_delay()
}

pub async fn connect(
    url: &str,
    request: HttpRequestBuilder,
    max: usize,
    timeout: Duration,
) -> Result<ClientSocket, ConnectError> {
    let url = url
        .parse()
        .map_err(|e: url::ParseError| ConnectError::Url(e.to_string()))?;
    let dialled = WebSocket::connect(url)
        .with_options(dialling(max))
        .with_connector(tls())
        .with_request(request);
    match tokio::time::timeout(timeout, dialled).await {
        Err(_) => Err(ConnectError::Timeout),
        Ok(Err(yawc::WebSocketError::InvalidStatusCode(status))) => Err(ConnectError::Status(status)),
        Ok(Err(e)) => Err(ConnectError::Failed(e.to_string())),
        Ok(Ok(socket)) => Ok(socket),
    }
}

/// The platform's roots, as the hub's certificate is whatever its operator's proxy was issued.
fn tls() -> tokio_rustls::TlsConnector {
    static CONFIG: OnceLock<Arc<rustls::ClientConfig>> = OnceLock::new();
    let config = CONFIG.get_or_init(|| {
        let mut roots = rustls::RootCertStore::empty();
        roots.add_parsable_certificates(rustls_native_certs::load_native_certs().certs);
        let provider = Arc::new(rustls::crypto::ring::default_provider());
        let config = rustls::ClientConfig::builder_with_provider(provider)
            .with_safe_default_protocol_versions()
            .expect("ring supports TLS 1.2 and 1.3")
            .with_root_certificates(roots)
            .with_no_client_auth();
        Arc::new(config)
    });
    tokio_rustls::TlsConnector::from(config.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_dialled_socket_sends_a_keystroke_s_frame_the_moment_it_is_written() {
        assert!(
            dialling(1).no_delay,
            "Nagle is on, so a frame written while the last is unacknowledged waits"
        );
    }
}
