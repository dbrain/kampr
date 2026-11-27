//! A muffit daemon that speaks the frames of `../muffit/docs/api/kampr.md` (wire 3.3 to 3.9, the
//! version it greets with set by the test) and records everything it is asked, so a test can say
//! what the node sent it.

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

mod rest;

pub const TOKEN: &str = "fake-muffit-token";
const HANG_UP: &str = "_hang_up";

#[derive(Debug, Clone)]
pub struct Seen {
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub origin: Option<String>,
    pub authorization: Option<String>,
    pub body: Value,
}

#[derive(Default)]
struct Inner {
    sessions: Vec<Value>,
    transcripts: HashMap<String, Vec<Value>>,
    seen: Vec<Seen>,
    attach_frames: Vec<Value>,
    created: u32,
    version: u32,
    children: Vec<String>,
    hung_up: bool,
    watchers_refused: bool,
}

#[derive(Clone)]
pub struct Fake {
    inner: Arc<Mutex<Inner>>,
    stream: broadcast::Sender<Value>,
    attach: broadcast::Sender<Value>,
    child: broadcast::Sender<Value>,
    pub url: String,
}

pub struct Daemon {
    pub fake: Fake,
    task: tokio::task::JoinHandle<()>,
}

impl std::ops::Deref for Daemon {
    type Target = Fake;
    fn deref(&self) -> &Fake {
        &self.fake
    }
}

impl Drop for Daemon {
    fn drop(&mut self) {
        self.task.abort();
    }
}

pub fn session(id: &str, state: &str, title: Option<&str>) -> Value {
    json!({ "branch": "main", "cwd": "/srv/work", "id": id, "model": "mock",
            "permission": null, "state": state, "title": title, "turns": 1 })
}

/// A finished conversation: in muffit's store, but not listed by `/sessions` or the stream.
pub fn finished(id: &str, title: &str) -> Value {
    let mut row = session(id, "completed", Some(title));
    row["_live"] = json!(false);
    row
}

fn is_live(row: &Value) -> bool {
    row["_live"] != json!(false)
}

/// A row as `/sessions` and the stream carry it: the fake's own bookkeeping left off.
fn shown(row: &Value) -> Value {
    let mut row = row.clone();
    if let Some(row) = row.as_object_mut() {
        row.retain(|k, _| !k.starts_with('_'));
    }
    row
}

pub fn text(seq: u64, role: &str, text: &str) -> Value {
    json!({ "seq": seq, "role": role, "blocks": [{ "type": "text", "text": text }] })
}

