use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::error::JournalError;

/// How many transcripts a cwd search will open. A pane that is live has by definition one of the
/// most recently written transcripts, so this is a bound on the search rather than on the history.
const MAX_CANDIDATES: usize = 64;

/// How far into a transcript to look for the working directory it declares. Claude writes `cwd`
/// on its first conversational record rather than on the header records above it, and a single
/// record can be large, so the byte bound is what keeps a search off a 40 MB file.
const HEAD_LINES: usize = 40;
const HEAD_BYTES: usize = 256 * 1024;

pub fn jsonl_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "jsonl"))
        .collect()
}

pub fn subdirectories(dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .collect()
}

pub fn descend(root: &Path, levels: usize) -> Vec<PathBuf> {
    let mut level = vec![root.to_path_buf()];
    for _ in 0..levels {
        level = level.iter().flat_map(|d| subdirectories(d)).collect();
    }
    level
}

fn modified(path: &Path) -> SystemTime {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

pub fn newest_first(mut files: Vec<PathBuf>) -> Vec<PathBuf> {
    files.sort_by_key(|p| std::cmp::Reverse(modified(p)));
    files.truncate(MAX_CANDIDATES);
    files
}

pub fn head(path: &Path) -> Vec<Value> {
    let Ok(file) = std::fs::File::open(path) else {
        return Vec::new();
    };
    let mut buffer = Vec::new();
    if std::io::Read::read_to_end(&mut file.take(HEAD_BYTES as u64), &mut buffer).is_err() {
        return Vec::new();
    }
    String::from_utf8_lossy(&buffer)
        .lines()
        .take(HEAD_LINES)
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// The newest candidate whose own records name `cwd`. Verifying rather than trusting a derived
/// filename is what stops a near-miss serving a different project's conversation.
///
/// Recency is the transcript's own RFC 3339 stamps, with mtime only as the tiebreak: a rollout
/// copied or checked out carries a modification time that says when it was written to this disk,
/// not when the conversation happened. mtime still bounds the *search*, so a machine with
/// thousands of sessions does not read them all.
pub fn newest_declaring(
    candidates: Vec<PathBuf>,
    cwd: &Path,
    declared: impl Fn(&Value) -> Option<String>,
) -> Option<PathBuf> {
    let wanted = cwd.to_string_lossy();
    let wanted = wanted.trim_end_matches('/');
    let mut matches: Vec<(String, SystemTime, PathBuf)> = newest_first(candidates)
        .into_iter()
        .filter_map(|path| {
            let head = head(&path);
            if !head.iter().any(|r| declared(r).as_deref() == Some(wanted)) {
                return None;
            }
            let stamp = head
                .iter()
                .filter_map(|r| r.get("timestamp").and_then(Value::as_str))
                .max()
                .unwrap_or_default()
                .to_string();
            Some((stamp, modified(&path), path))
        })
        .collect();
    matches.sort();
    matches.pop().map(|(_, _, path)| path)
}

pub fn not_found(cwd: &Path) -> JournalError {
    JournalError::NotFound(cwd.to_string_lossy().to_string())
}
