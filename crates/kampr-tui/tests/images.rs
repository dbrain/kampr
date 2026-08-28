//! W6 — the attachment fetch and the inline renderer, against a node scripted route by route.
//!
//! The client is written against `04-wire-protocol.md`, so the honest counterpart is a server
//! that answers exactly what that document says `/api/attachment` answers — including the shapes
//! it refuses. Nothing here touches a real transcript on a real node.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use image::{DynamicImage, GenericImageView, RgbaImage};
use kampr_client::{Role, Session, Via};
use kampr_tui::image::{Attachment, Caps, Images, MAX_BYTES, Protocol, caps_in, cell_in, file_id};
use ratatui::layout::Rect;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;

const PANE: &str = "01JNODE/w3:p2";
const ID: &str = "att-7f3";
const KITTY: Caps = Caps {
    kitty_graphics: true,
    sixel: false,
};
const SIXEL: Caps = Caps {
    kitty_graphics: false,
    sixel: true,
};

struct Seen {
    path: String,
    authorization: Option<String>,
}

struct Reply {
    status: u16,
    mime: Option<String>,
    body: Vec<u8>,
    /// The `Content-Length` this node claims. `None` frames the body chunked instead, which is
    /// the shape that leaves the client's running arithmetic as the only ceiling it has.
    announce: Option<u64>,
}

impl Reply {
    fn ok(mime: &str, body: Vec<u8>) -> Self {
        Self {
            status: 200,
            mime: Some(mime.into()),
            announce: Some(body.len() as u64),
            body,
        }
    }

    fn chunked(mime: &str, body: Vec<u8>) -> Self {
        Self {
            announce: None,
            ..Self::ok(mime, body)
        }
    }

    /// A node that says one thing and sends another. Nothing on this machine does that; a node one
    /// release ahead, or an origin that is not the one the token was minted for, might.
    fn claiming(self, bytes: u64) -> Self {
        Self {
            announce: Some(bytes),
            ..self
        }
    }

    fn refused(status: u16, message: &str) -> Self {
        let body = format!("{{\"error\":\"{message}\"}}").into_bytes();
        Self {
            status,
            mime: Some("application/json".into()),
            announce: Some(body.len() as u64),
            body,
        }
    }
}

/// A node that answers `/api/attachment` and remembers the request line exactly as it arrived —
/// which is the only place the pane's unescaped slash can be observed.
struct Node {
    origin: String,
    seen: Arc<Mutex<Vec<Seen>>>,
}

impl Node {
    async fn start(route: impl Fn(&str) -> Reply + Send + Sync + 'static) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.expect("a port");
        let origin = format!("http://{}", listener.local_addr().expect("an address"));
        let seen: Arc<Mutex<Vec<Seen>>> = Arc::new(Mutex::new(Vec::new()));
        let log = seen.clone();
        let route = Arc::new(route);
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let log = log.clone();
                let route = route.clone();
                tokio::spawn(async move {
                    let mut head = Vec::new();
                    let mut byte = [0u8; 1];
                    while !head.ends_with(b"\r\n\r\n") {
                        match stream.read(&mut byte).await {
                            Ok(1) => head.push(byte[0]),
                            _ => return,
                        }
                    }
                    let text = String::from_utf8_lossy(&head).to_string();
                    let path = text.split_whitespace().nth(1).unwrap_or_default().to_string();
                    let authorization = text.lines().find_map(|l| {
                        l.to_ascii_lowercase()
                            .starts_with("authorization:")
                            .then(|| l["authorization:".len()..].trim().to_string())
                    });
                    let reply = route(&path);
                    log.lock().expect("the log").push(Seen { path, authorization });
                    let mut out = format!(
                        "HTTP/1.1 {} X\r\nCache-Control: no-store\r\nConnection: close\r\n",
                        reply.status
                    )
                    .into_bytes();
                    if let Some(mime) = &reply.mime {
                        out.extend_from_slice(format!("Content-Type: {mime}\r\n").as_bytes());
                    }
                    match reply.announce {
                        Some(bytes) => {
                            out.extend_from_slice(format!("Content-Length: {bytes}\r\n\r\n").as_bytes());
                            out.extend_from_slice(&reply.body);
                        }
                        None => {
                            out.extend_from_slice(b"Transfer-Encoding: chunked\r\n\r\n");
                            for chunk in reply.body.chunks(64 * 1024) {
                                out.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                                out.extend_from_slice(chunk);
                                out.extend_from_slice(b"\r\n");
                            }
                            out.extend_from_slice(b"0\r\n\r\n");
                        }
                    }
                    let _ = stream.write_all(&out).await;
                    let _ = stream.flush().await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        Self { origin, seen }
    }

    fn session(&self) -> Session {
        Session {
            origin: self.origin.clone(),
            token: "scripted-token".into(),
            via: Via::Profile {
                name: "scripted".into(),
            },
        }
    }

    fn images(&self, caps: Caps, host: Option<&str>) -> Images {
        Images::with(&self.session(), host, caps, Some((10, 20)))
    }

    fn asked(&self) -> Vec<String> {
        self.seen
            .lock()
            .expect("the log")
            .iter()
            .map(|s| s.path.clone())
            .collect()
    }

    fn bearer(&self) -> Option<String> {
        self.seen
            .lock()
            .expect("the log")
            .first()
            .and_then(|s| s.authorization.clone())
    }
}

fn att(kind: &str) -> Attachment {
    Attachment {
        id: ID.into(),
        kind: kind.into(),
        mime: Some("image/png".into()),
        bytes: None,
        name: Some("shot.png".into()),
    }
}

fn bitmap(width: u32, height: u32, colour: [u8; 4]) -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(width, height, image::Rgba(colour)))
}