impl Fake {
    pub async fn start(sessions: Vec<Value>) -> Daemon {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}", listener.local_addr().unwrap());
        let (stream, _) = broadcast::channel(64);
        let (attach, _) = broadcast::channel(64);
        let (child, _) = broadcast::channel(64);
        let inner = Arc::new(Mutex::new(Inner {
            sessions,
            version: 309,
            ..Inner::default()
        }));
        let fake = Self {
            inner,
            stream,
            attach,
            child,
            url,
        };
        let app = Router::new()
            .route("/sessions", get(list).post(create))
            .route("/sessions/stream", get(stream_socket))
            .route("/sessions/{id}/attach", get(attach_socket))
            .route("/sessions/{id}/transcript", get(transcript))
            .merge(rest::routes())
            .layer(middleware::from_fn_with_state(fake.clone(), guard))
            .with_state(fake.clone());
        let task = tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        Daemon { fake, task }
    }

    pub fn seen(&self) -> Vec<Seen> {
        self.inner.lock().unwrap().seen.clone()
    }

    pub fn posts(&self, path: &str) -> Vec<Value> {
        self.seen()
            .into_iter()
            .filter(|s| s.method == "POST" && s.path == path)
            .map(|s| s.body)
            .collect()
    }

    pub fn attach_frames(&self, kind: &str) -> Vec<Value> {
        self.inner
            .lock()
            .unwrap()
            .attach_frames
            .iter()
            .filter(|f| f["type"] == kind)
            .cloned()
            .collect()
    }

    pub fn set_transcript(&self, id: &str, entries: Vec<Value>) {
        self.inner
            .lock()
            .unwrap()
            .transcripts
            .insert(id.to_string(), entries);
    }

    pub fn append(&self, id: &str, entries: Vec<Value>) {
        self.inner
            .lock()
            .unwrap()
            .transcripts
            .entry(id.to_string())
            .or_default()
            .extend(entries);
    }

    /// Replaces the session's row and tells the session stream, the way muffit does on a change.
    pub fn update(&self, id: &str, change: impl FnOnce(&mut Value)) {
        let row = {
            let mut inner = self.inner.lock().unwrap();
            let row = inner
                .sessions
                .iter_mut()
                .find(|s| s["id"] == id)
                .expect("a session");
            change(row);
            row.clone()
        };
        if is_live(&row) {
            let _ = self
                .stream
                .send(json!({ "type": "session", "session": shown(&row) }));
        }
    }

    fn live_rows(&self) -> Vec<Value> {
        live_rows(&self.inner.lock().unwrap().sessions)
    }

    /// Somebody else attached, or the last client left: the stream says so the way muffit does,
    /// a `session` frame for one that appears and a fresh list for one that goes.
    pub fn set_live(&self, id: &str, live: bool) {
        {
            let mut inner = self.inner.lock().unwrap();
            let row = inner
                .sessions
                .iter_mut()
                .find(|s| s["id"] == id)
                .expect("a session");
            row["_live"] = json!(live);
        }
        match live {
            true => self.update(id, |_| {}),
            false => {
                let _ = self
                    .stream
                    .send(json!({ "type": "sessions", "sessions": self.live_rows() }));
            }
        }
    }

    /// A session that appears, the way muffit's stream tells of one.
    pub fn insert(&self, row: Value) {
        let id = row["id"].as_str().expect("an id").to_string();
        self.inner.lock().unwrap().sessions.push(row);
        self.update(&id, |_| {});
    }

    pub fn push(&self, frame: Value) {
        let _ = self.attach.send(frame);
    }

    /// The wire version the daemon greets with, as `WIRE_VERSION` in muffit's `wire.rs`.
    pub fn speak(&self, version: u32) {
        self.inner.lock().unwrap().version = version;
    }

    /// A frame on every worker's observe socket (kampr.md §6, 3.5).
    pub fn push_child(&self, frame: Value) {
        let _ = self.child.send(frame);
    }

    /// The workers somebody has opened an observe socket on.
    pub fn children(&self) -> Vec<String> {
        self.inner.lock().unwrap().children.clone()
    }

    pub fn status(&self, permission: Value, subagents: Value) {
        self.snapshot(json!({
            "session_id": "s1", "state": "running", "title": "install ripgrep",
            "permission": permission, "subagents": subagents,
            "model": "mock", "thinking": "low", "plan": true, "permissions": "auto",
            "cwd": "/srv/work", "branch": "main",
            "context": { "measured_prompt_tokens": 1200 },
            "prompt_tokens": 4321, "completion_tokens": 210
        }));
    }

    pub fn snapshot(&self, snapshot: Value) {
        self.push(json!({ "type": "status", "cursor": 2, "snapshot": snapshot }));
    }

    /// Every attach socket closes and no new one is let in, so the node is left with REST alone.
    pub fn hang_up(&self) {
        self.inner.lock().unwrap().hung_up = true;
        let _ = self.attach.send(json!({ "type": HANG_UP }));
    }

    /// Every attach socket closes and only a watcher is turned away, as muffit treats a stored session:
    /// attaching without `observe` still restores it.
    pub fn hang_up_on_watchers(&self) {
        self.inner.lock().unwrap().watchers_refused = true;
        let _ = self.attach.send(json!({ "type": HANG_UP }));
    }

    pub fn attached(&self) -> usize {
        self.attach.receiver_count()
    }
}

