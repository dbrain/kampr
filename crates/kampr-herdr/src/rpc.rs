use anyhow::{Context, Result, bail};
use serde::Deserialize;
use serde::de::DeserializeOwned;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::ffi::OsStrExt;
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
                let stream = dial(&self.socket, &req).await?;
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
                let stream = dial(&self.socket, &req).await?;
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

/// One connection carrying one whole request, by the shortest path that cannot block a runtime
/// worker.
///
/// herdr looks at a fresh connection **once** — within tens of microseconds of the client's
/// `connect(2)` — and if the request is not whole by then it does not look again until ~100.5 ms
/// after the connect (#445). Every reply is therefore 0.2 ms or 100-107 ms, on every method
/// alike, and the only quantity this end owns is how wide the window between its connect and its
/// finished write is.
///
/// `UnixStream::connect(..).await` completes on the reactor, so the write after it runs only once
/// a worker has been woken and scheduled: 25 us at p50 idle and 85 us loaded, against the ~5 us a
/// client that connects and writes in one breath achieves. The timer never moves, so widening
/// that window is the whole of the load sensitivity — 1-17 % of calls stalling as the machine
/// fills up.
///
/// So the socket is made non-blocking *before* it is connected, and the connect and the write
/// happen back to back on this thread with nothing between them. Non-blocking is the point rather
/// than an optimisation: a synchronous `connect(2)` on a unix socket whose listen backlog is full
/// blocks the calling thread, and blocking a runtime worker is far worse than the 100 ms it would
/// save. `EAGAIN` there is a decline, not an error — the async path below takes it, exactly as
/// before.
///
/// Everything else declines the same way and for the same reason: a socket that will not open, a
/// path too long for `sun_path`, a connect that refuses. The fallback re-runs the whole exchange
/// and produces the error message this crate's callers already match on.
async fn dial(socket: &Path, req: &serde_json::Value) -> Result<UnixStream> {
    let line = request_line(req)?;
    if let Some((connected, written)) = connect_and_write(socket, &line) {
        let mut stream = UnixStream::from_std(connected)?;
        // A request is a few hundred bytes into an empty socket buffer, so a short write means
        // the kernel is under pressure rather than that the shape is wrong. The rest goes the
        // ordinary way; the connection is already made and the stall, if any, is already taken.
        if written < line.len() {
            stream.write_all(&line[written..]).await?;
        }
        return Ok(stream);
    }
    let mut stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("connecting to herdr socket {}", socket.display()))?;
    write_request(&mut stream, &line).await?;
    Ok(stream)
}

/// The request as herdr must find it: the JSON and its terminating newline, one buffer.
fn request_line(req: &serde_json::Value) -> Result<Vec<u8>> {
    let mut line = serde_json::to_vec(req)?;
    line.push(b'\n');
    Ok(line)
}

/// The whole request, terminating newline included, in **one** write.
///
/// A request written as body-then-newline puts a second `.await` inside the window herdr is
/// looking through. Two writes 200 us apart stalled 191 of 200 calls where one write stalled none
/// (#445). The gap is the runtime's, so it is nothing on a quiet machine and milliseconds on a
/// loaded one — which is why the test below asserts on the writes rather than on a clock.
async fn write_request(stream: &mut (impl tokio::io::AsyncWrite + Unpin), line: &[u8]) -> Result<()> {
    stream.write_all(line).await?;
    Ok(())
}

#[cfg(target_os = "linux")]
const SEND_FLAGS: libc::c_int = libc::MSG_NOSIGNAL;
#[cfg(not(target_os = "linux"))]
const SEND_FLAGS: libc::c_int = 0;

