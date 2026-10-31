//! The device store's watch, against the files sqlite actually makes and unmakes beside it.

use kampr_node::device_watch;
use std::io::Write;
use std::path::Path;
use std::time::Duration;
use tokio::time::Instant;

fn append(path: &Path, bytes: &[u8]) {
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .unwrap();
    file.write_all(bytes).unwrap();
}

async fn heard(watch: &mut device_watch::StoreWatch, what: &str) {
    assert!(
        watch
            .changes
            .changed_before(Instant::now() + Duration::from_millis(500))
            .await,
        "{what} woke nothing"
    );
    tokio::time::sleep(Duration::from_millis(50)).await;
    watch.changes.seen();
}

/// A `-wal` is a new inode whenever sqlite makes it: absent until a connection opens the store,
/// and unlinked by the last one to close. A watch put on the files that were there misses every
/// commit into one that came later.
#[tokio::test]
async fn a_commit_is_heard_in_a_wal_made_after_the_watch_and_in_one_made_again() {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("kampr.db");
    let wal = dir.path().join("kampr.db-wal");
    append(&db, b"");
    let mut watch = device_watch::store(&db).expect("a watch");

    append(&wal, b"");
    heard(&mut watch, "the wal appearing").await;
    append(&wal, b"a commit");
    heard(&mut watch, "a commit into a wal made after the watch").await;

    std::fs::remove_file(&wal).unwrap();
    heard(&mut watch, "the wal going").await;
    append(&wal, b"");
    heard(&mut watch, "the wal made again").await;
    append(&wal, b"another commit");
    heard(&mut watch, "a commit into a wal made again").await;

    append(&dir.path().join("reads.json"), b"{}");
    assert!(
        !watch
            .changes
            .changed_before(Instant::now() + Duration::from_millis(300))
            .await,
        "a file beside the store is not the store"
    );
}
