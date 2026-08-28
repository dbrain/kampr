//! W6 — the attachment fetch and the inline renderer.

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD};
use image::DynamicImage;
use image::imageops::FilterType;
use kampr_client::{Role, Session};
use ratatui::buffer::{Buffer, CellDiffOption};
use ratatui::layout::{Position, Rect};
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};

/// The same 8 MiB the node reads off a record before it allocates anything, applied again on this
/// side because the length arrives over the network. Between three and four times the largest
/// attachment ever measured (#247).
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

const FETCH_TIMEOUT: Duration = Duration::from_secs(30);

/// What a cell is worth in pixels when `CSI 16t` did not answer. The three emulators on this desk
/// answer 8x15 (konsole), 8x17 (ghostty) and 8x18 (kitty), so this is the middle of a narrow
/// range rather than a guess. It is only ever the sixel scaler's fallback: kitty and iTerm2 are
/// handed a cell count and do their own arithmetic.
const CELL: (u16, u16) = (8, 16);

/// Kitty's escape carries base64 in chunks, and 4096 is the size its own protocol document names.
const KITTY_CHUNK: usize = 4096;

/// What this terminal can draw inline, named from `CSI >0q` — the same question the fit ladder
/// already asks it (#291).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Protocol {
    #[default]
    None,
    Kitty,
    Iterm2,
    Sixel,
}

/// An `att` header off an `md` block. The bytes never travel on the websocket.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Attachment {
    pub id: String,
    pub kind: String,
    pub mime: Option<String>,
    pub bytes: Option<u64>,
    pub name: Option<String>,
}

/// What the terminal answered in band, which is worth more than its name: kitty's graphics query
/// and DA1's sixel attribute are the emulator's own answer about itself, and a name table is only
/// what is left when neither came back.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Caps {
    pub kitty_graphics: bool,
    pub sixel: bool,
}

/// A non-image attachment, and an image whose bytes have not landed. **Never dropped**: `kind` is
/// an open string and a client that does not recognise one treats it as a file, so a later
/// `video` needs no client release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Offer<'a> {
    pub kind: &'a str,
    pub mime: Option<&'a str>,
    pub bytes: Option<u64>,
    pub name: Option<&'a str>,
    /// Whether bytes are here to save. `false` while the fetch is in flight, and for ever once it
    /// answered `404` — an id names a record in a transcript and stops resolving when the
    /// transcript is rewritten, which is expected rather than an error state.
    pub ready: bool,
    /// This client will draw it inline rather than offer it as a download.
    pub inline: bool,
    /// The node's `413` — the one refusal that is **not** the single `404`, and the one worth
    /// saying out loud, because the picture is there and is simply bigger than 8 MiB.
    pub too_large: bool,
}

enum Landing {
    Bytes {
        data: Vec<u8>,
        mime: Option<String>,
    },
    /// Every refusal but the ceiling, which is the one answer the route gives on purpose.
    Gone,
    TooLarge,
}

struct Landed {
    pane: String,
    id: String,
    landing: Landing,
}

struct Encoded {
    cells: (u16, u16),
    payload: Vec<u8>,
}

struct Slot {
    pane: String,
    id: String,
    kind: String,
    mime: Option<String>,
    bytes: Option<u64>,
    name: Option<String>,
    landing: Option<Landing>,
    decoded: Option<DynamicImage>,
    encoded: Option<Encoded>,
    placed: Option<(u16, u16, u16, u16)>,
}

impl Slot {
    fn served(&self) -> Option<&str> {
        match &self.landing {
            Some(Landing::Bytes { mime, .. }) => mime.as_deref(),
            _ => None,
        }
    }

    fn data(&self) -> Option<&[u8]> {
        match &self.landing {
            Some(Landing::Bytes { data, .. }) => Some(data),
            _ => None,
        }
    }

