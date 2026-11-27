//! Newline-delimited JSON-RPC 2.0 against a `codex app-server --stdio` child, with the
//! transport quirk from probe row 3 handled here: the server only flushes a response when it
//! next reads a line from stdin, so every call is immediately followed by a nudge request
//! (`account/read` — the cheapest real method) and a nudge goes out on a timer as well, so
//! notifications stream even while we are waiting on nothing.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use thiserror::Error;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};
use tokio::sync::{mpsc, oneshot};

/// A method we ask of the server; `params` may be `Value::Null`.
#[derive(Debug, Clone, PartialEq)]
pub struct Call {
    pub id: u64,
    pub method: String,
    pub params: Value,
}

/// A server→client request we must answer to keep the server from stalling.
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: u64,
    pub method: String,
    pub params: Value,
}

/// A server notification: no id, no reply expected.
#[derive(Debug, Clone, PartialEq)]
pub struct Notification {
    pub method: String,
    pub params: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ServerMsg {
    /// A reply to one of our calls.
    Response { id: u64, result: Value },
    /// A failed call; `message` is the server's own wording.
    Error { id: u64, code: i64, message: String },
    /// A notification we do not answer.
    Notification(Notification),
    /// A request the server makes of us; answer it with [`Pump::respond`].
    Request(Request),
}

#[derive(Error, Debug, Clone)]
pub enum WireError {
    #[error("the codex app-server exited")]
    Exited,
    #[error("the wire to the codex app-server is closed")]
    Closed,
    #[error("the codex app-server did not answer in time")]
    Timeout,
    #[error("the codex app-server rejected the request ({code}): {message}")]
    Rejected { code: i64, message: String },
    #[error("not valid JSON-RPC: {0}")]
    Parse(String),
    #[error("no stdout from the codex app-server")]
    NoStdout,
    #[error("no stdin for the codex app-server")]
    NoStdin,
    #[error("the codex app-server will not start: {0}")]
    Spawn(String),
}

/// The one line of the protocol: anything that crosses the pipe, in either direction.
///
/// `jsonrpc` is optional: the real server's notifications and responses carry no such field
/// (probe row 3's wire log), and JSON-RPC 2.0 only requires it for requests we send.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Wire {
    /// A request: we are asking, or the server is.
    Request {
        #[serde(default)]
        jsonrpc: Option<String>,
        id: u64,
        method: String,
        #[serde(default)]
        params: Option<Value>,
    },
    /// A response: the `result` arm.
    Response {
        #[serde(default)]
        jsonrpc: Option<String>,
        id: u64,
        result: Value,
    },
    /// A response: the `error` arm.
    Error {
        #[serde(default)]
        jsonrpc: Option<String>,
        id: u64,
        error: WireRpcError,
    },
    /// A notification: a method, no id.
    Notification {
        #[serde(default)]
        jsonrpc: Option<String>,
        method: String,
        #[serde(default)]
        params: Option<Value>,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WireRpcError {
    pub code: i64,
    pub message: String,
}

impl Wire {
    pub fn request(id: u64, method: impl Into<String>, params: Value) -> Self {
        Wire::Request {
            jsonrpc: Some("2.0".into()),
            id,
            method: method.into(),
            params: Some(params),
        }
    }

    pub fn notification(method: impl Into<String>, params: Value) -> Self {
        Wire::Notification {
            jsonrpc: Some("2.0".into()),
            method: method.into(),
            params: Some(params),
        }
    }

    pub fn response(id: u64, result: Value) -> Self {
        Wire::Response {
            jsonrpc: Some("2.0".into()),
            id,
            result,
        }
    }

    /// The one line we write for it.
    pub fn to_line(&self) -> String {
        serde_json::to_string(self).unwrap_or_default()
    }

    /// Read one line back into the protocol.
    pub fn parse(line: &str) -> Result<Self, WireError> {
        serde_json::from_str(line).map_err(|e| WireError::Parse(e.to_string()))
    }

    /// What a line means to us: a response settles a call, a request is the server's ask,
    /// a notification is news.
    pub fn as_server_msg(&self) -> Option<ServerMsg> {
        match self {
            Wire::Response { id, result, .. } => Some(ServerMsg::Response {
                id: *id,
                result: result.clone(),
            }),
            Wire::Error { id, error, .. } => Some(ServerMsg::Error {
                id: *id,
                code: error.code,
                message: error.message.clone(),
            }),
            Wire::Notification { method, params, .. } => Some(ServerMsg::Notification(Notification {
                method: method.clone(),
                params: params.clone().unwrap_or(Value::Null),
            })),
            Wire::Request {
                id, method, params, ..
            } => Some(ServerMsg::Request(Request {
                id: *id,
                method: method.clone(),
                params: params.clone().unwrap_or(Value::Null),
            })),
        }
    }
}

type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<Result<Value, WireError>>>>>;

enum Outbound {
    Line(String),
    Call {
        id: u64,
        line: String,
        reply: oneshot::Sender<Result<Value, WireError>>,
    },
}

/// One spawned `codex app-server --stdio`: its pipe, its calls, its incoming stream.
///
/// Every call is pipelined — the line goes out and a nudge immediately follows, because the
/// server will not flush the call's response until it reads the nudge.
pub struct Pump {
    child: Child,
    out: mpsc::Sender<Outbound>,
    notifs: Option<mpsc::Receiver<Notification>>,
    asks: Option<mpsc::Receiver<Request>>,
    exit: Option<oneshot::Receiver<()>>,
    next_id: Arc<AtomicU64>,
}

/// How long a call may take before we call it a loss.
const CALL_TIMEOUT: Duration = Duration::from_secs(30);
/// A nudge on the timer, so notifications keep flowing while no call is in flight.
const NUDGE_EVERY: Duration = Duration::from_millis(250);
/// The nudge method: a real, parameterless, side-effect-free request.
const NUDGE: &str = "account/read";

impl Pump {
    /// Spawn the server and take the pipe. `env` is the child's whole environment; nothing of
    /// the node's own crosses. The reader and writer run as tasks; when the child dies the
    /// reader closes [`Self::exit`] and settles every pending call.
    pub async fn spawn(program: &str, args: &[String], env: &[(String, String)]) -> Result<Self, WireError> {
        let mut child = Command::new(program)
            .args(args)
            .env_clear()
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| WireError::Spawn(e.to_string()))?;
        let stdin = child.stdin.take().ok_or(WireError::NoStdin)?;
        let stdout = child.stdout.take().ok_or(WireError::NoStdout)?;

        let (out, out_rx) = mpsc::channel::<Outbound>(256);
        let (notifs, notifs_rx) = mpsc::channel::<Notification>(1024);
        let (asks, asks_rx) = mpsc::channel::<Request>(64);
        let (exit, exit_rx) = oneshot::channel::<()>();
        let next_id = Arc::new(AtomicU64::new(1));

        let pending: Pending = Arc::new(Mutex::new(HashMap::new()));
        tokio::spawn(writer(stdin, out_rx, pending.clone()));
        tokio::spawn(reader(stdout, notifs, asks, exit, pending));

        // The timer nudge: notifications must reach us even between calls.
        let (nudger, ids) = (out.clone(), next_id.clone());
        tokio::spawn(async move {
            let mut every = tokio::time::interval(NUDGE_EVERY);
            every.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
            loop {
                every.tick().await;
                let id = ids.fetch_add(1, Ordering::Relaxed);
                if nudger.send(Outbound::Line(nudge_line(id))).await.is_err() {
                    break;
                }
            }
        });

        Ok(Self {
            child,
            out,
            notifs: Some(notifs_rx),
            asks: Some(asks_rx),
            exit: Some(exit_rx),
            next_id,
        })
    }

    fn fresh_id(&self) -> u64 {
        self.next_id.fetch_add(1, Ordering::Relaxed)
    }

    /// Ask the server something and take its reply. The nudge that follows is what makes the
    /// reply actually come back (probe row 3).
    pub async fn call(&self, method: impl Into<String>, params: Value) -> Result<Value, WireError> {
        let id = self.fresh_id();
        let line = Wire::request(id, method, params).to_line();
        let (reply, rx) = oneshot::channel();
        self.out
            .send(Outbound::Call { id, line, reply })
            .await
            .map_err(|_| WireError::Closed)?;
        self.nudge().await?;
        tokio::time::timeout(CALL_TIMEOUT, rx)
            .await
            .map_err(|_| WireError::Timeout)?
            .map_err(|_| WireError::Closed)?
    }

    /// The flush trigger: a cheap request whose reply we do not read.
    pub async fn nudge(&self) -> Result<(), WireError> {
        let id = self.fresh_id();
        self.send_line(Wire::request(id, NUDGE, json!({})).to_line())
            .await
    }

    /// Send a notification: no id, no reply.
    pub async fn notify(&self, method: impl Into<String>, params: Value) -> Result<(), WireError> {
        self.send_line(Wire::notification(method, params).to_line()).await
    }

    /// Answer a server→client request (the schema leaves the shape free-form).
    pub async fn respond(&self, id: u64, result: Value) -> Result<(), WireError> {
        self.send_line(Wire::response(id, result).to_line()).await
    }

    async fn send_line(&self, line: String) -> Result<(), WireError> {
        self.out
            .send(Outbound::Line(line))
            .await
            .map_err(|_| WireError::Closed)
    }

    /// `initialize` and the `initialized` that closes the handshake.
    pub async fn handshake(&self) -> Result<Value, WireError> {
        let result = self
            .call(
                "initialize",
                json!({
                    "clientInfo": { "name": "kampr-codex", "version": env!("CARGO_PKG_VERSION") },
                    "capabilities": {},
                }),
            )
            .await?;
        self.notify("initialized", Value::Null).await?;
        Ok(result)
    }

    /// Block until the next turn ends, draining the notifications it sends on the way.
    /// Used by [`crate::source::CodexSource::create`] to run a first prompt to completion
    /// before the pane opens the thread.
    pub async fn wait_turn_end(&mut self, limit: Duration) -> Result<(), WireError> {
        let mut notifs = self.take_notifs();
        let deadline = tokio::time::Instant::now() + limit;
        loop {
            let now = tokio::time::Instant::now();
            if now >= deadline {
                return Err(WireError::Timeout);
            }
            match tokio::time::timeout(deadline - now, notifs.recv()).await {
                Err(_) => return Err(WireError::Timeout),
                Ok(None) => return Err(WireError::Closed),
                Ok(Some(n)) if n.method == "turn/completed" => return Ok(()),
                Ok(Some(_)) => {}
            }
        }
    }

    /// The server's notifications, taken out so the run loop can own them.
    pub fn take_notifs(&mut self) -> mpsc::Receiver<Notification> {
        self.notifs.take().expect("notifs are taken once")
    }

    /// The server's asks of us, taken out so the run loop can own them.
    pub fn take_asks(&mut self) -> mpsc::Receiver<Request> {
        self.asks.take().expect("asks are taken once")
    }

    /// Taken out; fires when the server process is gone.
    pub fn take_exit(&mut self) -> oneshot::Receiver<()> {
        self.exit.take().expect("exit is taken once")
    }

    /// The child's pid, for the registry that must be able to reach the process.
    pub fn pid(&self) -> u32 {
        self.child.id().unwrap_or(0)
    }

    /// Kill the server process we own and reap it. (Dropping the pump without this — as
    /// `create` does — closes stdin, and the server exits on EOF: probe row 6.)
    pub async fn shutdown(&mut self) {
        let _ = self.child.start_kill();
        let _ = tokio::time::timeout(Duration::from_secs(2), self.child.wait()).await;
    }
}

fn nudge_line(id: u64) -> String {
    Wire::request(id, NUDGE, json!({})).to_line()
}

async fn writer(stdin: ChildStdin, mut out: mpsc::Receiver<Outbound>, pending: Pending) {
    let mut stdin = stdin;
    while let Some(outbound) = out.recv().await {
        let line = match outbound {
            Outbound::Line(line) => line,
            Outbound::Call { id, line, reply } => {
                pending.lock().unwrap().insert(id, reply);
                line
            }
        };
        let ok = stdin.write_all(line.as_bytes()).await.is_ok()
            && stdin.write_all(b"\n").await.is_ok()
            && stdin.flush().await.is_ok();
        if !ok {
            settle_all(pending, WireError::Closed);
            return;
        }
    }
    settle_all(pending, WireError::Closed);
}

async fn reader(
    stdout: ChildStdout,
    notifs: mpsc::Sender<Notification>,
    asks: mpsc::Sender<Request>,
    exit: oneshot::Sender<()>,
    pending: Pending,
) {
    let mut lines = tokio::io::BufReader::new(stdout).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let Some(wire) = Wire::parse(line).ok() else {
            continue;
        };
        let Some(msg) = wire.as_server_msg() else {
            continue;
        };
        match msg {
            ServerMsg::Response { id, result } => settle(&pending, id, Ok(result)),
            ServerMsg::Error { id, code, message } => {
                settle(&pending, id, Err(WireError::Rejected { code, message }))
            }
            ServerMsg::Notification(notification) => {
                if notifs.send(notification).await.is_err() {
                    break;
                }
            }
            ServerMsg::Request(request) => {
                if asks.send(request).await.is_err() {
                    break;
                }
            }
        }
    }
    settle_all(pending, WireError::Exited);
    let _ = exit.send(());
}

