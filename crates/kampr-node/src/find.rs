use std::collections::VecDeque;
use std::path::{Component, Path, PathBuf};

use serde::Serialize;

use crate::git;

pub const MAX_MATCHES: usize = 20;

/// Directories nobody writes a path into by hand, and the ones that make a walk from `~` cost
/// minutes rather than milliseconds.
const NEVER_SOURCE: &[&str] = &["node_modules", "target", "build"];

#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub depth: usize,
    pub entries: usize,
}

/// A search is reachable by any device that may type and its base may be `~`, so the walk is held
/// to what a project tree needs rather than to what a home directory holds.
pub const BOUNDS: Bounds = Bounds {
    depth: 12,
    entries: 50_000,
};

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct Found {
    pub matches: Vec<String>,
    pub truncated: bool,
}

/// The files below `base` whose path ends in `tail`, whole segments only, shallowest first and
/// then by name, or `None` when there are none. A repository is asked through `git` so ignored
/// files stay out; anywhere else, and in a repository that lists nothing, `base` is walked.
pub fn below(base: &Path, tail: &str, bounds: Bounds) -> Option<Found> {
    let base = tidy(base);
    if !std::fs::metadata(&base).ok()?.is_dir() {
        return None;
    }
    let (mut matches, cut) = match git::files_below(&base) {
        Ok(listed) if !listed.is_empty() => {
            let found = listed
                .iter()
                .filter(|rel| ends_with(rel, tail))
                .map(|rel| base.join(rel))
                .filter(|path| path.is_file())
                .collect();
            (found, false)
        }
        _ => walk(&base, tail, bounds),
    };
    matches.sort_by(|a, b| {
        a.components()
            .count()
            .cmp(&b.components().count())
            .then_with(|| a.cmp(b))
    });
    matches.dedup();
    if matches.is_empty() {
        return None;
    }
    let truncated = cut || matches.len() > MAX_MATCHES;
    matches.truncate(MAX_MATCHES);
    Some(Found {
        matches: matches.iter().map(|p| p.display().to_string()).collect(),
        truncated,
    })
}

fn tidy(dir: &Path) -> PathBuf {
    dir.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect()
}

fn ends_with(rel: &str, tail: &str) -> bool {
    rel == tail || rel.strip_suffix(tail).is_some_and(|head| head.ends_with('/'))
}

/// Breadth first, so the bound on entries cuts the deepest part of the tree rather than whichever
/// branch happens to sort last. No link is followed into: a symlinked directory can loop.
fn walk(base: &Path, tail: &str, bounds: Bounds) -> (Vec<PathBuf>, bool) {
    let mut found = Vec::new();
    let mut seen = 0;
    let mut queue = VecDeque::from([(base.to_path_buf(), 0)]);
    while let Some((dir, depth)) = queue.pop_front() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.filter_map(Result::ok) {
            seen += 1;
            if seen > bounds.entries {
                return (found, true);
            }
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            let path = entry.path();
            if kind.is_dir() {
                if !name.starts_with('.') && !NEVER_SOURCE.contains(&name) && depth + 1 < bounds.depth {
                    queue.push_back((path, depth + 1));
                }
                continue;
            }
            let Ok(rel) = path.strip_prefix(base) else {
                continue;
            };
            if rel.to_str().is_some_and(|rel| ends_with(rel, tail)) && path.is_file() {
                found.push(path);
            }
        }
    }
    (found, false)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(root: &Path, rel: &str) {
        let path = root.join(rel);
        std::fs::create_dir_all(path.parent().expect("a parent")).expect("dirs");
        std::fs::write(path, "x\n").expect("write");
    }

    #[test]
    fn a_tail_matches_whole_segments_and_nothing_less() {
        assert!(ends_with("a.rs", "a.rs"));
        assert!(ends_with("src/a.rs", "a.rs"));
        assert!(ends_with("crates/x/src/a.rs", "x/src/a.rs"));
        assert!(!ends_with("src/ba.rs", "a.rs"));
        assert!(!ends_with("crates/notx/src/a.rs", "x/src/a.rs"));
    }

    #[test]
    fn the_walk_stops_at_its_depth() {
        let dir = tempfile::tempdir().expect("a directory");
        touch(dir.path(), "a/b/c/f.rs");
        touch(dir.path(), "a/f.rs");
        let shallow = Bounds {
            depth: 2,
            entries: 1000,
        };
        let found = below(dir.path(), "f.rs", shallow).expect("the shallow one");
        assert_eq!(found.matches, [dir.path().join("a/f.rs").display().to_string()]);
    }

    #[test]
    fn the_walk_stops_at_its_entry_bound_and_says_it_was_cut() {
        let dir = tempfile::tempdir().expect("a directory");
        for n in 0..10 {
            touch(dir.path(), &format!("d{n}/f.rs"));
        }
        let tight = Bounds {
            depth: 12,
            entries: 12,
        };
        let found = below(dir.path(), "f.rs", tight).expect("some of them");
        assert!(found.truncated);
        assert!(found.matches.len() < 10, "{:?}", found.matches);
    }

    #[test]
    fn a_link_to_a_directory_is_not_walked_into() {
        let dir = tempfile::tempdir().expect("a directory");
        touch(dir.path(), "real/f.rs");
        std::os::unix::fs::symlink(dir.path(), dir.path().join("real/loop")).expect("a link");
        let found = below(dir.path(), "f.rs", BOUNDS).expect("the real one");
        assert_eq!(
            found.matches,
            [dir.path().join("real/f.rs").display().to_string()]
        );
    }
}