    /// The **served** media type decides, not the header's `kind`. A node serves an image type
    /// only off its own allowlist and answers everything else `application/octet-stream`, so this
    /// is the one reading that cannot be talked into rendering a scriptable document.
    fn inline(&self) -> bool {
        self.served().is_some_and(|m| m.starts_with("image/"))
    }
}

pub struct Images {
    protocol: Protocol,
    cell: (u16, u16),
    origin: String,
    token: String,
    http: Option<reqwest::Client>,
    slots: Vec<Slot>,
    out: UnboundedSender<Landed>,
    inbox: UnboundedReceiver<Landed>,
    encodes: usize,
    decodes: usize,
}

impl Default for Images {
    fn default() -> Self {
        let (out, inbox) = unbounded_channel();
        Self {
            protocol: Protocol::None,
            cell: CELL,
            origin: String::new(),
            token: String::new(),
            http: None,
            slots: Vec::new(),
            out,
            inbox,
            encodes: 0,
            decodes: 0,
        }
    }
}

impl Images {
    /// The terminal's answers are handed in rather than asked for here: one prober asks the tty
    /// every question this client has, at start-up, before the event stream owns the keyboard
    /// ([`crate::render::fit::Tty`]). Two readers on one tty race each other for the answer.
    pub fn with(session: &Session, host: Option<&str>, caps: Caps, cell: Option<(u16, u16)>) -> Self {
        Self {
            protocol: protocol_for(caps, host),
            cell: cell.filter(|(w, h)| *w > 0 && *h > 0).unwrap_or(CELL),
            origin: session.origin.trim_end_matches('/').to_string(),
            token: session.token.clone(),
            http: reqwest::Client::builder().timeout(FETCH_TIMEOUT).build().ok(),
            ..Self::default()
        }
    }

    pub fn protocol(&self) -> Protocol {
        self.protocol
    }

    /// How many times an attachment has been scaled and re-encoded, and how many times its bytes
    /// have been decoded. A 2.22 MB PNG (#247) put through either once a frame stutters the whole
    /// client, so these are numbers a test holds down rather than statistics.
    pub fn encodes(&self) -> usize {
        self.encodes
    }

    pub fn decodes(&self) -> usize {
        self.decodes
    }

    /// How many attachments have escape bytes standing on the terminal right now. A drawn image
    /// is not in the buffer — its cells are `Skip` — so this is the only record that the pixels
    /// are there, and [`clear`](Self::clear) is what takes them down.
    pub fn drawn(&self) -> usize {
        self.slots.iter().filter(|s| s.placed.is_some()).count()
    }

    /// Take in whatever the fetches have landed. [`draw`](Self::draw) does this itself; a caller
    /// asking [`offer`](Self::offer) first calls it.
    pub fn collect(&mut self) {
        while let Ok(landed) = self.inbox.try_recv() {
            self.land(&landed.pane, &landed.id, landed.landing);
        }
    }

    /// `GET /api/attachment/{node}/{local}/{id}` with the device token, 8 MiB ceiling. A `404`
    /// is expected rather than an error state: an id names a record in a transcript and stops
    /// resolving when the transcript is rewritten.
    pub fn request(&mut self, pane: &str, attachment: &Attachment) {
        if attachment.id.is_empty() || self.at(pane, &attachment.id).is_some() {
            return;
        }
        self.slots.push(Slot {
            pane: pane.to_string(),
            id: attachment.id.clone(),
            kind: attachment.kind.clone(),
            mime: attachment.mime.clone(),
            bytes: attachment.bytes,
            name: attachment.name.clone(),
            landing: None,
            decoded: None,
            encoded: None,
            placed: None,
        });
        let id = attachment.id.clone();
        if attachment.bytes.is_some_and(|n| n > MAX_BYTES) {
            self.land(pane, &id, Landing::TooLarge);
            return;
        }
        if !self.start(pane.to_string(), id.clone()) {
            self.land(pane, &id, Landing::Gone);
        }
    }

