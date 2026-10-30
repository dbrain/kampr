use std::ffi::OsStr;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

/// How long `git` is given before it is abandoned.
///
/// A repository on a cold or network filesystem can take seconds to answer, and this route is
/// reachable by any device that may type — so the bound is on the node's own worker, not on the
/// operator's patience. An answer that has not arrived by now is one the reader has already
/// stopped waiting for.
const DEADLINE: Duration = Duration::from_secs(5);

/// The largest answer this node will hand back. Git's output is held whole to be checked and
/// typed, so this is a memory bound: a generated file with a million changed lines is a diff
/// nothing can render, and 64 MiB is far past any that something can.
const MAX_BYTES: usize = 64 * 1024 * 1024;

const MAX_CHANGES: usize = 2000;
const LOG_LENGTH: &str = "50";

#[derive(Debug, PartialEq, Eq)]
pub enum GitError {
    /// Not a repository, `git` is not installed, a rev that names nothing, or a file that is not
    /// tracked. All the same answer to a reader, and telling them apart from outside would map the
    /// filesystem.
    None,
    TooLarge,
}

/// `git -C <dir> <args>`, bounded in time and in bytes, and its stdout if it exited cleanly.
///
/// **Run in a directory the path names rather than in a repository this node worked out.**
/// Finding the root would mean walking upwards from a path that arrived over the network, and
/// `git` does that walk itself, under its own rules, including the ones about ownership and about
/// `.git` files that point elsewhere. `-C` is the whole of the sandboxing this needs.
fn run<I, S>(dir: &Path, args: I) -> Result<Vec<u8>, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .arg("--no-optional-locks")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // A pager here would never exit, and a credential prompt would wait for a terminal that
        // does not exist.
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .spawn()
        .map_err(|_| GitError::None)?;
    // Drained while the child runs: `git` blocks on a full pipe, so waiting for its exit first
    // stalls any answer past the pipe buffer until the deadline. Stopping at the ceiling closes
    // the pipe, and the child dies of it rather than writing on.
    let stdout = child.stdout.take().ok_or(GitError::None)?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout
            .take(MAX_BYTES as u64 + 1)
            .read_to_end(&mut out)
            .map(|_| out)
    });

    let deadline = Instant::now() + DEADLINE;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = reader.join();
                return Err(GitError::None);
            }
        }
    };
    let out = reader
        .join()
        .map_err(|_| GitError::None)?
        .map_err(|_| GitError::None)?;
    if out.len() > MAX_BYTES {
        return Err(GitError::TooLarge);
    }
    match status.success() {
        true => Ok(out),
        false => Err(GitError::None),
    }
}

fn run_text<I, S>(dir: &Path, args: I) -> Result<String, GitError>
where
    I: IntoIterator<Item = S>,
    S: AsRef<OsStr>,
{
    String::from_utf8(run(dir, args)?).map_err(|_| GitError::None)
}

fn split(path: &Path) -> Result<(&Path, &OsStr), GitError> {
    Ok((
        path.parent().ok_or(GitError::None)?,
        path.file_name().ok_or(GitError::None)?,
    ))
}

/// What `git` says has changed in one file since HEAD, as a unified diff.
///
/// An empty answer is [`GitError::None`] and not an empty diff: an unchanged file and a file
/// `git` has never heard of produce the same empty stdout, and a reader offered an empty diff
/// viewer has been told the file is unchanged when it may simply be untracked.
/// Which two states of a file a diff compares.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Between {
    /// HEAD and the worktree: every uncommitted change.
    Head,
    /// HEAD and the index: what a commit would take.
    Staged,
    /// The index and the worktree: what a commit would leave behind.
    Unstaged,
}

pub fn diff(path: &Path, between: Between) -> Result<String, GitError> {
    let (dir, name) = split(path)?;
    let mut args: Vec<&OsStr> = vec!["diff".as_ref(), "--no-color".as_ref(), "--no-ext-diff".as_ref()];
    match between {
        Between::Head => args.push("HEAD".as_ref()),
        Between::Staged => args.extend::<[&OsStr; 2]>(["--cached".as_ref(), "HEAD".as_ref()]),
        Between::Unstaged => {}
    }
    args.extend::<[&OsStr; 2]>(["--".as_ref(), name]);
    let text = run_text(dir, args)?;
    match text.trim().is_empty() {
        true => Err(GitError::None),
        false => Ok(text),
    }
}

