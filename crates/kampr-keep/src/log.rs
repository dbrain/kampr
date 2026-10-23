use crate::sizes::{Journal, Resize};
use rustix::event::{PollFd, PollFlags, poll};
use rustix::fs::inotify;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read as _, Seek, SeekFrom, Write};
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

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
    inotify: OwnedFd,
}

impl LogReader {
    pub fn at(dir: impl Into<PathBuf>, offset: u64) -> io::Result<Self> {
        let dir = dir.into();
        let inotify = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
        inotify::add_watch(
            &inotify,
            &dir,
            inotify::WatchFlags::MODIFY
                | inotify::WatchFlags::CREATE
                | inotify::WatchFlags::MOVED_TO
                | inotify::WatchFlags::DELETE_SELF,
        )?;
        Ok(Self {
            dir,
            offset,
            open: None,
            inotify,
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
        let deadline = Instant::now() + timeout;
        loop {
            self.drain_events()?;
            if self.end()? > self.offset {
                return Ok(true);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Ok(false);
            }
            let spec = rustix::event::Timespec::try_from(left).map_err(io::Error::other)?;
            let mut fds = [PollFd::new(&self.inotify, PollFlags::IN)];
            match poll(&mut fds, Some(&spec)) {
                Ok(0) => return Ok(self.end()? > self.offset),
                Ok(_) => {
                    self.drain_events()?;
                    return Ok(self.end()? > self.offset);
                }
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }

    fn drain_events(&self) -> io::Result<()> {
        let mut buf = [MaybeUninit::<u8>::uninit(); 4096];
        let mut reader = inotify::Reader::new(&self.inotify, &mut buf);
        loop {
            match reader.next() {
                Ok(_) => {}
                Err(rustix::io::Errno::AGAIN) => return Ok(()),
                Err(rustix::io::Errno::INTR) => {}
                Err(e) => return Err(e.into()),
            }
        }
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
}
