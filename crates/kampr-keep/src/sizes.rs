use crate::dir::{SessionDir, Size, write_json};
use crate::log::{LogReader, Read};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub const SIZES: &str = "sizes.jsonl";

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Resize {
    pub seq: u64,
    pub offset: u64,
    pub cols: u16,
    pub rows: u16,
    pub holder: String,
}

impl Resize {
    pub fn size(&self) -> Size {
        Size {
            cols: self.cols,
            rows: self.rows,
            holder: self.holder.clone(),
        }
    }
}

pub fn sizes(dir: &Path) -> io::Result<Vec<Resize>> {
    match fs::read(dir.join(SIZES)) {
        Ok(bytes) => Ok(parse(&bytes)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

// A keeper killed mid-append leaves a partial last line; it was never acted on (the TIOCSWINSZ
// comes after the append), so it is not a resize.
fn parse(bytes: &[u8]) -> Vec<Resize> {
    bytes
        .split(|&b| b == b'\n')
        .filter_map(|line| serde_json::from_slice(line).ok())
        .collect()
}

pub fn size_at(dir: &Path, offset: u64) -> io::Result<Size> {
    Ok(in_effect(&sizes(dir)?, offset, &spawned(dir)?))
}

fn spawned(dir: &Path) -> io::Result<Size> {
    let spec = SessionDir::at(dir).spec()?;
    Ok(Size {
        cols: spec.cols,
        rows: spec.rows,
        holder: "spawn".into(),
    })
}

fn in_effect(records: &[Resize], offset: u64, spawned: &Size) -> Size {
    records
        .iter()
        .rev()
        .find(|r| r.offset <= offset)
        .map(Resize::size)
        .unwrap_or_else(|| spawned.clone())
}

pub(crate) struct Journal {
    dir: PathBuf,
    file: File,
    next: u64,
}

impl Journal {
    pub(crate) fn open(dir: &Path) -> io::Result<Self> {
        let path = dir.join(SIZES);
        let records = sizes(dir)?;
        if fs::read(&path).is_ok_and(|b| !b.is_empty() && !b.ends_with(b"\n")) {
            rewrite(&path, &records)?;
        }
        Ok(Self {
            dir: dir.to_path_buf(),
            file: append(&path)?,
            next: records.last().map_or(0, |r| r.seq + 1),
        })
    }

    pub(crate) fn append(&mut self, offset: u64, cols: u16, rows: u16, holder: &str) -> io::Result<Resize> {
        let record = Resize {
            seq: self.next,
            offset,
            cols,
            rows,
            holder: holder.to_string(),
        };
        let mut line = serde_json::to_vec(&record).map_err(io::Error::other)?;
        line.push(b'\n');
        self.file.write_all(&line)?;
        self.next += 1;
        Ok(record)
    }

    pub(crate) fn trim(&mut self, oldest: u64) -> io::Result<()> {
        let records = sizes(&self.dir)?;
        let older = records.iter().take_while(|r| r.offset < oldest).count();
        if older < 2 {
            return Ok(());
        }
        let path = self.dir.join(SIZES);
        rewrite(&path, &records[older - 1..])?;
        self.file = append(&path)?;
        Ok(())
    }
}

fn rewrite(path: &Path, records: &[Resize]) -> io::Result<()> {
    let mut text = Vec::new();
    for record in records {
        text.extend(serde_json::to_vec(record).map_err(io::Error::other)?);
        text.push(b'\n');
    }
    let tmp = path.with_extension("jsonl.tmp");
    fs::write(&tmp, text)?;
    fs::rename(&tmp, path)
}

fn append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

pub(crate) fn write_size(dir: &Path, cols: u16, rows: u16, holder: &str) -> io::Result<()> {
    write_json(
        &dir.join(crate::dir::SIZE),
        &Size {
            cols,
            rows,
            holder: holder.to_string(),
        },
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Replay {
    Data(usize),
    Resize(Resize),
    Truncated { oldest: u64, size: Size },
    CaughtUp,
}

pub struct SizedReader {
    dir: PathBuf,
    log: LogReader,
    offset: u64,
    held: Vec<u8>,
    size: Size,
    spawned: Size,
    records: Vec<Resize>,
    seen: Option<(u64, u64, i64, i64)>,
    next: u64,
}

impl SizedReader {
    pub fn at(dir: impl Into<PathBuf>, offset: u64) -> io::Result<Self> {
        let dir = dir.into();
        let spawned = spawned(&dir)?;
        let mut reader = Self {
            log: LogReader::at(&dir, offset)?,
            dir,
            offset,
            held: Vec::new(),
            size: spawned.clone(),
            spawned,
            records: Vec::new(),
            seen: None,
            next: 0,
        };
        reader.refresh()?;
        reader.settle_at(offset);
        Ok(reader)
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn size(&self) -> &Size {
        &self.size
    }

    pub fn start(&self) -> io::Result<u64> {
        self.log.start()
    }

    pub fn end(&self) -> io::Result<u64> {
        self.log.end()
    }

    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<Replay> {
        if let Some(record) = self.due() {
            return Ok(self.apply(record));
        }
        let limit = self
            .upcoming()
            .map_or(buf.len(), |at| buf.len().min((at - self.offset) as usize));
        if !self.held.is_empty() {
            let n = limit.min(self.held.len());
            buf[..n].copy_from_slice(&self.held[..n]);
            self.held.drain(..n);
            self.offset += n as u64;
            return Ok(Replay::Data(n));
        }
        match self.log.read(&mut buf[..limit])? {
            Read::Data(n) => {
                // The keeper journals a resize before it reads a byte past it, so a journal read
                // after the bytes knows of every resize those bytes cross.
                self.refresh()?;
                if self.trimmed_past_us() {
                    return self.skip_to_start();
                }
                let cut = self.upcoming().map_or(n, |at| n.min((at - self.offset) as usize));
                self.held.extend_from_slice(&buf[cut..n]);
                self.offset += cut as u64;
                match self.due() {
                    Some(record) if cut == 0 => Ok(self.apply(record)),
                    _ => Ok(Replay::Data(cut)),
                }
            }
            Read::Truncated { oldest } => {
                self.refresh()?;
                self.truncated(oldest)
            }
            Read::CaughtUp => {
                self.refresh()?;
                if self.trimmed_past_us() {
                    return self.skip_to_start();
                }
                Ok(match self.due() {
                    Some(record) => self.apply(record),
                    None => Replay::CaughtUp,
                })
            }
        }
    }

    pub fn wait(&mut self, timeout: Duration) -> io::Result<bool> {
        let deadline = Instant::now() + timeout;
        loop {
            self.refresh()?;
            if !self.held.is_empty() || self.due().is_some() || self.log.end()? > self.log.offset() {
                return Ok(true);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(false);
            }
            if self.log.wait(left)? {
                return Ok(true);
            }
        }
    }

    fn refresh(&mut self) -> io::Result<()> {
        let key = match fs::metadata(self.dir.join(SIZES)) {
            Ok(m) => Some((m.ino(), m.len(), m.mtime(), m.mtime_nsec())),
            Err(e) if e.kind() == io::ErrorKind::NotFound => None,
            Err(e) => return Err(e),
        };
        if key != self.seen {
            self.records = sizes(&self.dir)?;
            self.seen = key;
        }
        Ok(())
    }

    fn settle_at(&mut self, offset: u64) {
        self.size = in_effect(&self.records, offset, &self.spawned);
        self.next = match self.records.iter().find(|r| r.offset > offset) {
            Some(r) => r.seq,
            None => self.records.last().map_or(0, |r| r.seq + 1),
        };
    }

    fn pending(&self) -> Option<&Resize> {
        self.records.iter().find(|r| r.seq >= self.next)
    }

    fn due(&self) -> Option<Resize> {
        self.pending().filter(|r| r.offset <= self.offset).cloned()
    }

    fn upcoming(&self) -> Option<u64> {
        self.pending().map(|r| r.offset.max(self.offset))
    }

    fn trimmed_past_us(&self) -> bool {
        self.records.first().is_some_and(|r| r.seq > self.next)
    }

    fn apply(&mut self, record: Resize) -> Replay {
        self.next = record.seq + 1;
        self.size = record.size();
        Replay::Resize(record)
    }

    fn truncated(&mut self, oldest: u64) -> io::Result<Replay> {
        self.held.clear();
        self.offset = oldest;
        self.settle_at(oldest);
        Ok(Replay::Truncated {
            oldest,
            size: self.size.clone(),
        })
    }

    // Only a reader still inside an unlinked segment can fall this far behind: the journal has
    // dropped resizes it has not applied, so it cannot carry on exactly and starts again.
    fn skip_to_start(&mut self) -> io::Result<Replay> {
        let oldest = self.log.start()?;
        self.log = LogReader::at(&self.dir, oldest)?;
        self.truncated(oldest)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dir::{SPEC, Spec};
    use crate::log::Appender;

    fn drain(reader: &mut SizedReader, buf: &mut [u8]) -> String {
        let mut got = String::new();
        loop {
            match reader.read(buf).expect("read") {
                Replay::Data(n) => got.push_str(&String::from_utf8_lossy(&buf[..n])),
                Replay::Resize(r) => got.push_str(&format!("<{}x{} at {}>", r.cols, r.rows, r.offset)),
                Replay::Truncated { oldest, size } => {
                    got.push_str(&format!("<from {oldest} at {}x{}>", size.cols, size.rows))
                }
                Replay::CaughtUp => return got,
            }
        }
    }

    #[test]
    fn a_resize_journalled_after_a_read_began_still_cuts_the_bytes_it_crossed() {
        let dir = tempfile::tempdir().expect("dir");
        write_json(&dir.path().join(SPEC), &Spec::new(vec!["sh".into()])).expect("spec");
        let mut log = Appender::open(dir.path(), crate::log::DEFAULT_CAP).expect("log");
        log.write(b"before\n").expect("write");
        let mut reader = SizedReader::at(dir.path(), 0).expect("reader");
        assert_eq!(reader.read(&mut [0u8; 4]).expect("read"), Replay::Data(4));

        log.resize(100, 40, "late").expect("resize");
        log.write(b"after\n").expect("write");
        assert_eq!(drain(&mut reader, &mut [0u8; 64]), "re\n<100x40 at 7>after\n");
        assert_eq!(reader.size().holder, "late");
    }
}
