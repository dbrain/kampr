//! A file of any size, streamed off disk rather than read whole: served entire, served by range,
//! and a directory served as a zip built while it is sent. The real HTTP route, a real node.

use crate::attachment::Harness;
use crate::raw::{self, Reply};
use kampr_auth::Role;
use kampr_journal::attach::{FileRef, Source};
use std::io::Read;
use std::path::{Path, PathBuf};

pub(crate) const MIB: usize = 1024 * 1024;

pub(crate) fn pattern(len: usize) -> Vec<u8> {
    (0..len).map(|n| (n % 251) as u8 ^ (n >> 16) as u8).collect()
}

pub(crate) fn a_big_file(dir: &Path, len: usize) -> (PathBuf, Vec<u8>) {
    let path = dir.join("big.bin");
    let bytes = pattern(len);
    std::fs::write(&path, &bytes).expect("a big file");
    (path, bytes)
}

/// A file that claims more bytes than this machine has memory, and costs nothing on disk.
pub(crate) fn a_sparse_file(dir: &Path, len: u64, tail: &[u8]) -> PathBuf {
    use std::io::{Seek, SeekFrom, Write};
    let path = dir.join("sparse.bin");
    let mut file = std::fs::File::create(&path).expect("a file");
    file.seek(SeekFrom::Start(len - tail.len() as u64)).expect("seek");
    file.write_all(tail).expect("a tail");
    path
}

/// A directory with something nested, something empty, and the three things a zip must leave out:
/// a symlink pointing out of it, a fifo, and a file this user cannot read.
pub(crate) fn a_tree(dir: &Path) -> (PathBuf, Vec<(&'static str, Vec<u8>)>) {
    let root = dir.join("tree");
    std::fs::create_dir_all(root.join("src/deep")).expect("dirs");
    std::fs::create_dir_all(root.join("empty")).expect("an empty dir");
    let files = vec![
        ("README.md", b"readme\n".to_vec()),
        ("src/lib.rs", b"fn main() {}\n".to_vec()),
        ("src/deep/data.bin", pattern(300 * 1024)),
    ];
    for (name, bytes) in &files {
        std::fs::write(root.join(name), bytes).expect("a file");
    }
    let outside = dir.join("secret.txt");
    std::fs::write(&outside, "outside\n").expect("a file outside");
    std::os::unix::fs::symlink(&outside, root.join("out")).expect("a link out");
    std::os::unix::fs::symlink("/", root.join("rootlink")).expect("a link to /");
    let status = std::process::Command::new("mkfifo")
        .arg(root.join("pipe"))
        .status()
        .expect("mkfifo");
    assert!(status.success());
    let locked = root.join("locked.txt");
    std::fs::write(&locked, "locked\n").expect("a file");
    std::fs::set_permissions(&locked, std::os::unix::fs::PermissionsExt::from_mode(0o000)).expect("chmod");
    (root, files)
}

/// A node that read the whole file before answering would take minutes over this, so it fails
/// here rather than at the runner's timeout.
pub(crate) async fn within_seconds<T>(work: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(std::time::Duration::from_secs(10), work)
        .await
        .expect("the answer took as long as reading the whole file would")
}

pub(crate) fn unzip(bytes: &[u8]) -> Vec<(String, Vec<u8>)> {
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes)).expect("a readable zip");
    let mut out = Vec::new();
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).expect("an entry");
        let mut data = Vec::new();
        entry.read_to_end(&mut data).expect("entry bytes");
        out.push((entry.name().expect("a name").to_string(), data));
    }
    out.sort();
    out
}

pub(crate) fn assert_tree(entries: &[(String, Vec<u8>)], files: &[(&str, Vec<u8>)]) {
    for (name, bytes) in files {
        let found = entries
            .iter()
            .find(|(n, _)| n == name)
            .unwrap_or_else(|| panic!("{name} is not in the zip: {:?}", names(entries)));
        assert_eq!(&found.1, bytes, "{name} came out of the zip changed");
    }
    assert!(entries.iter().any(|(n, _)| n == "empty/"), "{:?}", names(entries));
    for absent in ["out", "rootlink", "pipe", "locked.txt"] {
        assert!(
            !entries.iter().any(|(n, _)| n.starts_with(absent)),
            "{absent} was archived: {:?}",
            names(entries)
        );
    }
    assert!(
        !entries.iter().any(|(_, d)| d == b"outside\n"),
        "a symlink was followed out of the directory"
    );
}

fn names(entries: &[(String, Vec<u8>)]) -> Vec<&str> {
    entries.iter().map(|(n, _)| n.as_str()).collect()
}

pub(crate) struct Client {
    pub(crate) h: Harness,
    pub(crate) full: String,
}

impl Client {
    pub(crate) async fn start() -> Self {
        Self::start_with(|_| {}).await
    }

