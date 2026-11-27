use super::{BlockKind, Event, ProgramRecord, ProgramState};
use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};

/// The whole sequence is 4096 bytes, `OSC 7501 ;` and a two-byte terminator included.
const MAX_BODY: usize = 4096 - "\x1b]7501;".len() - 2;
const MAX_KEY: usize = 16;
const MAX_MSG: (usize, usize) = (2732, 2048);
const MAX_TITLE: (usize, usize) = (256, 192);
const MAX_APP: usize = 32;
const MAX_ID: usize = 128;
const MAX_SEGMENT: usize = 32;
const MAX_DEPTH: usize = 8;

const BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// One `OSC 7501` body, strictly. `None` is a report that does nothing: not UTF-8, over a limit,
/// bad base64 or a control character in decoded text, an id or app outside the grammar, a state
/// nobody defined. A pair that is not `key=value` is skipped and the rest read; the last of a
/// repeated key wins.
pub fn parse(body: &[u8]) -> Option<Event> {
    if body.len() > MAX_BODY {
        return None;
    }
    let body = std::str::from_utf8(body).ok()?;
    let mut pairs: Vec<(&str, &str)> = Vec::new();
    for pair in body.split(':') {
        let Some((key, value)) = pair.split_once('=') else {
            continue;
        };
        if !key_ok(key) {
            continue;
        }
        if !value_ok(value) {
            if matches!(key, "id" | "app" | "msg" | "title") {
                return None;
            }
            continue;
        }
        pairs.push((key, value));
    }
    let get = |key: &str| pairs.iter().rev().find(|(k, _)| *k == key).map(|(_, v)| *v);
    let id = match get("id") {
        Some(id) if id_ok(id) => Some(id.to_string()),
        Some(_) => return None,
        None => None,
    };
    let state = match get("state")? {
        "clear" => return Some(Event::Clear(id)),
        "idle" => ProgramState::Idle,
        "working" => ProgramState::Working,
        "blocked" => ProgramState::Blocked,
        "done" => ProgramState::Done,
        "error" => ProgramState::Error,
        _ => return None,
    };
    let app = match get("app") {
        Some(app) if app_ok(app) => Some(app.to_string()),
        Some(_) => return None,
        None => None,
    };
    let text = |key, (encoded, decoded)| match get(key) {
        Some(value) => decode(value, encoded, decoded).map(|t| (!t.is_empty()).then_some(t)),
        None => Some(None),
    };
    let title = text("title", MAX_TITLE)?;
    let msg = text("msg", MAX_MSG)?;
    let running = matches!(state, ProgramState::Working | ProgramState::Blocked);
    Some(Event::Set(ProgramRecord {
        id: id.unwrap_or_default(),
        state,
        kind: get("kind")
            .filter(|_| state == ProgramState::Blocked)
            .and_then(|k| match k {
                "permission" => Some(BlockKind::Permission),
                "question" => Some(BlockKind::Question),
                "auth" => Some(BlockKind::Auth),
                _ => None,
            }),
        progress: get("progress")
            .filter(|_| running)
            .filter(|p| p.bytes().all(|b| b.is_ascii_digit()))
            .and_then(|p| p.parse().ok())
            .filter(|p| *p <= 100),
        app,
        title,
        msg,
    }))
}

fn key_ok(key: &str) -> bool {
    (1..=MAX_KEY).contains(&key.len()) && key.bytes().all(|b| b.is_ascii_lowercase())
}

fn value_ok(value: &str) -> bool {
    value
        .bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_.,+/=-".contains(&b))
}

fn word_ok(word: &str) -> bool {
    word.bytes()
        .all(|b| b.is_ascii_alphanumeric() || b"_.+-".contains(&b))
}

fn app_ok(app: &str) -> bool {
    (1..=MAX_APP).contains(&app.len()) && word_ok(app)
}

fn id_ok(id: &str) -> bool {
    id.len() <= MAX_ID
        && id.split('/').count() <= MAX_DEPTH
        && id
            .split('/')
            .all(|segment| (1..=MAX_SEGMENT).contains(&segment.len()) && word_ok(segment))
}

/// Free text: base64 UTF-8 whose decoded form holds no control character. `None` discards the
/// report.
fn decode(value: &str, encoded: usize, decoded: usize) -> Option<String> {
    if value.len() > encoded || (value.ends_with('=') && !value.len().is_multiple_of(4)) {
        return None;
    }
    let text = String::from_utf8(BASE64.decode(value).ok()?).ok()?;
    (text.len() <= decoded && !text.chars().any(char::is_control)).then_some(text)
}
