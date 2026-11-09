//! permessage-deflate on both sockets, checked at the byte level with a client that is nothing but
//! zlib, the way a browser's and OkHttp's are: a node that only interoperated with its own
//! library's compressor would pass every other test here.

use crate::limits::Harness;
use flate2::{Compress, Compression, Decompress, FlushCompress, FlushDecompress};
use kampr_auth::Role;
use serde_json::{Value, json};
use std::time::Duration;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

/// What each client that ships puts in `Sec-WebSocket-Extensions`. Chrome asks to choose its own
/// window and is answered with a cap on it; a bare offer must be answered bare, or OkHttp fails
/// the handshake over a parameter it never offered (RFC 7692 §7.1.2.2).
const OFFERS: [(&str, &str); 2] = [
    ("Chrome", "permessage-deflate; client_max_window_bits"),
    ("Firefox and OkHttp", "permessage-deflate"),
];
const OFFER: &str = OFFERS[0].1;
const TAIL: [u8; 4] = [0x00, 0x00, 0xff, 0xff];
const RSV1: u8 = 0x40;
const CLOSE: u8 = 0x8;

struct Raw {
    stream: TcpStream,
    inflate: Decompress,
    head: String,
}

struct Frame {
    first: u8,
    payload: Vec<u8>,
}

impl Raw {
    async fn open(origin: &str, path: &str, headers: &[(&str, &str)]) -> Self {
        let address = origin.trim_start_matches("http://");
        let mut stream = TcpStream::connect(address).await.expect("connect");
        let extra: String = headers.iter().map(|(k, v)| format!("{k}: {v}\r\n")).collect();
        let request = format!(
            "GET {path} HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\n\
             Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n{extra}\r\n"
        );
        stream.write_all(request.as_bytes()).await.expect("write");
        let mut head = Vec::new();
        while !head.ends_with(b"\r\n\r\n") {
            head.push(stream.read_u8().await.expect("a response head"));
        }
        Self {
            stream,
            inflate: Decompress::new(false),
            head: String::from_utf8(head)
                .expect("an ASCII head")
                .to_ascii_lowercase(),
        }
    }

    fn header(&self, name: &str) -> Option<&str> {
        self.head
            .lines()
            .find_map(|line| line.strip_prefix(name)?.strip_prefix(':'))
            .map(str::trim)
    }

    async fn frame(&mut self) -> Option<Frame> {
        let first = self.stream.read_u8().await.ok()?;
        let len = match self.stream.read_u8().await.ok()? & 0x7f {
            126 => self.stream.read_u16().await.ok()? as usize,
            127 => self.stream.read_u64().await.ok()? as usize,
            n => n as usize,
        };
        let mut payload = vec![0; len];
        self.stream.read_exact(&mut payload).await.ok()?;
        Some(Frame { first, payload })
    }

    /// `Some(n)` is a message that crossed the wire deflated to `n` bytes.
    async fn message(&mut self) -> Option<(Option<usize>, Value)> {
        let frame = self.frame().await?;
        if frame.first & 0x0f == CLOSE {
            return None;
        }
        let (deflated, text) = match frame.first & RSV1 != 0 {
            true => (Some(frame.payload.len()), self.inflate(frame.payload)),
            false => (None, frame.payload),
        };
        Some((deflated, serde_json::from_slice(&text).expect("a JSON message")))
    }

    async fn until(&mut self, t: &str) -> (Option<usize>, Value) {
        tokio::time::timeout(Duration::from_secs(10), async {
            loop {
                let (compressed, message) = self.message().await.expect("the node hung up");
                if message["t"] == t {
                    return (compressed, message);
                }
            }
        })
        .await
        .unwrap_or_else(|_| panic!("no {t} arrived"))
    }

    fn inflate(&mut self, mut payload: Vec<u8>) -> Vec<u8> {
        payload.extend_from_slice(&TAIL);
        let mut out = Vec::with_capacity(payload.len() * 8);
        let mut at = 0;
        while at < payload.len() {
            out.reserve(64 * 1024);
            let before = self.inflate.total_in();
            self.inflate
                .decompress_vec(&payload[at..], &mut out, FlushDecompress::Sync)
                .expect("a valid deflate stream");
            at += (self.inflate.total_in() - before) as usize;
        }
        out
    }

    async fn send(&mut self, message: &Value, compressed: bool) -> std::io::Result<()> {
        let text = message.to_string().into_bytes();
        let payload = match compressed {
            true => deflate(&text),
            false => text,
        };
        let mut frame = vec![0x81 | if compressed { RSV1 } else { 0 }];
        match payload.len() {
            n if n < 126 => frame.push(0x80 | n as u8),
            n if n <= u16::MAX as usize => {
                frame.push(0x80 | 126);
                frame.extend_from_slice(&(n as u16).to_be_bytes());
            }
            n => {
                frame.push(0x80 | 127);
                frame.extend_from_slice(&(n as u64).to_be_bytes());
            }
        }
        let mask = [0x1d, 0x2e, 0x3f, 0x40];
        frame.extend_from_slice(&mask);
        frame.extend(payload.iter().enumerate().map(|(i, b)| b ^ mask[i % 4]));
        self.stream.write_all(&frame).await
    }