async fn guard(State(fake): State<Fake>, request: Request, next: Next) -> Response {
    let (origin, authorization) = {
        let header = |name: &str| {
            request
                .headers()
                .get(name)
                .and_then(|v| v.to_str().ok())
                .map(str::to_string)
        };
        (header("origin"), header("authorization"))
    };
    let method = request.method().to_string();
    let path = request.uri().path().to_string();
    let query = request.uri().query().map(str::to_string);
    let (parts, body) = request.into_parts();
    let bytes = axum::body::to_bytes(body, 1 << 20).await.unwrap_or_default();
    fake.inner.lock().unwrap().seen.push(Seen {
        method,
        path,
        query,
        origin: origin.clone(),
        authorization: authorization.clone(),
        body: serde_json::from_slice(&bytes).unwrap_or(Value::Null),
    });
    if origin.is_some() {
        return (
            StatusCode::FORBIDDEN,
            axum::Json(json!({ "error": "browser requests are refused (Origin header present)" })),
        )
            .into_response();
    }
    if authorization.as_deref() != Some(&format!("Bearer {TOKEN}")) {
        return (
            StatusCode::UNAUTHORIZED,
            axum::Json(json!({ "error": "missing Authorization: Bearer <token>" })),
        )
            .into_response();
    }
    next.run(Request::from_parts(parts, axum::body::Body::from(bytes)))
        .await
}

fn live_rows(rows: &[Value]) -> Vec<Value> {
    rows.iter().filter(|r| is_live(r)).map(shown).collect()
}

async fn list(State(fake): State<Fake>) -> axum::Json<Value> {
    let rows = fake.inner.lock().unwrap().sessions.clone();
    axum::Json(json!({ "remote": [], "sessions": live_rows(&rows) }))
}

async fn create(State(fake): State<Fake>, axum::Json(body): axum::Json<Value>) -> Response {
    let row = {
        let mut inner = fake.inner.lock().unwrap();
        inner.created += 1;
        let id = format!("new{}", inner.created);
        let mut row = session(&id, "running", body["prompt"].as_str());
        row["cwd"] = body["cwd"].clone();
        inner.sessions.push(row.clone());
        row
    };
    let _ = fake.stream.send(json!({ "type": "session", "session": row }));
    (StatusCode::CREATED, axum::Json(json!({ "id": row["id"] }))).into_response()
}

async fn transcript(
    State(fake): State<Fake>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
) -> Response {
    let Some(entries) = fake.inner.lock().unwrap().transcripts.get(&id).cloned() else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(json!({ "error": "no such session" })),
        )
            .into_response();
    };
    let number = |k: &str| query.get(k).and_then(|v| v.parse::<u64>().ok());
    let limit = number("limit").unwrap_or(100) as usize;
    let seq = |e: &Value| e["seq"].as_u64().unwrap();
    let (page, watermark): (Vec<Value>, bool) = match (number("before_seq"), number("after_seq")) {
        (Some(before), _) => {
            let older: Vec<Value> = entries.iter().filter(|e| seq(e) < before).cloned().collect();
            (older[older.len().saturating_sub(limit)..].to_vec(), false)
        }
        (None, Some(after)) => (
            entries
                .iter()
                .filter(|e| seq(e) > after)
                .take(limit)
                .cloned()
                .collect(),
            true,
        ),
        (None, None) => (entries[entries.len().saturating_sub(limit)..].to_vec(), true),
    };
    let has_older = page
        .first()
        .is_some_and(|first| entries.iter().any(|e| seq(e) < seq(first)));
    let watermark = watermark.then(|| page.last().map(seq)).flatten();
    axum::Json(json!({ "id": id, "state": "waiting_user", "transcript": page,
                       "watermark": watermark, "has_older": has_older }))
    .into_response()
}