fn settle(pending: &Pending, id: u64, reply: Result<Value, WireError>) {
    if let Some(tx) = pending.lock().unwrap().remove(&id) {
        let _ = tx.send(reply);
    }
}

fn settle_all(pending: Pending, err: WireError) {
    for tx in pending.lock().unwrap().drain() {
        let _ = tx.1.send(Err(err.clone()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_request_round_trips_through_its_line() {
        let wire = Wire::request(7, "thread/start", json!({"cwd": "/tmp"}));
        let parsed = Wire::parse(&wire.to_line()).unwrap();
        assert_eq!(wire, parsed);
        let ServerMsg::Request(req) = wire.as_server_msg().unwrap() else {
            panic!("not a request")
        };
        assert_eq!(req.id, 7);
        assert_eq!(req.method, "thread/start");
        assert_eq!(req.params, json!({"cwd": "/tmp"}));
    }

    #[test]
    fn a_notification_has_no_id_and_no_reply() {
        let line = r#"{"jsonrpc":"2.0","method":"turn/completed","params":{"threadId":"t1"}}"#;
        let wire = Wire::parse(line).unwrap();
        let ServerMsg::Notification(n) = wire.as_server_msg().unwrap() else {
            panic!("not a notification")
        };
        assert_eq!(n.method, "turn/completed");
        assert_eq!(n.params, json!({"threadId": "t1"}));
    }

    #[test]
    fn a_result_is_a_response_and_a_fault_is_an_error() {
        let ok = Wire::parse(r#"{"jsonrpc":"2.0","id":3,"result":{"thread":{"id":"t1"}}}"#).unwrap();
        let ServerMsg::Response { id, result } = ok.as_server_msg().unwrap() else {
            panic!("not a response")
        };
        assert_eq!(id, 3);
        assert_eq!(result, json!({"thread": {"id": "t1"}}));

        let bad =
            Wire::parse(r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32600,"message":"no such thread"}}"#)
                .unwrap();
        let ServerMsg::Error { id, code, message } = bad.as_server_msg().unwrap() else {
            panic!("not an error")
        };
        assert_eq!(id, 4);
        assert_eq!(code, -32600);
        assert_eq!(message, "no such thread");
    }

    #[test]
    fn a_server_request_is_recognised() {
        let line = r#"{"jsonrpc":"2.0","id":99,"method":"item/commandExecutionApproval","params":{"command":["ls"]}}"#;
        let wire = Wire::parse(line).unwrap();
        let ServerMsg::Request(req) = wire.as_server_msg().unwrap() else {
            panic!("not a request")
        };
        assert_eq!(req.id, 99);
        assert_eq!(req.method, "item/commandExecutionApproval");
    }

    #[test]
    fn the_real_servers_lines_carry_no_jsonrpc() {
        // Measured shapes from the probe's wire log: notifications and responses with no
        // `jsonrpc` field at all.
        let n = Wire::parse(
            r#"{"method":"thread/status/changed","params":{"threadId":"t1","status":{"type":"idle"}},"emittedAtMs":1795729052914}"#,
        )
        .unwrap();
        assert!(matches!(n, Wire::Notification { method, .. } if method == "thread/status/changed"));

        let r = Wire::parse(r#"{"id":36,"result":{"account":null,"userId":null,"requiresOpenaiAuth":false,"workspaceRouting":null}}"#).unwrap();
        assert!(matches!(r, Wire::Response { id, .. } if id == 36));

        let e = Wire::parse(r#"{"error":{"code":-32600,"message":"invalid thread id"},"id":49}"#).unwrap();
        assert!(matches!(e, Wire::Error { id, .. } if id == 49));
    }

    #[test]
    fn a_response_line_with_neither_arm_is_not_a_server_message_for_us() {
        // a line that is a request from our side parses as a Wire::Request and still yields
        // a ServerMsg::Request; what must not parse is a bare object with no shape at all
        assert!(matches!(
            Wire::parse(r#"{"jsonrpc":"2.0"}"#),
            Err(WireError::Parse(_))
        ));
        assert!(Wire::parse("not json").is_err());
        assert!(Wire::parse("[]").is_err());
    }

    #[test]
    fn params_default_to_null_when_absent() {
        let wire = Wire::parse(r#"{"jsonrpc":"2.0","id":1,"method":"thread/list"}"#).unwrap();
        let ServerMsg::Request(req) = wire.as_server_msg().unwrap() else {
            panic!("not a request")
        };
        assert_eq!(req.params, Value::Null);
    }
}
