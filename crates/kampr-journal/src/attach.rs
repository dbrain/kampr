use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};

use crate::adapter::Registry;
use crate::error::JournalError;
use crate::model::Attachment;
use crate::root::TranscriptRoot;

pub const IMAGE: &str = "image";

/// The largest body this node will hand back for one attachment.
///
/// The biggest single attachment measured on a real machine is a Codex `view_image` output of
/// **3 034 194 base64 characters — 2.22 MB decoded** (probe #247), inside a rollout of 88.7 MB.
/// 8 MiB is between three and four times that, so nothing measured is refused, and it is the
/// bound that matters: the length is read off the record's own base64 and checked *before*
/// anything is allocated, so a record claiming a gigabyte costs a comparison rather than a
/// gigabyte.
pub const MAX_BYTES: u64 = 8 * 1024 * 1024;

/// How much of the transcript one fetch will read looking for the end of its record. The largest
/// record measured is the 3 034 411-byte one above, and this is the ceiling the node already
/// applies to the largest thing it will read off any socket (`MAX_MESH_MESSAGE_BYTES`).
const MAX_RECORD_BYTES: u64 = 16 * 1024 * 1024;

/// Which transcript a parser is reading, so the ids it mints can be resolved again. The path is
/// relative to the adapter's root wherever it can be, so an id survives a root that moves.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Origin {
    pub agent: String,
    pub path: String,
}

impl Origin {
    pub fn new(agent: &str, root: &TranscriptRoot, path: &Path) -> Self {
        let relative = path.strip_prefix(root.path()).unwrap_or(path);
        Self {
            agent: agent.to_string(),
            path: relative.to_string_lossy().into_owned(),
        }
    }

    pub fn locate(&self, offset: u64, index: u32, bytes: u64) -> Locator {
        Locator {
            agent: self.agent.clone(),
            path: self.path.clone(),
            offset,
            index,
            bytes,
        }
    }
}

/// Where an attachment is, rather than what it is: the transcript, the byte the record starts at,
/// and which attachment of that record it is. The transcript on disk is already the store, so
/// nothing is copied and nothing has to be invalidated — an id simply stops resolving when the
/// record it names is no longer there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Locator {
    pub agent: String,
    pub path: String,
    pub offset: u64,
    pub index: u32,
    /// What the header said the body was. A transcript is a file that grows, and an offset into
    /// one that has been rewritten under it points at a record that is no longer the record the
    /// id was minted from — so the size is carried and checked, and a mismatch is a refusal
    /// rather than somebody else's picture arriving under this marker.
    pub bytes: u64,
}

/// Not a path separator, not legal in a JSON string, and not something a filename can hold.
const SEP: char = '\u{1f}';

impl Locator {
    pub fn encode(&self) -> String {
        let Self {
            agent,
            path,
            offset,
            index,
            bytes,
        } = self;
        URL_SAFE_NO_PAD.encode(format!("{agent}{SEP}{path}{SEP}{offset}{SEP}{index}{SEP}{bytes}"))
    }

    pub fn decode(id: &str) -> Result<Self, JournalError> {
        let refuse = || JournalError::NotFound(String::new());
        if id.is_empty() || id.len() > 4096 {
            return Err(refuse());
        }
        let raw = URL_SAFE_NO_PAD.decode(id).map_err(|_| refuse())?;
        let text = String::from_utf8(raw).map_err(|_| refuse())?;
        let mut parts = text.split(SEP);
        let (Some(agent), Some(path), Some(offset), Some(index), Some(bytes), None) = (
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
            parts.next(),
        ) else {
            return Err(refuse());
        };
        Ok(Self {
            agent: agent.to_string(),
            path: path.to_string(),
            offset: offset.parse().map_err(|_| refuse())?,
            index: index.parse().map_err(|_| refuse())?,
            bytes: bytes.parse().map_err(|_| refuse())?,
        })
    }
}

/// One attachment as it sits in a record, borrowed from the parsed record rather than copied: a
/// single `view_image` output is megabytes, and a parse that cloned it would pay that on every
/// record it walked past.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Att<'a> {
    pub kind: &'static str,
    pub mime: Option<&'a str>,
    pub name: Option<&'a str>,
    pub data: &'a str,
}