fn png(source: &DynamicImage) -> Vec<u8> {
    let mut out = std::io::Cursor::new(Vec::new());
    source.write_to(&mut out, image::ImageFormat::Png).expect("a png");
    out.into_inner()
}

/// Pump the runtime until the fetch has landed, or give up — a `draw` that never returns `true`
/// is the same shape as one that is merely late, and a test that waited for ever could not tell
/// them apart.
async fn settle(images: &mut Images, pane: &str, id: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        images.collect();
        if images.offer(pane, id).is_some_and(|o| o.ready) {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    images.collect();
    false
}

async fn quiet(images: &mut Images) {
    tokio::time::sleep(Duration::from_millis(150)).await;
    images.collect();
}

// ---------------------------------------------------------------------------------------------

#[tokio::test]
async fn the_route_is_three_segments_with_the_panes_slash_left_alone() {
    let node = Node::start(|_| Reply::ok("image/png", png(&bitmap(4, 4, [255, 0, 0, 255])))).await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    images.request(PANE, &att("image"));
    assert!(settle(&mut images, PANE, ID).await, "the fetch never landed");

    assert_eq!(
        node.asked(),
        vec![format!("/api/attachment/{PANE}/{ID}")],
        "the pane id's slash is sent literally, so the path is three segments and not two"
    );
    let asked = &node.asked()[0];
    assert!(
        !asked.contains("%2F") && !asked.contains("%2f"),
        "a percent-encoded slash is a 404 on every fetch: {asked}"
    );
    let segments: Vec<&str> = asked.trim_start_matches('/').split('/').collect();
    assert_eq!(
        segments,
        vec!["api", "attachment", "01JNODE", "w3:p2", ID],
        "exactly three segments after the route: the node, the local id, and the attachment"
    );
    assert_eq!(node.bearer().as_deref(), Some("Bearer scripted-token"));
}

#[tokio::test]
async fn a_404_leaves_the_marker_text_and_never_a_broken_image() {
    let node = Node::start(|_| Reply::refused(404, "no such attachment")).await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    images.request(PANE, &att("image"));
    quiet(&mut images).await;

    let offer = images.offer(PANE, ID).expect("the block is still there");
    assert!(!offer.ready, "a 404 has no bytes");
    assert!(!offer.inline, "a 404 is not drawn");
    assert!(!offer.too_large, "and it is not the ceiling either");
    assert_eq!(
        images.encode(Rect::new(0, 0, 40, 20), PANE, ID),
        None,
        "an id whose record was rewritten is expected, not an error state: no escape bytes"
    );
    let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 40, 20));
    assert!(
        !images.draw(&mut buffer, Rect::new(0, 0, 40, 20), PANE, ID),
        "false is what makes the caller render the [image · png] marker text"
    );
}

