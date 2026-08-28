use std::path::{Path, PathBuf};

use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;

use crate::error::JournalError;
use crate::root::TranscriptRoot;

/// Where a harness writes the transcripts of the conversations one of its own launched.
pub const LAUNCHED: &str = "subagents";

/// Not a path separator, not legal in a JSON string, and not something a filename can hold.
const SEP: char = '\u{1f}';

const TAG: &str = "sub";

/// A conversation another one launched, named the way an attachment is: the harness that wrote it
/// and the path inside that harness's root.
///
/// Nothing has to be held in memory for one of these to resolve again — the transcript on disk is
/// already the store — so a handle survives a node restart, and it stops resolving exactly when
/// the file it names is no longer there.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubRef {
    pub agent: String,
    pub path: String,
}

impl SubRef {
    pub fn new(agent: &str, root: &TranscriptRoot, path: &Path) -> Self {
        let relative = path.strip_prefix(root.path()).unwrap_or(path);
        Self {
            agent: agent.to_string(),
            path: relative.to_string_lossy().into_owned(),
        }
    }

    pub fn encode(&self) -> String {
        URL_SAFE_NO_PAD.encode(format!("{TAG}{SEP}{}{SEP}{}", self.agent, self.path))
    }

    pub fn decode(id: &str) -> Result<Self, JournalError> {
        if id.is_empty() || id.len() > 4096 {
            return Err(refuse());
        }
        let raw = URL_SAFE_NO_PAD.decode(id).map_err(|_| refuse())?;
        let text = String::from_utf8(raw).map_err(|_| refuse())?;
        match text.split(SEP).collect::<Vec<_>>().as_slice() {
            [tag, agent, path] if *tag == TAG && !agent.is_empty() && !path.is_empty() => Ok(Self {
                agent: (*agent).to_string(),
                path: (*path).to_string(),
            }),
            _ => Err(refuse()),
        }
    }
}

/// Everything one session wrote, whichever of its transcripts is in hand.
///
/// A handle is proved against this and not against the adapter's root alone, which holds every
/// project on the machine: a pane may open what its own session launched and nothing else. Walking
/// back out of [`LAUNCHED`] is what makes the proof hold at any depth, so a nested launch resolves
/// whether the caller anchors on the pane's transcript or on the one it is reading.
pub fn tree(transcript: &Path) -> PathBuf {
    let mut tree = transcript.with_extension("");
    while tree
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|n| n == LAUNCHED)
    {
        let Some(up) = tree.parent().and_then(Path::parent) else {
            break;
        };
        tree = up.to_path_buf();
    }
    tree
}

fn refuse() -> JournalError {
    JournalError::NotFound(String::new())
}
