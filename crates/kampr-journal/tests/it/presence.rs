//! Where the kernel lists no locks, as on macOS, a lock file held open is the lock held.

use crate::common::scratch_dir;
use kampr_journal::presence::held_open;
use std::time::{Duration, SystemTime};

#[test]
fn the_lock_files_a_process_holds_open_in_a_directory_are_its_newest_first() {
    let dir = scratch_dir("presence");
    let elsewhere = scratch_dir("presence-elsewhere");
    let open = |dir: &std::path::Path, name: &str, age: u64| {
        let path = dir.join(name);
        let file = std::fs::File::create(&path).expect("a lock file");
        file.set_modified(SystemTime::now() - Duration::from_secs(age))
            .expect("an mtime");
        file
    };
    let _old = open(&dir, "first.lock", 60);
    let _new = open(&dir, "second.lock", 1);
    let _not_a_lock = open(&dir, "notes.txt", 0);
    let _not_here = open(&elsewhere, "other.lock", 0);
    std::fs::write(dir.join("closed.lock"), b"").expect("a closed lock file");

    assert_eq!(held_open(&dir, std::process::id()), ["second", "first"]);
}
