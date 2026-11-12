use std::collections::{HashSet, VecDeque};
use std::path::Path;
use std::time::Instant;

use crate::find::{Bounds, NEVER_SOURCE};
use crate::git;

pub struct Item {
    pub rel: Box<str>,
    folded: Option<Box<str>>,
    pub dir: bool,
    pub depth: u16,
}

impl Item {
    pub(super) fn new(rel: String, dir: bool) -> Self {
        let lower = rel.to_lowercase();
        let folded = (lower != rel).then(|| lower.into_boxed_str());
        let depth = rel.matches('/').count().try_into().unwrap_or(u16::MAX);
        Self {
            rel: rel.into_boxed_str(),
            folded,
            dir,
            depth,
        }
    }

    /// Most paths are lowercase already, and a home's index is 200k of them, so only the ones that
    /// differ hold a second copy.
    pub fn lower(&self) -> &str {
        self.folded.as_deref().unwrap_or(&self.rel)
    }

    pub fn lower_name(&self) -> &str {
        let lower = self.lower();
        lower.rsplit('/').next().unwrap_or(lower)
    }
}

pub struct Index {
    pub items: Vec<Item>,
    pub cut: bool,
    pub built: Instant,
}

/// A repository is asked through `git` so an ignored file is never offered; anywhere else, and in
/// a repository that lists nothing (a base inside an ignored directory), `base` is walked.
pub fn build(base: &Path, bounds: Bounds) -> Index {
    let (items, cut) = match git::files_below(base) {
        Ok(listed) if !listed.is_empty() => from_git(listed, bounds),
        _ => walk(base, bounds),
    };
    Index {
        items,
        cut,
        built: Instant::now(),
    }
}

/// `git` names files only, so every directory is derived from the paths below it.
fn from_git(listed: Vec<String>, bounds: Bounds) -> (Vec<Item>, bool) {
    let mut items = Vec::new();
    let mut dirs = HashSet::new();
    for rel in listed {
        if rel.matches('/').count() >= bounds.depth {
            continue;
        }
        for (at, _) in rel.match_indices('/') {
            let dir = &rel[..at];
            if !dirs.contains(dir) {
                dirs.insert(dir.to_string());
                items.push(Item::new(dir.to_string(), true));
            }
        }
        items.push(Item::new(rel, false));
        if items.len() >= bounds.entries {
            items.truncate(bounds.entries);
            return (items, true);
        }
    }
    (items, false)
}

/// Breadth first, so the bound on entries cuts the deepest part of the tree. No link is followed
/// into: a symlinked directory can loop.
fn walk(base: &Path, bounds: Bounds) -> (Vec<Item>, bool) {
    let mut items = Vec::new();
    let mut queue = VecDeque::from([(base.to_path_buf(), String::new(), 0)]);
    while let Some((dir, prefix, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let rel = format!("{prefix}{name}");
            if kind.is_dir() {
                if name.starts_with('.') || NEVER_SOURCE.contains(&name) {
                    continue;
                }
                if depth + 1 < bounds.depth {
                    queue.push_back((entry.path(), format!("{rel}/"), depth + 1));
                }
                items.push(Item::new(rel, true));
            } else {
                items.push(Item::new(rel, false));
            }
            if items.len() >= bounds.entries {
                return (items, true);
            }
        }
    }
    (items, false)
}
