use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::{STANDARD, STANDARD_NO_PAD, URL_SAFE_NO_PAD};

use crate::adapter::Registry;
use crate::error::JournalError;
use crate::model::Attachment;
use crate::root::TranscriptRoot;

pub const IMAGE: &str = "image";

/// Anything a client should offer as a download rather than try to render.
pub const FILE: &str = "file";

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
        match Source::decode(id)? {
            Source::Record(locator) => Ok(locator),
            Source::File(_) => Err(refuse()),
        }
    }
}

/// The tag a file id carries in the first field. Nothing else is tagged: a record id is five
/// fields and has been since the first build that minted one, so arity is what tells the two
/// apart and an installed client's id keeps decoding to exactly what it decoded to before.
const FILE_TAG: &str = "file";

/// A plain path on the node's filesystem, with no transcript behind it and no working directory
/// to resolve against.
///
/// This is the form a **client** builds: it saw a path in a tool call and wants the bytes, and
/// nothing minted an id for it. Which is why the node gates it on a device that may send input —
/// a device that can type into a terminal can already `cat` the file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileRef {
    pub path: PathBuf,
}

impl FileRef {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{FILE_TAG}{SEP}{}", self.path.display()))
    }

    /// The path with a **leading** `~/` resolved against `home`, and nothing else touched.
    ///
    /// `~user/x` is deliberately not expanded: guessing at another account's home would hand over
    /// a different user's files under a gate that reasoned about this one, so it falls through as
    /// a relative path and is refused. A `~` anywhere but the front is an ordinary character in a
    /// filename and stays one.
    ///
    /// The separators after `~` belong to the prefix rather than starting a new root — `Path::join`
    /// with an absolute argument *replaces*, so without the trim `~//etc/hosts` would resolve to
    /// `/etc/hosts` rather than to one inside the home.
    fn against(&self, home: &Path) -> PathBuf {
        let Some(text) = self.path.to_str() else {
            return self.path.clone();
        };
        let Some(rest) = text.strip_prefix("~/") else {
            // A bare `~` is the home itself. `join("")` would leave a trailing separator, and a
            // path that ends in one cannot name a regular file.
            return match text {
                "~" => home.to_path_buf(),
                _ => self.path.clone(),
            };
        };
        home.join(rest.trim_start_matches('/'))
    }

    /// The bytes at that path, or the same refusal everything else here gives.
    ///
    /// `home` is the node's own — `Config::journal_home()`, which is the operator's home rather
    /// than the process's whenever the two differ. An empty one expands `~/x` to the relative `x`
    /// and the check below refuses it, which is the honest answer on a machine with no `$HOME`.
    ///
    /// **`stat` before `open`**: opening a fifo with no writer on the other end blocks for ever,
    /// and a path naming one has to be refused rather than waited on. The handle is stat'd again
    /// afterwards, so the size the ceiling is applied to is the size of the file that was
    /// actually opened.
    pub fn fetch(&self, home: &Path) -> Result<Fetched, JournalError> {
        let path = self.against(home);
        if !path.is_absolute() {
            return Err(refuse());
        }
        if !std::fs::metadata(&path).map_err(|_| refuse())?.is_file() {
            return Err(refuse());
        }
        let mut file = std::fs::File::open(&path).map_err(|_| refuse())?;
        let stat = file.metadata().map_err(|_| refuse())?;
        if !stat.is_file() {
            return Err(refuse());
        }
        if stat.len() > MAX_BYTES {
            return Err(JournalError::TooLarge(stat.len()));
        }
        let mut data = Vec::with_capacity(stat.len().min(MAX_BYTES) as usize);
        (&mut file)
            .take(MAX_BYTES + 1)
            .read_to_end(&mut data)
            .map_err(|_| refuse())?;
        // A file that grew between the two reads is refused rather than truncated: a body short of
        // what it claims is the shape of a wrong answer that looks right.
        if data.len() as u64 > MAX_BYTES {
            return Err(JournalError::TooLarge(data.len() as u64));
        }
        if data.is_empty() {
            return Err(refuse());
        }
        let mime = image_mime(&path);
        Ok(Fetched {
            kind: match mime {
                Some(_) => IMAGE.to_string(),
                None => FILE.to_string(),
            },
            mime: mime.map(str::to_string),
            name: path.file_name().map(|n| n.to_string_lossy().into_owned()),
            data,
        })
    }
}

/// The two things an attachment id can name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Source {
    Record(Locator),
    File(FileRef),
}

impl Source {
    pub fn encode(&self) -> String {
        match self {
            Self::Record(locator) => locator.encode(),
            Self::File(file) => file.encode(),
        }
    }

    pub fn decode(id: &str) -> Result<Self, JournalError> {
        if id.is_empty() || id.len() > 4096 {
            return Err(refuse());
        }
        let raw = URL_SAFE_NO_PAD.decode(id).map_err(|_| refuse())?;
        let text = String::from_utf8(raw).map_err(|_| refuse())?;
        let parts: Vec<&str> = text.split(SEP).collect();
        match parts.as_slice() {
            [agent, path, offset, index, bytes] => Ok(Self::Record(Locator {
                agent: (*agent).to_string(),
                path: (*path).to_string(),
                offset: offset.parse().map_err(|_| refuse())?,
                index: index.parse().map_err(|_| refuse())?,
                bytes: bytes.parse().map_err(|_| refuse())?,
            })),
            [tag, path] if *tag == FILE_TAG && !path.is_empty() => Ok(Self::File(FileRef::new(*path))),
            _ => Err(refuse()),
        }
    }
}

fn refuse() -> JournalError {
    JournalError::NotFound(String::new())
}

/// What an extension is worth, and it is the only thing a file on disk offers. Deliberately short:
/// a type that is not on it is a download, which is the safe answer for `text/html` and for the
/// scriptable document `image/svg+xml` names.
fn image_mime(path: &Path) -> Option<&'static str> {
    let extension = path.extension()?.to_str()?.to_ascii_lowercase();
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
