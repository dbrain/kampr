use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

#[derive(Debug, Clone)]
pub struct Herdr {
    socket: PathBuf,
}

#[derive(Debug, Deserialize)]
struct Envelope<T> {
    result: Option<T>,
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize, thiserror::Error)]
#[error("{code}: {message}")]
pub struct RpcError {
    pub code: String,
    pub message: String,
}

impl Herdr {
    pub fn new(socket: impl AsRef<Path>) -> Self {
        Self {
            socket: socket.as_ref().to_path_buf(),
        }
    }

    /// Resolution order matches herdr's own: `HERDR_SOCKET_PATH`, then `HERDR_SESSION`,
    /// then the default session socket.
    pub fn discover() -> Result<Self> {
        if let Ok(p) = std::env::var("HERDR_SOCKET_PATH") {
            return Ok(Self::new(p));
        }
        let base = dirs_config()?.join("herdr");
        let socket = match std::env::var("HERDR_SESSION") {
            Ok(name) if !name.is_empty() => base.join("sessions").join(name).join("herdr.sock"),
            _ => base.join("herdr.sock"),
        };
        Ok(Self::new(socket))
    }

    pub fn socket(&self) -> &Path {
        &self.socket
    }

    /// Herdr closes the connection after a single response, so every call dials fresh.
    /// `events.subscribe` is the sole exception and is handled by [`Self::subscribe`].
    pub async fn call<T: DeserializeOwned>(&self, method: &str, params: serde_json::Value) -> Result<T> {
        let mut stream = UnixStream::connect(&self.socket)
            .await
            .with_context(|| format!("connecting to herdr socket {}", self.socket.display()))?;
        let req = serde_json::json!({ "id": "kampr", "method": method, "params": params });
        stream.write_all(serde_json::to_string(&req)?.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;

        let mut line = String::new();
        BufReader::new(stream).read_line(&mut line).await?;
        if line.trim().is_empty() {
            bail!("herdr closed the connection without replying to {method}");
        }
        let env: Envelope<T> =
            serde_json::from_str(&line).with_context(|| format!("decoding reply to {method}: {line}"))?;
        match (env.result, env.error) {
            (Some(r), _) => Ok(r),
            (None, Some(e)) => Err(e.into()),
            (None, None) => bail!("herdr reply to {method} had neither result nor error"),
        }
    }

    /// **A subscription list is all-or-nothing, twice over.** An entry that omits a required
    /// `pane_id` is refused before the stream opens (probe #54); an entry naming a pane that has
    /// since closed is answered with `pane_not_found` and the socket is then closed (probe #76).
    /// Both take the whole call with them, so a caller re-derives its pane set from a fresh
    /// snapshot and retries rather than treating either as fatal.
    pub async fn subscribe(&self, subs: &[Sub]) -> Result<Subscription> {
        let mut stream = UnixStream::connect(&self.socket).await?;
        let subs: Vec<_> = subs
            .iter()
            .map(|s| match &s.pane_id {
                Some(pane_id) => serde_json::json!({ "type": s.kind, "pane_id": pane_id }),
                None => serde_json::json!({ "type": s.kind }),
            })
            .collect();
        let req = serde_json::json!({
            "id": "kampr-events", "method": "events.subscribe",
            "params": { "subscriptions": subs }
        });
        stream.write_all(serde_json::to_string(&req)?.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        stream.flush().await?;
        let mut reader = BufReader::new(stream);
        let mut ack = String::new();
        reader.read_line(&mut ack).await?;
        if ack.contains("\"error\"") {
            bail!("events.subscribe rejected: {}", ack.trim());
        }
        Ok(Subscription { reader })
    }
}

/// One entry in an `events.subscribe` list.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct Sub {
    pub kind: &'static str,
    /// Required by some event kinds and rejected as a missing field without it.
    pub pane_id: Option<String>,
}

impl Sub {
    pub fn kind(kind: &'static str) -> Self {
        Self { kind, pane_id: None }
    }

    pub fn pane(kind: &'static str, pane_id: &str) -> Self {
        Self {
            kind,
            pane_id: Some(pane_id.to_string()),
        }
    }
}

pub struct Subscription {
    reader: BufReader<UnixStream>,
}

impl Subscription {
    /// **Not cancel-safe.** This is a `read_line`, so dropping the future mid-line — a
    /// `tokio::time::timeout` or a `select!` branch that loses — discards the bytes already read and
    /// silently corrupts the stream. Drive it from a dedicated task feeding a channel, never
    /// directly inside a `select!`.
    pub async fn next(&mut self) -> Result<Option<serde_json::Value>> {
        let mut line = String::new();
        if self.reader.read_line(&mut line).await? == 0 {
            return Ok(None);
        }
        Ok(Some(serde_json::from_str(&line)?))
    }
}

fn dirs_config() -> Result<PathBuf> {
    if let Ok(x) = std::env::var("XDG_CONFIG_HOME")
        && !x.is_empty()
    {
        return Ok(PathBuf::from(x));
    }
    let home = std::env::var("HOME").context("HOME is not set")?;
    Ok(PathBuf::from(home).join(".config"))
}