    /// The second id form — a path a tool call named, for a picture whose record has been
    /// rewritten out from under its id.
    ///
    /// **Only a device that may send input may ask for one**, so the affordance is gated on the
    /// live role rather than on the one the greeting carried; a read-only device gets `403` from
    /// the node and this returns `None` before it spends the round trip. The record form keeps
    /// its looser gate: looking at a screenshot somebody pasted is reading.
    pub fn request_file(&mut self, pane: &str, path: &str, role: Role) -> Option<String> {
        if !role.writes() {
            return None;
        }
        let id = file_id(path);
        let mime = image_mime(path);
        self.request(
            pane,
            &Attachment {
                id: id.clone(),
                kind: match mime {
                    Some(_) => "image".into(),
                    None => "file".into(),
                },
                mime: mime.map(str::to_string),
                bytes: None,
                name: Path::new(path)
                    .file_name()
                    .and_then(|n| n.to_str())
                    .map(str::to_string),
            },
        );
        Some(id)
    }

    /// What this attachment is, for a caller drawing the block around it. `None` means nothing
    /// ever asked for it.
    pub fn offer(&self, pane: &str, id: &str) -> Option<Offer<'_>> {
        let slot = self.at(pane, id).map(|at| &self.slots[at])?;
        Some(Offer {
            kind: &slot.kind,
            mime: slot.served().or(slot.mime.as_deref()),
            bytes: slot.data().map(|d| d.len() as u64).or(slot.bytes),
            name: slot.name.as_deref(),
            ready: slot.data().is_some(),
            inline: slot.inline() && self.protocol != Protocol::None,
            too_large: matches!(slot.landing, Some(Landing::TooLarge)),
        })
    }

    pub fn save(&self, pane: &str, id: &str, to: &Path) -> std::io::Result<()> {
        let data = self
            .at(pane, id)
            .and_then(|at| self.slots[at].data())
            .ok_or_else(|| std::io::Error::other("nothing has landed for this attachment"))?;
        std::fs::write(to, data)
    }

    /// The escape sequence this terminal would be handed for the attachment, scaled into `area`.
    /// Cached against the cell footprint, so a steady frame costs a lookup.
    pub fn encode(&mut self, area: Rect, pane: &str, id: &str) -> Option<&[u8]> {
        let at = self.ready(area, pane, id)?;
        self.slots[at].encoded.as_ref().map(|e| e.payload.as_slice())
    }

    /// `false` means nothing has landed and the caller renders the `[image · png]` marker text
    /// that is already in the block — which is what a client ignoring `att` shows today.
    pub fn draw(&mut self, buf: &mut Buffer, area: Rect, pane: &str, id: &str) -> bool {
        self.collect();
        let area = self.confined(buf.area, area);
        let Some(at) = self.ready(area, pane, id) else {
            return false;
        };
        let Some(encoded) = self.slots[at].encoded.as_ref() else {
            return false;
        };
        let (cols, rows) = encoded.cells;
        let placement = (area.x, area.y, cols, rows);
        if self.slots[at].placed != Some(placement) {
            place(area.x, area.y, &encoded.payload);
            self.slots[at].placed = Some(placement);
        }
        for y in area.y..area.y.saturating_add(rows) {
            for x in area.x..area.x.saturating_add(cols) {
                if let Some(cell) = buf.cell_mut(Position::new(x, y)) {
                    cell.set_symbol(" ");
                    cell.set_diff_option(CellDiffOption::Skip);
                }
            }
        }
        true
    }

    /// Forget every placement, so the next [`draw`](Self::draw) writes its escape again.
    ///
    /// A drawn image is not in the buffer — its cells are `skip`, so ratatui's diff has nothing
    /// to repaint them from and the pixels outlive the view that put them there. A caller tearing
    /// a view down clears the terminal and calls this.
    pub fn clear(&mut self) {
        if self.protocol == Protocol::Kitty {
            let mut out = std::io::stdout().lock();
            let _ = out.write_all(b"\x1b_Ga=d,d=A\x1b\\");
            let _ = out.flush();
        }
        for slot in &mut self.slots {
            slot.placed = None;
        }
    }

    /// A sixel and an iTerm2 inline image advance the cursor, so one that reaches the last row
    /// scrolls the whole TUI up by however many rows it took — and the frame the renderer just
    /// drew goes with it. Kitty's `C=1` leaves the cursor where it was and keeps its row.
    fn confined(&self, screen: Rect, area: Rect) -> Rect {
        if self.protocol == Protocol::Kitty || area.bottom() < screen.bottom() {
            return area;
        }
        Rect {
            height: area.height.saturating_sub(1),
            ..area
        }
    }

    fn at(&self, pane: &str, id: &str) -> Option<usize> {
        self.slots.iter().position(|s| s.pane == pane && s.id == id)
    }

    fn land(&mut self, pane: &str, id: &str, landing: Landing) {
        if let Some(at) = self.at(pane, id) {
            self.slots[at].landing = Some(landing);
            self.slots[at].decoded = None;
            self.slots[at].encoded = None;
            self.slots[at].placed = None;
        }
    }

    fn start(&mut self, pane: String, id: String) -> bool {
        let (Some(http), Ok(handle)) = (self.http.clone(), tokio::runtime::Handle::try_current()) else {
            return false;
        };
        let url = url(&self.origin, &pane, &id);
        let token = self.token.clone();
        let out = self.out.clone();
        handle.spawn(async move {
            let landing = fetch(&http, &url, &token).await;
            let _ = out.send(Landed { pane, id, landing });
        });
        true
    }

    /// The slot with an escape sequence sized for `area` in it, or `None` — which is every case
    /// the caller answers with the marker text: nothing asked for, still in flight, `404`, over
    /// the ceiling, a download rather than a picture, and a terminal that draws no images.
    fn ready(&mut self, area: Rect, pane: &str, id: &str) -> Option<usize> {
        if self.protocol == Protocol::None || area.width == 0 || area.height == 0 {
            return None;
        }
        let at = self.at(pane, id)?;
        if !self.slots[at].inline() {
            return None;
        }
        if self.slots[at].decoded.is_none() {
            let data = self.slots[at].data()?;
            self.slots[at].decoded = Some(image::load_from_memory(data).ok()?);
            self.decodes += 1;
        }
        let source = self.slots[at].decoded.as_ref()?;
        let want = footprint(source, area, self.cell);
        if want.0 == 0 || want.1 == 0 {
            return None;
        }
        if self.slots[at].encoded.as_ref().is_some_and(|e| e.cells == want) {
            return Some(at);
        }
        let payload = render(self.protocol, source, want, self.cell)?;
        self.encodes += 1;
        self.slots[at].encoded = Some(Encoded { cells: want, payload });
        self.slots[at].placed = None;
        Some(at)
    }
}

