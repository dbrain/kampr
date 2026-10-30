//! `POST /api/upload/{node}/{local}?dir=&name=` — a file from the device, streamed into a
//! directory on this node or relayed to the peer that owns `{node}`.

use std::sync::Arc;

use axum::Json;
use axum::body::Body;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use futures_util::StreamExt;
use kampr_mesh::FetchError;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::http::{Authenticated, refuse};
use crate::state::Node;
use crate::upload::{Staged, UploadError, Written, audit, idle, valid_name};

#[derive(Deserialize)]
pub struct Target {
    dir: Option<String>,
    name: Option<String>,
}

/// The upload's own refusal codes, which are also what a peer says in `up.error`.
fn status(code: &str) -> Response {
    match code {
        "read_only" => refuse(StatusCode::FORBIDDEN, "this device is read-only"),
        "bad_name" => refuse(StatusCode::BAD_REQUEST, "that is not a file name"),
        "busy" => refuse(StatusCode::SERVICE_UNAVAILABLE, "that node is busy"),
        IDLE => refuse(StatusCode::REQUEST_TIMEOUT, "the upload stopped moving"),
        UNFINISHED => refuse(StatusCode::BAD_REQUEST, "the upload did not finish"),
        FAILED => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the upload could not be written",
        ),
        _ => refuse(StatusCode::NOT_FOUND, "no such directory"),
    }
}

const UNFINISHED: &str = "unfinished";
const IDLE: &str = "idle";
const FAILED: &str = "failed";

fn created(path: &str, bytes: u64) -> Response {
    (StatusCode::CREATED, Json(json!({ "path": path, "bytes": bytes }))).into_response()
}

/// Gated as typing, before anything else is looked at: a device that may type into a terminal can
/// already write a file there. `{local}` is ignored, as it is for every path form, so `_` works.
pub async fn upload(
    State(node): State<Arc<Node>>,
    auth: Authenticated,
    Path(locator): Path<String>,
    Query(target): Query<Target>,
    body: Body,
) -> Response {
    if let Some(response) = auth.refused(&node, "api.upload") {
        return response;
    }
    let Some((node_id, _)) = locator
        .split_once('/')
        .filter(|(id, local)| !id.is_empty() && !local.is_empty())
    else {
        return status("not_found");
    };
    let name = target.name.unwrap_or_default();
    if !valid_name(&name) {
        return status("bad_name");
    }
    let Some(dir) = target.dir.filter(|d| !d.is_empty()) else {
        return status("not_found");
    };
    let written = match node.route(node_id) {
        Some(_) => local(&node, &dir, &name, body).await,
        None => relayed(&node, node_id, &dir, &name, body).await,
    };
    match written {
        Ok(written) => {
            audit(&node, &auth.device, &auth.peer, &written);
            created(&written.path.display().to_string(), written.bytes)
        }
        Err(code) => status(&code),
    }
}

async fn local(node: &Node, dir: &str, name: &str, body: Body) -> Result<Written, String> {
    let mut staged = Staged::open(node, dir, name)
        .await
        .map_err(|e: UploadError| e.code().to_string())?;
    let mut body = body.into_data_stream();
    while let Some(chunk) = next(node, &mut body).await? {
        staged.write(&chunk).await.map_err(|e| failed(&e))?;
    }
    staged.commit().await.map_err(|e| failed(&e))
}

async fn next(
    node: &Node,
    body: &mut axum::body::BodyDataStream,
) -> Result<Option<axum::body::Bytes>, String> {
    match tokio::time::timeout(idle(node), body.next()).await {
        Ok(Some(Ok(chunk))) => Ok(Some(chunk)),
        Ok(Some(Err(_))) => Err(UNFINISHED.to_string()),
        Ok(None) => Ok(None),
        Err(_) => Err(IDLE.to_string()),
    }
}

fn failed(e: &dyn std::fmt::Display) -> String {
    tracing::debug!(error = %e, "an upload could not be written");
    FAILED.to_string()
}

/// Streams the body to the peer a window at a time, so the hub holds a chunk and never the file.
/// A client that walks away drops the upload, which tells the peer to throw away what it has.
async fn relayed(node: &Node, node_id: &str, dir: &str, name: &str, body: Body) -> Result<Written, String> {
    let refused = |e: FetchError| match e {
        FetchError::Refused(code) => code,
        e => {
            tracing::debug!(node = node_id, error = %e, "an upload to a peer failed");
            UploadError::Missing.code().to_string()
        }
    };
    let mut upload = node.peers.upload(node_id, dir, name).await.map_err(refused)?;
    let mut body = body.into_data_stream();
    while let Some(chunk) = next(node, &mut body).await? {
        upload.send(&chunk).await.map_err(refused)?;
    }
    let written: Value = upload.finish().await.map_err(refused)?;
    Ok(Written {
        path: written["path"].as_str().unwrap_or_default().into(),
        bytes: written["bytes"].as_u64().unwrap_or_default(),
    })
}
