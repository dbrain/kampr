use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::attach::Origin;
use crate::error::JournalError;
use crate::live::{self, ScreenReader};
use crate::model::{Block, Page, Turn};
use crate::store::TurnStore;

pub trait TranscriptParser: Send {
    /// `at` is the byte the record starts at, and it is half of what an attachment id is made of.
    fn push_line(&mut self, line: &str, at: u64);
    /// The default is a parser that mints no attachment ids, so it has nothing to mint them from.
    fn set_origin(&mut self, _origin: Origin) {}
    fn reset(&mut self);
    fn store(&self) -> &TurnStore;
    fn store_mut(&mut self) -> &mut TurnStore;
}

pub trait Journal: Send {
    /// Turns added or revised since the last call. Empty when the transcript has not grown.
    fn poll(&mut self) -> Result<Vec<Turn>, JournalError>;
    fn page_before(&self, before: Option<&str>, limit: usize) -> Page;
    fn path(&self) -> &Path;
    /// Every turn id this journal has produced. What a client is holding for this pane is a
    /// subset of it, which is what lets a conversation be taken off the screen when the pane
    /// moves to a different one.
    fn turn_ids(&self) -> Vec<String>;
    /// A best-effort turn for the message the harness is painting right now, checked against the
    /// transcript so that a message which has already been recorded is never previewed beside its
    /// own record.
    fn preview(&self, screen: &[&str]) -> Option<Turn>;
}

/// How far back a preview is checked. Claude writes an assistant record per message, so the newest
/// turn is usually the answer; a tool card can sit between the message being painted and the last
/// text record, and the operator's own prompt is the turn a clipped preview is most likely to
/// have drifted into.
const RECENT: usize = 4;

pub struct FileJournal {
    path: PathBuf,
    offset: u64,
    /// Where `partial[0]` sits in the file. `offset` is where reading has reached, which is past
    /// the end of the record being assembled, so it cannot answer where that record began.
    line_start: u64,
    partial: Vec<u8>,
    parser: Box<dyn TranscriptParser>,
    screen: Option<ScreenReader>,
}

impl FileJournal {
    pub fn new(path: PathBuf, parser: Box<dyn TranscriptParser>, screen: Option<ScreenReader>) -> Self {
        Self {
            path,
            offset: 0,
            line_start: 0,
            partial: Vec::new(),
            parser,
            screen,
        }
    }

    fn recent(&self) -> Vec<String> {
        let turns = self.parser.store().turns();
        turns[turns.len().saturating_sub(RECENT)..]
            .iter()
            .rev()
            .map(|turn| {
                turn.blocks
                    .iter()
                    .filter_map(|b| match b {
                        Block::Md { text, .. } => Some(text.as_str()),
                        _ => None,
                    })
                    .collect::<Vec<_>>()
                    .join("\n")
            })
            .collect()
    }

    fn rewind(&mut self) {
        self.offset = 0;
        self.line_start = 0;
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
            let start = self.line_start;
            self.line_start += line.len() as u64;
            let text = String::from_utf8_lossy(&line[..at]);
            let text = text.trim_end_matches('\r');
            if !text.is_empty() {
                self.parser.push_line(text, start);
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

    fn turn_ids(&self) -> Vec<String> {
        self.parser.store().turns().iter().map(|t| t.id.clone()).collect()
    }

    fn preview(&self, screen: &[&str]) -> Option<Turn> {
        live::preview(self.screen, screen, &self.recent())
    }
}
