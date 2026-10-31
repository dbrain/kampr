use crate::sizes::{Journal, Resize};
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const SEGMENT: u64 = 1 << 20;
pub const DEFAULT_CAP: u64 = 32 << 20;
const PREFIX: &str = "log.";

pub fn segment_name(offset: u64) -> String {
    format!("{PREFIX}{offset:020}")
}

pub fn segments(dir: &Path) -> io::Result<Vec<u64>> {
    let mut offsets: Vec<u64> = fs::read_dir(dir)?
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let rest = name.to_str()?.strip_prefix(PREFIX)?;
            (rest.len() == 20).then(|| rest.parse().ok()).flatten()
        })
        .collect();
    offsets.sort_unstable();
    Ok(offsets)
}

pub(crate) fn bounds(dir: &Path) -> io::Result<(u64, u64)> {
    let offsets = segments(dir)?;
    let (Some(&first), Some(&last)) = (offsets.first(), offsets.last()) else {
        return Ok((0, 0));
    };
    let len = match fs::metadata(dir.join(segment_name(last))) {
        Ok(m) => m.len(),
        Err(e) if e.kind() == io::ErrorKind::NotFound => 0,
        Err(e) => return Err(e),
    };
    Ok((first, last + len))
}

pub(crate) struct Appender {
    dir: PathBuf,
    cap: u64,
    file: File,
    segment: u64,
    end: u64,
    sizes: Journal,
}

impl Appender {
    pub(crate) fn open(dir: &Path, cap: u64) -> io::Result<Self> {
        let (_, end) = bounds(dir)?;
        let segment = segments(dir)?.last().copied().unwrap_or(0);
        let file = open_append(&dir.join(segment_name(segment)))?;
        let mut appender = Self {
            dir: dir.to_path_buf(),
            cap: cap.max(SEGMENT),
            file,
            segment,
            end,
            sizes: Journal::open(dir)?,
        };
        if appender.end - appender.segment >= SEGMENT {
            appender.roll()?;
        }
        Ok(appender)
    }

    pub(crate) fn write(&mut self, mut data: &[u8]) -> io::Result<()> {
        while !data.is_empty() {
            let room = (SEGMENT - (self.end - self.segment)) as usize;
            let n = room.min(data.len());
            self.file.write_all(&data[..n])?;
            self.end += n as u64;
            data = &data[n..];
            if self.end - self.segment >= SEGMENT {
                self.roll()?;
            }
        }
        Ok(())
    }

    pub(crate) fn resize(&mut self, cols: u16, rows: u16, holder: &str) -> io::Result<Resize> {
        self.sizes.append(self.end, cols, rows, holder)
    }

    fn roll(&mut self) -> io::Result<()> {
        self.file = open_append(&self.dir.join(segment_name(self.end)))?;
        self.segment = self.end;
        for oldest in segments(&self.dir)? {
            if oldest >= self.segment || self.end - oldest < self.cap {
                break;
            }
            match fs::remove_file(self.dir.join(segment_name(oldest))) {
                Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
                _ => {}
            }
        }
        match segments(&self.dir)?.first() {
            Some(&oldest) => self.sizes.trim(oldest),
            None => Ok(()),
        }
    }
}