/// `base64url-no-pad("file" U+001F <absolute path>)`. The record form is five separator-delimited
/// fields, so the number of fields is what tells the two apart and every id an installed client
/// holds decodes to exactly what it decoded to before.
pub fn file_id(path: &str) -> String {
    URL_SAFE_NO_PAD.encode(format!("file\u{1f}{path}"))
}

/// The pane id's slash is sent **literally**, so the path is three segments and not two. A path of
/// any other shape is refused by the node rather than guessed at, and percent-encoding the slash
/// makes every fetch a `404`.
fn url(origin: &str, pane: &str, id: &str) -> String {
    format!("{origin}/api/attachment/{pane}/{id}")
}

/// What an extension is worth, which is the only thing a path in a tool call offers. `.svg` and
/// `.html` are deliberately absent: the node will not render them inline either.
pub fn image_mime(path: &str) -> Option<&'static str> {
    let extension = Path::new(path).extension()?.to_str()?.to_ascii_lowercase();
    Some(match extension.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "avif" => "image/avif",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        _ => return None,
    })
}

/// The in-band answer first, the name table second, and `None` for a terminal that said neither —
/// never a guess, because a guess writes escape bytes at somebody who will see them as mojibake.
pub fn protocol_for(caps: Caps, host: Option<&str>) -> Protocol {
    if caps.kitty_graphics {
        return Protocol::Kitty;
    }
    if caps.sixel {
        return Protocol::Sixel;
    }
    named(host)
}

