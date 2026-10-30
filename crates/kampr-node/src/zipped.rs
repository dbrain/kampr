//! A directory as a zip, written while it is sent.
//!
//! `ZipWriter::new_stream` never seeks: each entry's sizes and CRC go in a data descriptor after
//! its bytes, so the archive can be handed out a chunk at a time with nothing held but the chunk
//! being filled, the deflate state and the [`AHEAD`] chunks a slow reader has not taken yet.

use std::fs::Metadata;
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use tokio::sync::mpsc;
use zip::write::SimpleFileOptions;
use zip::{CompressionMethod, ZipWriter};

use crate::served::{CHUNK, Refused, Served};

const AHEAD: usize = 2;

/// Past this an entry needs zip64 sizes, and with no seeking that has to be decided before its
/// first byte. The margin covers deflate making an incompressible file slightly larger.
const LARGE: u64 = 0xF000_0000;

pub const ZIP: &str = "application/zip";

/// The kind a zip is served under: a download, never a document.
pub const ARCHIVE: &str = "archive";

pub fn serve(dir: &Path) -> Result<Served, Refused> {
    if !std::fs::metadata(dir).map_err(|_| Refused::Missing)?.is_dir() {
        return Err(Refused::Missing);
    }
    let (tx, rx) = mpsc::channel(AHEAD);
    let root = dir.to_path_buf();
    tokio::task::spawn_blocking(move || {
        let failed = tx.clone();
        if let Err(e) = build(&root, Pipe { tx, buf: Vec::new() }) {
            tracing::debug!(dir = %root.display(), error = %e, "a zip stopped part way");
            let _ = failed.blocking_send(Err(e));
        }
    });
    let body = futures_util::stream::unfold(
        rx,
        |mut rx| async move { rx.recv().await.map(|chunk| (chunk, rx)) },
    );
    let name = dir
        .file_name()
        .map_or_else(|| "archive".to_string(), |n| n.to_string_lossy().into_owned());
    Ok(Served {
        kind: ARCHIVE.to_string(),
        mime: Some(ZIP.to_string()),
        name: Some(format!("{name}.zip")),
        length: None,
        slice: None,
        ranges: false,
        body: Box::pin(body),
    })
}

struct Pipe {
    tx: mpsc::Sender<io::Result<Vec<u8>>>,
    buf: Vec<u8>,
}

impl Pipe {
    fn send(&mut self, chunk: Vec<u8>) -> io::Result<()> {
        self.tx
            .blocking_send(Ok(chunk))
            .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
    }

    fn finish(mut self) -> io::Result<()> {
        let rest = std::mem::take(&mut self.buf);
        match rest.is_empty() {
            true => Ok(()),
            false => self.send(rest),
        }
    }
}

impl Write for Pipe {
    fn write(&mut self, data: &[u8]) -> io::Result<usize> {
        self.buf.extend_from_slice(data);
        while self.buf.len() >= CHUNK {
            let rest = self.buf.split_off(CHUNK);
            let chunk = std::mem::replace(&mut self.buf, rest);
            self.send(chunk)?;
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Depth first, each directory's entries in name order. A symlink is never followed — not out of
/// the directory and not within it, so a loop cannot make the archive endless — and anything that
/// is neither a directory nor a regular file, or that this user cannot open, is left out.
fn build(root: &Path, pipe: Pipe) -> io::Result<()> {
    let mut zip = ZipWriter::new_stream(pipe);
    let mut pending: Vec<(PathBuf, String)> = vec![(root.to_path_buf(), String::new())];
    while let Some((dir, prefix)) = pending.pop() {
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        let mut entries: Vec<_> = read.filter_map(Result::ok).collect();
        entries.sort_by_key(std::fs::DirEntry::file_name);
        let mut below = Vec::new();
        for entry in entries {
            let path = entry.path();
            let name = format!("{prefix}{}", entry.file_name().to_string_lossy());
            let Ok(meta) = std::fs::symlink_metadata(&path) else {
                continue;
            };
            if meta.is_dir() {
                zip.add_directory(format!("{name}/"), options(&meta))
                    .map_err(io::Error::other)?;
                below.push((path, format!("{name}/")));
            } else if meta.is_file() {
                add_file(&mut zip, &path, &name)?;
            }
        }
        pending.extend(below.into_iter().rev());
    }
    zip.finish().map_err(io::Error::other)?.into_inner().finish()
}

fn add_file<W: Write + io::Seek>(zip: &mut ZipWriter<W>, path: &Path, name: &str) -> io::Result<()> {
    let Ok(file) = std::fs::File::open(path) else {
        return Ok(());
    };
    let Ok(meta) = file.metadata() else { return Ok(()) };
    if !meta.is_file() {
        return Ok(());
    }
    zip.start_file(name, options(&meta).large_file(meta.len() >= LARGE))
        .map_err(io::Error::other)?;
    io::copy(&mut file.take(meta.len()), zip)?;
    Ok(())
}

fn options(meta: &Metadata) -> SimpleFileOptions {
    let options = SimpleFileOptions::default()
        .compression_method(CompressionMethod::Deflated)
        .unix_permissions(meta.permissions().mode() & 0o777);
    match meta.modified().ok().and_then(modified) {
        Some(when) => options.last_modified_time(when),
        None => options,
    }
}

fn modified(when: std::time::SystemTime) -> Option<zip::DateTime> {
    let when = time::OffsetDateTime::from(when);
    zip::DateTime::from_date_and_time(
        u16::try_from(when.year()).ok()?,
        when.month() as u8,
        when.day(),
        when.hour(),
        when.minute(),
        when.second(),
    )
    .ok()
}
