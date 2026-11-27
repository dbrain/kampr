//! The REST routes of `muffit-server/src/lib.rs` and `prompts_rest.rs` that this node calls, with
//! the daemon's request shapes copied, so a route or a body muffit refuses is refused here too.

use super::{Fake, shown};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use serde::Deserialize;
use serde_json::{Value, json};

pub(super) fn routes() -> Router<Fake> {
    Router::new()
        .route("/sessions/{id}/title", post(title))
        .route("/sessions/{id}/stop", post(stop))
        .route("/sessions/{id}/pause", post(acknowledged))
        .route("/sessions/{id}/resume", post(acknowledged))
        .route("/sessions/{id}/interrupt", post(acknowledged))
        .route("/sessions/{id}/input", post(input))
        .route("/sessions/{id}/fork", post(fork))
        .route("/sessions/{id}/permission", post(permission))
        .route("/sessions/{id}/question", post(question))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InputRequest {
    #[allow(dead_code)]
    text: String,
    #[serde(default)]
    #[allow(dead_code)]
    images: Vec<Value>,
    #[serde(default)]
    #[allow(dead_code)]
    msg_id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ForkRequest {
    from_seq: Option<u64>,
    changeset: Option<usize>,
    #[allow(dead_code)]
    id: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct AnswerRequest {
    id: u64,
    answer: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct QuestionAnswerRequest {
    id: u64,
    answer: Value,
}

fn refused(status: StatusCode, error: impl Into<String>) -> Response {
    (status, Json(json!({ "error": error.into() }))).into_response()
}

async fn acknowledged(Path(id): Path<String>) -> Response {
    Json(json!({ "id": id })).into_response()
}

async fn input(Path(id): Path<String>, Json(_): Json<InputRequest>) -> Response {
    Json(json!({ "id": id })).into_response()
}

async fn title(State(fake): State<Fake>, Path(id): Path<String>, Json(body): Json<Value>) -> Response {
    let title = body["title"]
        .as_str()
        .map(str::trim)
        .unwrap_or_default()
        .to_string();
    if title.is_empty() {
        return refused(StatusCode::BAD_REQUEST, "a title must not be empty");
    }
    if !fake.inner.lock().unwrap().sessions.iter().any(|s| s["id"] == id) {
        return refused(StatusCode::NOT_FOUND, format!("unknown session: {id}"));
    }
    fake.update(&id, |row| row["title"] = json!(title));
    Json(json!({ "id": id })).into_response()
}

async fn fork(Path(id): Path<String>, Json(request): Json<ForkRequest>) -> Response {
    if request.from_seq.is_some() == request.changeset.is_some() {
        return refused(StatusCode::BAD_REQUEST, "give one of from_seq or changeset");
    }
    (
        StatusCode::CREATED,
        Json(json!({ "id": format!("{id}-fork"), "parent": id })),
    )
        .into_response()
}

async fn stop(State(fake): State<Fake>, Path(id): Path<String>) -> Response {
    let row = {
        let mut inner = fake.inner.lock().unwrap();
        inner.sessions.iter_mut().find(|s| s["id"] == id).map(|row| {
            row["state"] = json!("stopped");
            row.clone()
        })
    };
    if let Some(row) = row {
        let _ = fake
            .stream
            .send(json!({ "type": "session", "session": shown(&row) }));
    }
    Json(json!({ "id": id })).into_response()
}

/// Settles the prompt under `field` when its id is the one answered, as `p.answer` does.
fn settle(fake: &Fake, id: &str, field: &str, answered: u64) -> Response {
    let mut inner = fake.inner.lock().unwrap();
    let Some(row) = inner.sessions.iter_mut().find(|s| s["id"] == id) else {
        return refused(StatusCode::NOT_FOUND, format!("unknown session: {id}"));
    };
    if row[field]["id"] != answered {
        return refused(StatusCode::CONFLICT, "that request is no longer waiting");
    }
    row[field] = Value::Null;
    Json(json!({ "id": answered })).into_response()
}

async fn permission(
    State(fake): State<Fake>,
    Path(id): Path<String>,
    Json(request): Json<AnswerRequest>,
) -> Response {
    if !matches!(request.answer.as_str(), "allow" | "always" | "deny") {
        return refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "answer must be allow, always or deny",
        );
    }
    settle(&fake, &id, "permission", request.id)
}

async fn question(
    State(fake): State<Fake>,
    Path(id): Path<String>,
    Json(request): Json<QuestionAnswerRequest>,
) -> Response {
    if !(request.answer.is_u64() || request.answer.is_string()) {
        return refused(
            StatusCode::UNPROCESSABLE_ENTITY,
            "answer must be an option index (number) or free text (string)",
        );
    }
    settle(&fake, &id, "question", request.id)
}
