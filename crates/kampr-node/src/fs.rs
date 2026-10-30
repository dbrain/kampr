use std::path::{Component, Path, PathBuf};
use std::time::UNIX_EPOCH;

use serde::Serialize;

use crate::git::{self, Change, Head};

pub const MAX_ENTRIES: usize = 5000;

#[derive(Debug, Serialize)]
pub struct Listing {
    pub path: String,
    pub parent: Option<String>,
    pub home: String,
    pub entries: Vec<Entry>,
    pub truncated: bool,
    pub repo: Option<RepoHead>,
}

#[derive(Debug, Serialize)]
pub struct RepoHead {
    #[serde(flatten)]
    pub head: Head,
    pub changed: usize,
}

#[derive(Debug, Serialize)]
pub struct Entry {
    pub name: String,
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target_kind: Option<Option<&'static str>>,
    pub bytes: Option<u64>,
    pub mtime: Option<i64>,
    pub git: Option<String>,
}

impl Entry {
    fn is_dirish(&self) -> bool {
        self.kind == "dir" || self.target_kind == Some(Some("dir"))
    }
}

/// `dir` without its trailing separators or `.` segments, and not canonicalised: the client navigates by
/// the path it asked for, so a symlinked directory stays where the reader thinks it is.
fn tidy(dir: &Path) -> PathBuf {
    dir.components()
        .filter(|c| !matches!(c, Component::CurDir))
        .collect()
}

fn kind_of(meta: &std::fs::Metadata) -> &'static str {
    if meta.is_dir() {
        "dir"
    } else if meta.is_file() {
        "file"
    } else {
        "other"
    }
}

/// One directory, sorted dirs-first then by name ignoring case, with what `git` says of each
/// entry.
///
/// **Nothing in it is opened.** `symlink_metadata` reads the entry itself and `metadata` only
/// stats a link's target, so a fifo or a device in the listing is named rather than read from —
/// opening a fifo with no writer blocks for ever.
pub fn list(dir: &Path, home: &Path) -> Option<Listing> {
    let dir = tidy(dir);
    if !std::fs::metadata(&dir).ok()?.is_dir() {
        return None;
    }
    let mut entries: Vec<Entry> = std::fs::read_dir(&dir)
        .ok()?
        .filter_map(Result::ok)
        .map(|item| {
            let name = item.file_name().to_string_lossy().into_owned();
            let own = std::fs::symlink_metadata(item.path()).ok();
            let link = own.as_ref().is_some_and(|m| m.file_type().is_symlink());
            let kind = match (&own, link) {
                (_, true) => "link",
                (Some(meta), false) => kind_of(meta),
                (None, false) => "other",
            };
            Entry {
                name,
                kind,
                target_kind: link.then(|| std::fs::metadata(item.path()).ok().map(|m| kind_of(&m))),
                bytes: own.as_ref().filter(|m| m.is_file()).map(std::fs::Metadata::len),
                mtime: own
                    .as_ref()
                    .and_then(|m| m.modified().ok())
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map(|d| d.as_secs() as i64),
                git: None,
            }
        })
        .collect();
    entries.sort_by(|a, b| {
        b.is_dirish()
            .cmp(&a.is_dirish())
            .then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase()))
            .then_with(|| a.name.cmp(&b.name))
    });
    let truncated = entries.len() > MAX_ENTRIES;
    entries.truncate(MAX_ENTRIES);

    let repo = git::status(&dir, false).ok().map(|status| {
        mark(&mut entries, &status.prefix, &status.changes);
        RepoHead {
            head: status.head,
            changed: status.changes.len(),
        }
    });
    Some(Listing {
        path: dir.display().to_string(),
        parent: dir.parent().map(|p| p.display().to_string()),
        home: home.display().to_string(),
        entries,
        truncated,
        repo,
    })
}

/// Each entry's letter, out of the one `status` taken for the whole repository. Paths there are
/// relative to the root and `prefix` is where this directory sits under it.
///
/// Untracked files are collapsed to their directory (`-unormal`), so a listing *inside* one is
/// matched by its ancestor and every entry in it is untracked.
fn mark(entries: &mut [Entry], prefix: &str, changes: &[Change]) {
    let inside_untracked = changes
        .iter()
        .any(|c| c.marker() == '?' && c.path.ends_with('/') && prefix.starts_with(&c.path));
    for entry in entries.iter_mut() {
        let own = format!("{prefix}{}", entry.name);
        let beneath = format!("{own}/");
        let letter = match (inside_untracked, entry.kind) {
            (true, _) => Some('?'),
            (false, "dir") => changes
                .iter()
                .any(|c| c.path.starts_with(&beneath))
                .then_some('M'),
            (false, _) => changes.iter().find(|c| c.path == own).map(Change::marker),
        };
        entry.git = letter.map(String::from);
    }
}
