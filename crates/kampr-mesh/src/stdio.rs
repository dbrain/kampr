//! A mesh link over a byte stream: one JSON message per line. A node hosting a child over a
//! command's stdin and stdout (a container's `docker exec`) speaks it from both ends.

use crate::MAX_MESSAGE_BYTES;
use crate::transport::{Heard, Incoming, Link, Outgoing};
use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};

pub struct StdioOut<W>(Option<W>);

pub struct StdioIn<R> {
    reader: BufReader<R>,
    heard: Arc<Heard>,
    line: Vec<u8>,
}

impl<R> StdioIn<R> {
    pub fn heard(&self) -> Arc<Heard> {
        self.heard.clone()
    }
}

pub fn link<R, W>(reader: R, writer: W) -> Link<StdioOut<W>, StdioIn<R>>
where
    R: AsyncRead + Unpin + Send + 'static,
    W: AsyncWrite + Unpin + Send + 'static,
{
    Link::new(
        StdioOut(Some(writer)),
        StdioIn {
            reader: BufReader::new(reader),
            heard: Arc::new(Heard::default()),
            line: Vec::new(),
        },
    )
}

impl<W: AsyncWrite + Unpin + Send + 'static> Outgoing for StdioOut<W> {
    async fn send(&mut self, text: String) -> bool {
        let Some(writer) = &mut self.0 else {
            return false;
        };
        // A newline inside a message would split it in two at the far end; JSON never has one.
        if text.contains('\n') {
            return false;
        }
        let sent = async {
            writer.write_all(text.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await
        }
        .await;
        if sent.is_err() {
            self.0 = None;
        }
        sent.is_ok()
    }

    async fn close(&mut self) {
        if let Some(mut writer) = self.0.take() {
            let _ = writer.shutdown().await;
        }
    }
}

impl<R: AsyncRead + Unpin + Send + 'static> Incoming for StdioIn<R> {
    async fn recv(&mut self) -> Option<String> {
        loop {
            self.line.clear();
            let limit = MAX_MESSAGE_BYTES as u64 + 1;
            let read = (&mut self.reader)
                .take(limit)
                .read_until(b'\n', &mut self.line)
                .await
                .ok()?;
            if read == 0 || self.line.last() != Some(&b'\n') {
                return None;
            }
            self.heard.note();
            self.line.pop();
            if self.line.last() == Some(&b'\r') {
                self.line.pop();
            }
            if self.line.is_empty() {
                continue;
            }
            match std::str::from_utf8(&self.line) {
                Ok(text) => return Some(text.to_string()),
                Err(_) => tracing::debug!("a stdio mesh link sent a line that is not UTF-8"),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn lines_cross_both_ways_and_the_end_of_the_stream_ends_the_link() {
        let (a, b) = tokio::io::duplex(64);
        let (ar, aw) = tokio::io::split(a);
        let (br, bw) = tokio::io::split(b);
        let (mut left, mut right) = (link(ar, aw), link(br, bw));
        assert!(left.out.send(r#"{"t":"hello"}"#.into()).await);
        assert_eq!(right.incoming.recv().await.as_deref(), Some(r#"{"t":"hello"}"#));
        assert!(right.incoming.heard().take());
        assert!(!left.out.send("two\nlines".into()).await);
        left.out.close().await;
        drop(left);
        assert_eq!(right.incoming.recv().await, None);
        assert!(!right.out.send("{}".into()).await);
    }

    #[tokio::test]
    async fn a_line_longer_than_a_mesh_message_ends_the_link() {
        let (a, b) = tokio::io::duplex(1 << 20);
        let (_ar, mut aw) = tokio::io::split(a);
        let (br, bw) = tokio::io::split(b);
        let mut right = link(br, bw);
        let writer = tokio::spawn(async move {
            let chunk = vec![b'x'; 1 << 20];
            for _ in 0..=MAX_MESSAGE_BYTES >> 20 {
                if aw.write_all(&chunk).await.is_err() {
                    return;
                }
            }
            let _ = aw.write_all(b"\n").await;
        });
        assert_eq!(right.incoming.recv().await, None);
        writer.abort();
    }
}
