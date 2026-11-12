use std::process::Command;

use super::index::Item;
use super::*;

fn touch(root: &Path, rel: &str) {
    let path = root.join(rel);
    std::fs::create_dir_all(path.parent().expect("a parent")).expect("dirs");
    std::fs::write(path, "x\n").expect("write");
}

fn rels(root: &Path, names: &Names) -> Vec<String> {
    names
        .matches
        .iter()
        .map(|m| {
            let rel = Path::new(&m.path).strip_prefix(root).expect("below the base");
            format!("{}{}", rel.display(), if m.dir { "/" } else { "" })
        })
        .collect()
}

fn items(rels: &[&str]) -> Vec<Item> {
    rels.iter()
        .map(|rel| match rel.strip_suffix('/') {
            Some(dir) => Item::new(dir.to_string(), true),
            None => Item::new(rel.to_string(), false),
        })
        .collect()
}

fn fresh() -> Cache {
    Cache::new(BOUNDS, FRESH_FOR)
}

#[test]
fn names_rank_exact_then_prefix_then_substring_then_in_order() {
    let cases: &[(&str, &[&str], &[&str])] = &[
        (
            "lib",
            &[
                "src/mylib.rs",
                "a/b/lib",
                "src/library.rs",
                "lib",
                "l/i/x/b.rs",
                "liXb.rs",
            ],
            &["lib", "a/b/lib", "src/library.rs", "src/mylib.rs", "liXb.rs"],
        ),
        (
            "READ",
            &["docs/readme.md", "README.md"],
            &["README.md", "docs/readme.md"],
        ),
        ("fsrs", &["src/fs.rs", "src/srf.rs"], &["src/fs.rs"]),
        (
            "node/src",
            &["crates/kampr-node/src/fs.rs", "node/src/", "src/node.rs"],
            &["node/src/", "crates/kampr-node/src/fs.rs"],
        ),
        ("zz", &["a.rs", "b/"], &[]),
        (
            "a.rs",
            &["b/a.rs", "a/a.rs", "c.rs/a.rs"],
            &["a/a.rs", "b/a.rs", "c.rs/a.rs"],
        ),
    ];
    for (query, have, want) in cases {
        let have = items(have);
        let needle = query.to_lowercase();
        let (best, total) = rank::best(&have, &needle, MAX_MATCHES);
        let got: Vec<String> = best
            .iter()
            .map(|i| format!("{}{}", i.rel, if i.dir { "/" } else { "" }))
            .collect();
        assert_eq!(got, *want, "{query:?}");
        assert_eq!(total, want.len(), "{query:?}");
    }
}

#[test]
fn past_the_limit_the_best_are_kept_and_the_total_says_how_many_matched() {
    let have: Vec<Item> = (0..120)
        .rev()
        .map(|n| Item::new(format!("d{n:03}/f.rs"), false))
        .collect();
    let (best, total) = rank::best(&have, "f.rs", 3);
    let got: Vec<&str> = best.iter().map(|i| &*i.rel).collect();
    assert_eq!(got, ["d000/f.rs", "d001/f.rs", "d002/f.rs"]);
    assert_eq!(total, 120);
}

#[test]
fn the_walk_names_directories_and_skips_what_is_never_source_and_links() {
    let dir = tempfile::tempdir().expect("a directory");
    let root = dir.path();
    for rel in [
        "src/parse/parser.rs",
        ".cache/parser.rs",
        "node_modules/x/parser.rs",
        "target/debug/parser.rs",
        "build/parser.rs",
        ".parser.toml",
    ] {
        touch(root, rel);
    }
    std::os::unix::fs::symlink(root, root.join("src/loop")).expect("a link");

    let found = fresh().search(root, "PARSE").expect("a directory");

    assert_eq!(
        rels(root, &found),
        ["src/parse/", "src/parse/parser.rs", ".parser.toml"]
    );
    assert!(!found.cut);
    assert!(!found.truncated);
    let looped = fresh().search(root, "loop").expect("a directory");
    assert_eq!(
        rels(root, &looped),
        ["src/loop"],
        "a link is named, never walked into"
    );
}

