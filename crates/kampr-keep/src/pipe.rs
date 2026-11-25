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
    let offsets = segments(dir)?;
    let Some(&last) = offsets.last() else {
        return Ok(None);
    };
    let bytes = std::fs::read(dir.join(segment_name(last)))?;
    let Some(end) = bytes.iter().rposition(|&b| b == b'\n') else {
        return Ok(None);
    };
    let start = bytes[..end]
        .iter()
        .rposition(|&b| b == b'\n')
        .map(|p| p + 1)
        .unwrap_or(0);
    let entry: Entry = serde_json::from_slice(&bytes[start..end])?;
    Ok(Some(entry.s))
}

pub struct SeqReader {
    inner: LogReader,
    start: u64,
    line: Vec<u8>,
    buf: Vec<u8>,
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
