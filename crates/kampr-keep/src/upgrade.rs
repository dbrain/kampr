use crate::dir::{SessionDir, list_tree};
use rustix::process::{Pid, Signal, kill_process};
use std::ffi::OsStr;
use std::fs;
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Duration;

// `/proc/<pid>/exe` stats the image the keeper is running even once its file is unlinked, and a
// same-uid non-ancestor may read it under yama (#598); a keeper re-execs whatever is at its own
// path, so that file is the one it must match.
pub fn stale(session: &SessionDir) -> bool {
    let Some(keeper) = session.keeper().filter(|_| session.alive()) else {
        return false;
    };
    if !keeps(keeper.keeper, session.path()) {
        return false;
    }
    match (
        fs::metadata(format!("/proc/{}/exe", keeper.keeper)),
        fs::metadata(&keeper.exe),
    ) {
        (Ok(running), Ok(installed)) => (running.dev(), running.ino()) != (installed.dev(), installed.ino()),
        _ => false,
    }
}

// keeper.json outlives its keeper, and a reused pid could be anything of ours, a node included,
// which SIGUSR1 would kill.
fn keeps(pid: u32, dir: &Path) -> bool {
    let Ok(cmdline) = fs::read(format!("/proc/{pid}/cmdline")) else {
        return false;
    };
    let args: Vec<&[u8]> = cmdline.split(|&b| b == 0).collect();
    let [_, b"keep", served, ..] = args.as_slice() else {
        return false;
    };
    let served = Path::new(OsStr::from_bytes(served));
    let served = match served.is_absolute() {
        true => served.to_path_buf(),
        false => PathBuf::from(format!("/proc/{pid}/cwd")).join(served),
    };
    matches!(
        (fs::canonicalize(served), fs::canonicalize(dir)),
        (Ok(a), Ok(b)) if a == b
    )
}

pub fn upgrade_stale(root: &Path) -> io::Result<Vec<String>> {
    let mut upgraded = Vec::new();
    for session in list_tree(root)? {
        if !stale(&session) {
            continue;
        }
        let Some(pid) = session.keeper().and_then(|k| Pid::from_raw(k.keeper as i32)) else {
            continue;
        };
        match kill_process(pid, Signal::USR1) {
            Ok(()) => upgraded.push(session.id().to_string()),
            Err(rustix::io::Errno::SRCH) => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(upgraded)
}

pub fn upgrade_while(
    root: PathBuf,
    every: Duration,
    mut report: impl FnMut(io::Result<Vec<String>>) -> bool + Send + 'static,
) -> JoinHandle<()> {
    std::thread::Builder::new()
        .name("kampr-keep-upgrade".into())
        .spawn(move || {
            while report(upgrade_stale(&root)) {
                std::thread::sleep(every);
            }
        })
        .expect("spawning the keeper upgrade thread")
}
