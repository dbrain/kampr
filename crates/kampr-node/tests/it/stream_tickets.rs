//! `POST /api/attachment/{locator}` mints a stream ticket and `GET /api/stream/{ticket}` serves it
//! with no header at all, because a media element cannot send one. The real HTTP route, a real node.

use crate::raw::{self, Reply};
use crate::streaming::{self, Client};
use kampr_auth::Role;
use kampr_journal::attach::{FileRef, Source};
use std::path::Path;

pub(crate) async fn mint(origin: &str, bearer: &str, locator: &str, extra: &[(&str, &str)]) -> Reply {
    let mut headers = vec![("Authorization", bearer)];
    headers.extend_from_slice(extra);
    raw::send(
        origin,
        "POST",
        &format!("/api/attachment/{locator}"),
        &headers,
        b"",
    )
    .await
}

pub(crate) async fn ticket(origin: &str, bearer: &str, locator: &str) -> String {
    let minted = mint(origin, bearer, locator, &[]).await;
    assert_eq!(minted.status, 200, "{}", minted.text());
    assert_eq!(minted.header("cache-control"), Some("no-store"));
    minted.json()["ticket"].as_str().expect("a ticket").to_string()
}

pub(crate) async fn stream(origin: &str, ticket: &str, extra: &[(&str, &str)]) -> Reply {
    raw::send(origin, "GET", &format!("/api/stream/{ticket}"), extra, b"").await
}

async fn bearer_get(c: &Client, locator: &str, extra: &[(&str, &str)]) -> Reply {
    let mut headers = vec![("Authorization", c.full.as_str())];
    headers.extend_from_slice(extra);
    raw::send(
        &c.h.origin,
        "GET",
        &format!("/api/attachment/{locator}"),
        &headers,
        b"",
    )
    .await
}

fn pane(c: &Client) -> String {
    format!("{}/w1:p1", c.h.node.node_id())
}

fn a_paste(c: &Client, name: &str, bytes: &[u8]) -> String {
    let pastes = c.h.node.state_dir.join("pastes");
    std::fs::create_dir_all(&pastes).expect("a pastes directory");
    let path = pastes.join(name);
    std::fs::write(&path, bytes).expect("a pasted file");
    format!("{}/{}", pane(c), Source::Paste(FileRef::new(&path)).encode())
}

fn a_file(c: &Client, dir: &Path, name: &str, bytes: &[u8]) -> String {
    let path = dir.join(name);
    std::fs::write(&path, bytes).expect("a file");
    format!("{}/{}", pane(c), streaming::file(&path).encode())
}

fn png() -> Vec<u8> {
    let mut bytes = b"\x89PNG\r\n\x1a\n".to_vec();
    bytes.extend(streaming::pattern(4096));
    bytes
}

