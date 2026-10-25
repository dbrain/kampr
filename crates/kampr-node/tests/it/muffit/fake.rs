//! A muffit daemon that speaks the frames of `../muffit/docs/api/kampr.md` (wire 3.3) and records
//! everything it is asked, so a test can say what the node sent it.

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::sync::broadcast;

pub const TOKEN: &str = "fake-muffit-token";

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
}

#[derive(Clone)]
pub struct Fake {
    inner: Arc<Mutex<Inner>>,
    stream: broadcast::Sender<Value>,
    attach: broadcast::Sender<Value>,
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
        let inner = Arc::new(Mutex::new(Inner {
            sessions,
            ..Inner::default()
        }));
        let fake = Self {
            inner,
            stream,
            attach,
            url,
        };
        let app = Router::new()
            .route("/sessions", get(list).post(create))
            .route("/sessions/stream", get(stream_socket))
            .route("/sessions/{id}/attach", get(attach_socket))
            .route("/sessions/{id}/transcript", get(transcript))
            .route("/sessions/{id}/permission", post(permission))
            .route("/sessions/{id}/{verb}", post(control))
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

    pub fn status(&self, permission: Value, subagents: Value) {
        self.push(json!({ "type": "status", "cursor": 2, "snapshot": {
            "session_id": "s1", "state": "running", "title": "install ripgrep",
            "permission": permission, "subagents": subagents } }));
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

async fn permission(
    State(fake): State<Fake>,
    Path(id): Path<String>,
    axum::Json(body): axum::Json<Value>,
) -> Response {
    let mut inner = fake.inner.lock().unwrap();
    let Some(row) = inner.sessions.iter_mut().find(|s| s["id"] == id) else {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(json!({ "error": format!("unknown session: {id}") })),
        )
            .into_response();
    };
    if row["permission"]["id"] != body["id"] {
        return (
            StatusCode::CONFLICT,
            axum::Json(json!({ "error": "that request is no longer waiting" })),
        )
            .into_response();
    }
    row["permission"] = Value::Null;
    axum::Json(json!({ "id": body["id"] })).into_response()
}

async fn control(State(fake): State<Fake>, Path((id, verb)): Path<(String, String)>) -> Response {
    if verb == "stop" {
        let row = {
            let mut inner = fake.inner.lock().unwrap();
            let row = inner.sessions.iter_mut().find(|s| s["id"] == id);
            row.map(|row| {
                row["state"] = json!("stopped");
                row.clone()
            })
        };
        if let Some(row) = row {
            let _ = fake
                .stream
                .send(json!({ "type": "session", "session": shown(&row) }));
        }
    }
    axum::Json(json!({ "id": id })).into_response()
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
        let greeting = json!({ "type": "hello", "version": 303, "cursor": 1, "session": row,
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
                    fake.inner.lock().unwrap().attach_frames.push(serde_json::from_str(&text).unwrap_or_default());
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