    pub(crate) async fn start_with(tweak: impl FnOnce(&mut kampr_node::Config)) -> Self {
        let h = Harness::start_with(tweak).await;
        let full = format!("Bearer {}", h.token_for(Role::Full).await);
        Self { h, full }
    }

    pub(crate) async fn get(&self, node: &str, source: &Source, extra: &[(&str, &str)]) -> Reply {
        let mut headers = vec![("Authorization", self.full.as_str())];
        headers.extend_from_slice(extra);
        raw::send(
            &self.h.origin,
            "GET",
            &format!("/api/attachment/{node}/_/{}", source.encode()),
            &headers,
            b"",
        )
        .await
    }
}

pub(crate) fn file(path: &Path) -> Source {
    Source::File(FileRef::new(path))
}

/// Everything one node has to say about one big file: whole, two ranges, and the one it cannot.
pub(crate) async fn assert_ranges(c: &Client, node: &str, path: &Path, bytes: &[u8]) {
    let whole = c.get(node, &file(path), &[("Accept-Encoding", "gzip, br")]).await;
    assert_eq!(whole.status, 200, "{}", whole.text());
    assert_eq!(
        whole.header("content-length"),
        Some(bytes.len().to_string().as_str())
    );
    assert_eq!(whole.header("accept-ranges"), Some("bytes"));
    assert!(whole.body == bytes, "the whole file came back changed");

    let (start, end) = (5 * MIB + 3, 9 * MIB + 7);
    let span = c
        .get(
            node,
            &file(path),
            &[
                ("Range", &format!("bytes={start}-{end}")),
                ("Accept-Encoding", "gzip, br"),
            ],
        )
        .await;
    assert_eq!(span.status, 206, "{}", span.text());
    assert_eq!(
        span.header("content-range"),
        Some(format!("bytes {start}-{end}/{}", bytes.len()).as_str())
    );
    assert_eq!(
        span.header("content-length"),
        Some((end - start + 1).to_string().as_str())
    );
    assert!(span.body == bytes[start..=end], "the range came back changed");

    let from = bytes.len() - 1000;
    let tail = c
        .get(node, &file(path), &[("Range", &format!("bytes={from}-"))])
        .await;
    assert_eq!(tail.status, 206);
    assert_eq!(
        tail.header("content-range"),
        Some(format!("bytes {from}-{}/{}", bytes.len() - 1, bytes.len()).as_str())
    );
    assert!(tail.body == bytes[from..]);

    let past = c
        .get(
            node,
            &file(path),
            &[("Range", &format!("bytes={}-", bytes.len()))],
        )
        .await;
    assert_eq!(past.status, 416, "{}", past.text());
    assert_eq!(
        past.header("content-range"),
        Some(format!("bytes */{}", bytes.len()).as_str())
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_file_past_the_old_ceiling_is_served_whole_and_by_range() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let (path, bytes) = a_big_file(dir.path(), 20 * MIB);
    assert_ranges(&c, c.h.node.node_id(), &path, &bytes).await;
}

/// The tail of a file no node could hold in memory comes back at once, which a node that read the
/// file before answering could not do.
#[tokio::test(flavor = "multi_thread")]
async fn the_tail_of_a_file_larger_than_memory_is_served_without_reading_the_rest() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let len: u64 = 12 * 1024 * 1024 * 1024;
    let path = a_sparse_file(dir.path(), len, b"the very end");
    let started = std::time::Instant::now();
    let tail = within_seconds(c.get(
        c.h.node.node_id(),
        &file(&path),
        &[("Range", &format!("bytes={}-", len - 12))],
    ))
    .await;
    assert_eq!(tail.status, 206, "{}", tail.text());
    assert_eq!(tail.body, b"the very end");
    assert!(started.elapsed() < std::time::Duration::from_secs(5));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_directory_comes_back_as_a_zip_of_everything_inside_it_and_nothing_outside() {
    let c = Client::start().await;
    let dir = tempfile::tempdir().expect("a dir");
    let (root, files) = a_tree(dir.path());
    let zip = c
        .get(
            c.h.node.node_id(),
            &Source::Zip(FileRef::new(&root)),
            &[("Accept-Encoding", "gzip, br")],
        )
        .await;
    assert_eq!(zip.status, 200, "{}", zip.text());
    assert_eq!(zip.header("content-type"), Some("application/zip"));
    assert_eq!(
        zip.header("content-disposition"),
        Some("attachment; filename=\"tree.zip\"")
    );
    assert_tree(&unzip(&zip.body), &files);

    let missing = c
        .get(
            c.h.node.node_id(),
            &Source::Zip(FileRef::new(root.join("README.md"))),
            &[],
        )
        .await;
    assert_eq!(missing.status, 404);
}