#[tokio::test]
async fn an_unrecognised_kind_is_offered_as_a_file_rather_than_dropped() {
    let node =
        Node::start(|_| Reply::ok("application/octet-stream", b"\x00\x01not a picture".to_vec())).await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    images.request(
        PANE,
        &Attachment {
            id: ID.into(),
            kind: "video".into(),
            mime: Some("video/mp4".into()),
            bytes: Some(15),
            name: Some("clip.mp4".into()),
        },
    );
    assert!(settle(&mut images, PANE, ID).await, "the fetch never landed");

    let offer = images
        .offer(PANE, ID)
        .expect("a kind nobody knows is never dropped");
    assert_eq!(offer.kind, "video");
    assert_eq!(offer.name, Some("clip.mp4"));
    assert!(offer.ready, "the bytes are here to save");
    assert!(!offer.inline, "a kind this client does not know is a download");
    assert_eq!(
        images.encode(Rect::new(0, 0, 40, 20), PANE, ID),
        None,
        "and it is never fed to the inline renderer"
    );

    let to = std::env::temp_dir().join(format!("kampr-w6-{}.bin", std::process::id()));
    images.save(PANE, ID, &to).expect("a download");
    assert_eq!(
        std::fs::read(&to).expect("the saved file"),
        b"\x00\x01not a picture"
    );
    let _ = std::fs::remove_file(&to);
}

#[tokio::test]
async fn a_terminal_that_answered_nothing_draws_nothing_and_writes_no_escape_bytes() {
    let node = Node::start(|_| Reply::ok("image/png", png(&bitmap(64, 64, [0, 128, 255, 255])))).await;
    let mut images = node.images(Caps::default(), Some("SomeTerminal 9.9"));
    assert_eq!(images.protocol(), Protocol::None);
    images.request(PANE, &att("image"));
    assert!(settle(&mut images, PANE, ID).await, "the fetch never landed");

    assert_eq!(
        images.encode(Rect::new(0, 0, 40, 20), PANE, ID),
        None,
        "an unknown terminal gets the marker text, never a guess that vomits escape bytes"
    );
    let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 40, 20));
    assert!(!images.draw(&mut buffer, Rect::new(0, 0, 40, 20), PANE, ID));
    assert_eq!(images.encodes(), 0);
    assert!(
        images.offer(PANE, ID).is_some_and(|o| o.ready && !o.inline),
        "the bytes are still savable — only the drawing is off"
    );
}

/// Chunked, so the node never says how big it is and the client's running arithmetic is the only
/// ceiling it has.
#[tokio::test]
async fn a_body_past_the_ceiling_is_refused_rather_than_buffered() {
    let node = Node::start(|_| Reply::chunked("image/png", vec![0u8; MAX_BYTES as usize + 1024])).await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    images.request(PANE, &att("image"));
    quiet(&mut images).await;
    tokio::time::sleep(Duration::from_millis(300)).await;
    images.collect();

    let offer = images.offer(PANE, ID).expect("the block is still there");
    assert!(!offer.ready, "8 MiB is the ceiling and this body is past it");
    assert!(
        offer.too_large,
        "the client stops reading rather than buffering past the ceiling"
    );
    assert_eq!(images.encode(Rect::new(0, 0, 40, 20), PANE, ID), None);
}

/// A node that announces more than it sends. The claim is checked before the body is read, so a
/// record claiming a gigabyte costs a comparison here exactly as it does on the node.
#[tokio::test]
async fn a_content_length_past_the_ceiling_is_refused_before_the_body_is_read() {
    let node =
        Node::start(|_| Reply::ok("image/png", png(&bitmap(4, 4, [1, 2, 3, 255]))).claiming(MAX_BYTES + 1))
            .await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    images.request(PANE, &att("image"));
    quiet(&mut images).await;

    let offer = images.offer(PANE, ID).expect("the block is still there");
    assert!(
        !offer.ready,
        "the announced length is the ceiling, whatever follows it"
    );
    assert!(offer.too_large);
    assert_eq!(images.encode(Rect::new(0, 0, 40, 20), PANE, ID), None);
}

#[tokio::test]
async fn a_413_is_the_ceiling_and_not_a_crash() {
    let node =
        Node::start(|_| Reply::refused(413, "this attachment is larger than the node will serve")).await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    images.request(PANE, &att("image"));
    quiet(&mut images).await;

    let offer = images.offer(PANE, ID).expect("the block is still there");
    assert!(!offer.ready);
    assert!(!offer.inline);
    assert!(
        offer.too_large,
        "413 is the ceiling and not the single 404 — the picture is there and is simply too big"
    );
    let mut buffer = ratatui::buffer::Buffer::empty(Rect::new(0, 0, 40, 20));
    assert!(!images.draw(&mut buffer, Rect::new(0, 0, 40, 20), PANE, ID));
}

