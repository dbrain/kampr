//! `OSC 7577`, Rex's program resume protocol (docs/25): a program tells the terminal the command that
//! brings it back. Pure: one report and the session's token in, a verdict out.

use base64::Engine as _;
use base64::engine::{DecodePaddingMode, GeneralPurpose, GeneralPurposeConfig};
use serde::{Deserialize, Serialize};
use sha2::{Digest as _, Sha256};

pub const TOKEN_ENV: &str = "OSC7577_TOKEN";
/// How far a report's `time` may be from this machine's clock when it is first read live.
pub const WINDOW_SECS: i64 = 60;
const MAX_BODY: usize = 8192;
const MAX_ARGV: usize = 128;
const MAX_CWD: usize = 4096;
/// Everything this implements, which is what `need=` may name.
const IMPLEMENTED: [&str; 7] = ["pid", "time", "mac", "argv", "cwd", "clear", "need"];

const BASE64: GeneralPurpose = GeneralPurpose::new(
    &base64::alphabet::STANDARD,
    GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// A session's secret: 22 to 64 characters of `[A-Za-z0-9_-]`, made once per session and put in its
/// first process's environment.
#[derive(Clone, PartialEq, Eq)]
pub struct Token(String);

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Token(..)")
    }
}

impl Token {
    pub fn generate() -> Result<Self, getrandom::Error> {
        let mut bytes = [0u8; 16];
        getrandom::fill(&mut bytes)?;
        Ok(Self(
            base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes),
        ))
    }

    pub fn parse(text: &str) -> Option<Self> {
        let ok = (22..=64).contains(&text.len())
            && text
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
        ok.then(|| Self(text.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// What a program asked the terminal to remember: `argv` run as a list, never through a shell.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Command {
    pub argv: Vec<String>,
    pub cwd: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    Set(Command),
    Clear,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Report {
    pub pid: u32,
    pub time: i64,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Verdict {
    /// Nothing happens: no or wrong `mac`, a `mac` not last, no `time`, a `time` outside the
    /// window, no `pid`, or neither `argv` nor `clear`.
    Ignored,
    /// Authentic and fresh but invalid, which removes the owner's records.
    Rejected {
        pid: u32,
    },
    Accepted(Report),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// First read as it is written: `time` must be within [`WINDOW_SECS`] of `now`.
    Live { now: i64 },
    /// Read again from a log after a restart, when `time` is long past by construction.
    Replayed,
}

pub fn read(body: &[u8], token: &Token, freshness: Freshness) -> Verdict {
    let Ok(body) = std::str::from_utf8(body) else {
        return Verdict::Ignored;
    };
    if body.len() > MAX_BODY {
        return Verdict::Ignored;
    }
    let Some((signed, mac)) = body.rsplit_once(":mac=") else {
        return Verdict::Ignored;
    };
    if !authentic(signed, mac, token) {
        return Verdict::Ignored;
    }
    let Some(pairs) = pairs(signed) else {
        return match owner(signed) {
            Some(pid) => Verdict::Rejected { pid },
            None => Verdict::Ignored,
        };
    };
    let (Some(pid), Some(time)) = (number(&pairs, "pid"), number(&pairs, "time")) else {
        return Verdict::Ignored;
    };
    let (Ok(pid), Ok(time)) = (u32::try_from(pid), i64::try_from(time)) else {
        return Verdict::Ignored;
    };
    if let Freshness::Live { now } = freshness
        && (time - now).abs() > WINDOW_SECS
    {
        return Verdict::Ignored;
    }
    match action(&pairs) {
        Ok(Some(action)) => Verdict::Accepted(Report { pid, time, action }),
        Ok(None) => Verdict::Ignored,
        Err(Invalid) => Verdict::Rejected { pid },
    }
}

fn authentic(signed: &str, mac: &str, token: &Token) -> bool {
    if mac.len() != 64 || !mac.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f')) {
        return false;
    }
    let mut hash = Sha256::new();
    hash.update(signed.as_bytes());
    hash.update(b":");
    hash.update(token.as_str().as_bytes());
    let want: String = hash.finalize().iter().map(|b| format!("{b:02x}")).collect();
    want.bytes().zip(mac.bytes()).fold(0u8, |d, (a, b)| d | (a ^ b)) == 0
}

/// `None` when a pair is malformed or a key repeats, which the spec rejects whole.
fn pairs(signed: &str) -> Option<Vec<(&str, &str)>> {
    let mut out: Vec<(&str, &str)> = Vec::new();
    for pair in signed.split(':') {
        let (key, value) = pair.split_once('=')?;
        let value_ok = value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"_.,+/=-".contains(&b));
        if key.is_empty()
            || !key.bytes().all(|b| b.is_ascii_lowercase())
            || !value_ok
            || out.iter().any(|(k, _)| *k == key)
        {
            return None;
        }
        out.push((key, value));
    }
    Some(out)
}

/// The owner of a report whose pairs would not parse, so that its records can still be removed.
fn owner(signed: &str) -> Option<u32> {
    signed
        .split(':')
        .find_map(|pair| pair.strip_prefix("pid="))
        .filter(|pid| digits(pid))
        .and_then(|pid| pid.parse().ok())
}

fn digits(text: &str) -> bool {
    !text.is_empty() && !text.starts_with('0') && text.bytes().all(|b| b.is_ascii_digit())
}

fn number(pairs: &[(&str, &str)], key: &str) -> Option<u64> {
    let (_, value) = pairs.iter().find(|(k, _)| *k == key)?;
    digits(value).then(|| value.parse().ok())?
}

struct Invalid;

fn action(pairs: &[(&str, &str)]) -> Result<Option<Action>, Invalid> {
    let get = |key: &str| pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| *v);
    if let Some(need) = get("need")
        && need.split(',').any(|key| !IMPLEMENTED.contains(&key))
    {
        return Err(Invalid);
    }
    match (get("argv"), get("clear")) {
        (Some(_), Some(_)) => Err(Invalid),
        (None, Some("1")) => Ok(Some(Action::Clear)),
        (None, Some(_)) => Err(Invalid),
        (None, None) => Ok(None),
        (Some(argv), None) => {
            let argv = argv_of(argv).ok_or(Invalid)?;
            let cwd = match get("cwd") {
                Some(cwd) => Some(path_of(cwd).ok_or(Invalid)?),
                None => None,
            };
            Ok(Some(Action::Set(Command { argv, cwd })))
        }
    }
}

fn argv_of(value: &str) -> Option<Vec<String>> {
    if value.is_empty() {
        return None;
    }
    let argv: Vec<String> = value.split(',').map(text).collect::<Option<_>>()?;
    let program = argv.first()?;
    let named = !program.is_empty() && (program.starts_with('/') || !program.contains('/'));
    (named && argv.len() <= MAX_ARGV).then_some(argv)
}

fn path_of(value: &str) -> Option<String> {
    let path = text(value)?;
    (path.starts_with('/') && path.len() <= MAX_CWD).then_some(path)
}

/// One base64 element of UTF-8 with no control character; padding, where present, is exact.
fn text(element: &str) -> Option<String> {
    if element.ends_with('=') && !element.len().is_multiple_of(4) {
        return None;
    }
    let text = String::from_utf8(BASE64.decode(element).ok()?).ok()?;
    (!text.chars().any(char::is_control)).then_some(text)
}