/// A connected-in-a-moment socket that is non-blocking, close-on-exec, and will not raise SIGPIPE.
///
/// Close-on-exec matters here specifically: this crate also spawns `herdr terminal session
/// observe`, and a window where a fresh fd is inheritable is a descriptor leaked into somebody
/// else's process.
#[cfg(target_os = "linux")]
fn nonblocking_cloexec_socket() -> Option<OwnedFd> {
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

/// The BSDs have neither flag on `socket(2)` and no `MSG_NOSIGNAL` the kernel honours, so all three
/// properties are set afterwards. The inheritable window this reopens is the reason Linux does it
/// in the one call; there is no way to close it here, and `libc`'s Apple bindings *define*
/// `MSG_NOSIGNAL` while the kernel ignores it, so the flag compiles and silently does nothing —
/// which is why this is `SO_NOSIGPIPE` on the socket rather than a flag on the send.
#[cfg(not(target_os = "linux"))]
fn nonblocking_cloexec_socket() -> Option<OwnedFd> {
    let fd = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM, 0) };
    if fd < 0 {
        return None;
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    let raw = fd.as_raw_fd();
    let on: libc::c_int = 1;
    let flags = unsafe { libc::fcntl(raw, libc::F_GETFL) };
    let ok = flags >= 0
        && unsafe { libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) } == 0
        && unsafe { libc::fcntl(raw, libc::F_SETFD, libc::FD_CLOEXEC) } == 0
        && unsafe {
            libc::setsockopt(
                raw,
                libc::SOL_SOCKET,
                libc::SO_NOSIGPIPE,
                std::ptr::addr_of!(on).cast::<libc::c_void>(),
                size_of::<libc::c_int>() as libc::socklen_t,
            )
        } == 0;
    ok.then_some(fd)
}