#[tokio::test]
async fn a_header_that_claims_more_than_the_ceiling_is_never_asked_for() {
    let node = Node::start(|_| Reply::ok("image/png", png(&bitmap(4, 4, [1, 2, 3, 255])))).await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    images.request(
        PANE,
        &Attachment {
            bytes: Some(MAX_BYTES + 1),
            ..att("image")
        },
    );
    quiet(&mut images).await;

    assert!(
        node.asked().is_empty(),
        "the ceiling costs a comparison, not a round trip"
    );
    assert!(!images.offer(PANE, ID).expect("the block is still there").ready);
}

#[tokio::test]
async fn an_absent_mime_and_an_absent_name_stay_absent_rather_than_empty() {
    let node = Node::start(|_| Reply::ok("image/png", png(&bitmap(8, 8, [9, 9, 9, 255])))).await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    // A pasted screenshot has no filename and no dimensions; the media type is all there is, and
    // sometimes not even that (#248).
    images.request(
        PANE,
        &Attachment {
            id: ID.into(),
            kind: "image".into(),
            mime: None,
            bytes: None,
            name: None,
        },
    );
    assert!(settle(&mut images, PANE, ID).await, "the fetch never landed");

    let offer = images.offer(PANE, ID).expect("the block is there");
    assert_eq!(offer.name, None, "absent, not empty");
    assert_eq!(
        offer.mime,
        Some("image/png"),
        "the node answers Content-Type off the bytes when the record named none"
    );
    assert!(offer.inline);
}

#[tokio::test]
async fn a_readonly_device_may_fetch_a_record_and_is_never_offered_a_path() {
    let node = Node::start(|path| match path.contains(ID) {
        true => Reply::ok("image/png", png(&bitmap(8, 8, [7, 7, 7, 255]))),
        false => Reply::refused(403, "this device is read-only"),
    })
    .await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));

    images.request(PANE, &att("image"));
    assert!(
        settle(&mut images, PANE, ID).await,
        "looking at a screenshot somebody pasted is reading"
    );

    assert_eq!(
        images.request_file(PANE, "/var/lib/kampr/shot.png", Role::Readonly),
        None,
        "only a device that may send input may ask for a path, and the affordance is absent"
    );
    quiet(&mut images).await;
    assert_eq!(node.asked().len(), 1, "the refusal costs no round trip either");

    let id = images
        .request_file(PANE, "/var/lib/kampr/shot.png", Role::Full)
        .expect("a full device may");
    quiet(&mut images).await;
    assert!(
        node.asked().iter().any(|p| p.ends_with(&id)),
        "a full device's path fetch reaches the route: {:?}",
        node.asked()
    );
    assert!(
        !images.offer(PANE, &id).expect("the block is there").ready,
        "and a 403 is the same non-answer a 404 is"
    );
}

#[test]
fn the_second_id_form_is_the_one_the_node_decodes() {
    assert_eq!(
        file_id("/var/lib/kampr/shot.png"),
        "ZmlsZR8vdmFyL2xpYi9rYW1wci9zaG90LnBuZw"
    );
    assert_eq!(file_id("~/shot.png"), "ZmlsZR9-L3Nob3QucG5n");
    let decoded = URL_SAFE_NO_PAD
        .decode(file_id("/tmp/a~b.png"))
        .expect("base64url with no padding");
    assert_eq!(
        String::from_utf8(decoded).expect("utf8"),
        "file\u{1f}/tmp/a~b.png"
    );
}

