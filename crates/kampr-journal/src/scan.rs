use std::fs::File;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use crate::attach::MAX_RECORD_BYTES;

/// How far a reader has got through a transcript, and whether it stopped inside a record too long
/// to serve. Small and `Copy` so a fold can hold one, hand it to the next read and take back where
/// that read reached.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Cursor {
    at: u64,
    skipping: bool,
}

/// Whole records off a transcript, one string at a time, from the [`Cursor`] a previous read
/// stopped at rather than from the start of the file — so a fold that has already read a
/// transcript can be fed only what it has grown by.
///
/// The ceiling is the one the fetch path reads to: a record longer than [`MAX_RECORD_BYTES`]
/// could never be served back either, so it is discarded to its newline rather than held whole —
/// a 204.8 MB single-line transcript is a real shape.
///
/// **This yields exactly what [`crate::tail::FileJournal`] hands its parser, in the same order
/// and with the same lines left out.** A facet that names a turn by the id the parser minted from
/// a line's position depends on that: a line skipped here and kept there shifts every id after it.
/// A record with no newline yet is therefore not yielded and the cursor stays in front of it — a
/// transcript is appended to while this reads it, and folding half a record now and its remainder
/// as a record of its own is how a queued prompt would arrive twice, or as nothing at all.
pub struct Appended {
    reader: Option<BufReader<File>>,
    cursor: Cursor,
    restarted: bool,
}

impl Appended {
    pub fn open(transcript: &Path, from: Cursor) -> Self {
        let mut cursor = from;
        let mut restarted = false;
        let reader = File::open(transcript).ok().and_then(|mut file| {
            // A transcript shorter than what has already been read is not the transcript that was
            // read: `/clear` opens a new file under a new session id (#259), and a fold that kept
            // its accumulator would carry one session's queue into the next.
            if file.metadata().ok()?.len() < cursor.at {
                cursor = Cursor::default();
                restarted = true;
            }
            file.seek(SeekFrom::Start(cursor.at)).ok()?;
            Some(BufReader::new(file))
        });
        Self {
            reader,
            cursor,
            restarted,
        }
    }

    pub fn restarted(&self) -> bool {
        self.restarted
    }

    /// Where the next read starts: the end of the last *whole* record, once this has run dry.
    pub fn cursor(&self) -> Cursor {
        self.cursor
    }
}

impl Iterator for Appended {
    type Item = String;

    fn next(&mut self) -> Option<String> {
        let reader = self.reader.as_mut()?;
        loop {
            if self.cursor.skipping {
                let (used, ended) = skip_record(reader);
                self.cursor.at += used;
                if !ended {
                    return None;
                }
                self.cursor.skipping = false;
            }
            let mut buf = Vec::new();
            let read = reader
                .take(MAX_RECORD_BYTES + 1)
                .read_until(b'\n', &mut buf)
                .ok()?;
            if read == 0 {
                return None;
            }
            if !buf.ends_with(b"\n") {
                if read as u64 <= MAX_RECORD_BYTES {
                    return None;
                }
                self.cursor.at += read as u64;
                self.cursor.skipping = true;
                continue;
            }
            self.cursor.at += read as u64;
            let text = String::from_utf8_lossy(&buf);
            let text = text.trim_end_matches('\n').trim_end_matches('\r');
            if !text.is_empty() {
                return Some(text.to_string());
            }
        }
    }
}

/// Bytes consumed, and whether the newline that ends the record was among them.
fn skip_record(reader: &mut impl BufRead) -> (u64, bool) {
    let mut used = 0;
    loop {
        let (ended, take) = match reader.fill_buf() {
            Err(_) | Ok([]) => return (used, false),
            Ok(buf) => match buf.iter().position(|b| *b == b'\n') {
                Some(at) => (true, at + 1),
                None => (false, buf.len()),
            },
        };
        reader.consume(take);
        used += take as u64;
        if ended {
            return (used, true);
        }
    }
}