#[test]
fn the_walk_stops_at_its_bounds_and_says_it_was_cut() {
    let dir = tempfile::tempdir().expect("a directory");
    touch(dir.path(), "a/b/c/deep.rs");
    touch(dir.path(), "a/shallow.rs");
    let shallow = Cache::new(
        Bounds {
            depth: 2,
            entries: 1000,
        },
        FRESH_FOR,
    );
    let found = shallow.search(dir.path(), "rs").expect("a directory");
    assert_eq!(rels(dir.path(), &found), ["a/shallow.rs"]);
    assert!(!found.cut);

    for n in 0..10 {
        touch(dir.path(), &format!("d{n}/f.rs"));
    }
    let tight = Cache::new(
        Bounds {
            depth: 16,
            entries: 12,
        },
        FRESH_FOR,
    );
    let found = tight.search(dir.path(), "f.rs").expect("a directory");
    assert!(found.cut);
    assert_eq!(found.indexed, 12);
}

fn git(dir: &Path, args: &[&str]) {
    let ok = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["-c", "user.email=t@example.com", "-c", "user.name=t"])
        .args(args)
        .env("GIT_CONFIG_GLOBAL", "/dev/null")
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .status()
        .expect("git")
        .success();
    assert!(ok, "git {args:?}");
}

#[test]
fn in_a_repository_git_decides_and_its_directories_are_named_too() {
    let dir = tempfile::tempdir().expect("a directory");
    let root = dir.path();
    git(root, &["init", "-q"]);
    touch(root, "crates/kampr-node/src/fs.rs");
    touch(root, "vendor/kampr-node/fs.rs");
    touch(root, "untracked/fs.rs");
    std::fs::write(root.join(".gitignore"), "vendor/\n").expect("write");
    git(root, &["add", "crates", ".gitignore"]);

    let found = fresh().search(root, "fs").expect("a directory");
    assert_eq!(
        rels(root, &found),
        ["untracked/fs.rs", "crates/kampr-node/src/fs.rs"]
    );

    let found = fresh().search(root, "kampr-node").expect("a directory");
    assert_eq!(rels(root, &found), ["crates/kampr-node/"]);

    let below = root.join("crates");
    let found = fresh().search(&below, "src").expect("a directory");
    assert_eq!(
        rels(&below, &found),
        ["kampr-node/src/"],
        "only what is below the base"
    );
}

#[test]
fn an_index_is_reused_while_fresh_and_rebuilt_once_it_is_not() {
    let dir = tempfile::tempdir().expect("a directory");
    touch(dir.path(), "a/one.rs");
    let cache = fresh();
    assert_eq!(cache.search(dir.path(), "one").expect("a dir").matches.len(), 1);
    touch(dir.path(), "b/two.rs");
    assert!(
        cache.search(dir.path(), "two").expect("a dir").matches.is_empty(),
        "the second keystroke scans what the first built"
    );

    let stale = Cache::new(BOUNDS, Duration::ZERO);
    assert_eq!(stale.search(dir.path(), "one").expect("a dir").matches.len(), 1);
    touch(dir.path(), "c/three.rs");
    assert_eq!(stale.search(dir.path(), "three").expect("a dir").matches.len(), 1);
}

#[test]
fn only_the_most_recent_bases_are_held() {
    let dirs: Vec<_> = (0..=BASES)
        .map(|_| tempfile::tempdir().expect("a directory"))
        .collect();
    let cache = fresh();
    for dir in &dirs {
        cache.search(dir.path(), "x");
    }
    let held: Vec<PathBuf> = cache
        .slots
        .lock()
        .expect("unpoisoned")
        .iter()
        .map(|(at, _)| at.clone())
        .collect();
    assert_eq!(held.len(), BASES);
    assert!(!held.contains(&dirs[0].path().to_path_buf()));
}

#[test]
fn a_base_that_is_not_a_directory_is_the_only_refusal() {
    let dir = tempfile::tempdir().expect("a directory");
    touch(dir.path(), "f.rs");
    assert!(fresh().search(&dir.path().join("f.rs"), "f").is_none());
    assert!(fresh().search(&dir.path().join("absent"), "f").is_none());
    let none = fresh()
        .search(dir.path(), "nothing-like-it")
        .expect("a directory");
    assert!(none.matches.is_empty());
    assert_eq!(none.indexed, 1);
}
