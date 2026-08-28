use std::time::Duration;
use tokio::net::TcpStream;
use tokio_tungstenite::tungstenite::client::IntoClientRequest;
use tokio_tungstenite::{MaybeTlsStream, WebSocketStream};

pub type Socket = WebSocketStream<MaybeTlsStream<TcpStream>>;

/// The exact spelling the node resolves a token from, and the only one it accepts.
const TOKEN_PROTOCOL: &str = "kampr.token.";

#[derive(Debug, thiserror::Error)]
pub enum DialError {
    #[error("connecting to {0}: timed out")]
    Timeout(String),
    #[error("connecting to {0}: {1}")]
    Connect(String, String),
    #[error("{0} is not a URL this client can dial: {1}")]
    Url(String, String),
}

/// `ws://host:port/ws` from the origin the node published. A `wss` or `ws` origin is taken as
/// already being one, so whatever `kampr status` printed can be pasted in.
pub fn ws_url(origin: &str) -> String {
    let trimmed = origin.trim().trim_end_matches('/');
    let (scheme, rest) = match trimmed.split_once("://") {
        Some(("https", rest)) | Some(("wss", rest)) => ("wss", rest),
        Some(("http", rest)) | Some(("ws", rest)) => ("ws", rest),
        Some((_, rest)) => ("wss", rest),
        None => ("ws", trimmed),
    };
    match rest.ends_with("/ws") {
        true => format!("{scheme}://{rest}"),
        false => format!("{scheme}://{rest}/ws"),
    }
}

/// One authenticated socket.
///
/// **The token rides in the subprotocol and nowhere else.** A browser cannot set a header on a
/// WebSocket handshake, so the node reads `kampr.token.<token>` off `Sec-WebSocket-Protocol` and
/// echoes it back verbatim; any other spelling fails the handshake rather than falling back to
/// something that works.
pub async fn connect(origin: &str, token: &str, timeout: Duration) -> Result<Socket, DialError> {
    // tokio-tungstenite builds its rustls client config from the process default provider, and
    // this tree carries exactly one. Installing twice is not an error worth reporting.
    let _ = rustls::crypto::ring::default_provider().install_default();
    let url = ws_url(origin);
    let mut request = url
        .as_str()
        .into_client_request()
        .map_err(|e| DialError::Url(url.clone(), e.to_string()))?;
    let protocol = format!("{TOKEN_PROTOCOL}{token}");
    let value = protocol
        .parse()
        .map_err(|_| DialError::Url(url.clone(), "the token is not a header value".into()))?;
    request.headers_mut().insert("sec-websocket-protocol", value);
    let (socket, _) = tokio::time::timeout(timeout, tokio_tungstenite::connect_async(request))
        .await
        .map_err(|_| DialError::Timeout(url.clone()))?
        .map_err(|e| DialError::Connect(url.clone(), e.to_string()))?;
    Ok(socket)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_spelling_of_an_origin_becomes_one_socket_url() {
        for input in [
            "https://kampr.example.com",
            "https://kampr.example.com/",
            "wss://kampr.example.com/ws",
        ] {
            assert_eq!(ws_url(input), "wss://kampr.example.com/ws", "{input}");
        }
        assert_eq!(ws_url("http://127.0.0.1:8790"), "ws://127.0.0.1:8790/ws");
    }
}