fn open_append(path: &Path) -> io::Result<File> {
    OpenOptions::new().create(true).append(true).open(path)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Read {
    Data(usize),
    Truncated { oldest: u64 },
    CaughtUp,
}

pub(crate) struct LogReader {
    dir: PathBuf,
    offset: u64,
    open: Option<(u64, File)>,
    watch: Follow,
}

impl LogReader {
    pub fn at(dir: impl Into<PathBuf>, offset: u64) -> io::Result<Self> {
        let dir = dir.into();
        let watch = Follow::new(&dir)?;
        Ok(Self {
            dir,
            offset,
            open: None,
            watch,
        })
    }

    pub fn offset(&self) -> u64 {
        self.offset
    }

    pub fn start(&self) -> io::Result<u64> {
        bounds(&self.dir).map(|(start, _)| start)
    }

    pub fn end(&self) -> io::Result<u64> {
        bounds(&self.dir).map(|(_, end)| end)
    }

    pub fn read(&mut self, buf: &mut [u8]) -> io::Result<Read> {
        loop {
            if let Some((segment, file)) = &mut self.open {
                let n = file.read(buf)?;
                if n > 0 {
                    self.offset += n as u64;
                    return Ok(Read::Data(n));
                }
                if self.offset - *segment < SEGMENT {
                    return Ok(Read::CaughtUp);
                }
                self.open = None;
            }
            let offsets = segments(&self.dir)?;
            let Some(&oldest) = offsets.first() else {
                return Ok(Read::CaughtUp);
            };
            if self.offset < oldest {
                self.offset = oldest;
                return Ok(Read::Truncated { oldest });
            }
            let Some(&segment) = offsets.iter().rev().find(|&&s| s <= self.offset) else {
                return Ok(Read::CaughtUp);
            };
            let mut file = match File::open(self.dir.join(segment_name(segment))) {
                Ok(file) => file,
                Err(e) if e.kind() == io::ErrorKind::NotFound => continue,
                Err(e) => return Err(e),
            };
            let within = self.offset - segment;
            if within > file.metadata()?.len() {
                return Ok(Read::CaughtUp);
            }
            file.seek(SeekFrom::Start(within))?;
            self.open = Some((segment, file));
        }
    }

    pub fn wait(&mut self, timeout: Duration) -> io::Result<bool> {
        self.watch.wait(Duration::ZERO)?;
        if self.end()? > self.offset {
            return Ok(true);
        }
        if timeout.is_zero() {
            return Ok(false);
        }
        self.watch.wait(timeout)?;
        Ok(self.end()? > self.offset)
    }
}

/// A session directory's entries and the end of its newest segment: everything a reader of the
/// log, the sizes and the session's ending waits on. The keeper appends to a segment it holds
/// open, which neither inotify nor kqueue reports on the directory, so the segment is watched as a
/// file and moved to the next one when a roll adds it.
pub(crate) struct Follow {
    dir: PathBuf,
    watch: kampr_watch::Watch,
    _stop: kampr_watch::Stop,
    dir_key: Option<kampr_watch::Key>,
    tail: Option<(u64, kampr_watch::Key)>,
}

impl Follow {
    pub fn new(dir: &Path) -> io::Result<Self> {
        let (mut watch, stop) = kampr_watch::Watch::new()?;
        let dir_key = Some(watch.add(dir)?);
        let mut follow = Self {
            dir: dir.to_path_buf(),
            watch,
            _stop: stop,
            dir_key,
            tail: None,
        };
        follow.arm()?;
        Ok(follow)
    }

    /// True when something moved in the directory or the newest segment grew; false when `timeout`
    /// passed first. Anything since the last wait counts, so a check made between waits loses
    /// nothing.
    pub fn wait(&mut self, timeout: Duration) -> io::Result<bool> {
        let changed = match self.watch.wait(Some(timeout))? {
            kampr_watch::Waited::Changed(events) => {
                for event in events.iter().filter(|e| e.gone) {
                    if self.dir_key == Some(event.key) {
                        self.dir_key = None;
                    }
                    if self.tail.is_some_and(|(_, key)| key == event.key) {
                        self.tail = None;
                    }
                }
                true
            }
            kampr_watch::Waited::TimedOut | kampr_watch::Waited::Stopped => false,
        };
        self.arm()?;
        Ok(changed)
    }

    fn arm(&mut self) -> io::Result<()> {
        if self.dir_key.is_none() {
            return Ok(());
        }
        let newest = match segments(&self.dir) {
            Ok(offsets) => offsets.last().copied(),
            Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(e),
        };
        if newest == self.tail.map(|(segment, _)| segment) {
            return Ok(());
        }
        if let Some((_, key)) = self.tail.take() {
            self.watch.remove(key);
        }
        let Some(segment) = newest else {
            return Ok(());
        };
        match self.watch.add(&self.dir.join(segment_name(segment))) {
            Ok(key) => self.tail = Some((segment, key)),
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segments_split_at_exact_mebibytes_whatever_the_write_sizes() {
        let dir = tempfile::tempdir().expect("dir");
        let data: Vec<u8> = (0..(7 * SEGMENT / 2)).map(|i| (i % 251) as u8).collect();
        let mut log = Appender::open(dir.path(), 2 * SEGMENT).expect("open");
        for chunk in data.chunks(4093) {
            log.write(chunk).expect("write");
        }
        drop(log);
        let offsets = segments(dir.path()).expect("segments");
        assert_eq!(offsets, vec![2 * SEGMENT, 3 * SEGMENT]);

        let mut appender = Appender::open(dir.path(), 2 * SEGMENT).expect("reopen");
        appender.write(b"more").expect("write");
        let mut reader = LogReader::at(dir.path(), 0).expect("reader");
        assert_eq!(reader.end().expect("end"), data.len() as u64 + 4);
        let mut buf = vec![0u8; 100_000];
        assert_eq!(
            reader.read(&mut buf).expect("read"),
            Read::Truncated { oldest: 2 * SEGMENT }
        );
        let mut got = Vec::new();
        while let Read::Data(n) = reader.read(&mut buf).expect("read") {
            got.extend_from_slice(&buf[..n]);
        }
        assert_eq!(&got[..got.len() - 4], &data[2 * SEGMENT as usize..]);
        assert_eq!(&got[got.len() - 4..], b"more");
    }

    #[test]
    fn a_caught_up_reader_wakes_on_appends_to_the_segment_a_roll_started() {
        let dir = tempfile::tempdir().expect("dir");
        let mut log = Appender::open(dir.path(), DEFAULT_CAP).expect("open");
        log.write(&vec![b'a'; SEGMENT as usize - 1]).expect("write");
        let mut reader = LogReader::at(dir.path(), 0).expect("reader");
        let mut buf = vec![0u8; SEGMENT as usize];
        let mut catch_up =
            |reader: &mut LogReader| while let Read::Data(_) = reader.read(&mut buf).expect("read") {};
        catch_up(&mut reader);
        let quiet = std::time::Instant::now();
        assert!(!reader.wait(Duration::from_millis(100)).expect("wait"));
        assert!(quiet.elapsed() >= Duration::from_millis(90));

        // The appender stays open throughout, as the keeper's does: closing it would wake a Linux
        // directory watch however the segment was followed.
        for (bytes, past) in [(&b"bc"[..], SEGMENT + 1), (&b"d"[..], SEGMENT + 2)] {
            let writer = std::thread::spawn(move || {
                std::thread::sleep(Duration::from_millis(50));
                log.write(bytes).expect("write");
                log
            });
            let woke = std::time::Instant::now();
            assert!(reader.wait(Duration::from_secs(5)).expect("wait"));
            assert!(woke.elapsed() < Duration::from_secs(2));
            log = writer.join().expect("writer");
            catch_up(&mut reader);
            assert_eq!(reader.offset(), past);
        }
    }
}
