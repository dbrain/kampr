use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::error::JournalError;
use crate::model::{Page, Turn};
use crate::store::TurnStore;

pub trait TranscriptParser: Send {
    fn push_line(&mut self, line: &str);
    fn reset(&mut self);
    fn store(&self) -> &TurnStore;
    fn store_mut(&mut self) -> &mut TurnStore;
}

pub trait Journal: Send {
    /// Turns added or revised since the last call. Empty when the transcript has not grown.
    fn poll(&mut self) -> Result<Vec<Turn>, JournalError>;
    fn page_before(&self, before: Option<&str>, limit: usize) -> Page;
    fn path(&self) -> &Path;
}

pub struct FileJournal {
    path: PathBuf,
    offset: u64,
    partial: Vec<u8>,
    parser: Box<dyn TranscriptParser>,
}

impl FileJournal {
    pub fn new(path: PathBuf, parser: Box<dyn TranscriptParser>) -> Self {
        Self {
            path,
            offset: 0,
            partial: Vec::new(),
            parser,
        }
    }

    fn rewind(&mut self) {
        self.offset = 0;
        self.partial.clear();
        self.parser.reset();
    }
}

impl Journal for FileJournal {
    fn poll(&mut self) -> Result<Vec<Turn>, JournalError> {
        let mut file = File::open(&self.path)?;
        let len = file.metadata()?.len();
        if len < self.offset {
            self.rewind();
        }
        if len > self.offset {
            file.seek(SeekFrom::Start(self.offset))?;
            let mut fresh = Vec::with_capacity((len - self.offset) as usize);
            let read = file.read_to_end(&mut fresh)? as u64;
            self.offset += read;
            self.partial.extend_from_slice(&fresh);
        }

        // A transcript is appended to while we read it, so the tail may be a torn line. Only
        // whole lines are parsed; the remainder waits for the next poll.
        while let Some(at) = self.partial.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.partial.drain(..=at).collect();
            let text = String::from_utf8_lossy(&line[..at]);
            let text = text.trim_end_matches('\r');
            if !text.is_empty() {
                self.parser.push_line(text);
            }
        }

        Ok(self.parser.store_mut().drain_changed())
    }

    fn page_before(&self, before: Option<&str>, limit: usize) -> Page {
        let store = self.parser.store();
        let turns = store.turns();
        let end = before.and_then(|id| store.position(id)).unwrap_or(turns.len());
        let start = end.saturating_sub(limit);
        let slice = turns[start..end].to_vec();
        Page {
            cursor: slice.first().map(|t| t.id.clone()),
            more: start > 0,
            turns: slice,
        }
    }

    fn path(&self) -> &Path {
        &self.path
    }
}
