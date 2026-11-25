use crate::log::{Appender, LogReader, Read, segment_name, segments};
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PipeDir {
    In,
    Out,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct Entry {
    s: u64,
    d: PipeDir,
    l: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipeEntry {
    pub seq: u64,
    pub dir: PipeDir,
    pub line: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PipeRead {
    Entry(PipeEntry),
    Truncated { oldest_seq: u64 },
    CaughtUp,
}

pub struct PipeLog {
    inner: Appender,
    seq: u64,
    partial: Vec<u8>,
}

impl PipeLog {
    pub fn open(dir: &Path, cap: u64) -> io::Result<Self> {
        let inner = Appender::open(dir, cap)?;
        let seq = last_seq(dir)?.map(|s| s + 1).unwrap_or(0);
        Ok(Self {
            inner,
            seq,
            partial: Vec::new(),
        })
    }

    pub fn append_out(&mut self, data: &[u8]) -> io::Result<()> {
        self.partial.extend_from_slice(data);
        while let Some(pos) = self.partial.iter().position(|&b| b == b'\n') {
            let line = self.partial[..pos].to_vec();
            self.partial.drain(..=pos);
            self.append(PipeDir::Out, String::from_utf8_lossy(&line).into_owned())?;
        }
        Ok(())
    }

    pub fn append_in(&mut self, line: &str) -> io::Result<u64> {
        self.append(PipeDir::In, line.to_string())
    }

    fn append(&mut self, dir: PipeDir, line: String) -> io::Result<u64> {
        let entry = Entry {
            s: self.seq,
            d: dir,
            l: line,
        };
        let mut bytes = serde_json::to_vec(&entry)?;
        bytes.push(b'\n');
        self.inner.write(&bytes)?;
        let seq = self.seq;
        self.seq += 1;
        Ok(seq)
    }
}

fn last_seq(dir: &Path) -> io::Result<Option<u64>> {
    // A roll opens the next segment empty, and a crash can tear its first entry, so the newest
    // file may hold no line the numbering can resume from; walk back to one that does.
    for &offset in segments(dir)?.iter().rev() {
        let bytes = std::fs::read(dir.join(segment_name(offset)))?;
        let Some(end) = bytes.iter().rposition(|&b| b == b'\n') else {
            continue;
        };
        let start = bytes[..end]
            .iter()
            .rposition(|&b| b == b'\n')
            .map(|p| p + 1)
            .unwrap_or(0);
        if let Ok(entry) = serde_json::from_slice::<Entry>(&bytes[start..end]) {
            return Ok(Some(entry.s));
        }
    }
    Ok(None)
}

pub struct SeqReader {
    inner: LogReader,
    start: u64,
    line: Vec<u8>,
    buf: Vec<u8>,
    truncated: bool,
}

impl SeqReader {
    pub fn at(dir: impl Into<PathBuf>, start_seq: u64) -> io::Result<Self> {
        let dir = dir.into();
        let inner = LogReader::at(&dir, 0)?;
        Ok(Self {
            inner,
            start: start_seq,
            line: Vec::new(),
            buf: vec![0u8; 64 * 1024],
            truncated: false,
        })
    }

    pub fn read(&mut self) -> io::Result<PipeRead> {
        loop {
            if let Some(pos) = self.line.iter().position(|&b| b == b'\n') {
                let complete = self.line[..pos].to_vec();
                self.line.drain(..=pos);
                if let Ok(entry) = serde_json::from_slice::<Entry>(&complete)
                    && entry.s >= self.start
                {
                    if self.truncated {
                        self.truncated = false;
                        if entry.s > self.start {
                            return Ok(PipeRead::Truncated { oldest_seq: entry.s });
                        }
                    }
                    return Ok(PipeRead::Entry(PipeEntry {
                        seq: entry.s,
                        dir: entry.d,
                        line: entry.l,
                    }));
                }
                continue;
            }
            match self.inner.read(&mut self.buf) {
                Ok(Read::Data(n)) => self.line.extend_from_slice(&self.buf[..n]),
                Ok(Read::CaughtUp) => return Ok(PipeRead::CaughtUp),
                Ok(Read::Truncated { .. }) => {
                    self.truncated = true;
                    self.line.clear();
                    continue;
                }
                Err(e) => return Err(e),
            }
        }
    }

    pub fn wait(&mut self, timeout: Duration) -> io::Result<bool> {
        self.inner.wait(timeout)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::log::SEGMENT;

    fn append_len(seq: u64, line: &str) -> u64 {
        serde_json::to_vec(&Entry {
            s: seq,
            d: PipeDir::In,
            l: line.to_string(),
        })
        .expect("json")
        .len() as u64
            + 1
    }

    fn total_size(dir: &Path) -> u64 {
        segments(dir)
            .expect("segments")
            .into_iter()
            .map(|o| std::fs::metadata(dir.join(segment_name(o))).expect("meta").len())
            .sum()
    }

    fn first_seq_in(dir: &Path, offset: u64) -> u64 {
        let bytes = std::fs::read(dir.join(segment_name(offset))).expect("read");
        let mut start = 0usize;
        while let Some(pos) = bytes[start..].iter().position(|&b| b == b'\n') {
            if let Ok(entry) = serde_json::from_slice::<Entry>(&bytes[start..start + pos]) {
                return entry.s;
            }
            start += pos + 1;
        }
        panic!("segment {offset} holds no entry")
    }

    // A write that lands exactly on a segment boundary rolls an empty segment behind the
    // entries, so the newest file holds no line at all.
    #[test]
    fn reopening_a_log_whose_newest_segment_is_empty_keeps_numbering() {
        let dir = tempfile::tempdir().expect("dir");
        let cap = 4 * SEGMENT;
        let mut log = PipeLog::open(dir.path(), cap).expect("open");
        let line = "x".repeat((SEGMENT - append_len(0, "")) as usize);
        log.append_in(&line).expect("append");
        assert_eq!(total_size(dir.path()), SEGMENT, "the log ends on the boundary");
        assert_eq!(
            segments(dir.path()).expect("segments").last(),
            Some(&SEGMENT),
            "the roll opened an empty segment"
        );

        drop(log);
        let next = PipeLog::open(dir.path(), cap)
            .expect("reopen")
            .append_in("after the gap")
            .expect("append");
        assert_eq!(next, 1, "numbering continued past the surviving entry");
    }

    #[test]
    fn a_reader_reopened_from_a_rolled_off_seq_is_told_the_oldest_that_remains() {
        let dir = tempfile::tempdir().expect("dir");
        let mut log = PipeLog::open(dir.path(), SEGMENT).expect("open");
        let line = "x".repeat(64 * 1024);
        for _ in 0..40 {
            log.append_in(&line).expect("append");
        }
        drop(log);

        let mut reader = SeqReader::at(dir.path(), 0).expect("reader");
        match reader.read().expect("read") {
            PipeRead::Truncated { oldest_seq } => assert!(oldest_seq > 0, "seq 0 was rolled off"),
            other => panic!("the rolled-off start was lost: {other:?}"),
        }
    }

    #[test]
    fn a_reader_reopened_from_a_surviving_seq_is_not_told_it_rolled_off() {
        let dir = tempfile::tempdir().expect("dir");
        let cap = 2 * SEGMENT;
        let mut log = PipeLog::open(dir.path(), cap).expect("open");
        let line = "x".repeat(64 * 1024);
        for _ in 0..40 {
            log.append_in(&line).expect("append");
        }
        drop(log);

        let offsets = segments(dir.path()).expect("segments");
        assert!(*offsets.first().expect("a segment") > 0, "segment 0 rolled off");
        let start = first_seq_in(dir.path(), *offsets.first().unwrap());

        let mut reader = SeqReader::at(dir.path(), start).expect("reader");
        match reader.read().expect("read") {
            PipeRead::Entry(e) => assert_eq!(e.seq, start, "the surviving standing seq came back"),
            other => panic!("the surviving standing seq was lost: {other:?}"),
        }
        match reader.read().expect("read") {
            PipeRead::Entry(e) => assert_eq!(e.seq, start + 1, "no spurious Truncated followed"),
            other => panic!("a spurious truncation was reported: {other:?}"),
        }
    }
}
