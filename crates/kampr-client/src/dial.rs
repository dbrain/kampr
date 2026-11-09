use kampr_mesh::ws::{self, ConnectError, HttpRequestBuilder};
use reqwest::header::HeaderValue;
use std::time::Duration;

pub type Socket = ws::ClientSocket;

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
    /// The node answered and said no to the token: revoked, expired, or minted by a node that has
    /// since been reset. Retrying the same token cannot fix it.
    #[error("{0} no longer accepts this device")]
    Refused(String),
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
    let url = ws_url(origin);
    let protocol = HeaderValue::from_str(&format!("{TOKEN_PROTOCOL}{token}"))
        .map_err(|_| DialError::Url(url.clone(), "the token is not a header value".into()))?;
    let request = HttpRequestBuilder::new().header("sec-websocket-protocol", protocol);
    // A node's frames are read here the way a hub reads a peer's, scrollback documents included.
    ws::connect(&url, request, kampr_mesh::MAX_MESSAGE_BYTES, timeout)
        .await
        .map_err(|e| match e {
            ConnectError::Timeout => DialError::Timeout(url.clone()),
            ConnectError::Status(401) => DialError::Refused(origin.trim_end_matches('/').to_string()),
            ConnectError::Url(e) => DialError::Url(url.clone(), e),
            e => DialError::Connect(url.clone(), e.to_string()),
        })
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