/// Connects and writes with nothing between, or declines.
///
/// `Some((stream, written))` is a connection herdr's first look will find a request on; `written`
/// is what the one `send(2)` took. `None` is every reason not to have tried, and the caller falls
/// back to the async path rather than turning a decline into a failure.
fn connect_and_write(socket: &Path, line: &[u8]) -> Option<(std::os::unix::net::UnixStream, usize)> {
    let address = sockaddr_un(socket)?;
    let fd = nonblocking_cloexec_socket()?;
    let connected = unsafe {
        libc::connect(
            fd.as_raw_fd(),
            std::ptr::addr_of!(address).cast::<libc::sockaddr>(),
            size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if connected < 0 {
        return None;
    }
    // Or a herdr that closed between the connect and the write kills this process with SIGPIPE
    // rather than answering an error. Linux says so per send; the BSDs said it once on the socket,
    // in `nonblocking_cloexec_socket`.
    let sent = unsafe {
        libc::send(
            fd.as_raw_fd(),
            line.as_ptr().cast::<libc::c_void>(),
            line.len(),
            SEND_FLAGS,
        )
    };
    if sent <= 0 {
        return None;
    }
    Some((std::os::unix::net::UnixStream::from(fd), sent as usize))
}

/// The address `connect(2)` needs, or `None` for a path `sun_path` cannot hold.
///
/// `sun_path` is 108 bytes including its terminator and there is no error for overrunning it —
/// the path is silently truncated and the connect goes somewhere else. herdr itself refuses to
/// start on a session socket past that length, saying only "local socket name length exceeds
/// capacity", so a path this long is a misconfiguration rather than a case to handle here: it
/// declines, and the async path reports it.
fn sockaddr_un(socket: &Path) -> Option<libc::sockaddr_un> {
    let path = socket.as_os_str().as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    address.sun_family = libc::AF_UNIX as libc::sa_family_t;
    if path.is_empty() || path.len() >= address.sun_path.len() {
        return None;
    }
    for (slot, byte) in address.sun_path.iter_mut().zip(path) {
        *slot = *byte as libc::c_char;
    }
    Some(address)
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

    /// **The newline is part of the request, not a second one.**
    ///
    /// herdr reads a fresh connection once and then leaves it alone for 100 ms, so a request that
    /// arrives as body-then-newline can be caught between the two and costs a whole poll
    /// interval. The gap is the one between two `.await`s, which is nothing on an idle machine
    /// and milliseconds on a loaded one — which is why this is asserted on the writes rather than
    /// on a clock: a timing test here would be green on every quiet run and prove nothing.
    #[tokio::test]
    async fn a_request_reaches_herdr_in_one_write_because_a_second_one_waits_out_the_poll() {
        #[derive(Default)]
        struct Writes(Vec<Vec<u8>>);

        impl tokio::io::AsyncWrite for Writes {
            fn poll_write(
                mut self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
                buf: &[u8],
            ) -> std::task::Poll<std::io::Result<usize>> {
                self.0.push(buf.to_vec());
                std::task::Poll::Ready(Ok(buf.len()))
            }

            fn poll_flush(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Ready(Ok(()))
            }

            fn poll_shutdown(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
            ) -> std::task::Poll<std::io::Result<()>> {
                std::task::Poll::Ready(Ok(()))
            }
        }

        let req = json!({ "id": "kampr", "method": "pane.send_text", "params": { "text": "a" } });
        let mut writes = Writes::default();
        let line = request_line(&req).expect("serialised");
        write_request(&mut writes, &line).await.expect("written");

        assert_eq!(
            writes.0.len(),
            1,
            "the request went out in {} writes, so herdr can look between them: {:?}",
            writes.0.len(),
            writes
                .0
                .iter()
                .map(|w| String::from_utf8_lossy(w).to_string())
                .collect::<Vec<_>>()
        );
        let sent = String::from_utf8(writes.0[0].clone()).expect("utf-8");
        assert_eq!(sent, format!("{req}\n"));
    }

    /// **The connect and the write must not have a scheduling hop between them.**
    ///
    /// herdr looks at a fresh connection once and then not again for ~100.5 ms (#445), so the
    /// window between this end's `connect(2)` and its finished write is the whole of the stall
    /// rate. `UnixStream::connect(..).await` completes on the reactor, which puts a worker wakeup
    /// in that window: 79-152 us at p50 on a loaded machine against 26-34 us for a connect and a
    /// write back to back, and 3.2-24.1 % of calls stalling against 0-0.5 %.
    ///
    /// Asserted as **one poll**, not as a duration. A future that reaches the write without ever
    /// returning `Pending` cannot have yielded to the runtime in between, and that is the property
    /// — where a timing assertion would be green on every quiet run and prove nothing.
    #[tokio::test]
    async fn dialling_herdr_never_yields_between_the_connect_and_the_write() {
        let (_dir, socket) = listening(Some(json!({ "result": {} }).to_string()));
        let req = json!({ "id": "kampr", "method": "pane.list", "params": {} });

        let dialling = std::pin::pin!(dial(&socket, &req));
        let waker = std::task::Waker::noop();
        let polled = dialling.poll(&mut std::task::Context::from_waker(waker));

        assert!(
            matches!(polled, std::task::Poll::Ready(Ok(_))),
            "dial yielded to the runtime before the request was written, \
             so herdr's one look can land in the gap"
        );
    }

    /// **The hazard the eager path must not introduce.** A `connect(2)` on a unix socket whose
    /// listen backlog is full blocks the calling thread, and blocking a runtime worker is far
    /// worse than the 100 ms it saves. The socket is therefore non-blocking *before* it is
    /// connected, so a full backlog is `EAGAIN` — a decline the async path picks up — and never a
    /// stalled worker.
    ///
    /// The flag itself is the assertion, because it is what makes blocking impossible; a test that
    /// filled a backlog and waited would be asserting on a clock. It is load-bearing twice over:
    /// `UnixStream::from_std` is documented to require a non-blocking socket, so a blocking fd
    /// handed to the runtime would stall a worker on every read as well.
    #[tokio::test]
    async fn the_socket_is_non_blocking_before_it_is_connected() {
        use std::os::fd::AsRawFd;

        let (_dir, socket) = listening(None);
        let line = request_line(&json!({ "id": "kampr", "method": "ping", "params": {} })).expect("line");
        let (connected, written) = connect_and_write(&socket, &line).expect("a listening socket dials");

        assert_eq!(
            written,
            line.len(),
            "the request was not whole when the connect returned"
        );
        let flags = unsafe { libc::fcntl(connected.as_raw_fd(), libc::F_GETFL) };
        assert!(flags >= 0, "F_GETFL failed");
        assert_eq!(
            flags & libc::O_NONBLOCK,
            libc::O_NONBLOCK,
            "a blocking socket parks a runtime worker whenever herdr's backlog is full"
        );
    }

    /// A decline is not a failure. Everything the eager path will not do — a socket that is not
    /// there, a path too long for `sun_path` — falls through to the async path, which is what
    /// produces the message every caller of this crate already reads.
    #[tokio::test]
    async fn a_socket_the_eager_path_declines_still_fails_with_the_message_callers_read() {
        let dir = tempfile::tempdir().expect("a dir");
        let missing = dir.path().join("herdr.sock");
        assert!(
            connect_and_write(&missing, b"{}\n").is_none(),
            "nothing is listening"
        );

        let said = Herdr::new(&missing)
            .call::<Value>("session.snapshot", json!({}))
            .await
            .expect_err("a socket that is not there is not a success")
            .to_string();
        assert!(said.contains("connecting to herdr socket"), "{said}");
        assert!(said.contains(&missing.display().to_string()), "{said}");

        let too_long = PathBuf::from("/tmp").join("x".repeat(200));
        assert!(
            sockaddr_un(&too_long).is_none(),
            "sun_path holds 108 bytes and there is no error for overrunning it: \
             a path copied in short dials whatever the truncation names"
        );
        assert!(connect_and_write(&too_long, b"{}\n").is_none());
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
