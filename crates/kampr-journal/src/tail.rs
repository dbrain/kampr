use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

use crate::attach::{MAX_RECORD_BYTES, Origin};
use crate::error::JournalError;
use crate::live::{self, ScreenReader};
use crate::model::{Block, Page, Role, Turn};
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
    /// A record longer than anything the fetch path will read is being discarded to its newline.
    skipping: bool,
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
            skipping: false,
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
        self.skipping = false;
        self.parser.reset();
    }

    /// Parses every whole line the buffer holds and drops them from it in one move, leaving the
    /// torn tail. Per-line removal is what this must not do: it memmoves the remainder down once
    /// per line, which is quadratic in the buffer and cost 16.8 s on a 40 MB transcript against
    /// 15.7 ms for this.
    fn take_lines(&mut self) {
        // A line with no end to it is buffered whole, and nothing here caps what a pane's harness
        // may write on one: a 204.8 MB single-line transcript held 203 MB resident to produce
        // nothing at all. `MAX_RECORD_BYTES` is how far `attach::read_record` will read looking for
        // the end of a record, so a record longer than this could never be fetched back either —
        // it is discarded to its newline and the parse goes on from the next record.
        if self.skipping {
            match self.partial.iter().position(|b| *b == b'\n') {
                Some(end) => {
                    self.line_start += end as u64 + 1;
                    self.partial.drain(..=end);
                    self.skipping = false;
                }
                None => {
                    self.line_start += self.partial.len() as u64;
                    self.partial.clear();
                    return;
                }
            }
        }
        let mut whole = 0;
        for line in self.partial.split_inclusive(|b| *b == b'\n') {
            let Some(text) = line.strip_suffix(b"\n") else {
                break;
            };
            let start = self.line_start;
            self.line_start += line.len() as u64;
            whole += line.len();
            let text = String::from_utf8_lossy(text);
            let text = text.trim_end_matches('\r');
            if !text.is_empty() {
                self.parser.push_line(text, start);
            }
        }
        self.partial.drain(..whole);
        if self.partial.len() as u64 > MAX_RECORD_BYTES {
            self.line_start += self.partial.len() as u64;
            self.partial.clear();
            self.skipping = true;
        }
    }
}

/// How much of a transcript one read pulls in before its whole lines are parsed and dropped. The
/// largest rollout measured on a real machine is 88.7 MB (probe #247), and reading it in full
/// would hold all of it — twice over, while the buffer grows — on a tokio worker.
const CHUNK: u64 = 1024 * 1024;

/// How many pages back a page will reach for the question that opens the reply it landed in,
/// before the cut stands instead.
///
/// **A page counted in turns opens partway into a reply**, because every harness writes one tool
/// call per record and every record is a turn: one prompt and the answer to it measured **53**
/// against a page of 40, so the question was off the first page of a session that had only ever
/// been asked one thing — on a view pinned to its own end, with nothing above it to scroll to. A
/// reply has no bound of its own, so the reach does: past this the page is cut mid-reply and
/// `more` is what says so.
const REACH: usize = 4;

/// Where a page starts: the cut, moved back to the question that opens the reply it landed in.
///
/// A cut that already lands on a question is left alone — walking back from one would reach past
/// it to the *previous* exchange and hand the reader a page they did not ask for.
fn opening(turns: &[Turn], cut: usize, floor: usize) -> usize {
    if turns.get(cut).is_some_and(|turn| turn.role == Role::User) {
        return cut;
    }
    turns[floor..cut]
        .iter()
        .rposition(|turn| turn.role == Role::User)
        .map_or(cut, |at| floor + at)
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
        }
        // A transcript is appended to while we read it, so the tail may be a torn line. Only
        // whole lines are parsed; the remainder waits for the next poll.
        while self.offset < len {
            let read = file.by_ref().take(CHUNK).read_to_end(&mut self.partial)?;
            // The file shrank under the read that the length said would fill.
            if read == 0 {
                break;
            }
            self.offset += read as u64;
            self.take_lines();
        }

        Ok(self.parser.store_mut().drain_changed())
    }

    fn page_before(&self, before: Option<&str>, limit: usize) -> Page {
        let store = self.parser.store();
        let turns = store.turns();
        let end = before.and_then(|id| store.position(id)).unwrap_or(turns.len());
        let cut = end.saturating_sub(limit);
        let start = opening(turns, cut, end.saturating_sub(limit.saturating_mul(REACH)));
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
