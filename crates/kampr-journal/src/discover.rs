use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use serde_json::Value;

use crate::error::JournalError;
use crate::process::active_since;

/// How many transcripts a cwd search will open. A pane that is live has by definition one of the
/// most recently written transcripts, so this is a bound on the search rather than on the history.
const MAX_CANDIDATES: usize = 64;

/// How far into a transcript to look for the working directory it declares. Claude writes `cwd`
/// on its first conversational record rather than on the header records above it, and a single
/// record can be large, so the byte bound is what keeps a search off a 40 MB file.
const HEAD_LINES: usize = 40;
const HEAD_BYTES: usize = 256 * 1024;

/// How much of a transcript's *end* to read for its latest timestamp. The same bound in the other
/// direction, and enough for many records even where one is large.
const TAIL_BYTES: u64 = 256 * 1024;

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

/// The newest timestamp anywhere in the last [`TAIL_BYTES`] of a transcript.
///
/// **This, not the head, is when a conversation last happened.** Ranking on the head asks when a
/// session *opened*, so a long-running one that started yesterday lost to a five-minute one
/// started this morning — and the long-running one is the conversation the pane is still sitting
/// in. Measured against a real pane: a 12-hour-dead 20 KB transcript served in preference to the
/// 9.9 MB one on screen.
pub fn latest_stamp(path: &Path) -> Option<String> {
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let from = len.saturating_sub(TAIL_BYTES);
    std::io::Seek::seek(&mut file, std::io::SeekFrom::Start(from)).ok()?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).ok()?;
    let text = String::from_utf8_lossy(&buffer);
    // A mid-file seek lands inside a record, and half a record is not JSON.
    let lines = text.lines().skip(usize::from(from > 0));
    lines
        .filter_map(|line| serde_json::from_str::<Value>(line).ok())
        .filter_map(|record| {
            record
                .get("timestamp")
                .and_then(Value::as_str)
                .map(str::to_string)
        })
        .max()
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

/// The newest candidate whose own records name `cwd`, out of those still being written after
/// `since`. Verifying rather than trusting a derived filename is what stops a near-miss serving a
/// different project's conversation.
///
/// Recency is the transcript's own RFC 3339 stamps, with mtime only as the tiebreak: a rollout
/// copied or checked out carries a modification time that says when it was written to this disk,
/// not when the conversation happened. mtime still bounds the *search*, so a machine with
/// thousands of sessions does not read them all.
///
/// The head decides *whether* a transcript belongs to this directory; the tail decides *how
/// recent* it is. Reading only the head answers the second question with the first one's data.
///
/// `since` is when the pane's harness process started, and it is the difference between "the
/// newest conversation in this directory" and "the newest conversation this process could
/// possibly have had": a transcript whose last record predates the process was written by some
/// other run, in the same directory, and serving it is the tool lying about what the agent said.
pub fn newest_declaring(
    candidates: Vec<PathBuf>,
    cwd: &Path,
    since: Option<SystemTime>,
    declared: impl Fn(&Value) -> Option<String>,
) -> Option<PathBuf> {
    let wanted = cwd.to_string_lossy();
    let wanted = wanted.trim_end_matches('/');
    let mut matches: Vec<(String, SystemTime, PathBuf)> = newest_first(candidates)
        .into_iter()
        .filter_map(|path| {
            if !head(&path).iter().any(|r| declared(r).as_deref() == Some(wanted)) {
                return None;
            }
            let stamp = latest_stamp(&path);
            let modified = modified(&path);
            if since.is_some_and(|since| !active_since(stamp.as_deref(), modified, since)) {
                return None;
            }
            Some((stamp.unwrap_or_default(), modified, path))
        })
        .collect();
    matches.sort();
    matches.pop().map(|(_, _, path)| path)
}

pub fn not_found(cwd: &Path) -> JournalError {
    JournalError::NotFound(cwd.to_string_lossy().to_string())
}
