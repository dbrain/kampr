use std::path::Path;

use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use kampr_journal::{Fetched, JournalError, Registry as Journals};

use crate::http::refuse;

/// What a transcript's own `media_type` is allowed to become on a response from this origin.
///
/// **The recorded type is attacker-influenced**: it is a string in a file some agent wrote, and a
/// node that echoed it would serve `text/html` from its own origin, past a CSP written for the
/// bundle. Anything not on this list is bytes to download, never a document to render — and SVG
/// is deliberately absent, because it is a scriptable document wearing an image's name.
const RENDERABLE: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/bmp",
    "image/x-icon",
];

const OPAQUE: &str = "application/octet-stream";

pub fn serve(journals: &Journals, transcript: &Path, id: &str) -> Response {
    match kampr_journal::attach::fetch(journals, id, transcript) {
        Ok(found) => body(found),
        Err(JournalError::TooLarge(bytes)) => {
            tracing::debug!(bytes, "refusing an attachment past the ceiling");
            refuse(
                StatusCode::PAYLOAD_TOO_LARGE,
                "this attachment is larger than the node will serve",
            )
        }
        // Every other refusal is one answer on purpose. An escape, a stale id and an id for
        // somebody else's transcript must not be distinguishable from outside.
        Err(e) => {
            tracing::debug!(error = %e, "refusing an attachment");
            refuse(StatusCode::NOT_FOUND, "no such attachment")
        }
    }
}

fn body(found: Fetched) -> Response {
    let rendered = rendered_as(&found);
    let content_type = rendered.unwrap_or(OPAQUE).to_string();
    let disposition = match rendered {
        Some(_) => "inline".to_string(),
        None => format!("attachment; filename=\"{}\"", filename(&found)),
    };
    (
        StatusCode::OK,
        [
            (CONTENT_TYPE, content_type),
            (CONTENT_LENGTH, found.data.len().to_string()),
            (CONTENT_DISPOSITION, disposition),
            (CACHE_CONTROL, "no-store".to_string()),
        ],
        found.data,
    )
        .into_response()
}

/// The type this response is served as, or `None` for bytes to download.
///
/// A recorded media type decides what the node *shows* and is never second-guessed: a record that
/// says `text/html` is a download whatever the bytes look like. **A record that says nothing is
/// the case worth sniffing** — a pasted screenshot may carry no media type at all, and the
/// `Content-Type` is what a client names the saved file from. Sniffing can only ever produce a
/// type off the list above, so it widens what is shown without widening what is trusted.
fn rendered_as(found: &Fetched) -> Option<&'static str> {
    if found.kind != kampr_journal::attach::IMAGE {
        return None;
    }
    match found.mime.as_deref() {
        Some(recorded) => RENDERABLE.iter().find(|m| **m == recorded).copied(),
        None => sniff(&found.data),
    }
}

fn sniff(data: &[u8]) -> Option<&'static str> {
    let at = |from: usize, magic: &[u8]| {
        data.len() >= from + magic.len() && &data[from..from + magic.len()] == magic
    };
    if at(0, b"\x89PNG\r\n\x1a\n") {
        return Some("image/png");
    }
    if at(0, b"\xff\xd8\xff") {
        return Some("image/jpeg");
    }
    if at(0, b"GIF87a") || at(0, b"GIF89a") {
        return Some("image/gif");
    }
    if at(0, b"RIFF") && at(8, b"WEBP") {
        return Some("image/webp");
    }
    if at(4, b"ftypavif") {
        return Some("image/avif");
    }
    if at(0, b"BM") {
        return Some("image/bmp");
    }
    None
}

/// A name out of a transcript reaches a header and then a filesystem, so it is reduced to
/// something that can be neither: no separators, no quotes, no control characters, and never
/// empty.
fn filename(found: &Fetched) -> String {
    let safe: String = found
        .name
        .as_deref()
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(80)
        .collect();
    let safe = safe.trim_matches('.');
    if !safe.is_empty() {
        return safe.to_string();
    }
    let extension = found
        .mime
        .as_deref()
        .and_then(|m| m.rsplit('/').next())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    format!("attachment.{extension}")
}