/// The fallback, and only for names that have been measured or that the emulator's own
/// documentation names. `ghostty 1.3.1-arch2.2`, `kitty(0.48.2)` and `Konsole 26.04.3` are the
/// exact strings #291 read back from `CSI >0q`.
///
/// **A row here goes stale before an emulator does**, which is the whole reason it is second:
/// konsole 26.04.3 answers kitty's graphics query, so it never reaches its own row.
fn named(host: Option<&str>) -> Protocol {
    let name = host.unwrap_or_default().to_ascii_lowercase();
    if name.starts_with("kitty") || name.starts_with("ghostty") {
        return Protocol::Kitty;
    }
    if name.starts_with("konsole") {
        return Protocol::Sixel;
    }
    if name.starts_with("iterm2") || name.starts_with("wezterm") {
        return Protocol::Iterm2;
    }
    Protocol::None
}

async fn fetch(http: &reqwest::Client, url: &str, token: &str) -> Landing {
    let Ok(mut response) = http.get(url).bearer_auth(token).send().await else {
        return Landing::Gone;
    };
    match response.status().as_u16() {
        200 => {}
        413 => return Landing::TooLarge,
        _ => return Landing::Gone,
    }
    if response.content_length().is_some_and(|n| n > MAX_BYTES) {
        return Landing::TooLarge;
    }
    let mime = response
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.split(';').next().unwrap_or(v).trim().to_ascii_lowercase());
    let mut data: Vec<u8> = Vec::new();
    loop {
        match response.chunk().await {
            Ok(Some(chunk)) => {
                if data.len() as u64 + chunk.len() as u64 > MAX_BYTES {
                    return Landing::TooLarge;
                }
                data.extend_from_slice(&chunk);
            }
            Ok(None) => break,
            Err(_) => return Landing::Gone,
        }
    }
    // A `200` always has a body: an attachment with no bytes in it is a `404`, so an empty one is
    // a truncation and the marker text is the honest answer.
    match data.is_empty() {
        true => Landing::Gone,
        false => Landing::Bytes { data, mime },
    }
}

/// The cells this picture takes inside `area`, never more than it and never scaled up past its
/// own pixels. A terminal cell is not square, so the aspect is worked in pixels and only then
/// divided back into cells (#291 measured 8x15 on konsole).
fn footprint(source: &DynamicImage, area: Rect, cell: (u16, u16)) -> (u16, u16) {
    let (w, h) = (source.width().max(1) as f64, source.height().max(1) as f64);
    let (cw, ch) = (cell.0.max(1) as f64, cell.1.max(1) as f64);
    let room = (area.width as f64 * cw, area.height as f64 * ch);
    let scale = (room.0 / w).min(room.1 / h).min(1.0);
    let cols = ((w * scale) / cw).ceil().max(1.0) as u16;
    let rows = ((h * scale) / ch).ceil().max(1.0) as u16;
    (cols.min(area.width), rows.min(area.height))
}

fn render(protocol: Protocol, source: &DynamicImage, cells: (u16, u16), cell: (u16, u16)) -> Option<Vec<u8>> {
    let pixels = (
        u32::from(cells.0) * u32::from(cell.0.max(1)),
        u32::from(cells.1) * u32::from(cell.1.max(1)),
    );
    let scaled = match source.width() > pixels.0 || source.height() > pixels.1 {
        true => source.resize(pixels.0, pixels.1, FilterType::Triangle),
        false => source.clone(),
    };
    match protocol {
        Protocol::None => None,
        Protocol::Kitty => Some(kitty(&png(&scaled)?, cells)),
        Protocol::Iterm2 => Some(iterm2(&png(&scaled)?, cells)),
        Protocol::Sixel => sixel(&scaled),
    }
}