/// The file's bytes as they were at `rev`. `./` makes the name relative to `-C`'s directory
/// rather than to the repository's root, so no root has to be worked out here either.
///
/// `rev` is trusted to be the alphabet `Source::decode` allows: no `:`, no leading `-`.
pub fn show_at(path: &Path, rev: &str) -> Result<Vec<u8>, GitError> {
    let (dir, name) = split(path)?;
    let mut object = std::ffi::OsString::from(format!("{rev}:./"));
    object.push(name);
    run(dir, [OsStr::new("show"), OsStr::new("--no-textconv"), &object])
}

/// One commit as `git show` prints it. `^{commit}` keeps a sha naming a blob or a tree from being
/// shown as one.
pub fn commit_patch(dir: &Path, sha: &str) -> Result<String, GitError> {
    run_text(
        dir,
        [
            "show",
            "--no-color",
            "--no-ext-diff",
            "--stat",
            "--patch",
            "--format=fuller",
            &format!("{sha}^{{commit}}"),
            "--",
        ],
    )
}

#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct Head {
    pub root: String,
    pub branch: Option<String>,
    pub head: Option<String>,
    pub upstream: Option<String>,
    pub ahead: u64,
    pub behind: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Change {
    pub path: String,
    pub orig: Option<String>,
    pub staged: char,
    pub unstaged: char,
}

impl Change {
    /// The one letter a listing shows. Conflicts first, then what changes a file's existence or
    /// name, then an edit, so `AM` reads as the new file it is.
    pub fn marker(&self) -> char {
        const WORST: &[char] = &['U', 'D', 'A', 'R', 'C', 'M', '?'];
        WORST
            .iter()
            .copied()
            .find(|c| self.staged == *c || self.unstaged == *c)
            .unwrap_or('M')
    }
}

/// Where a directory sits in its repository, and the repository's state, out of one `status`.
pub struct Status {
    pub head: Head,
    /// The directory's own path relative to the root, with a trailing `/`, or empty at the root.
    pub prefix: String,
    pub changes: Vec<Change>,
}

/// `untracked_all` lists every untracked file rather than the directory that holds them, which
/// is what a list of changes wants and what a listing one directory deep does not need to pay for.
pub fn status(dir: &Path, untracked_all: bool) -> Result<Status, GitError> {
    let located = run_text(dir, ["rev-parse", "--show-toplevel", "--show-prefix"])?;
    let mut lines = located.lines();
    let root = lines
        .next()
        .filter(|r| !r.is_empty())
        .ok_or(GitError::None)?
        .to_string();
    let prefix = lines.next().unwrap_or_default().to_string();
    let untracked = match untracked_all {
        true => "-uall",
        false => "-unormal",
    };
    let raw = run(dir, ["status", "--porcelain=v2", "--branch", "-z", untracked])?;
    let (mut head, changes) = parse_status(&raw, root);
    if head.head.is_some() {
        head.head = run_text(dir, ["rev-parse", "--short", "HEAD"])
            .ok()
            .map(|s| s.trim().to_string());
    }
    Ok(Status {
        head,
        prefix,
        changes,
    })
}

fn parse_status(raw: &[u8], root: String) -> (Head, Vec<Change>) {
    let mut head = Head {
        root,
        branch: None,
        head: None,
        upstream: None,
        ahead: 0,
        behind: 0,
    };
    let mut changes = Vec::new();
    let mut fields = raw
        .split(|b| *b == 0)
        .map(|f| String::from_utf8_lossy(f).into_owned());
    while let Some(field) = fields.next() {
        if let Some(header) = field.strip_prefix("# ") {
            let (key, value) = header.split_once(' ').unwrap_or((header, ""));
            match key {
                "branch.oid" if value != "(initial)" => head.head = Some(value.to_string()),
                "branch.head" if value != "(detached)" => head.branch = Some(value.to_string()),
                "branch.upstream" => head.upstream = Some(value.to_string()),
                "branch.ab" => {
                    for part in value.split(' ') {
                        if let Some(n) = part.strip_prefix('+') {
                            head.ahead = n.parse().unwrap_or(0);
                        } else if let Some(n) = part.strip_prefix('-') {
                            head.behind = n.parse().unwrap_or(0);
                        }
                    }
                }
                _ => {}
            }
            continue;
        }
        let state = |xy: &str| {
            let mut chars = xy.chars().map(|c| match c {
                '.' => ' ',
                'T' => 'M',
                c => c,
            });
            (chars.next().unwrap_or(' '), chars.next().unwrap_or(' '))
        };
        let change = match field.as_bytes().first() {
            Some(b'1') => field.splitn(9, ' ').collect::<Vec<_>>().get(8).map(|path| {
                let (staged, unstaged) = state(&field[2..4]);
                Change {
                    path: (*path).to_string(),
                    orig: None,
                    staged,
                    unstaged,
                }
            }),
            Some(b'2') => {
                let orig = fields.next();
                field.splitn(10, ' ').collect::<Vec<_>>().get(9).map(|path| {
                    let (staged, unstaged) = state(&field[2..4]);
                    Change {
                        path: (*path).to_string(),
                        orig,
                        staged,
                        unstaged,
                    }
                })
            }
            Some(b'u') => field
                .splitn(11, ' ')
                .collect::<Vec<_>>()
                .get(10)
                .map(|path| Change {
                    path: (*path).to_string(),
                    orig: None,
                    staged: 'U',
                    unstaged: 'U',
                }),
            Some(b'?') => field.get(2..).map(|path| Change {
                path: path.to_string(),
                orig: None,
                staged: '?',
                unstaged: '?',
            }),
            _ => None,
        };
        changes.extend(change);
    }
    (head, changes)
}

