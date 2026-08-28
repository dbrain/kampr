use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

/// How long `git` is given before it is abandoned.
///
/// A repository on a cold or network filesystem can take seconds to answer, and this route is
/// reachable by any device that may type — so the bound is on the node's own worker, not on the
/// operator's patience. A diff that has not arrived by now is one the reader has already stopped
/// waiting for.
const DEADLINE: Duration = Duration::from_secs(5);

/// The largest diff this node will hand back.
///
/// The same ceiling the attachment route applies, and for the same reason: a generated file with
/// a million changed lines is a diff nothing can render and a body nothing should carry.
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub enum DiffError {
    /// Not a repository, `git` is not installed, or the file is not tracked. All three are the
    /// same answer to a reader — there is no diff to show — and telling them apart from outside
    /// would map the filesystem.
    None,
    TooLarge,
}

/// What `git` says has changed in one file since HEAD, as a unified diff.
///
/// **Run in the file's own directory rather than in a repository this node worked out.** Finding
/// the root would mean walking upwards from a path that arrived over the network, and `git` does
/// that walk itself, under its own rules, including the ones about ownership and about `.git`
/// files that point elsewhere. `-C` is the whole of the sandboxing this needs.
///
/// An empty answer is [`DiffError::None`] and not an empty diff: an unchanged file and a file
/// `git` has never heard of produce the same empty stdout, and a reader offered an empty diff
/// viewer has been told the file is unchanged when it may simply be untracked.
pub fn diff_against_head(path: &Path) -> Result<String, DiffError> {
    let dir = path.parent().ok_or(DiffError::None)?;
    let name = path.file_name().ok_or(DiffError::None)?;
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args([
            "--no-optional-locks",
            "diff",
            "--no-color",
            "--no-ext-diff",
            "HEAD",
            "--",
        ])
        .arg(name)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        // A pager here would never exit, and a credential prompt would wait for a terminal that
        // does not exist.
        .env("GIT_PAGER", "cat")
        .env("GIT_TERMINAL_PROMPT", "0")
        .spawn()
        .map_err(|_| DiffError::None)?;

    let deadline = std::time::Instant::now() + DEADLINE;
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(20));
            }
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(DiffError::None);
            }
            Err(_) => return Err(DiffError::None),
        }
    }
    let out = child.wait_with_output().map_err(|_| DiffError::None)?;
    if !out.status.success() {
        return Err(DiffError::None);
    }
    if out.stdout.len() > MAX_BYTES {
        return Err(DiffError::TooLarge);
    }
    let text = String::from_utf8(out.stdout).map_err(|_| DiffError::None)?;
    match text.trim().is_empty() {
        true => Err(DiffError::None),
        false => Ok(text),
    }
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

        let diff = diff_against_head(&dir.join("notes.md")).expect("a diff");

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

        assert_eq!(diff_against_head(&dir.join("notes.md")), Err(DiffError::None));
        assert_eq!(diff_against_head(&dir.join("fresh.md")), Err(DiffError::None));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_path_outside_any_repository_is_no_diff_rather_than_an_error_worth_reporting() {
        let dir = std::env::temp_dir().join(format!("kampr-git-bare-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("scratch");
        std::fs::write(dir.join("loose.md"), "no repo here\n").expect("write");

        assert_eq!(diff_against_head(&dir.join("loose.md")), Err(DiffError::None));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