impl Att<'_> {
    pub fn header(&self, locator: Locator) -> Attachment {
        Attachment {
            id: locator.encode(),
            kind: self.kind.to_string(),
            mime: self.mime.map(str::to_string),
            bytes: Some(decoded_len(self.data)),
            name: self.name.map(str::to_string),
        }
    }

    pub fn fetch(&self) -> Result<Fetched, JournalError> {
        if decoded_len(self.data) > MAX_BYTES {
            return Err(JournalError::TooLarge(decoded_len(self.data)));
        }
        let data = STANDARD
            .decode(self.data)
            .or_else(|_| STANDARD_NO_PAD.decode(self.data))
            .map_err(|_| JournalError::NotFound(String::new()))?;
        // A 200 with nothing behind it is a client telling its operator the node answered with no
        // bytes at all, which is a true sentence about a broken route and a confusing one here.
        if data.is_empty() {
            return Err(JournalError::NotFound(String::new()));
        }
        Ok(Fetched {
            kind: self.kind.to_string(),
            mime: self.mime.map(str::to_string),
            name: self.name.map(str::to_string),
            data,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Fetched {
    pub kind: String,
    pub mime: Option<String>,
    pub name: Option<String>,
    pub data: Vec<u8>,
}

/// The exact length the base64 decodes to, without decoding it.
fn decoded_len(b64: &str) -> u64 {
    let padding = b64.bytes().rev().take_while(|b| *b == b'=').count();
    (b64.len().saturating_sub(padding) as u64) * 3 / 4
}

/// The bytes behind an `att.id`, or a refusal.
///
/// **Two independent checks, and both are load-bearing.** The id arrives from the network, so the
/// path inside it is resolved through the adapter's own [`TranscriptRoot`] — canonicalised, and
/// proved to be inside it, which is what stops `../`, an absolute path and a symlink pointing out.
/// That alone would still let a caller name *another* pane's transcript, which is inside the root
/// and perfectly readable, so the resolved file must also be the one this pane is on — a path the
/// node derived itself and the request had no say in.
pub fn fetch(journals: &Registry, id: &str, transcript: &Path) -> Result<Fetched, JournalError> {
    let locator = Locator::decode(id)?;
    let adapter = journals
        .get(&locator.agent)
        .ok_or_else(|| JournalError::NotFound(locator.agent.clone()))?;
    let resolved = adapter.root().contain(&locator.path)?;
    let expected = transcript
        .canonicalize()
        .map_err(|_| JournalError::NotFound(locator.path.clone()))?;
    if resolved != expected {
        return Err(JournalError::Escape(locator.path.clone()));
    }
    let record = read_record(&resolved, locator.offset)?;
    let found = adapter.attachment(&record, locator.index)?;
    if found.data.len() as u64 != locator.bytes {
        return Err(JournalError::NotFound(id.to_string()));
    }
    Ok(found)
}

/// The `index`th attachment of a record that has already been parsed, decoded.
pub fn nth<'a>(found: Vec<Att<'a>>, index: u32) -> Result<Fetched, JournalError> {
    found
        .into_iter()
        .nth(index as usize)
        .ok_or_else(|| JournalError::NotFound(index.to_string()))?
        .fetch()
}

/// The headers a record's attachments go on the wire as, in the order a parser meets them.
pub fn headers(origin: &Origin, offset: u64, found: &[Att<'_>]) -> Vec<Attachment> {
    found
        .iter()
        .enumerate()
        .map(|(i, att)| {
            let bytes = decoded_len(att.data);
            att.header(origin.locate(offset, i as u32, bytes))
        })
        .collect()
}

fn read_record(path: &Path, offset: u64) -> Result<String, JournalError> {
    let mut file = std::fs::File::open(path)?;
    if file.metadata()?.len() <= offset {
        return Err(JournalError::NotFound(offset.to_string()));
    }
    file.seek(SeekFrom::Start(offset))?;
    let mut line = Vec::new();
    BufReader::new(file.take(MAX_RECORD_BYTES))
        .read_until(b'\n', &mut line)
        .map_err(JournalError::Io)?;
    String::from_utf8(line).map_err(|_| JournalError::NotFound(offset.to_string()))
}