#[tokio::test]
async fn a_screenshot_is_decoded_and_encoded_once_however_many_frames_draw_it() {
    let node = Node::start(|_| Reply::ok("image/png", png(&bitmap(400, 300, [12, 34, 56, 255])))).await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    images.request(PANE, &att("image"));
    assert!(settle(&mut images, PANE, ID).await, "the fetch never landed");

    let area = Rect::new(4, 2, 30, 12);
    let first = images.encode(area, PANE, ID).expect("a picture").to_vec();
    for _ in 0..20 {
        assert_eq!(images.encode(area, PANE, ID), Some(first.as_slice()));
    }
    assert_eq!(
        images.encodes(),
        1,
        "re-encoding a 2.22 MB PNG per frame stutters the client"
    );
    assert_eq!(images.decodes(), 1, "and re-decoding it costs the same");

    images
        .encode(Rect::new(4, 2, 20, 8), PANE, ID)
        .expect("a picture");
    assert_eq!(
        images.encodes(),
        2,
        "a different footprint is a different picture"
    );
    assert_eq!(images.decodes(), 1, "but the decode is still the one that landed");
}

#[tokio::test]
async fn a_kitty_terminal_is_handed_a_png_scaled_into_the_cells_it_was_given() {
    let node = Node::start(|_| Reply::ok("image/png", png(&bitmap(400, 300, [200, 30, 40, 255])))).await;
    let mut images = node.images(KITTY, Some("kitty(0.48.2)"));
    images.request(PANE, &att("image"));
    assert!(settle(&mut images, PANE, ID).await, "the fetch never landed");

    let area = Rect::new(0, 0, 20, 10);
    let payload = images.encode(area, PANE, ID).expect("a picture").to_vec();
    let text = String::from_utf8(payload).expect("kitty's escape is ascii");
    assert!(
        text.starts_with("\x1b_Ga=T,f=100,c="),
        "{}",
        &text[..40.min(text.len())]
    );
    assert!(
        text.contains(",C=1,"),
        "the cursor must not move or the whole TUI scrolls"
    );
    assert!(text.ends_with("\x1b\\"));

    let head = text.split(';').next().expect("the control half");
    let cols: u16 = between(head, "c=", ",").parse().expect("a column count");
    let rows: u16 = between(head, "r=", ",").parse().expect("a row count");
    assert!(
        cols <= area.width && rows <= area.height,
        "{cols}x{rows} must fit {area:?}"
    );
    assert!(
        cols > 1 && rows > 1,
        "a 400x300 picture in a 20x10 rect is not one cell"
    );

    let data = STANDARD
        .decode(
            text.split(';')
                .nth(1)
                .expect("the payload")
                .trim_end_matches("\x1b\\"),
        )
        .expect("base64");
    assert_eq!(
        &data[..8],
        b"\x89PNG\r\n\x1a\n",
        "f=100 is a PNG the terminal decodes itself"
    );
    let (w, h) = image::load_from_memory(&data).expect("a png").dimensions();
    assert!(
        w <= u32::from(cols) * 10 && h <= u32::from(rows) * 20,
        "{w}x{h} px must fit {cols}x{rows} cells of 10x20"
    );
    assert!(
        w > u32::from(cols - 1) * 10 && h > u32::from(rows - 1) * 20,
        "and must not claim a cell it does not reach into: {w}x{h} px in {cols}x{rows}"
    );
}

#[tokio::test]
async fn a_sixel_terminal_is_handed_a_picture_that_decodes_back_to_the_bitmap_that_went_in() {
    let want = [17u8, 200, 90];
    let node = Node::start(move |_| {
        Reply::ok(
            "image/png",
            png(&bitmap(400, 400, [want[0], want[1], want[2], 255])),
        )
    })
    .await;
    let mut images = node.images(SIXEL, Some("Konsole 26.04.3"));
    assert_eq!(images.protocol(), Protocol::Sixel);
    images.request(PANE, &att("image"));
    assert!(settle(&mut images, PANE, ID).await, "the fetch never landed");

    let payload = images
        .encode(Rect::new(0, 0, 8, 4), PANE, ID)
        .expect("a picture")
        .to_vec();
    assert!(payload.starts_with(b"\x1bP"), "a sixel is a DCS string");
    assert!(payload.ends_with(b"\x1b\\"));

    // The raster attributes are `" Pan ; Pad ; Ph ; Pv`, and `Ph;Pv` is the pixel box the
    // terminal draws into. 8x4 cells of 10x20 px is 80x80 — a cell is not square, and scaling a
    // picture against the width twice is how it comes out squashed (#291).
    let text = String::from_utf8_lossy(&payload);
    assert!(
        text.contains("\"1;1;80;80"),
        "the sixel must fill the cells it was given: {}",
        &text[..40.min(text.len())]
    );

    let back = icy_sixel::SixelImage::decode(&payload).expect("the sixel decodes");
    assert!(back.width > 0 && back.height > 0);
    let near = |a: u8, b: u8| a.abs_diff(b) <= 24;
    let pixel = &back.pixels[..4];
    assert!(
        near(pixel[0], want[0]) && near(pixel[1], want[1]) && near(pixel[2], want[2]),
        "quantisation may move the colour but not replace it: {pixel:?} against {want:?}"
    );
}