/// The wire version each control verb arrived in, from `handle_control` in muffit's `ws.rs`;
/// `None` is a verb it answers `unknown verb` to (`pause`, `resume` and `stop` are REST only).
fn verb_since(verb: &str) -> Option<u32> {
    match verb {
        "models.list" | "model.swap" | "model.add" | "thinking.set" | "mode.set" | "permissions.set"
        | "btw" | "context" | "context.compact" | "cost" | "revert" | "rewind" | "changesets"
        | "permission.answer" => Some(300),
        v if v.starts_with("mcp.") => Some(300),
        "question.answer" | "question.withdraw" => Some(304),
        "job.stop" => Some(306),
        "rename" | "clear" | "queue.unqueue" | "fork" => Some(307),
        "commands.list" => Some(308),
        _ => None,
    }
}

/// `crates/muffit-server/src/commands.rs` at wire 3.9.
const CATALOG: &[(&str, &str)] = &[
    ("models.list", "list the configured models"),
    ("model.swap", "switch the session's active model"),
    ("model.add", "add a provider's model to the user config"),
    ("thinking.set", "set the thinking level (off/low/medium/high)"),
    ("mode.set", "set the permission mode (toggle/plan/normal)"),
    ("permissions.set", "set the permission mode (ask/auto/yolo/…)"),
    ("btw", "an aside answer that stays out of the history"),
    ("context", "show, or switch, the context strategy"),
    ("context.compact", "compact the conversation"),
    ("cost", "the session's token usage and cost"),
    ("revert", "revert the working tree to a checkpoint"),
    ("rewind", "rewind the conversation to a seq or changeset"),
    ("changesets", "list the changesets (the turns that changed files)"),
    ("job.stop", "stop a background shell task"),
    ("rename", "rename the session"),
    ("clear", "clear the pane's view (a display action)"),
    ("queue.unqueue", "drop a sent-but-unconsumed input by msg_id"),
    ("fork", "fork the conversation into a new session"),
    (
        "permission.answer",
        "answer a permission prompt (allow/always/deny)",
    ),
    ("question.answer", "answer the model's ask question"),
    ("question.withdraw", "withdraw the model's ask question"),
    ("mcp.status", "list the MCP servers"),
    ("mcp.add", "add an MCP server"),
    ("mcp.approve", "approve a pending MCP server"),
    ("mcp.disable", "disable an MCP server"),
    ("mcp.enable", "enable an MCP server"),
    ("mcp.remove", "remove an MCP server"),
];

fn command_result(id: &str, verb: &str, version: u32) -> Value {
    if verb_since(verb).is_none_or(|since| version < since) {
        return json!({ "type": "command_result", "verb": verb, "ok": false,
                       "payload": format!("unknown verb: {verb}") });
    }
    let payload = match verb {
        "models.list" => json!({ "providers": [
            { "name": "mock", "kind": "anthropic", "model": "mock" },
            { "name": "mock2", "kind": "anthropic", "model": "mock-2" }
        ] }),
        "commands.list" => json!({ "commands": CATALOG.iter()
            .map(|(verb, summary)| json!({ "verb": verb, "summary": summary }))
            .collect::<Vec<_>>() }),
        _ => json!({ "id": id }),
    };
    json!({ "type": "command_result", "verb": verb, "ok": true, "payload": payload })
}

async fn stream_socket(State(fake): State<Fake>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |mut socket| async move {
        let mut frames = fake.stream.subscribe();
        if !said_hello(&mut socket).await {
            return;
        }
        let sessions = fake.live_rows();
        if !send(&mut socket, json!({ "type": "sessions", "sessions": sessions })).await {
            return;
        }
        while let Ok(frame) = frames.recv().await {
            if !send(&mut socket, frame).await {
                return;
            }
        }
    })
}

