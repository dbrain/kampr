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

/// How far into an Ogg stream the codec's own identification packet is looked for. The page header
/// is 27 bytes plus a segment table, so the first packet starts around byte 28 and the name is
/// inside it; 64 bytes covers that without letting a `vorbis` somewhere in a payload decide.
const OGG_CODEC_WINDOW: usize = 64;

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

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_journal::attach::{FILE, IMAGE};

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
            bytes: body.len() as u64,
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
            bytes: body.len() as u64,
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

    #[test]
    fn a_text_file_is_still_bytes_to_download() {
        assert_eq!(served(FILE, None, b"# notes\n\nnothing here\n"), OPAQUE);
    }
}
