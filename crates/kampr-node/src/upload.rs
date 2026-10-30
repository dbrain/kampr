//! A file from a device, written into a directory on this node.
//!
//! **Written beside its destination and renamed into place**, so a reader of the directory never
//! sees half a file, and an upload that does not finish leaves nothing: the temp file is removed
//! when the [`Staged`] holding it is dropped, which is what a dropped request or a closed link
//! does. **Never over an existing file**: the name is claimed with `link(2)`, which fails rather
//! than replaces, so two uploads of one name race to two names instead of one overwriting the
//! other. **Remembered in the state directory while it is open**, because a node that is killed
//! runs no drop: the next start removes every temp file the last run left in [`sweep`].

use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

use kampr_auth::{Device, Entry};
use kampr_journal::FileRef;
use serde_json::json;
use tokio::io::AsyncWriteExt;

use crate::Node;

#[derive(Debug, PartialEq, Eq)]
pub enum UploadError {
    BadName,
    /// Not there, not a directory, or not writable — one answer, as every path refusal is.
    Missing,
}

impl UploadError {
    pub fn code(&self) -> &'static str {
        match self {
            Self::BadName => "bad_name",
            Self::Missing => "not_found",
        }
    }
}

/// One path component, and nothing that could be read as more than one.
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && name != "." && name != ".." && !name.contains(['/', '\0'])
}

pub struct Written {
    pub path: PathBuf,
    pub bytes: u64,
}

pub struct Staged {
    file: Option<tokio::fs::File>,
    temp: PathBuf,
    entry: PathBuf,
    dir: PathBuf,
    name: String,
    bytes: u64,
}

impl Staged {
    pub async fn open(node: &Node, dir: &str, name: &str) -> Result<Self, UploadError> {
        if !valid_name(name) {
            return Err(UploadError::BadName);
        }
        let dir = FileRef::new(dir)
            .absolute(&node.config.journal_home())
            .map_err(|_| UploadError::Missing)?;
        let meta = tokio::fs::metadata(&dir)
            .await
            .map_err(|_| UploadError::Missing)?;
        if !meta.is_dir() {
            return Err(UploadError::Missing);
        }
        let id = ulid::Ulid::generate();
        let temp = dir.join(format!("{TEMP_PREFIX}{id}"));
        let entry = ledger(&node.state_dir).join(id.to_string());
        if let Err(e) = remember(&entry, &temp).await {
            tracing::warn!(error = %e, "an upload could not be recorded, so it is refused");
            return Err(UploadError::Missing);
        }
        let file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .await
        {
            Ok(file) => file,
            Err(_) => {
                let _ = tokio::fs::remove_file(&entry).await;
                return Err(UploadError::Missing);
            }
        };
        Ok(Self {
            file: Some(file),
            temp,
            entry,
            dir,
            name: name.to_string(),
            bytes: 0,
        })
    }

    pub async fn write(&mut self, data: &[u8]) -> io::Result<()> {
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("already committed"))?;
        file.write_all(data).await?;
        self.bytes += data.len() as u64;
        Ok(())
    }

    pub async fn commit(mut self) -> io::Result<Written> {
        let mut file = self
            .file
            .take()
            .ok_or_else(|| io::Error::other("already committed"))?;
        file.flush().await?;
        file.sync_all().await?;
        drop(file);
        let (temp, dir, name) = (self.temp.clone(), self.dir.clone(), self.name.clone());
        let path = tokio::task::spawn_blocking(move || claim(&temp, &dir, &name))
            .await
            .map_err(io::Error::other)??;
        Ok(Written {
            path,
            bytes: self.bytes,
        })
    }
}

impl Drop for Staged {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.temp);
        let _ = std::fs::remove_file(&self.entry);
    }
}

const TEMP_PREFIX: &str = ".kampr-upload-";

pub fn idle(node: &Node) -> Duration {
    Duration::from_secs(node.config.limits.upload_idle_secs.max(1))
}

fn ledger(state_dir: &Path) -> PathBuf {
    state_dir.join("uploads")
}

async fn remember(entry: &Path, temp: &Path) -> io::Result<()> {
    if let Some(parent) = entry.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    tokio::fs::write(entry, temp.as_os_str().as_encoded_bytes()).await
}

/// Removes what a node killed mid-upload left behind. Only a name this module makes is ever
/// removed, so an entry that names anything else is dropped without touching what it names.
pub fn sweep(state_dir: &Path) {
    let Ok(entries) = std::fs::read_dir(ledger(state_dir)) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if let Ok(temp) = std::fs::read_to_string(&path) {
            let temp = PathBuf::from(temp);
            let ours = temp
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(TEMP_PREFIX));
            if ours && std::fs::remove_file(&temp).is_ok() {
                tracing::info!(path = %temp.display(), "removed an upload a stopped node left unfinished");
            }
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// `name`, else `name (1).ext`, `name (2).ext`… — the first one nothing holds.
///
/// A filesystem with no hard links (FAT on a mounted card) cannot make the claim atomic, so there
/// it falls back to checking and renaming, which a concurrent upload of the same name could race.
fn claim(temp: &Path, dir: &Path, name: &str) -> io::Result<PathBuf> {
    for n in 0..10_000u32 {
        let candidate = dir.join(numbered(name, n));
        match std::fs::hard_link(temp, &candidate) {
            Ok(()) => {
                let _ = std::fs::remove_file(temp);
                return Ok(candidate);
            }
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(_) if std::fs::symlink_metadata(&candidate).is_err() => {
                std::fs::rename(temp, &candidate)?;
                return Ok(candidate);
            }
            Err(_) => continue,
        }
    }
    Err(io::Error::from(io::ErrorKind::AlreadyExists))
}

fn numbered(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_string();
    }
    let path = Path::new(name);
    let stem = path.file_stem().map(|s| s.to_string_lossy()).unwrap_or_default();
    match path.extension() {
        Some(ext) => format!("{stem} ({n}).{}", ext.to_string_lossy()),
        None => format!("{stem} ({n})"),
    }
}

pub fn audit(node: &Node, device: &Device, peer: &str, written: &Written) {
    node.auth.audit().record(
        &Entry::new("file.upload")
            .device(&device.id, &device.name, device.role.as_str())
            .peer(peer)
            .detail(json!({ "path": written.path.display().to_string(), "bytes": written.bytes })),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_taken_name_is_numbered_before_its_last_extension() {
        assert_eq!(numbered("notes.tar.gz", 0), "notes.tar.gz");
        assert_eq!(numbered("notes.tar.gz", 2), "notes.tar (2).gz");
        assert_eq!(numbered("Makefile", 1), "Makefile (1)");
        assert_eq!(numbered(".env", 1), ".env (1)");
    }
}