#[derive(Debug, Serialize)]
pub struct RepoChange {
    pub path: String,
    pub abs: String,
    pub staged: String,
    pub unstaged: String,
    pub orig: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct RepoCommit {
    pub sha: String,
    pub short: String,
    pub subject: String,
    pub author: String,
    pub when: i64,
}

#[derive(Debug, Serialize)]
pub struct RepoSummary {
    #[serde(flatten)]
    pub head: Head,
    pub changes: Vec<RepoChange>,
    pub log: Vec<RepoCommit>,
}

/// The repository `dir` is in: where it stands, every change in it and its recent history.
pub fn summary(dir: &Path) -> Result<RepoSummary, GitError> {
    let status = status(dir, true)?;
    let root = PathBuf::from(&status.head.root);
    let changes = status
        .changes
        .into_iter()
        .take(MAX_CHANGES)
        .map(|c| RepoChange {
            abs: root.join(&c.path).display().to_string(),
            path: c.path,
            staged: c.staged.to_string(),
            unstaged: c.unstaged.to_string(),
            orig: c.orig,
        })
        .collect();
    // A repository with no commits has no log, and that is not a failure of the summary.
    let log = run_text(
        &root,
        ["log", "-n", LOG_LENGTH, "--format=%H%x1f%h%x1f%s%x1f%an%x1f%ct"],
    )
    .unwrap_or_default()
    .lines()
    .filter_map(|line| {
        let mut f = line.split('\u{1f}');
        Some(RepoCommit {
            sha: f.next()?.to_string(),
            short: f.next()?.to_string(),
            subject: f.next()?.to_string(),
            author: f.next()?.to_string(),
            when: f.next()?.parse().ok()?,
        })
    })
    .collect();
    Ok(RepoSummary {
        head: status.head,
        changes,
        log,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("kampr-git-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("scratch");
        let git = |args: &[&str]| {
            Command::new("git")
                .arg("-C")
                .arg(&dir)
                .args(args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("git")
        };
        git(&["init", "-q"]);
        git(&["config", "user.email", "t@example.com"]);
        git(&["config", "user.name", "t"]);
        std::fs::write(dir.join("notes.md"), "one\ntwo\n").expect("write");
        git(&["add", "-A"]);
        git(&["commit", "-qm", "first"]);
        dir
    }

    #[test]
    fn an_edited_file_comes_back_as_a_unified_diff_against_head() {
        let dir = repo("edited");
        std::fs::write(dir.join("notes.md"), "one\nTWO\n").expect("write");

        let diff = diff(&dir.join("notes.md"), Between::Head).expect("a diff");

        assert!(diff.contains("-two"), "{diff}");
        assert!(diff.contains("+TWO"), "{diff}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// An unchanged file and an untracked one both produce empty stdout, and a reader offered an
    /// empty diff has been told the file is unchanged when it may never have been committed.
    #[test]
    fn an_unchanged_file_and_an_untracked_one_are_both_no_diff_rather_than_an_empty_one() {
        let dir = repo("quiet");
        std::fs::write(dir.join("fresh.md"), "never committed\n").expect("write");

        assert_eq!(diff(&dir.join("notes.md"), Between::Head), Err(GitError::None));
        assert_eq!(diff(&dir.join("fresh.md"), Between::Head), Err(GitError::None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_outside_any_repository_is_no_diff_rather_than_an_error_worth_reporting() {
        let dir = std::env::temp_dir().join(format!("kampr-git-bare-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(dir.join("loose.md"), "no repo here\n").expect("write");

        assert_eq!(diff(&dir.join("loose.md"), Between::Head), Err(GitError::None));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
