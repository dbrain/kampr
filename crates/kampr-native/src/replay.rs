use crate::Screen;
use kampr_keep::SizedReader;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

const PREFIX: &str = "screen.";
const KEPT: usize = 2;
/// A log is trimmed a segment at a time and keeps at least its cap less one segment, so a
/// checkpoint this often is always retained.
pub(crate) const EVERY: u64 = kampr_keep::SEGMENT / 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Origin {
    Checkpoint(u64),
    Start,
    /// The log was trimmed past every usable checkpoint: the screen starts blank, in ground, at
    /// the oldest byte, so whatever pen, modes and screen the program had set before are lost.
    MidStream(u64),
}

pub(crate) struct Resumed {
    pub screen: Screen,
    pub reader: SizedReader,
    pub origin: Origin,
}

fn name(offset: u64) -> String {
    format!("{PREFIX}{offset:020}")
}

pub(crate) fn checkpoints(dir: &Path) -> Vec<u64> {
    let mut offsets: Vec<u64> = fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name();
            let rest = name.to_str()?.strip_prefix(PREFIX)?;
            (rest.len() == 20).then(|| rest.parse().ok()).flatten()
        })
        .collect();
    offsets.sort_unstable();
    offsets
}

/// The newest checkpoint the log still holds the bytes after, and a reader from its offset; the
/// oldest byte otherwise. A checkpoint is only usable while its offset is retained: one the log
/// was trimmed past, while nobody was reading, would splice two parts of the stream together.
pub(crate) fn resume(dir: &Path, scrollback: usize) -> io::Result<Resumed> {
    let (start, end) = {
        let probe = SizedReader::at(dir, 0)?;
        (probe.start()?, probe.end()?)
    };
    for offset in checkpoints(dir).into_iter().rev() {
        if offset < start || offset > end {
            continue;
        }
        let Some(screen) = fs::read(dir.join(name(offset))).ok().and_then(|bytes| {
            let body = bytes.strip_prefix(offset.to_le_bytes().as_slice())?;
            Screen::from_checkpoint(body, scrollback)
        }) else {
            tracing::warn!(dir = %dir.display(), offset, "ignoring a checkpoint that does not load");
            continue;
        };
        let reader = SizedReader::at(dir, offset)?;
        if (reader.size().cols, reader.size().rows) != screen.size() {
            tracing::warn!(dir = %dir.display(), offset, "ignoring a checkpoint at a size the journal disagrees with");
            continue;
        }
        return Ok(Resumed {
            screen,
            reader,
            origin: Origin::Checkpoint(offset),
        });
    }
    let reader = SizedReader::at(dir, start)?;
    let screen = Screen::new(reader.size().cols, reader.size().rows, scrollback);
    let origin = match start {
        0 => Origin::Start,
        oldest => {
            tracing::info!(dir = %dir.display(), oldest, "no checkpoint is retained: replaying from mid-stream");
            Origin::MidStream(oldest)
        }
    };
    Ok(Resumed {
        screen,
        reader,
        origin,
    })
}

pub(crate) fn write(dir: &Path, offset: u64, bytes: &[u8]) -> io::Result<()> {
    let path: PathBuf = dir.join(name(offset));
    let tmp = dir.join(format!("{}.tmp", name(offset)));
    fs::write(&tmp, [offset.to_le_bytes().as_slice(), bytes].concat())?;
    fs::rename(&tmp, &path)?;
    let all = checkpoints(dir);
    for old in &all[..all.len().saturating_sub(KEPT)] {
        match fs::remove_file(dir.join(name(*old))) {
            Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    Ok(())
}
