//! A bare HTTP/1.1 client: what goes on the wire is exactly what the test wrote, and what comes
//! back is read whole, chunked bodies decoded, so a header the node did or did not send is visible.

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;

pub(crate) struct Reply {
    pub(crate) status: u16,
    pub(crate) headers: Vec<(String, String)>,
    pub(crate) body: Vec<u8>,
}

impl Reply {
    pub(crate) fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(n, _)| n.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
    }

    pub(crate) fn text(&self) -> String {
        String::from_utf8_lossy(&self.body).into_owned()
    }

    pub(crate) fn json(&self) -> serde_json::Value {
        serde_json::from_slice(&self.body).unwrap_or_else(|_| panic!("not JSON: {}", self.text()))
    }
}

async fn connect(origin: &str) -> TcpStream {
    let rest = origin.trim_start_matches("http://");
    TcpStream::connect(rest).await.expect("connect")
}

fn head(method: &str, path: &str, headers: &[(&str, &str)], length: usize) -> String {
    let extra: String = headers.iter().map(|(n, v)| format!("{n}: {v}\r\n")).collect();
    format!(
        "{method} {path} HTTP/1.1\r\nHost: x\r\n{extra}Content-Length: {length}\r\nConnection: close\r\n\r\n"
    )
}

pub(crate) async fn send(
    origin: &str,
    method: &str,
    path: &str,
    headers: &[(&str, &str)],
    body: &[u8],
) -> Reply {
    let mut stream = connect(origin).await;
    stream
        .write_all(head(method, path, headers, body.len()).as_bytes())
        .await
        .expect("write");
    for piece in body.chunks(256 * 1024) {
        stream.write_all(piece).await.expect("write");
    }
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read");
    parse(&raw)
}

/// A request that promises `declared` bytes, sends `sent` of them and hangs up: what a phone
/// losing its signal halfway through an upload looks like from the node.
pub(crate) async fn abandon(
    origin: &str,
    path: &str,
    headers: &[(&str, &str)],
    declared: usize,
    sent: &[u8],
) {
    let mut stream = connect(origin).await;
    stream
        .write_all(head("POST", path, headers, declared).as_bytes())
        .await
        .expect("write");
    stream.write_all(sent).await.expect("write");
    stream.flush().await.expect("flush");
    tokio::time::sleep(std::time::Duration::from_millis(200)).await;
}

/// A request that promises `declared` bytes, sends `sent` of them and then sends nothing more
/// while keeping the connection open, and returns whatever the node answers.
pub(crate) async fn stall(
    origin: &str,
    path: &str,
    headers: &[(&str, &str)],
    declared: usize,
    sent: &[u8],
) -> Reply {
    let mut stream = connect(origin).await;
    stream
        .write_all(head("POST", path, headers, declared).as_bytes())
        .await
        .expect("write");
    stream.write_all(sent).await.expect("write");
    stream.flush().await.expect("flush");
    let mut raw = Vec::new();
    stream.read_to_end(&mut raw).await.expect("read");
    parse(&raw)
}

pub(crate) fn parse(raw: &[u8]) -> Reply {
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .unwrap_or_else(|| panic!("no head in {:?}", String::from_utf8_lossy(raw)));
    let head = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut lines = head.lines();
    let status = lines
        .next()
        .and_then(|l| l.split_whitespace().nth(1))
        .and_then(|s| s.parse().ok())
        .expect("a status");
    let headers: Vec<(String, String)> = lines
        .filter_map(|l| l.split_once(": "))
        .map(|(n, v)| (n.to_ascii_lowercase(), v.to_string()))
        .collect();
    let rest = &raw[split + 4..];
    let chunked = headers
        .iter()
        .any(|(n, v)| n == "transfer-encoding" && v.contains("chunked"));
    let body = if chunked { dechunk(rest) } else { rest.to_vec() };
    Reply {
        status,
        headers,
        body,
    }
}

fn dechunk(mut rest: &[u8]) -> Vec<u8> {
    let mut body = Vec::new();
    loop {
        let line = rest.windows(2).position(|w| w == b"\r\n").expect("a chunk size");
        let size = usize::from_str_radix(std::str::from_utf8(&rest[..line]).expect("hex").trim(), 16)
            .expect("a chunk size");
        rest = &rest[line + 2..];
        if size == 0 {
            return body;
        }
        body.extend_from_slice(&rest[..size]);
        rest = &rest[size + 2..];
    }
}
