//! Browsing a node's filesystem through the path-form attachment ids: `ls`, `repo`, `show` and
//! `commit`, over the real HTTP route, against a real git repository in a temp dir.

use crate::attachment::Harness;
use kampr_auth::Role;
use kampr_journal::attach::{FileRef, Source};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::process::Command;

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=Tester"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .output()
        .expect("git");
    assert!(
        out.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8").trim().to_string()
}

struct Repo {
    _dir: tempfile::TempDir,
    root: PathBuf,
    first: String,
}

/// Two commits, then a worktree with one of everything the listing has to say something about.
fn a_repo() -> Repo {
    let dir = tempfile::tempdir().expect("a directory");
    let root = dir.path().canonicalize().expect("a root").join("work");
    std::fs::create_dir_all(root.join("src")).expect("src");
    git(&root, &["init", "-q", "-b", "main"]);
    std::fs::write(root.join("README.md"), "readme\n").expect("write");
    std::fs::write(root.join("src/lib.rs"), "one\n").expect("write");
    std::fs::write(root.join("src/old.rs"), "old\n").expect("write");
    std::fs::write(root.join(".hidden"), "h\n").expect("write");
    std::os::unix::fs::symlink("src", root.join("link")).expect("a link");
    std::os::unix::fs::symlink("nowhere", root.join("dangling")).expect("a dangling link");
    git(&root, &["add", "-A"]);
    git(&root, &["commit", "-qm", "first"]);
    let first = git(&root, &["rev-parse", "HEAD"]);
    std::fs::write(root.join("src/lib.rs"), "two\n").expect("write");
    git(&root, &["commit", "-qam", "second"]);

    std::fs::write(root.join("src/lib.rs"), "three\n").expect("write");
    git(&root, &["mv", "src/old.rs", "src/new.rs"]);
    std::fs::write(root.join("Zeta.txt"), "z\n").expect("write");
    std::fs::create_dir_all(root.join("alpha")).expect("alpha");
    std::fs::write(root.join("alpha/inner.txt"), "i\n").expect("write");
    nix_mkfifo(&root.join("pipe"));
    Repo {
        _dir: dir,
        root,
        first,
    }
}

fn nix_mkfifo(path: &Path) {
    let status = Command::new("mkfifo").arg(path).status().expect("mkfifo");
    assert!(status.success());
}

struct Answer {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

impl Answer {
    fn json(&self) -> Value {
        assert_eq!(self.status, 200, "{}", String::from_utf8_lossy(&self.body));
        assert_eq!(self.content_type, "application/json");
        serde_json::from_slice(&self.body).expect("JSON")
    }

    fn text(&self) -> String {
        assert_eq!(self.status, 200, "{}", String::from_utf8_lossy(&self.body));
        assert_eq!(self.content_type, "text/plain; charset=utf-8");
        String::from_utf8(self.body.clone()).expect("utf-8")
    }
}

struct Browser {
    h: Harness,
    full: String,
}

impl Browser {
    async fn start() -> Self {
        let h = Harness::start().await;
        let full = format!("Bearer {}", h.token_for(Role::Full).await);
        Self { h, full }
    }

    fn anchor(&self) -> String {
        format!("{}/_", self.h.node.node_id())
    }

    async fn fetch(&self, source: &Source) -> Answer {
        self.fetch_as(&self.anchor(), &source.encode(), &self.full).await
    }