fn png(source: &DynamicImage) -> Option<Vec<u8>> {
    let mut out = std::io::Cursor::new(Vec::new());
    source
        .write_to(&mut out, image::ImageFormat::Png)
        .ok()
        .map(|()| out.into_inner())
}

/// `f=100` is a PNG the terminal decodes itself; `c`/`r` are the cells it is scaled into and
/// `C=1` stops it moving the cursor, which would otherwise scroll the whole TUI.
fn kitty(data: &[u8], cells: (u16, u16)) -> Vec<u8> {
    let encoded = STANDARD.encode(data);
    let mut out = Vec::with_capacity(encoded.len() + 64);
    let chunks: Vec<&str> = encoded
        .as_bytes()
        .chunks(KITTY_CHUNK)
        .map(|c| std::str::from_utf8(c).unwrap_or_default())
        .collect();
    for (n, chunk) in chunks.iter().enumerate() {
        let more = u8::from(n + 1 < chunks.len());
        match n {
            0 => {
                let _ = write!(
                    out,
                    "\x1b_Ga=T,f=100,c={},r={},C=1,q=2,m={more};{chunk}\x1b\\",
                    cells.0, cells.1
                );
            }
            _ => {
                let _ = write!(out, "\x1b_Gm={more};{chunk}\x1b\\");
            }
        }
    }
    out
}

fn iterm2(data: &[u8], cells: (u16, u16)) -> Vec<u8> {
    let encoded = STANDARD.encode(data);
    let mut out = Vec::with_capacity(encoded.len() + 64);
    let _ = write!(
        out,
        "\x1b]1337;File=inline=1;size={};width={};height={};preserveAspectRatio=1:{encoded}\x07",
        data.len(),
        cells.0,
        cells.1
    );
    out
}

fn sixel(source: &DynamicImage) -> Option<Vec<u8>> {
    let rgba = source.to_rgba8();
    let (w, h) = (rgba.width() as usize, rgba.height() as usize);
    icy_sixel::SixelImage::from_rgba(rgba.into_raw(), w, h)
        .encode()
        .ok()
        .map(String::into_bytes)
}

/// The escape goes to the terminal rather than into the buffer, so the cursor is parked at the
/// top left of the rect and put back where the renderer left it.
fn place(x: u16, y: u16, payload: &[u8]) {
    let mut out = std::io::stdout().lock();
    let _ = write!(out, "\x1b7\x1b[{};{}H", y.saturating_add(1), x.saturating_add(1));
    let _ = out.write_all(payload);
    let _ = out.write_all(b"\x1b8");
    let _ = out.flush();
}

/// What the terminal claimed, read out of one batch of answers.
///
/// **Kitty answers its own graphics query** — `\x1b_Gi=31;OK\x1b\\` — and **DA1's attribute 4 is
/// sixel**, which is the emulator's claim about what it can do rather than this client's opinion
/// of its name. Neither is a name table, and both cover an emulator no table has heard of.
pub fn caps_in(answer: &str) -> Caps {
    Caps {
        kitty_graphics: answer.contains("_Gi=31;OK"),
        sixel: da1(answer).is_some_and(|p| p.split(';').any(|a| a.trim() == "4")),
    }
}

fn da1(answer: &str) -> Option<&str> {
    let rest = &answer[answer.find("\x1b[?")? + 3..];
    Some(&rest[..rest.find('c')?])
}

/// `CSI 6 ; height ; width t` — the cell in pixels, which is what a sixel has to be scaled
/// against because a terminal cell is not square (#291).
pub fn cell_in(answer: &str) -> Option<(u16, u16)> {
    let rest = &answer[answer.find("\x1b[6;")? + 2..];
    let mut parts = rest[..rest.find('t')?].split(';');
    parts.next();
    let h: u16 = parts.next()?.trim().parse().ok()?;
    let w: u16 = parts.next()?.trim().parse().ok()?;
    Some((w, h))
}
