use std::path::Path;

use axum::body::Body;
use axum::http::StatusCode;
use axum::http::header::{CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE};
use axum::response::{IntoResponse, Response};
use futures_util::{StreamExt, TryStreamExt};
use kampr_journal::attach::MAX_BYTES;
use kampr_journal::{Fetched, FileRef, JournalError, Registry as Journals};
use kampr_mesh::{FetchError, Peers};

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

/// The one answer every refusal but the ceiling wears. An escape, a stale id and an id for
/// somebody else's transcript must not be distinguishable from outside, and a pane one hop away
/// over the mesh must not be either.
fn missing() -> Response {
    refuse(StatusCode::NOT_FOUND, "no such attachment")
}

fn past_the_ceiling() -> Response {
    refuse(
        StatusCode::PAYLOAD_TOO_LARGE,
        "this attachment is larger than the node will serve",
    )
}

pub fn serve(journals: &Journals, transcript: &Path, id: &str) -> Response {
    respond(kampr_journal::attach::fetch(journals, id, transcript))
}

/// The bytes at a plain path on this machine, for a caller the route has already established may
/// send input — a device that can type into a terminal can `cat` this file anyway.
///
/// `home` is what a leading `~/` resolves against, and it is the node's own
/// [`crate::Config::journal_home`] — the home the transcripts are under, which is the operator's
/// rather than the process's whenever a node runs as a service user.
///
/// **Every refusal is the same 404 the record form gives.** A relative path, a directory, a file
/// that is not there and one this user cannot read must not be distinguishable from outside, or
/// the route is a way to map the filesystem by response code.
pub fn serve_file(file: &FileRef, home: &Path) -> Response {
    respond(file.fetch(home))
}

/// What `git` says has changed in that file since HEAD, as `text/plain`.
///
/// Gated exactly as [`serve_file`] is and refused in exactly the same words, because it reads the
/// same paths: a device that may type into the terminal can already run `git diff` there, and one
/// that may not must not learn from a response code which of this machine's files are tracked.
pub fn serve_diff(file: &FileRef, home: &Path) -> Response {
    let Ok(path) = file.resolve(home) else {
        return missing();
    };
    match crate::git::diff_against_head(&path) {
        Ok(text) => (
            [
                (CONTENT_TYPE, "text/plain; charset=utf-8"),
                (CACHE_CONTROL, "private, no-store"),
            ],
            text,
        )
            .into_response(),
        Err(crate::git::DiffError::TooLarge) => past_the_ceiling(),
        Err(crate::git::DiffError::None) => missing(),
    }
}

/// The bytes of one of this node's own pastes, and nothing else.
///
/// **The containment check is the whole of the gate.** A `paste` id is answered for a read-only
/// device — looking at a screenshot somebody pasted into an agent session is reading — which is
/// only safe while the id cannot name anything but a file this node wrote out of a client's paste.
/// Both paths are canonicalised before they are compared, so a `..` in the id and a symlink out of
/// the directory are the same refusal as a path that was never in it. A file directly *in* the
/// directory: nothing writes a subdirectory there, so a nested path is a path nothing minted.
pub fn serve_paste(file: &FileRef, pastes: &Path) -> Response {
    let (Ok(dir), Ok(path)) = (std::fs::canonicalize(pastes), std::fs::canonicalize(&file.path)) else {
        return missing();
    };
    if path.parent() != Some(dir.as_path()) {
        return missing();
    }
    // The home is only what a leading `~/` resolves against and a path this node minted is
    // absolute, so there is nothing here for one to expand.
    respond(FileRef::new(path).fetch(Path::new("")))
}

fn respond(found: Result<Fetched, JournalError>) -> Response {
    match found {
        Ok(found) => body(found),
        Err(JournalError::TooLarge(bytes)) => {
            tracing::debug!(bytes, "refusing an attachment past the ceiling");
            past_the_ceiling()
        }
        Err(e) => {
            tracing::debug!(error = %e, "refusing an attachment");
            missing()
        }
    }
}

/// What a record claims to be, which is all the response headers are ever derived from — and
/// none of it is trusted: see [`rendered_as`].
struct Claim<'a> {
    kind: &'a str,
    mime: Option<&'a str>,
    name: Option<&'a str>,
    bytes: u64,
}

/// The four headers an attachment is served under, decided from a record's claims and the first
/// bytes of its body.
///
/// One implementation for both paths on purpose. A relayed attachment is described by the peer
/// and served from *this* origin, so the allowlist that decides what may render has to be applied
/// here rather than taken on trust — a peer that said `text/html` would otherwise get a document
/// out of the hub's origin, past a CSP written for the bundle.
fn headers(claim: &Claim<'_>, prefix: &[u8]) -> [(axum::http::HeaderName, String); 4] {
    let rendered = rendered_as(claim, prefix);
    let disposition = match rendered {
        Some(_) => "inline".to_string(),
        None => format!("attachment; filename=\"{}\"", filename(claim)),
    };
    [
        (CONTENT_TYPE, rendered.unwrap_or(OPAQUE).to_string()),
        (CONTENT_LENGTH, claim.bytes.to_string()),
        (CONTENT_DISPOSITION, disposition),
        (CACHE_CONTROL, "no-store".to_string()),
    ]
}

