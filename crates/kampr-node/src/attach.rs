use std::path::Path;

use axum::body::Body;
use axum::http::StatusCode;
use axum::http::header::{
    ACCEPT_RANGES, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_RANGE, CONTENT_TYPE,
};
use axum::http::{HeaderName, HeaderValue};
use axum::response::{AppendHeaders, IntoResponse, Response};
use futures_util::{Stream, StreamExt, TryStreamExt};
use std::path::PathBuf;

use kampr_journal::attach::{FILE, IMAGE, MAX_BYTES, TEXT, image_mime};
use kampr_journal::{Fetched, FileRef, JournalError, Registry as Journals, Source};
use kampr_mesh::{FetchError, Peers, Span};

use crate::git::{self, Between, GitError};
use crate::http::refuse;
use crate::served::{Refused, STREAM_CEILING, Served, Slice};
use crate::zipped::{ARCHIVE, ZIP};

/// What a transcript's own `media_type` is allowed to become on a response from this origin.
///
/// **The recorded type is attacker-influenced**: it is a string in a file some agent wrote, and a
/// node that echoed it would serve `text/html` from its own origin, past a CSP written for the
/// bundle. Anything not on this list is bytes to download, never a document to render — and SVG
/// is deliberately absent, because it is a scriptable document wearing an image's name.
///
/// The audio types are here on the same argument that keeps SVG off. A container on this list is
/// read by a media decoder and by nothing else: it cannot name an origin, cannot carry script, and
/// is not a shape a sniffing browser ever promotes to HTML. `audio/ogg` and `audio/mp4` are the two
/// that could have named a video track, and neither one is reached from a recorded string alone —
/// [`sniff`] proves the codec out of the bytes before either is minted here.
const RENDERABLE: &[&str] = &[
    "image/png",
    "image/jpeg",
    "image/gif",
    "image/webp",
    "image/avif",
    "image/bmp",
    "image/x-icon",
    "audio/wav",
    "audio/mpeg",
    "audio/mp4",
    "audio/ogg",
    "audio/flac",
    "audio/aiff",
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

fn unsatisfiable(total: u64) -> Response {
    let mut response = refuse(
        StatusCode::RANGE_NOT_SATISFIABLE,
        "that range is past the end of the file",
    );
    if let Ok(value) = HeaderValue::from_str(&format!("bytes */{total}")) {
        response.headers_mut().insert(CONTENT_RANGE, value);
    }
    response
}

fn refused(why: Refused) -> Response {
    match why {
        Refused::Missing => missing(),
        Refused::TooLarge(bytes) => {
            tracing::debug!(bytes, "refusing an attachment past the ceiling");
            past_the_ceiling()
        }
        Refused::Unsatisfiable(total) => unsatisfiable(total),
    }
}

pub fn serve(journals: &Journals, transcript: &Path, id: &str) -> Response {
    respond(kampr_journal::attach::fetch(journals, id, transcript))
}

/// The answer to any path form (`Source::is_path_form`), for the HTTP route and the mesh peer
/// alike, for a caller already established as one that may send input — a device that can type
/// into a terminal can `cat`, `ls`, `zip` and `git show` there anyway.
///
/// `home` is what a leading `~` resolves against, and it is the node's own
/// [`crate::Config::journal_home`] — the operator's home rather than the process's whenever a
/// node runs as a service user. `span` is honoured by a `file` and by nothing else.
///
/// **Every refusal is the same 404 the record form gives.** A relative path, a directory, a file
/// that is not there, one this user cannot read and a rev that names nothing must not be
/// distinguishable from outside, or the route is a way to map the filesystem by response code.
/// Blocking: it opens files, reads directories and runs `git`. What it returns is not — a file
/// and a zip are read as the body is polled, never here.
pub fn open_path_form(source: &Source, home: &Path, span: Option<Span>) -> Result<Served, Refused> {
    match source {
        Source::File(file) => {
            let (path, opened, len) = file.open(home)?;
            Served::file(&path, opened, len, span)
        }
        Source::Zip(dir) => crate::zipped::serve(&dir.absolute(home)?),
        _ => Ok(Served::whole(composed(source, home)?)),
    }
}

/// The path forms whose answer the node writes itself and holds whole: a diff, a listing, a
/// summary, a file out of history, a commit.
fn composed(source: &Source, home: &Path) -> Result<Fetched, JournalError> {
    let refused = || JournalError::NotFound(String::new());
    let git = |e: GitError| match e {
        GitError::TooLarge => JournalError::TooLarge(STREAM_CEILING),
        GitError::None => refused(),
    };
    let diff = |file: &FileRef, between| -> Result<Fetched, JournalError> {
        let path = file.resolve(home)?;
        let diff = git::diff(&path, between).map_err(git)?;
        Ok(text(diff.into_bytes(), named(&path, ".diff")))
    };
    match source {
        Source::Diff(file) => diff(file, Between::Head),
        Source::Staged(file) => diff(file, Between::Staged),
        Source::Unstaged(file) => diff(file, Between::Unstaged),
        Source::Ls(dir) => {
            let listing = crate::fs::list(&dir.absolute(home)?, home).ok_or_else(refused)?;
            json(&listing)
        }
        Source::Repo(path) => json(&git::summary(&directory_of(&path.absolute(home)?)?).map_err(git)?),
        Source::Show { file, rev } => {
            let path = file.absolute(home)?;
            let data = git::show_at(&path, rev).map_err(git)?;
            Ok(as_found(&path, data))
        }
        Source::Commit { repo, sha } => {
            let dir = directory_of(&repo.absolute(home)?)?;
            let patch = git::commit_patch(&dir, sha).map_err(git)?;
            Ok(text(patch.into_bytes(), Some(format!("{sha}.patch"))))
        }
        Source::File(_) | Source::Zip(_) | Source::Record(_) | Source::Paste(_) => Err(refused()),
    }
}

pub async fn serve_path(source: &Source, home: &Path, span: Option<Span>) -> Response {
    let (source, home) = (source.clone(), home.to_path_buf());
    match tokio::task::spawn_blocking(move || open_path_form(&source, &home, span)).await {
        Ok(Ok(served)) => answer(served).await,
        Ok(Err(why)) => refused(why),
        Err(_) => refuse(
            StatusCode::INTERNAL_SERVER_ERROR,
            "the attachment could not be read",
        ),
    }
}

/// The directory to run `git` in for a path that may name a file or a directory.
fn directory_of(path: &Path) -> Result<PathBuf, JournalError> {
    let meta = std::fs::metadata(path).map_err(|_| JournalError::NotFound(String::new()))?;
    match (meta.is_dir(), path.parent()) {
        (true, _) => Ok(path.to_path_buf()),
        (false, Some(parent)) => Ok(parent.to_path_buf()),
        (false, None) => Err(JournalError::NotFound(String::new())),
    }
}

fn named(path: &Path, suffix: &str) -> Option<String> {
    path.file_name()
        .map(|n| format!("{}{suffix}", n.to_string_lossy()))
}

pub(crate) const PLAIN: &str = "text/plain; charset=utf-8";
const JSON: &str = "application/json";

/// The types the node's own answers are served as. Never a type read off a file or a record: the
/// node decided these, so only these are trusted when a peer's claim comes back over the mesh.
const COMPOSED: &[&str] = &[PLAIN, JSON];

fn text(data: Vec<u8>, name: Option<String>) -> Fetched {
    Fetched {
        kind: TEXT.to_string(),
        mime: Some(PLAIN.to_string()),
        name,
        data,
    }
}

fn json(value: &impl serde::Serialize) -> Result<Fetched, JournalError> {
    Ok(Fetched {
        kind: TEXT.to_string(),
        mime: Some(JSON.to_string()),
        name: None,
        data: serde_json::to_vec(value).map_err(|_| JournalError::NotFound(String::new()))?,
    })
}

/// A file's bytes out of history are typed the way a file on disk is — an image by its extension,
/// otherwise by its bytes — except that text is said to be text, because a reader comparing two
/// revisions wants to read them.
fn as_found(path: &Path, data: Vec<u8>) -> Fetched {
    if let Some(mime) = image_mime(path) {
        return Fetched {
            kind: IMAGE.to_string(),
            mime: Some(mime.to_string()),
            name: named(path, ""),
            data,
        };
    }
    if !data.contains(&0) && std::str::from_utf8(&data).is_ok() {
        return text(data, named(path, ""));
    }
    Fetched {
        kind: FILE.to_string(),
        mime: None,
        name: named(path, ""),
        data,
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
        Ok(found) => {
            let prefix = found.data[..found.data.len().min(OGG_CODEC_WINDOW)].to_vec();
            response(Served::whole(found), &prefix, Vec::new())
        }
        Err(e) => {
            tracing::debug!(error = %e, "refusing an attachment");
            refused(e.into())
        }
    }
}

/// The first chunk is pulled before the status line goes out, because a body that has no type on
/// record is typed from its own first bytes.
async fn answer(mut served: Served) -> Response {
    let first = match served.body.next().await {
        Some(Ok(chunk)) => chunk,
        Some(Err(e)) => {
            tracing::debug!(error = %e, "an attachment failed before its first byte");
            return missing();
        }
        None => Vec::new(),
    };
    response(served, &first.clone(), first)
}

/// What a record claims to be, which is all the response headers are ever derived from — and
/// none of it is trusted: see [`rendered_as`].
struct Claim<'a> {
    kind: &'a str,
    mime: Option<&'a str>,
    name: Option<&'a str>,
    bytes: Option<u64>,
}

