mod index;
mod rank;

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Duration;

use serde::Serialize;

use crate::find::Bounds;
use index::Index;

pub const MAX_MATCHES: usize = 50;

/// Measured on the operator's workstation (release build): a home of 526k entries indexes its
/// first 200k in 0.36 s warm and 4-7 s with a cold page cache, and one query scans them in ~11 ms;
/// `~/dev` (125k) indexes in 0.13 s and this repository through `git` in ~15 ms. All 526k took
/// 3.3 s warm and ~114 MB, which is why the bound stops well short of a whole home.
pub const BOUNDS: Bounds = Bounds {
    depth: 16,
    entries: 200_000,
};

/// Long enough that every keystroke of one search is a scan of memory, short enough that a file
/// an agent just wrote turns up while the person is still looking for it.
const FRESH_FOR: Duration = Duration::from_secs(45);

const BASES: usize = 4;

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Named {
    pub path: String,
    pub dir: bool,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Names {
    pub matches: Vec<Named>,
    pub truncated: bool,
    pub indexed: usize,
    pub cut: bool,
}

#[derive(Default)]
struct Slot {
    index: Mutex<Option<Arc<Index>>>,
}

pub struct Cache {
    bounds: Bounds,
    fresh_for: Duration,
    slots: Mutex<VecDeque<(PathBuf, Arc<Slot>)>>,
}

static SHARED: LazyLock<Cache> = LazyLock::new(|| Cache::new(BOUNDS, FRESH_FOR));

impl Cache {
    pub fn new(bounds: Bounds, fresh_for: Duration) -> Self {
        Self {
            bounds,
            fresh_for,
            slots: Mutex::new(VecDeque::new()),
        }
    }

    fn slot(&self, base: &Path) -> Arc<Slot> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        let slot = match slots.iter().position(|(at, _)| at == base) {
            Some(i) => slots.remove(i).map(|(_, slot)| slot).unwrap_or_default(),
            None => Arc::default(),
        };
        slots.push_front((base.to_path_buf(), slot.clone()));
        slots.truncate(BASES);
        slot
    }

    /// The slot's lock is held while it builds, so the keystrokes that arrive during the first
    /// walk of a large base wait for that one walk rather than each starting their own.
    fn index(&self, base: &Path) -> Arc<Index> {
        let slot = self.slot(base);
        let mut held = slot.index.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(index) = held.as_ref().filter(|i| i.built.elapsed() < self.fresh_for) {
            return index.clone();
        }
        let index = Arc::new(index::build(base, self.bounds));
        *held = Some(index.clone());
        index
    }

    /// `None` only when `base` is not a directory; no match is an empty answer, so a client can
    /// read a refusal as a node that cannot search at all.
    pub fn search(&self, base: &Path, query: &str) -> Option<Names> {
        let base = crate::find::tidy(base);
        if !std::fs::metadata(&base).ok()?.is_dir() {
            return None;
        }
        let index = self.index(&base);
        let needle = query.trim().to_lowercase();
        let (best, total) = rank::best(&index.items, &needle, MAX_MATCHES);
        Some(Names {
            matches: best
                .into_iter()
                .map(|item| Named {
                    path: base.join(&*item.rel).display().to_string(),
                    dir: item.dir,
                })
                .collect(),
            truncated: total > MAX_MATCHES,
            indexed: index.items.len(),
            cut: index.cut,
        })
    }
}

pub fn search(base: &Path, query: &str) -> Option<Names> {
    SHARED.search(base, query)
}

#[cfg(test)]
mod tests;