    async fn fetch_as(&self, pane: &str, id: &str, token: &str) -> Answer {
        let raw = self
            .h
            .get_raw(
                &format!("/api/attachment/{pane}/{id}"),
                &[("Authorization", token)],
            )
            .await;
        let split = raw.windows(4).position(|w| w == b"\r\n\r\n").expect("a head");
        let head = String::from_utf8_lossy(&raw[..split]).to_string();
        let status = head
            .split_whitespace()
            .nth(1)
            .and_then(|s| s.parse().ok())
            .expect("a status");
        let content_type = head
            .lines()
            .find_map(|l| l.strip_prefix("content-type: "))
            .unwrap_or_default()
            .to_string();
        Answer {
            status,
            content_type,
            body: raw[split + 4..].to_vec(),
        }
    }
}

fn ls(path: impl Into<PathBuf>) -> Source {
    Source::Ls(FileRef::new(path))
}

fn names(listing: &Value) -> Vec<&str> {
    listing["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .map(|e| e["name"].as_str().expect("a name"))
        .collect()
}

fn entry<'a>(listing: &'a Value, name: &str) -> &'a Value {
    listing["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .find(|e| e["name"] == name)
        .unwrap_or_else(|| panic!("no {name} in {listing}"))
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_lists_dirs_first_then_by_name_with_each_entrys_kind_and_git_state() {
    let b = Browser::start().await;
    let repo = a_repo();

    let listing = b.fetch(&ls(&repo.root)).await.json();

    assert_eq!(listing["path"], repo.root.display().to_string());
    assert_eq!(
        listing["parent"],
        repo.root.parent().unwrap().display().to_string()
    );
    assert_eq!(listing["home"], b.h.journals_home().display().to_string());
    assert_eq!(listing["truncated"], false);
    assert_eq!(
        names(&listing),
        [
            ".git",
            "alpha",
            "link",
            "src",
            ".hidden",
            "dangling",
            "pipe",
            "README.md",
            "Zeta.txt"
        ]
    );

    let readme = entry(&listing, "README.md");
    assert_eq!(readme["kind"], "file");
    assert_eq!(readme["bytes"], 7);
    assert!(readme["mtime"].as_i64().expect("an mtime") > 1_600_000_000);
    assert!(readme["git"].is_null(), "{readme}");
    assert!(readme.get("target_kind").is_none(), "{readme}");

    assert_eq!(entry(&listing, "src")["kind"], "dir");
    assert!(entry(&listing, "src")["bytes"].is_null());
    assert_eq!(entry(&listing, "src")["git"], "M");
    assert_eq!(entry(&listing, "alpha")["git"], "M");
    assert_eq!(entry(&listing, "Zeta.txt")["git"], "?");
    assert_eq!(entry(&listing, "link")["kind"], "link");
    assert_eq!(entry(&listing, "link")["target_kind"], "dir");
    assert_eq!(entry(&listing, "dangling")["kind"], "link");
    assert!(entry(&listing, "dangling")["target_kind"].is_null());
    assert_eq!(entry(&listing, "pipe")["kind"], "other");

    let head = git(&repo.root, &["rev-parse", "--short", "HEAD"]);
    let summary = &listing["repo"];
    assert_eq!(summary["root"], repo.root.display().to_string());
    assert_eq!(summary["branch"], "main");
    assert_eq!(summary["head"], head);
    assert!(summary["upstream"].is_null());
    assert_eq!(summary["ahead"], 0);
    assert_eq!(summary["behind"], 0);
    assert_eq!(summary["changed"], 4, "{summary}");

    let src = b.fetch(&ls(repo.root.join("src/"))).await.json();
    assert_eq!(src["path"], repo.root.join("src").display().to_string());
    assert_eq!(names(&src), ["lib.rs", "new.rs"]);
    assert_eq!(entry(&src, "lib.rs")["git"], "M");
    assert_eq!(entry(&src, "new.rs")["git"], "R");

    let inside = b.fetch(&ls(repo.root.join("alpha"))).await.json();
    assert_eq!(entry(&inside, "inner.txt")["git"], "?");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_outside_any_repository_has_no_repo_and_no_git_markers() {
    let b = Browser::start().await;
    let home = b.h.journals_home();
    std::fs::write(home.join("plain.txt"), "p").expect("write");

    let listing = b.fetch(&ls("~")).await.json();

    assert_eq!(listing["path"], home.display().to_string());
    assert!(listing["repo"].is_null(), "{listing}");
    assert!(entry(&listing, "plain.txt")["git"].is_null());

    let root = b.fetch(&ls("/")).await.json();
    assert_eq!(root["path"], "/");
    assert!(root["parent"].is_null());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_listing_past_five_thousand_entries_is_the_first_five_thousand_sorted() {
    let b = Browser::start().await;
    let dir = tempfile::tempdir().expect("a directory");
    for n in 0..5001 {
        std::fs::write(dir.path().join(format!("f{n:05}")), "").expect("write");
    }

    let listing = b.fetch(&ls(dir.path())).await.json();

    assert_eq!(listing["truncated"], true);
    let names = names(&listing);
    assert_eq!(names.len(), 5000);
    assert_eq!(names.first(), Some(&"f00000"));
    assert_eq!(names.last(), Some(&"f04999"));
}

#[tokio::test(flavor = "multi_thread")]
async fn the_repo_summary_names_every_change_and_the_recent_log() {
    let b = Browser::start().await;
    let repo = a_repo();

    let summary = b
        .fetch(&Source::Repo(FileRef::new(repo.root.join("src/lib.rs"))))
        .await
        .json();

    assert_eq!(summary["root"], repo.root.display().to_string());
    assert_eq!(summary["branch"], "main");
    let changes = summary["changes"].as_array().expect("changes");
    let change = |path: &str| {
        changes
            .iter()
            .find(|c| c["path"] == path)
            .unwrap_or_else(|| panic!("no {path} in {changes:?}"))
    };
    assert_eq!(changes.len(), 4, "{changes:?}");
    let lib = change("src/lib.rs");
    assert_eq!(lib["abs"], repo.root.join("src/lib.rs").display().to_string());
    assert_eq!(lib["staged"], " ");
    assert_eq!(lib["unstaged"], "M");
    assert!(lib["orig"].is_null());
    let renamed = change("src/new.rs");
    assert_eq!(renamed["staged"], "R");
    assert_eq!(renamed["orig"], "src/old.rs");
    assert_eq!(change("alpha/inner.txt")["staged"], "?");
    assert_eq!(change("Zeta.txt")["unstaged"], "?");

    let log = summary["log"].as_array().expect("a log");
    assert_eq!(log.len(), 2);
    assert_eq!(log[0]["subject"], "second");
    assert_eq!(log[0]["author"], "Tester");
    assert_eq!(log[1]["sha"], repo.first);
    assert_eq!(log[0]["short"], summary["head"]);
    assert!(log[1]["when"].as_i64().expect("a time") > 1_600_000_000);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_at_a_revision_comes_back_as_it_was_then() {
    let b = Browser::start().await;
    let repo = a_repo();
    let show = |path: &str, rev: &str| Source::Show {
        file: FileRef::new(repo.root.join(path)),
        rev: rev.into(),
    };

    assert_eq!(b.fetch(&show("src/lib.rs", "HEAD")).await.text(), "two\n");
    assert_eq!(b.fetch(&show("src/lib.rs", "HEAD~1")).await.text(), "one\n");
    assert_eq!(b.fetch(&show("src/lib.rs", &repo.first)).await.text(), "one\n");
    assert_eq!(
        b.fetch(&show("src/old.rs", "HEAD")).await.text(),
        "old\n",
        "a file gone from the worktree is still there at HEAD"
    );
    assert_eq!(b.fetch(&show("src/new.rs", "HEAD")).await.status, 404);
    assert_eq!(b.fetch(&show("src/lib.rs", "nosuchrev")).await.status, 404);
}

#[tokio::test(flavor = "multi_thread")]
async fn a_commit_comes_back_as_its_full_header_stat_and_patch() {
    let b = Browser::start().await;
    let repo = a_repo();

    let text = b
        .fetch(&Source::Commit {
            repo: FileRef::new(&repo.root),
            sha: repo.first[..8].to_string(),
        })
        .await
        .text();

    assert!(text.starts_with(&format!("commit {}", repo.first)), "{text}");
    assert!(text.contains("AuthorDate:"), "not --format=fuller: {text}");
    assert!(text.contains("first"), "{text}");
    assert!(
        text.contains("6 files changed, 6 insertions(+)"),
        "no --stat: {text}"
    );
    assert!(text.contains("+readme"), "no patch: {text}");

    let tree = git(&repo.root, &["rev-parse", "HEAD^{tree}"]);
    assert_eq!(
        b.fetch(&Source::Commit {
            repo: FileRef::new(&repo.root),
            sha: tree,
        })
        .await
        .status,
        404,
        "a sha that is not a commit"
    );
}

/// A change past what a pipe buffers. `git` blocks writing it until somebody reads, so a runner
/// that waits for the exit before reading stdout waits out its whole deadline and calls it no diff.
#[tokio::test(flavor = "multi_thread")]
async fn a_diff_larger_than_a_pipe_buffer_comes_back_whole() {
    let b = Browser::start().await;
    let repo = a_repo();
    let big: String = (0..20_000).map(|n| format!("line {n}\n")).collect();
    std::fs::write(repo.root.join("README.md"), &big).expect("write");

    let started = std::time::Instant::now();
    let diff = b
        .fetch(&Source::Diff(FileRef::new(repo.root.join("README.md"))))
        .await
        .text();

    assert!(diff.len() > 128 * 1024, "{}", diff.len());
    assert!(diff.contains("+line 19999"));
    assert!(started.elapsed() < std::time::Duration::from_secs(4));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_read_only_device_is_refused_every_browsing_form() {
    let b = Browser::start().await;
    let repo = a_repo();
    let readonly = format!("Bearer {}", b.h.token_for(Role::Readonly).await);
    for source in [
        ls(&repo.root),
        Source::Repo(FileRef::new(&repo.root)),
        Source::Show {
            file: FileRef::new(repo.root.join("README.md")),
            rev: "HEAD".into(),
        },
        Source::Commit {
            repo: FileRef::new(&repo.root),
            sha: repo.first.clone(),
        },
    ] {
        let answer = b.fetch_as(&b.anchor(), &source.encode(), &readonly).await;
        assert_eq!(answer.status, 403, "{source:?}");
        assert_eq!(b.fetch(&source).await.status, 200, "{source:?} as a full device");
    }
}

/// Everything that is not there, is not a directory or is not a valid ask is one answer, so the
/// route cannot be used to map the filesystem by status code.
#[tokio::test(flavor = "multi_thread")]
async fn every_refusal_is_the_same_404() {
    let b = Browser::start().await;
    let repo = a_repo();
    let plain = tempfile::tempdir().expect("a directory");
    let raw = |fields: &[&str]| {
        use base64::Engine;
        base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(fields.join("\u{1f}"))
    };
    let root = repo.root.display().to_string();
    let readme = repo.root.join("README.md").display().to_string();
    for id in [
        ls(repo.root.join("README.md")).encode(),
        ls(repo.root.join("absent")).encode(),
        ls("relative/dir").encode(),
        Source::Repo(FileRef::new(plain.path())).encode(),
        Source::Repo(FileRef::new(repo.root.join("absent/deeper"))).encode(),
        raw(&["show", &readme, "-p"]),
        raw(&["show", &readme, "--output=/tmp/kampr-pwned"]),
        raw(&["show", &readme, "HEAD:src/lib.rs"]),
        raw(&["commit", &root, "--stat"]),
        raw(&["commit", &root, "HEAD"]),
    ] {
        let answer = b.fetch_as(&b.anchor(), &id, &b.full).await;
        assert_eq!(
            answer.status,
            404,
            "{id}: {}",
            String::from_utf8_lossy(&answer.body)
        );
        assert_eq!(
            String::from_utf8_lossy(&answer.body),
            r#"{"error":"no such attachment"}"#
        );
    }
    assert!(!Path::new("/tmp/kampr-pwned").exists());
}

/// Path forms are routed by node id alone: the pane half is whatever the client had to hand.
#[tokio::test(flavor = "multi_thread")]
async fn a_path_form_is_answered_whatever_pane_it_is_anchored_on() {
    let b = Browser::start().await;
    let repo = a_repo();
    let id = ls(&repo.root).encode();
    let node = b.h.node.node_id().to_string();

    for pane in [format!("{node}/_"), format!("{node}/w9:p9")] {
        assert_eq!(b.fetch_as(&pane, &id, &b.full).await.status, 200, "{pane}");
    }
    assert_eq!(b.fetch_as("01JSOMEONEELSE/_", &id, &b.full).await.status, 404);
}

/// A file with a change in the index and another on top of it in the worktree: the two diffs are
/// two different answers, and a file with nothing staged has no staged diff at all.
#[tokio::test(flavor = "multi_thread")]
async fn staged_and_unstaged_changes_are_two_different_diffs() {
    let b = Browser::start().await;
    let repo = a_repo();
    let readme = repo.root.join("README.md");
    std::fs::write(&readme, "staged\n").expect("write");
    git(&repo.root, &["add", "README.md"]);
    std::fs::write(&readme, "unstaged\n").expect("write");

    let staged = b.fetch(&Source::Staged(FileRef::new(&readme))).await.text();
    let unstaged = b.fetch(&Source::Unstaged(FileRef::new(&readme))).await.text();
    assert!(
        staged.contains("-readme") && staged.contains("+staged"),
        "{staged}"
    );
    assert!(!staged.contains("unstaged"), "{staged}");
    assert!(
        unstaged.contains("-staged") && unstaged.contains("+unstaged"),
        "{unstaged}"
    );
    assert!(!unstaged.contains("readme"), "{unstaged}");

    let lib = FileRef::new(repo.root.join("src/lib.rs"));
    assert_eq!(b.fetch(&Source::Staged(lib.clone())).await.status, 404);
    assert!(b.fetch(&Source::Unstaged(lib)).await.text().contains("+three"));

    let readonly = format!("Bearer {}", b.h.token_for(Role::Readonly).await);
    for source in [
        Source::Staged(FileRef::new(&readme)),
        Source::Unstaged(FileRef::new(&readme)),
    ] {
        assert_eq!(
            b.fetch_as(&b.anchor(), &source.encode(), &readonly).await.status,
            403
        );
    }
}