/// The headers an attachment is served under, decided from a record's claims and the first bytes
/// of its body. No `Content-Length` when nobody knows it, which makes the body chunked.
///
/// One implementation for both paths on purpose. A relayed attachment is described by the peer
/// and served from *this* origin, so the allowlist that decides what may render has to be applied
/// here rather than taken on trust — a peer that said `text/html` would otherwise get a document
/// out of the hub's origin, past a CSP written for the bundle.
fn headers(claim: &Claim<'_>, prefix: &[u8]) -> Vec<(HeaderName, String)> {
    let rendered = rendered_as(claim, prefix);
    let disposition = match rendered {
        Some(ZIP) | None => format!("attachment; filename=\"{}\"", filename(claim)),
        Some(_) => "inline".to_string(),
    };
    let mut headers = vec![
        (CONTENT_TYPE, rendered.unwrap_or(OPAQUE).to_string()),
        (CONTENT_DISPOSITION, disposition),
        (CACHE_CONTROL, "no-store".to_string()),
    ];
    if let Some(bytes) = claim.bytes {
        headers.push((CONTENT_LENGTH, bytes.to_string()));
    }
    headers
}

/// `prefix` is what an untyped body is sniffed from, and a slice is never sniffed: bytes from the
/// middle of a file say nothing about what the file is.
fn response(served: Served, prefix: &[u8], first: Vec<u8>) -> Response {
    let claim = Claim {
        kind: &served.kind,
        mime: served.mime.as_deref(),
        name: served.name.as_deref(),
        bytes: served.length,
    };
    let mut headers = headers(&claim, if served.slice.is_some() { &[] } else { prefix });
    if served.ranges {
        headers.push((ACCEPT_RANGES, "bytes".to_string()));
    }
    let status = match served.slice {
        Some(Slice { start, end, total }) => {
            headers.push((CONTENT_RANGE, format!("bytes {start}-{end}/{total}")));
            StatusCode::PARTIAL_CONTENT
        }
        None => StatusCode::OK,
    };
    (
        status,
        AppendHeaders(headers),
        Body::from_stream(chunked(first, served.body)),
    )
        .into_response()
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
    if claim.kind == ARCHIVE {
        return (claim.mime == Some(ZIP)).then_some(ZIP);
    }
    if claim.kind == TEXT {
        return COMPOSED.iter().find(|m| Some(**m) == claim.mime).copied();
    }
    if claim.kind != IMAGE && claim.kind != FILE {
        return None;
    }
    match claim.mime {
        Some(recorded) => RENDERABLE.iter().find(|m| **m == recorded).copied(),
        None => sniff(prefix),
    }
}