    async fn hung_up(&mut self, within: Duration) -> bool {
        tokio::time::timeout(within, async {
            while let Some(frame) = self.frame().await {
                if frame.first & 0x0f == CLOSE {
                    return;
                }
            }
        })
        .await
        .is_ok()
    }
}

fn deflate(text: &[u8]) -> Vec<u8> {
    let mut deflate = Compress::new(Compression::fast(), false);
    let mut out = Vec::with_capacity(text.len() / 4 + 64);
    let mut at = 0;
    loop {
        out.reserve(64 * 1024);
        let before = deflate.total_in();
        deflate
            .compress_vec(&text[at..], &mut out, FlushCompress::Sync)
            .expect("deflate");
        at += (deflate.total_in() - before) as usize;
        if at == text.len() && out.ends_with(&TAIL) {
            break;
        }
    }
    out.truncate(out.len() - TAIL.len());
    out
}

async fn client(h: &Harness, offer: Option<&str>) -> Raw {
    let token = h.token(Role::Full).await;
    let protocol = format!("kampr.token.{token}");
    let mut headers = vec![("Sec-WebSocket-Protocol", protocol.as_str())];
    headers.extend(offer.map(|offer| ("Sec-WebSocket-Extensions", offer)));
    let raw = Raw::open(&h.origin, "/ws", &headers).await;
    assert!(raw.head.starts_with("http/1.1 101"), "{}", raw.head);
    assert_eq!(
        raw.header("sec-websocket-protocol"),
        Some(protocol.to_ascii_lowercase().as_str()),
        "a browser fails the handshake unless the token subprotocol is echoed"
    );
    raw
}

/// A phone on a cell link is where the bytes are paid for: a busy pane's `grid.patch` stream
/// measured 183 KB as text and 7 KB through one deflate context for the connection.
#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_offers_compression_is_answered_in_it_both_ways() {
    let h = Harness::start(|_| {}).await;
    for (who, offer) in OFFERS {
        let mut raw = client(&h, Some(offer)).await;
        let agreed = raw.header("sec-websocket-extensions").unwrap_or_default();
        assert!(
            agreed.starts_with("permessage-deflate"),
            "{who}: nothing agreed: {}",
            raw.head
        );
        if !offer.contains("client_max_window_bits") {
            assert_eq!(
                agreed, "permessage-deflate",
                "{who} is answered with what it did not offer"
            );
        }

        let (deflated, _) = raw.until("hello").await;
        assert!(deflated.is_some(), "{who}: the greeting went out as plain text");

        let mut sizes = Vec::new();
        for _ in 0..2 {
            raw.send(&json!({ "t": "ping", "n": 7 }), true)
                .await
                .expect("a ping");
            let (deflated, pong) = raw.until("pong").await;
            assert_eq!(
                pong["n"], 7,
                "{who}: a compressed message from the client was not read: {pong}"
            );
            sizes.push(deflated.expect("a pong in plain text"));
        }
        assert!(
            sizes[1] * 2 <= sizes[0],
            "{who}: a repeat of the last message cost {} bytes against {}, so each message is \
             deflated alone and a pane's patches lose the context that makes them small",
            sizes[1],
            sizes[0]
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_client_that_offers_nothing_is_answered_in_plain_text() {
    let h = Harness::start(|_| {}).await;
    let mut raw = client(&h, None).await;
    assert_eq!(raw.header("sec-websocket-extensions"), None, "{}", raw.head);
    let (deflated, _) = raw.until("hello").await;
    assert_eq!(deflated, None);
}

/// The size ceiling is on what a message *inflates* to. Thirty-two MiB of one byte deflates to
/// about 32 KB, so a bound on the wire's bytes would hand the whole thing to the JSON parser.
#[tokio::test(flavor = "multi_thread")]
async fn a_compressed_message_is_bounded_by_what_it_inflates_to() {
    let h = Harness::start(|_| {}).await;
    let mut raw = client(&h, Some(OFFER)).await;
    assert!(raw.header("sec-websocket-extensions").is_some(), "{}", raw.head);
    raw.until("hello").await;

    let bomb = json!({ "t": "prefs", "pane": "x", "pad": "a".repeat(32 * 1024 * 1024) });
    assert!(deflate(bomb.to_string().as_bytes()).len() < 1024 * 1024);
    let refused_mid_write = raw.send(&bomb, true).await.is_err();
    assert!(
        refused_mid_write || raw.hung_up(Duration::from_secs(10)).await,
        "32 MiB inflated past the message ceiling and the socket stayed open"
    );
}

/// The WAN hop between a peer and its hub carries every pane any client of the hub is watching.
#[tokio::test(flavor = "multi_thread")]
async fn the_mesh_socket_agrees_to_compression() {
    let h = Harness::start(|c| c.mesh.accept = true).await;
    let raw = Raw::open(&h.origin, "/mesh", &[("Sec-WebSocket-Extensions", OFFER)]).await;
    assert!(raw.head.starts_with("http/1.1 101"), "{}", raw.head);
    let agreed = raw.header("sec-websocket-extensions").unwrap_or_default();
    assert!(
        agreed.starts_with("permessage-deflate"),
        "nothing agreed: {}",
        raw.head
    );
}
