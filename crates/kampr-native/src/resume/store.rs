use kampr_core::resume::Command;
use serde::{Deserialize, Serialize};
use std::io::Write as _;
use std::os::unix::fs::{DirBuilderExt as _, OpenOptionsExt as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// One program that said how to bring it back.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Owner {
    pub pid: u32,
    /// Which process `pid` was, so that a recycled pid is not taken for it.
    pub start: Option<String>,
    pub time: i64,
    pub command: Command,
    /// The log offset of the report, below which a replay reads nothing.
    pub at: u64,
    /// Set before a resume runs and never cleared: a resume is not retried.
    #[serde(default)]
    pub tried: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct File {
    pub label: Option<String>,
    /// The log offset the last report that changed anything was read at.
    pub offset: u64,
    pub owners: Vec<Owner>,
}

/// `<dir>/<session id>.json`, 0600 in a 0700 directory, written whole and renamed into place.
/// Every change holds an `flock` on `<dir>/.lock`: the node and `kampr attach --restored` are
/// separate processes, and a record both take would be launched twice.
#[derive(Debug, Clone)]
pub struct Store {
    dir: PathBuf,
}

static SAVES: AtomicU64 = AtomicU64::new(0);

fn named(session: &str) -> bool {
    !session.is_empty() && session.bytes().all(|b| b.is_ascii_alphanumeric())
}

impl Store {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn read(&self, session: &str) -> File {
        self.load(session)
    }

    fn lock(&self) -> std::io::Result<std::fs::File> {
        private_dir(&self.dir)?;
        let lock = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(self.dir.join(".lock"))?;
        rustix::fs::flock(&lock, rustix::fs::FlockOperation::LockExclusive)?;
        Ok(lock)
    }

    /// `change` on the stored file, saved if it changed anything and removed once it holds nothing.
    pub fn update<T>(&self, session: &str, change: impl FnOnce(&mut File) -> T) -> std::io::Result<T> {
        if !named(session) {
            return Err(std::io::Error::other("not a session id"));
        }
        let _held = self.lock()?;
        let mut file = self.load(session);
        let before = serde_json::to_vec(&file).unwrap_or_default();
        let out = change(&mut file);
        if serde_json::to_vec(&file).unwrap_or_default() != before {
            self.save(session, &file)?;
        }
        Ok(out)
    }

    pub fn remove(&self, session: &str) {
        if named(session) {
            let _held = self.lock();
            let _ = std::fs::remove_file(self.path(session));
        }
    }

    pub fn all(&self) -> Vec<(String, File)> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let session = name.strip_suffix(".json")?.to_string();
                named(&session).then(|| (session.clone(), self.load(&session)))
            })
            .collect()
    }

    fn path(&self, session: &str) -> PathBuf {
        self.dir.join(format!("{session}.json"))
    }

    fn load(&self, session: &str) -> File {
        std::fs::read(self.path(session))
            .ok()
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    fn save(&self, session: &str, file: &File) -> std::io::Result<()> {
        if file.owners.is_empty() && file.offset == 0 {
            let _ = std::fs::remove_file(self.path(session));
            return Ok(());
        }
        let tmp = self.dir.join(format!(
            ".{session}.json.{}.{}.tmp",
            std::process::id(),
            SAVES.fetch_add(1, Ordering::Relaxed)
        ));
        let mut out = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        out.write_all(&serde_json::to_vec(file)?)?;
        out.sync_all()?;
        std::fs::rename(&tmp, self.path(session))
    }
}

fn private_dir(dir: &Path) -> std::io::Result<()> {
    std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir)
}