/// How far into an Ogg stream the codec's own identification packet is looked for. The page header
/// is 27 bytes plus a segment table, so the first packet starts around byte 28 and the name is
/// inside it; 64 bytes covers that without letting a `vorbis` somewhere in a payload decide.
const OGG_CODEC_WINDOW: usize = 64;

pub(crate) fn sniff(data: &[u8]) -> Option<&'static str> {
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
    if at(0, b"RIFF") && at(8, b"WAVE") {
        return Some("audio/wav");
    }
    if at(0, b"FORM") && (at(8, b"AIFF") || at(8, b"AIFC")) {
        return Some("audio/aiff");
    }
    if at(0, b"fLaC") {
        return Some("audio/flac");
    }
    // An MPEG audio file either opens with a tag or with a frame. The frame test is the sync word
    // plus **layer III** — mask `0xE6`, value `0xE2` — rather than the sync word alone, which two
    // bytes of anything hit once in eight.
    if at(0, b"ID3") || (data.len() >= 2 && data[0] == 0xFF && data[1] & 0xE6 == 0xE2) {
        return Some("audio/mpeg");
    }
    // Ogg and the ISO base media format are the two containers on the list that can hold video, so
    // neither is minted from the container alone: the codec's own name has to be in the bytes.
    if at(0, b"OggS") {
        let window = &data[..data.len().min(OGG_CODEC_WINDOW)];
        if window.windows(6).any(|w| w == b"vorbis") || window.windows(8).any(|w| w == b"OpusHead") {
            return Some("audio/ogg");
        }
        return None;
    }
    if at(4, b"ftypM4A") {
        return Some("audio/mp4");
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
/// bytes arrive, because the claim came off the network. A record keeps its 8 MiB; a path form
/// is streamed end to end and is held only to [`STREAM_CEILING`].
///
/// A `span` is passed on for a `file` id and answered as `206` when the peer says it honoured it
/// — an `att.open` with a `total` — and as the whole file when it did not.
///
/// **Every refusal but the ceiling and the range is the one 404.** Offline, unknown pane, a peer
/// that never answered and a stale id are the same sentence from outside; which one it was goes
/// to the log.
pub async fn relay(peers: &Peers, pane: &str, id: &str, span: Option<Span>) -> Response {
    let source = Source::decode(id).ok();
    let ceiling = match source.as_ref().is_some_and(Source::is_path_form) {
        true => STREAM_CEILING,
        false => MAX_BYTES,
    };
    let ranges = matches!(source, Some(Source::File(_)));
    let span = span.filter(|_| ranges);
    let mut transfer = match peers.fetch_attachment(pane, id, ceiling, span).await {
        Ok(transfer) => transfer,
        Err(FetchError::TooLarge(bytes)) => {
            tracing::debug!(pane, bytes, "refusing a peer's attachment past the ceiling");
            return past_the_ceiling();
        }
        Err(FetchError::Unsatisfiable(total)) => return unsatisfiable(total),
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
    let slice = match (span, header.total, header.bytes) {
        (Some(span), Some(total), Some(bytes)) if bytes > 0 => Some(Slice {
            start: span.start,
            end: span.start + bytes - 1,
            total,
        }),
        _ => None,
    };
    let pane = pane.to_string();
    let rest = futures_util::stream::unfold(transfer, |mut transfer| async move {
        transfer.next_chunk().await.map(|chunk| (chunk, transfer))
    })
    .map(move |chunk| {
        chunk.map_err(|e| {
            tracing::debug!(pane, error = %e, "a peer's attachment stopped mid-body");
            std::io::Error::other(e.to_string())
        })
    });
    let served = Served {
        kind: header.kind,
        mime: header.mime,
        name: header.name,
        length: header.bytes,
        slice,
        ranges,
        body: Box::pin(rest),
    };
    response(served, &first.clone(), first)
}

/// The first chunk, held back to decide the headers, and then the rest of them.
///
/// **The chunk source is fused, and a peer's attachment is why.** `Chain` does not remember that
/// its second stream has finished — it polls it again on the next poll — and the source here is a
/// `stream::unfold`, which *panics* rather than answering `None` twice. Hyper polls a body once
/// more after its last chunk, so every relayed attachment ended in a panicked tokio worker and a
/// connection dropped with no response header on it: the operator's browser showed `502` from the
/// proxy in front of the node, twice to the second in its error log, while the node's own log
/// carried the panic and nothing else (probe #505). A truncation is still visible to the client as
/// a body shorter than the `Content-Length` it was promised, which is the honest end for a
/// transfer that cannot be completed once the status line has gone out.
fn chunked<S>(first: Vec<u8>, rest: S) -> impl Stream<Item = std::io::Result<axum::body::Bytes>> + Send
where
    S: Stream<Item = std::io::Result<Vec<u8>>> + Send,
{
    futures_util::stream::iter((!first.is_empty()).then_some(Ok(first)))
        .chain(rest.fuse())
        .map_ok(axum::body::Bytes::from)
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

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_journal::attach::{FILE, IMAGE};

    /// The chunk source the relay actually has: `stream::unfold`, which panics rather than
    /// answering `None` a second time. Nothing here is a stand-in for the panic — this *is* the
    /// contract the real source keeps, and the body has to hold to it.
    fn refuses_a_second_ending(chunks: Vec<Vec<u8>>) -> impl Stream<Item = std::io::Result<Vec<u8>>> + Send {
        futures_util::stream::unfold(chunks.into_iter(), |mut left| async move {
            left.next().map(|chunk| (Ok(chunk), left))
        })
    }

    /// Hyper polls a body once more after its last chunk. Before the fuse that poll reached a
    /// spent `unfold` through `Chain`, which panics — the worker died, the connection was dropped
    /// with no header on it, and every relayed attachment came back to the operator as the
    /// proxy's `502` (probe #505).
    #[tokio::test]
    async fn a_relayed_body_polled_after_its_last_chunk_ends_rather_than_panicking() {
        let body = chunked(
            b"first".to_vec(),
            refuses_a_second_ending(vec![b"second".to_vec()]),
        );
        let mut body = Box::pin(body);
        let mut seen = Vec::new();
        while let Some(chunk) = body.next().await {
            seen.extend_from_slice(&chunk.expect("a chunk"));
        }
        assert_eq!(seen, b"firstsecond");
        assert!(
            body.next().await.is_none(),
            "a body polled after its last chunk must end again, not panic"
        );
    }

    fn wav() -> Vec<u8> {
        let mut bytes = b"RIFF\x24\x00\x00\x00WAVEfmt ".to_vec();
        bytes.extend_from_slice(&[0u8; 32]);
        bytes
    }

    fn served(kind: &str, mime: Option<&str>, body: &[u8]) -> String {
        let claim = Claim {
            kind,
            mime,
            name: Some("clip.wav"),
            bytes: Some(body.len() as u64),
        };
        let headers = headers(&claim, body);
        headers
            .into_iter()
            .find(|(name, _)| name == CONTENT_TYPE)
            .map(|(_, value)| value)
            .expect("a content type")
    }

    fn disposition(kind: &str, mime: Option<&str>, body: &[u8]) -> String {
        let claim = Claim {
            kind,
            mime,
            name: Some("clip.wav"),
            bytes: Some(body.len() as u64),
        };
        headers(&claim, body)
            .into_iter()
            .find(|(name, _)| name == CONTENT_DISPOSITION)
            .map(|(_, value)| value)
            .expect("a disposition")
    }

    // A file on disk carries no recorded type at all — `image_mime` answers only for pictures — so
    // the bytes are the only thing that can say a `.wav` is a `.wav`.
    #[test]
    fn a_recording_with_no_recorded_type_is_served_as_the_audio_it_is() {
        assert_eq!(served(FILE, None, &wav()), "audio/wav");
        assert_eq!(disposition(FILE, None, &wav()), "inline");
    }

    #[test]
    fn each_audio_container_on_the_list_is_recognised_by_its_own_bytes() {
        let mut ogg = b"OggS\x00\x02".to_vec();
        ogg.extend_from_slice(&[0u8; 23]);
        ogg.extend_from_slice(b"\x01vorbis");
        let mut m4a = b"\x00\x00\x00\x20ftypM4A ".to_vec();
        m4a.extend_from_slice(&[0u8; 16]);
        for (name, body, want) in [
            ("wav", wav(), "audio/wav"),
            ("flac", b"fLaC\x00\x00\x00\x22".to_vec(), "audio/flac"),
            (
                "mp3 with a tag",
                b"ID3\x04\x00\x00\x00\x00".to_vec(),
                "audio/mpeg",
            ),
            ("bare mp3 frame", vec![0xFF, 0xFB, 0x90, 0x00], "audio/mpeg"),
            ("aiff", b"FORM\x00\x00\x00\x20AIFFCOMM".to_vec(), "audio/aiff"),
            ("ogg vorbis", ogg, "audio/ogg"),
            ("m4a", m4a, "audio/mp4"),
        ] {
            assert_eq!(served(FILE, None, &body), want, "{name} was not recognised");
        }
    }

    // The whole argument for widening the list: what may render is decided here, from a fixed
    // table, and never from the string a transcript happens to carry.
    #[test]
    fn a_type_that_is_not_on_the_list_is_bytes_to_download_however_it_is_claimed() {
        for claimed in [
            "text/html",
            "image/svg+xml",
            "audio/x-scriptable",
            "application/pdf",
        ] {
            assert_eq!(
                served(IMAGE, Some(claimed), &wav()),
                OPAQUE,
                "{claimed} was served as itself",
            );
        }
        assert!(disposition(IMAGE, Some("text/html"), &wav()).starts_with("attachment;"));
    }

    // An Ogg page can carry Theora as easily as Vorbis, and the ISO container is worse — `ftyp`
    // alone is an mp4 video. Neither is minted from the container.
    #[test]
    fn a_container_that_could_hold_video_is_not_audio_until_the_codec_says_so() {
        let mut theora = b"OggS\x00\x02".to_vec();
        theora.extend_from_slice(&[0u8; 23]);
        theora.extend_from_slice(b"\x80theora");
        assert_eq!(served(FILE, None, &theora), OPAQUE);

        let mut mp4 = b"\x00\x00\x00\x20ftypmp42".to_vec();
        mp4.extend_from_slice(&[0u8; 16]);
        assert_eq!(served(FILE, None, &mp4), OPAQUE);
    }

    // The bytes only ever get a vote where the record had nothing to say. A record that named a
    // type keeps being judged by that name, whatever its body opens with.
    #[test]
    fn bytes_that_look_like_audio_do_not_override_a_recorded_type() {
        assert_eq!(served(FILE, Some("text/html"), &wav()), OPAQUE);
        assert_eq!(served(FILE, Some("audio/wav"), b"not audio at all"), "audio/wav");
    }

    // A `RIFF` header is a family, not a format: the same four bytes open a WebP.
    #[test]
    fn a_riff_container_is_told_apart_by_its_second_tag() {
        let mut webp = b"RIFF\x24\x00\x00\x00WEBPVP8 ".to_vec();
        webp.extend_from_slice(&[0u8; 16]);
        assert_eq!(served(FILE, None, &webp), "image/webp");
    }

    // A peer's `att.open` is relayed from this origin, so a composed answer is served as one of
    // the two types the node itself mints and a peer claiming anything else under that kind is a
    // download.
    #[test]
    fn a_composed_answer_is_served_as_its_own_type_and_nothing_a_peer_claims_beyond_it() {
        use kampr_journal::attach::TEXT;
        assert_eq!(served(TEXT, Some(PLAIN), b"diff"), PLAIN);
        assert_eq!(served(TEXT, Some(JSON), b"{}"), JSON);
        for claimed in [Some("text/html"), Some("text/plain"), None] {
            assert_eq!(served(TEXT, claimed, b"<html>"), OPAQUE, "{claimed:?}");
        }
    }

    #[test]
    fn a_text_file_is_still_bytes_to_download() {
        assert_eq!(served(FILE, None, b"# notes\n\nnothing here\n"), OPAQUE);
    }
}