/// Compressible on purpose: what must stop it being compressed is that it is media.
fn wav() -> Vec<u8> {
    let mut bytes = b"RIFF\x00\x00\x00\x00WAVEfmt ".to_vec();
    bytes.extend(std::iter::repeat_n(0u8, 64 * 1024));
    bytes
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_serves_exactly_what_the_bearer_route_serves() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    for (form, locator) in [
        ("paste", a_paste(&c, "shot.png", &png())),
        (
            "file",
            a_file(&c, dir.path(), "notes.bin", &streaming::pattern(10_000)),
        ),
        ("audio file", a_file(&c, dir.path(), "take.wav", &wav())),
    ] {
        let direct = bearer_get(&c, &locator, &[]).await;
        assert_eq!(direct.status, 200, "{form}: {}", direct.text());
        let ticket = ticket(&c.h.origin, &c.full, &locator).await;
        let streamed = stream(&c.h.origin, &ticket, &[]).await;
        assert_eq!(streamed.status, 200, "{form}: {}", streamed.text());
        for name in [
            "content-type",
            "content-length",
            "content-disposition",
            "cache-control",
            "accept-ranges",
            "x-content-type-options",
        ] {
            assert_eq!(streamed.header(name), direct.header(name), "{form}: {name}");
        }
        assert_eq!(streamed.header("cache-control"), Some("no-store"), "{form}");
        assert!(streamed.body == direct.body, "{form}: the bytes differ");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_range_through_a_ticket_is_a_206_of_that_range() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let bytes = streaming::pattern(10_000);
    let locator = a_file(&c, dir.path(), "take.bin", &bytes);
    let ticket = ticket(&c.h.origin, &c.full, &locator).await;

    for (range, start, end) in [("bytes=100-199", 100, 199), ("bytes=9000-", 9000, 9999)] {
        let span = stream(&c.h.origin, &ticket, &[("Range", range)]).await;
        assert_eq!(span.status, 206, "{range}: {}", span.text());
        assert_eq!(
            span.header("content-range"),
            Some(format!("bytes {start}-{end}/10000").as_str())
        );
        assert!(
            span.body == bytes[start..=end],
            "{range}: the range came back changed"
        );
    }
}

/// An audio body is already compressed, and a media element reads its duration and seeks from the
/// `Content-Length` a compressed response drops.
#[tokio::test(flavor = "multi_thread")]
async fn media_goes_out_as_the_bytes_it_is_whoever_asks() {
    let c = Client::start().await;
    let locator = a_paste(&c, "voice.wav", &wav());
    let ticket = ticket(&c.h.origin, &c.full, &locator).await;
    let gzip = [("Accept-Encoding", "gzip, br")];

    for (route, reply) in [
        ("bearer", bearer_get(&c, &locator, &gzip).await),
        ("ticket", stream(&c.h.origin, &ticket, &gzip).await),
    ] {
        assert_eq!(reply.status, 200, "{route}: {}", reply.text());
        assert_eq!(reply.header("content-type"), Some("audio/wav"), "{route}");
        assert_eq!(reply.header("content-encoding"), None, "{route}");
        assert_eq!(
            reply.header("content-length"),
            Some(wav().len().to_string().as_str()),
            "{route}"
        );
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_ticket_nobody_minted_is_the_one_404() {
    let c = Client::start().await;
    let locator = a_paste(&c, "shot.png", &png());
    let real = ticket(&c.h.origin, &c.full, &locator).await;
    let last = if real.ends_with('A') { 'B' } else { 'A' };
    let forged = format!("{}{last}", &real[..real.len() - 1]);

    for ticket in ["nope", forged.as_str(), "kmp_whatever"] {
        let reply = stream(&c.h.origin, ticket, &[("Authorization", c.full.as_str())]).await;
        assert_eq!(reply.status, 404, "{ticket}: {}", reply.text());
        assert_eq!(reply.json()["error"], "no such attachment");
    }
}

/// The ticket stands for the device, not for a moment: every request is the device's own token
/// checked again, so a revoke stops a player mid-recording.
#[tokio::test(flavor = "multi_thread")]
async fn revoking_the_device_stops_its_tickets_on_the_next_request() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let locator = a_file(&c, dir.path(), "take.bin", &streaming::pattern(10_000));
    let token = c.h.token_for(Role::Full).await;
    let bearer = format!("Bearer {token}");
    let ticket = ticket(&c.h.origin, &bearer, &locator).await;
    assert_eq!(
        stream(&c.h.origin, &ticket, &[("Range", "bytes=0-9")])
            .await
            .status,
        206
    );

    let device =
        c.h.node
            .auth
            .authenticate(&token, "test")
            .await
            .expect("a device");
    assert!(c.h.node.auth.revoke(&device.id, &device).await.expect("revoked"));

    let after = stream(&c.h.origin, &ticket, &[("Range", "bytes=10-19")]).await;
    assert_eq!(after.status, 401, "{}", after.text());
}

#[tokio::test(flavor = "multi_thread")]
async fn minting_answers_as_the_get_would_without_reading_a_byte() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let readonly = format!("Bearer {}", c.h.token_for(Role::Readonly).await);
    let paste = a_paste(&c, "shot.png", &png());
    let file = a_file(&c, dir.path(), "notes.bin", b"secret");
    let nowhere = format!(
        "{}/{}",
        pane(&c),
        streaming::file(&dir.path().join("not-there")).encode()
    );
    let node = c.h.node.node_id();

    for (case, bearer, locator, status) in [
        ("read-only, paste", readonly.as_str(), paste.as_str(), 200),
        ("read-only, path", readonly.as_str(), file.as_str(), 403),
        ("full, path not on disk", c.full.as_str(), nowhere.as_str(), 200),
        (
            "two segments",
            c.full.as_str(),
            &format!("{node}/w1:p1") as &str,
            404,
        ),
        (
            "four segments",
            c.full.as_str(),
            &format!("{paste}/extra") as &str,
            404,
        ),
        (
            "empty id",
            c.full.as_str(),
            &format!("{node}/w1:p1/") as &str,
            404,
        ),
        ("no token", "", paste.as_str(), 401),
    ] {
        let reply = mint(&c.h.origin, bearer, locator, &[]).await;
        assert_eq!(reply.status, status, "{case}: {}", reply.text());
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_page_on_another_origin_cannot_mint_one() {
    let c = Client::start().await;
    let locator = a_paste(&c, "shot.png", &png());
    let reply = mint(
        &c.h.origin,
        &c.full,
        &locator,
        &[("Origin", "https://evil.example")],
    )
    .await;
    assert_eq!(reply.status, 403, "{}", reply.text());
}

#[tokio::test(flavor = "multi_thread")]
async fn the_page_may_play_media_from_this_origin_and_nowhere_else() {
    let c = Client::start().await;
    let reply = raw::send(&c.h.origin, "GET", "/healthz", &[], b"").await;
    let csp = reply.header("content-security-policy").expect("a CSP");
    let media = csp
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("media-src"))
        .unwrap_or_else(|| panic!("no media-src in {csp}"));
    assert_eq!(media, "media-src 'self'");
}

/// An `.mp4` on disk carries no recorded type, so its brand is all that can make it a video a
/// player will take, and compressible on purpose like [`wav`].
fn mp4() -> Vec<u8> {
    let mut bytes = b"\x00\x00\x00\x20ftypisom\x00\x00\x02\x00isomiso2avc1mp41".to_vec();
    bytes.extend(std::iter::repeat_n(0u8, 64 * 1024));
    bytes
}

#[tokio::test(flavor = "multi_thread")]
async fn a_video_file_goes_out_as_video_whole_and_by_range() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let bytes = mp4();
    let locator = a_file(&c, dir.path(), "clip.mp4", &bytes);
    let ticket = ticket(&c.h.origin, &c.full, &locator).await;
    let gzip = ("Accept-Encoding", "gzip, br");
    let total = bytes.len();

    for (route, whole, span) in [
        (
            "bearer",
            bearer_get(&c, &locator, &[gzip]).await,
            bearer_get(&c, &locator, &[gzip, ("Range", "bytes=100-199")]).await,
        ),
        (
            "ticket",
            stream(&c.h.origin, &ticket, &[gzip]).await,
            stream(&c.h.origin, &ticket, &[gzip, ("Range", "bytes=100-199")]).await,
        ),
    ] {
        assert_eq!(whole.status, 200, "{route}: {}", whole.text());
        assert_eq!(whole.header("content-type"), Some("video/mp4"), "{route}");
        assert_eq!(whole.header("content-encoding"), None, "{route}");
        assert_eq!(
            whole.header("content-length"),
            Some(total.to_string().as_str()),
            "{route}"
        );
        assert!(whole.body == bytes, "{route}: the bytes differ");

        assert_eq!(span.status, 206, "{route}: {}", span.text());
        assert_eq!(span.header("content-type"), Some("video/mp4"), "{route}");
        assert_eq!(span.header("content-encoding"), None, "{route}");
        assert_eq!(
            span.header("content-range"),
            Some(format!("bytes 100-199/{total}").as_str()),
            "{route}"
        );
        assert!(
            span.body == bytes[100..=199],
            "{route}: the range came back changed"
        );
    }
}