async fn attach_socket(
    State(fake): State<Fake>,
    Path(id): Path<String>,
    Query(query): Query<HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    let observe = query.get("observe").is_some_and(|v| v == "true");
    if let Some((parent, _)) = id.split_once(".s") {
        return child_socket(fake.clone(), parent.to_string(), id.clone(), ws);
    }
    let row = fake
        .inner
        .lock()
        .unwrap()
        .sessions
        .iter()
        .find(|s| s["id"] == id)
        .cloned();
    let Some(row) = row else {
        return (StatusCode::NOT_FOUND, "no such session").into_response();
    };
    let refused = {
        let inner = fake.inner.lock().unwrap();
        inner.hung_up || (observe && inner.watchers_refused)
    };
    if refused {
        return (StatusCode::NOT_FOUND, "no such live session").into_response();
    }
    match (is_live(&row), observe) {
        (true, _) => {}
        (false, true) => return (StatusCode::NOT_FOUND, "no such live session").into_response(),
        (false, false) => fake.set_live(&id, true),
    }
    let row = shown(&row);
    ws.on_upgrade(move |mut socket| async move {
        let mut frames = fake.attach.subscribe();
        let Some(Ok(Message::Text(hello))) = socket.recv().await else {
            return;
        };
        fake.inner
            .lock()
            .unwrap()
            .attach_frames
            .push(serde_json::from_str(&hello).unwrap_or_default());
        let version = fake.inner.lock().unwrap().version;
        let greeting = json!({ "type": "hello", "version": version, "cursor": 1, "session": row,
                               "capabilities": ["input", "interrupt", "history", "status", "subscribe"] });
        if !send(&mut socket, greeting).await {
            return;
        }
        loop {
            tokio::select! {
                incoming = socket.recv() => {
                    let Some(Ok(Message::Text(text))) = incoming else {
                        return;
                    };
                    let frame: Value = serde_json::from_str(&text).unwrap_or_default();
                    fake.inner.lock().unwrap().attach_frames.push(frame.clone());
                    if frame["type"] == "control" {
                        let verb = frame["verb"].as_str().unwrap_or_default();
                        if !send(&mut socket, command_result(&id, verb, version)).await {
                            return;
                        }
                    }
                }
                frame = frames.recv() => {
                    let Ok(frame) = frame else {
                        return;
                    };
                    if frame["type"] == HANG_UP || !send(&mut socket, frame).await {
                        return;
                    }
                }
            }
        }
    })
    .into_response()
}

/// A worker's observe socket (kampr.md §6): 404 unless its ancestor is live on a 3.5 daemon, then
/// the child's greeting and whatever the test pushes to children.
fn child_socket(fake: Fake, parent: String, id: String, ws: WebSocketUpgrade) -> Response {
    let (live, version) = {
        let inner = fake.inner.lock().unwrap();
        let live = inner.sessions.iter().any(|s| s["id"] == parent && is_live(s));
        (live, inner.version)
    };
    if !live || version < 305 {
        return (StatusCode::NOT_FOUND, "no such child session").into_response();
    }
    fake.inner.lock().unwrap().children.push(id.clone());
    ws.on_upgrade(move |mut socket| async move {
        let mut frames = fake.child.subscribe();
        if !said_hello(&mut socket).await {
            return;
        }
        let greeting = json!({ "type": "hello", "version": version, "cursor": 1,
                               "session": { "id": id, "state": "running", "turns": 0 } });
        if !send(&mut socket, greeting).await {
            return;
        }
        loop {
            tokio::select! {
                incoming = socket.recv() => {
                    if !matches!(incoming, Some(Ok(Message::Text(_)))) {
                        return;
                    }
                }
                frame = frames.recv() => {
                    let Ok(frame) = frame else {
                        return;
                    };
                    if !send(&mut socket, frame).await {
                        return;
                    }
                }
            }
        }
    })
    .into_response()
}

async fn said_hello(socket: &mut WebSocket) -> bool {
    matches!(socket.recv().await, Some(Ok(Message::Text(text)))
        if serde_json::from_str::<Value>(&text).is_ok_and(|v| v["type"] == "hello"))
}

async fn send(socket: &mut WebSocket, frame: Value) -> bool {
    socket.send(Message::Text(frame.to_string().into())).await.is_ok()
}
