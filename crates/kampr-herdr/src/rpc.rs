use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

/// **Every hop of a call is inside this.** A herdr that accepts the connection and then never
/// answers used to hang its caller for ever — and the herd sweep, every `manage` op and the
/// width probe all dial through here, so one wedged socket stalled every surface built on it with
/// no error and no recovery. Far above anything herdr takes: a 5000-line read answers in 1 ms
/// (probe #27).
const CALL_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub struct Herdr {
    socket: PathBuf,
    timeout: Duration,
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
            timeout: CALL_TIMEOUT,
        }
    }

    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
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
        let req = serde_json::json!({ "id": "kampr", "method": method, "params": params });
        let line = self
            .within_the_timeout(method, async {
                let mut stream = UnixStream::connect(&self.socket)
                    .await
                    .with_context(|| format!("connecting to herdr socket {}", self.socket.display()))?;
                stream.write_all(serde_json::to_string(&req)?.as_bytes()).await?;
                stream.write_all(b"\n").await?;
                stream.flush().await?;
                let mut line = String::new();
                BufReader::new(stream).read_line(&mut line).await?;
                Ok(line)
            })
            .await?;
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
        // The ack alone is inside the timeout. What follows it is the event stream, which is
        // meant to sit open for hours.
        let (reader, ack) = self
            .within_the_timeout("events.subscribe", async {
                let mut stream = UnixStream::connect(&self.socket).await?;
                stream.write_all(serde_json::to_string(&req)?.as_bytes()).await?;
                stream.write_all(b"\n").await?;
                stream.flush().await?;
                let mut reader = BufReader::new(stream);
                let mut ack = String::new();
                reader.read_line(&mut ack).await?;
                Ok((reader, ack))
            })
            .await?;
        if ack.trim().is_empty() {
            bail!("herdr closed the connection without acknowledging events.subscribe");
        }
        // The envelope, not a substring scan: `ack.contains("\"error\"")` both missed an error
        // shaped differently and refused a legitimate payload that happened to contain the word.
        let env: Envelope<serde_json::Value> = serde_json::from_str(&ack)
            .with_context(|| format!("decoding the events.subscribe ack: {}", ack.trim()))?;
        // The `RpcError` itself, not a wrapper: anyhow's `Display` is the outermost context
        // alone, and callers match on the code herdr sent (`pane_not_found` is a race to retry,
        // probe #107).
        if let Some(e) = env.error {
            return Err(e.into());
        }
        Ok(Subscription { reader })
    }

    async fn within_the_timeout<T>(
        &self,
        method: &str,
        exchange: impl Future<Output = Result<T>>,
    ) -> Result<T> {
        match tokio::time::timeout(self.timeout, exchange).await {
            Ok(answered) => answered,
            Err(_) => bail!(
                "herdr did not answer {method} within {:?} on {}",
                self.timeout,
                self.socket.display()
            ),
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use tokio::net::UnixListener;

    /// Accepts, optionally answers with one line, and never closes.
    fn listening(answer: Option<String>) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().expect("a dir");
        let socket = dir.path().join("herdr.sock");
        let listener = UnixListener::bind(&socket).expect("bind");
        tokio::spawn(async move {
            let held = std::sync::Mutex::new(Vec::new());
            while let Ok((mut stream, _)) = listener.accept().await {
                let mut line = String::new();
                if BufReader::new(&mut stream).read_line(&mut line).await.is_err() {
                    continue;
                }
                if let Some(answer) = &answer {
                    let _ = stream.write_all(answer.as_bytes()).await;
                    let _ = stream.write_all(b"\n").await;
                    let _ = stream.flush().await;
                }
                held.lock().expect("held").push(stream);
            }
        });
        (dir, socket)
    }

    /// One wedged socket used to stall the herd sweep, every `manage` op and the width probe, for
    /// as long as the process lived.
    #[tokio::test]
    async fn a_herdr_that_accepts_and_never_answers_is_an_error_rather_than_a_hang() {
        let (_dir, socket) = listening(None);
        let herdr = Herdr::new(&socket).with_timeout(Duration::from_millis(80));
        let asked = tokio::time::timeout(
            Duration::from_secs(2),
            herdr.call::<Value>("session.snapshot", json!({})),
        )
        .await
        .expect("the call hung past its own timeout");
        let said = asked
            .expect_err("a socket that never answers is not a success")
            .to_string();
        assert!(said.contains("session.snapshot"), "{said}");
        assert!(said.contains("within"), "{said}");
    }

    /// `ack.contains("\"error\"")` is a substring scan over the whole line: it misses an error
    /// envelope shaped any other way and refuses an ack whose payload merely says the word.
    #[tokio::test]
    async fn a_subscribe_ack_is_read_as_an_envelope_and_not_scanned_for_a_word() {
        // A success envelope that carries an explicit null `error`, which is what a great many
        // JSON-RPC servers emit and what a substring scan reads as a refusal.
        let started = json!({
            "id": "kampr-events",
            "error": Value::Null,
            "result": { "type": "subscription_started" }
        });
        let (_dir, socket) = listening(Some(started.to_string()));
        Herdr::new(&socket)
            .with_timeout(Duration::from_secs(2))
            .subscribe(&[Sub::kind("pane.created")])
            .await
            .expect("an explicit null error is a success envelope");

        let refused = json!({
            "id": "kampr-events",
            "error": { "code": "pane_not_found", "message": "no pane w1:p9" }
        });
        let (_dir, socket) = listening(Some(refused.to_string()));
        let refusal = Herdr::new(&socket)
            .with_timeout(Duration::from_secs(2))
            .subscribe(&[Sub::pane("pane.agent_status_changed", "w1:p9")])
            .await;
        let Err(said) = refusal else {
            panic!("an error envelope is a refusal");
        };
        let said = said.to_string();
        assert!(said.contains("pane_not_found"), "{said}");
        assert!(said.contains("no pane w1:p9"), "{said}");
    }
}