fn body(found: Fetched) -> Response {
    let claim = Claim {
        kind: &found.kind,
        mime: found.mime.as_deref(),
        name: found.name.as_deref(),
        bytes: found.data.len() as u64,
    };
    (StatusCode::OK, headers(&claim, &found.data), found.data).into_response()
}

/// The type this response is served as, or `None` for bytes to download.
///
/// A recorded media type decides what the node *shows* and is never second-guessed: a record that
/// says `text/html` is a download whatever the bytes look like. **A claim that says nothing is
/// the case worth sniffing** — a pasted screenshot may carry no media type at all, and a file on
/// disk has only its extension, which for anything but an image yields `file` and no type. The
/// `Content-Type` is what a client names the saved file from, and sniffing can only ever produce
/// a type off the list above, so it widens what is shown without widening what is trusted.
fn rendered_as(claim: &Claim<'_>, prefix: &[u8]) -> Option<&'static str> {
    use kampr_journal::attach::{FILE, IMAGE};
    if claim.kind != IMAGE && claim.kind != FILE {
        return None;
    }
    match claim.mime {
        Some(recorded) => RENDERABLE.iter().find(|m| **m == recorded).copied(),
        None => sniff(prefix),
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

/// The bytes behind an id on a pane this node does not own, pulled off the peer that does.
///
/// **The hub never holds the record.** It pulls a bounded window of chunks and hands each one to
/// the response body, so what it spends is the window whatever the attachment's size is, and the
/// rate it pulls at is the rate the client is reading at. A client that walks away drops the
/// transfer, and the peer is told to stop rather than left streaming into nothing.
///
/// **The ceiling is enforced on the peer's claim before a chunk is asked for**, so a record
/// claiming a gigabyte costs a comparison here exactly as it does locally — and again as the
/// bytes arrive, because the claim came off the network.
///
/// **Every refusal but the ceiling is the one 404.** Offline, unknown pane, a peer that never
/// answered and a stale id are the same sentence from outside; which one it was goes to the log.
pub async fn relay(peers: &Peers, pane: &str, id: &str) -> Response {
    let mut transfer = match peers.fetch_attachment(pane, id, MAX_BYTES).await {
        Ok(transfer) => transfer,
        Err(FetchError::TooLarge(bytes)) => {
            tracing::debug!(pane, bytes, "refusing a peer's attachment past the ceiling");
            return past_the_ceiling();
        }
        Err(e) => {
            tracing::debug!(pane, error = %e, "refusing a peer's attachment");
            return missing();
        }
    };
    // The headers are decided from the first chunk as well as from the peer's claims, because a
    // record with no recorded media type is sniffed — so the first chunk is held for exactly as
    // long as it takes to write a header, and never a second one.
    let first = match transfer.next_chunk().await {
        Some(Ok(chunk)) => chunk,
        Some(Err(e)) => {
            tracing::debug!(pane, error = %e, "a peer's attachment ended before its first chunk");
            return missing();
        }
        None => return missing(),
    };
    let header = transfer.header().clone();
    let claim = Claim {
        kind: &header.kind,
        mime: header.mime.as_deref(),
        name: header.name.as_deref(),
        bytes: header.bytes,
    };
    let headers = headers(&claim, &first);
    let pane = pane.to_string();
    let rest = futures_util::stream::unfold(transfer, |mut transfer| async move {
        transfer.next_chunk().await.map(|chunk| (chunk, transfer))
    });
    // A truncation is visible to the client as a body shorter than the `Content-Length` it was
    // promised, which is the honest end for a transfer that cannot be completed once the status
    // line has gone out.
    let body = futures_util::stream::once(async move { Ok(first) })
        .chain(rest.map(move |chunk| {
            chunk.map_err(|e| {
                tracing::debug!(pane, error = %e, "a peer's attachment stopped mid-body");
                std::io::Error::other(e.to_string())
            })
        }))
        .map_ok(axum::body::Bytes::from);
    (StatusCode::OK, headers, Body::from_stream(body)).into_response()
}

/// A name out of a transcript reaches a header and then a filesystem, so it is reduced to
/// something that can be neither: no separators, no quotes, no control characters, and never
/// empty.
fn filename(claim: &Claim<'_>) -> String {
    let safe: String = claim
        .name
        .unwrap_or_default()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
        .take(80)
        .collect();
    let safe = safe.trim_matches('.');
    if !safe.is_empty() {
        return safe.to_string();
    }
    let extension = claim
        .mime
        .and_then(|m| m.rsplit('/').next())
        .filter(|e| e.chars().all(|c| c.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    format!("attachment.{extension}")
}
