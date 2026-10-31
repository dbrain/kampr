use kampr_core::push::{FileWatch, PulseRx, watch_files};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::time::Instant;
use tracing::debug;

/// One piece of work a wake makes owing, run no sooner than `floor` after it last ran: the first
/// change after a quiet spell is read at once, and a flood is read at the floor's rate.
pub(super) struct Paced {
    floor: Duration,
    last: Option<Instant>,
    owed: Option<Instant>,
}

impl Paced {
    pub(super) fn new(floor: Duration) -> Self {
        Self {
            floor,
            last: None,
            owed: None,
        }
    }

    pub(super) fn owe(&mut self) {
        let at = self.last.map_or_else(Instant::now, |last| last + self.floor);
        self.owe_at(at);
    }

    pub(super) fn owe_after(&mut self, wait: Duration) {
        self.owe_at(Instant::now() + wait);
    }

    fn owe_at(&mut self, at: Instant) {
        self.owed = Some(self.owed.map_or(at, |owed| owed.min(at)));
    }

    pub(super) fn owed(&self) -> bool {
        self.owed.is_some()
    }

    pub(super) fn at(&self) -> Option<Instant> {
        self.owed
    }

    pub(super) fn due(&self) -> bool {
        self.owed.is_some_and(|at| at <= Instant::now())
    }

    pub(super) fn ran(&mut self) {
        self.last = Some(Instant::now());
        self.owed = None;
    }
}

/// A watch on one transcript file, rebuilt when the pane moves to another or the file at the same
/// path is a new one: a watch is on an inode, so a transcript replaced by a rename is no longer
/// the file it watches.
#[derive(Default)]
pub(super) struct Tail {
    on: Option<(PathBuf, Option<(u64, u64)>)>,
    watch: Option<FileWatch>,
    rx: Option<PulseRx>,
}

impl Tail {
    /// True when a watch was put on a new file, whose writes since it was last read nothing saw.
    pub(super) fn follow(&mut self, path: Option<&Path>) -> bool {
        let on = path.map(|p| (p.to_path_buf(), inode(p)));
        if on == self.on {
            return false;
        }
        self.on = on;
        self.watch = path.and_then(|p| {
            watch_files(&[p.to_path_buf()])
                .inspect_err(|e| debug!(path = %p.display(), error = %e, "no watch on a transcript"))
                .ok()
        });
        self.rx = self.watch.as_ref().map(FileWatch::changes);
        path.is_some()
    }

    pub(super) fn path(&self) -> Option<&Path> {
        self.on.as_ref().map(|(p, _)| p.as_path())
    }

    pub(super) fn watching(&self) -> bool {
        self.rx.is_some()
    }

    pub(super) async fn changed(&mut self) {
        match &mut self.rx {
            Some(rx) => rx.changed().await,
            None => std::future::pending().await,
        }
    }
}

fn inode(path: &Path) -> Option<(u64, u64)> {
    std::fs::metadata(path).ok().map(|m| (m.dev(), m.ino()))
}

/// A watch on the directories a harness files its transcripts and its process-to-session records
/// in. The list is asked for again after every change, because a directory that did not exist
/// when it was last asked (a first run in a working directory, a new day) is announced by one
/// that did.
#[derive(Default)]
pub(super) struct Dirs {
    list: Vec<PathBuf>,
    stale: bool,
    watch: Option<FileWatch>,
    rx: Option<PulseRx>,
}

impl Dirs {
    pub(super) fn follow(&mut self, list: impl FnOnce() -> Vec<PathBuf>, key_moved: bool) {
        if !key_moved && !self.stale {
            return;
        }
        self.stale = false;
        let list = list();
        if list == self.list && self.watch.is_some() {
            return;
        }
        self.watch = (!list.is_empty())
            .then(|| {
                watch_files(&list)
                    .inspect_err(|e| debug!(error = %e, "no watch on a harness's directories"))
                    .ok()
            })
            .flatten();
        self.rx = self.watch.as_ref().map(FileWatch::changes);
        self.list = list;
    }

    pub(super) fn watching(&self) -> bool {
        self.rx.is_some()
    }

    pub(super) async fn changed(&mut self) {
        match &mut self.rx {
            Some(rx) => rx.changed().await,
            None => std::future::pending().await,
        }
        self.stale = true;
    }
}

/// True on a frame, false when `net` passed without one or there is no frame signal at all, which
/// the caller takes as a frame it may have missed. Waits on a clone, so the caller marks the
/// frame seen when it reads the screen and a wait cancelled by another arm loses nothing.
pub(super) async fn frame(frames: Option<&PulseRx>, net: Duration, fallback: Duration) -> bool {
    match frames {
        Some(rx) => rx.clone().changed_before(Instant::now() + net).await,
        None => {
            tokio::time::sleep(fallback).await;
            false
        }
    }
}

pub(super) async fn until(at: Option<Instant>) {
    match at {
        Some(at) => tokio::time::sleep_until(at).await,
        None => std::future::pending().await,
    }
}
