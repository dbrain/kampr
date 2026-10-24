use crate::dir::{Image, SessionDir, list_tree};
use kampr_proc::Table;
use rustix::process::{Pid, Signal, kill_process};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::thread::JoinHandle;
use std::time::Duration;

// A keeper re-execs whatever is at its own path, so that file is the one it must match. It
// records the file it started from, which is all a Mac can tell; a keeper from before the record
// is read from `/proc/<pid>/exe`, which stats the image it runs even once its file is unlinked,
// and which a same-uid non-ancestor may read under yama (#598).
pub fn stale(session: &SessionDir) -> bool {
    let Some(keeper) = session.keeper().filter(|_| session.alive()) else {
        return false;
    };
    if !keeps(keeper.keeper, session.path()) {
        return false;
    }
    let running = keeper.image.or_else(|| {
        fs::metadata(format!("/proc/{}/exe", keeper.keeper))
            .ok()
            .map(|meta| Image::of(&meta))
    });
    match (running, fs::metadata(&keeper.exe)) {
        (Some(running), Ok(installed)) => running != Image::of(&installed),
        _ => false,
    }
}

// keeper.json outlives its keeper, and a reused pid could be anything of ours, a node included,
// which SIGUSR1 would kill.
fn keeps(pid: u32, dir: &Path) -> bool {
    let table = kampr_proc::here();
    let Some(args) = table.argv(pid) else {
        return false;
    };
    let [_, keep, served, ..] = args.as_slice() else {
        return false;
    };
    if keep != "keep" {
        return false;
    }
    let served = Path::new(served);
    let served = match served.is_absolute() {
        true => served.to_path_buf(),
        false => match table.cwd(pid) {
            Some(cwd) => cwd.join(served),
            None => return false,
        },
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