#[test]
fn the_terminals_own_answer_beats_its_name_and_a_name_nobody_measured_beats_nothing() {
    // kitty answers its graphics query; DA1's attribute 4 is sixel. Both are in-band.
    let kitty = "\x1b_Gi=31;OK\x1b\\\x1b[?62;4;22c";
    assert_eq!(
        caps_in(kitty),
        Caps {
            kitty_graphics: true,
            sixel: true
        }
    );
    assert_eq!(
        kampr_tui::image::protocol_for(caps_in(kitty), None),
        Protocol::Kitty
    );

    let sixel_only = "\x1b[?62;4;22c";
    assert_eq!(
        caps_in(sixel_only),
        Caps {
            kitty_graphics: false,
            sixel: true
        }
    );
    assert_eq!(
        kampr_tui::image::protocol_for(caps_in(sixel_only), Some("SomeTerminal 1.0")),
        Protocol::Sixel,
        "an emulator no name table has heard of still gets its picture"
    );

    // A table row goes stale before an emulator does: konsole answering kitty's graphics query is
    // the emulator's own claim about a build this table was written before.
    assert_eq!(
        kampr_tui::image::protocol_for(KITTY, Some("Konsole 26.04.3")),
        Protocol::Kitty,
        "what the terminal answered beats what it is called"
    );

    let plain = "\x1b[?62;22c";
    assert_eq!(caps_in(plain), Caps::default());
    assert_eq!(
        caps_in("\x1b[?64;22c"),
        Caps::default(),
        "attribute 64 is not attribute 4 — DA1 is a parameter list, not a haystack"
    );
    assert_eq!(
        kampr_tui::image::protocol_for(caps_in(plain), None),
        Protocol::None
    );

    // The name table is the fallback, and only for strings that have been read back (#291).
    let named = |host| kampr_tui::image::protocol_for(Caps::default(), Some(host));
    assert_eq!(named("kitty(0.48.2)"), Protocol::Kitty);
    assert_eq!(named("ghostty 1.3.1-arch2.2"), Protocol::Kitty);
    assert_eq!(named("Konsole 26.04.3"), Protocol::Sixel);
    assert_eq!(named("xterm(390)"), Protocol::None, "unmeasured is not a guess");

    // `CSI 6;height;width t` — a cell is not square, which is the whole reason to ask.
    assert_eq!(cell_in("\x1b[6;15;8t"), Some((8, 15)));
    assert_eq!(cell_in("\x1b[?62;4c"), None);
}

fn between<'a>(text: &'a str, from: &str, to: &str) -> &'a str {
    let rest = &text[text.find(from).expect("the marker") + from.len()..];
    &rest[..rest.find(to).unwrap_or(rest.len())]
}

/// A sixel advances the cursor. One that reaches the last row of the screen scrolls the frame the
/// renderer just drew off the top of it, so the bottom row stays the caller's.
#[tokio::test]
async fn a_cursor_moving_protocol_never_takes_the_last_row_of_the_screen() {
    let node = Node::start(|_| Reply::ok("image/png", png(&bitmap(400, 400, [5, 5, 5, 255])))).await;
    let mut images = node.images(SIXEL, Some("Konsole 26.04.3"));
    images.request(PANE, &att("image"));
    assert!(settle(&mut images, PANE, ID).await, "the fetch never landed");

    let screen = Rect::new(0, 0, 40, 10);
    let mut buffer = ratatui::buffer::Buffer::filled(screen, ratatui::buffer::Cell::new("X"));
    assert!(images.draw(&mut buffer, Rect::new(0, 6, 20, 4), PANE, ID));

    let last: String = (0..screen.width)
        .map(|x| buffer[(x, screen.height - 1)].symbol().to_string())
        .collect();
    assert_eq!(
        last,
        "X".repeat(screen.width as usize),
        "the bottom row is the caller's, or the whole frame scrolls away"
    );
}
