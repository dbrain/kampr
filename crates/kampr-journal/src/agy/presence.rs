use std::os::unix::fs::MetadataExt;
use std::path::Path;

/// The kernel's lock table. World-readable, and the reason identity here does not go through
/// another process's file descriptors: `/proc/<pid>/fd` needs the same privileges as debugging
/// that process, and a node watching a herd is not always its owner.
pub const LOCK_TABLE: &str = "/proc/locks";

/// Every `flock` the kernel says is *held*, as `(pid, major, minor, inode)`.
///
/// A process queued behind a lock is listed under the same index with a `->` and its own pid. It
/// has not got the lock, so it is not on that conversation — two `agy` processes opening one
/// conversation is exactly the case that produces the line.
pub fn flocks(table: &str) -> Vec<(u32, u32, u32, u64)> {
    table
        .lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace().skip(1);
            let kind = fields.next()?;
            if kind != "FLOCK" {
                return None;
            }
            let mut fields = fields.skip(2);
            let pid = fields.next()?.parse().ok()?;
            let mut at = fields.next()?.splitn(3, ':');
            let major = u32::from_str_radix(at.next()?, 16).ok()?;
            let minor = u32::from_str_radix(at.next()?, 16).ok()?;
            let inode = at.next()?.parse().ok()?;
            Some((pid, major, minor, inode))
        })
        .collect()
}

/// The conversation `pid` holds the presence lock on, if it holds exactly one.
///
/// The lock *file* outlives the conversation — nothing unlinks it — so a directory of them says
/// only which conversations have ever existed. The lock itself is released when the process
/// exits, which is what makes holding one the claim that this pid is on this conversation *now*,
/// and what makes it follow the pid through `/new` into a fresh conversation.
///
/// More than one held lock is a shape nothing observed produces, and there is no way to tell
/// which of them the operator is looking at, so it answers nothing rather than guessing.
pub fn holder(presence: &Path, pid: u32) -> Option<String> {
    let table = std::fs::read_to_string(LOCK_TABLE).ok()?;
    let held: Vec<(u32, u32, u64)> = flocks(&table)
        .into_iter()
        .filter(|(owner, ..)| *owner == pid)
        .map(|(_, major, minor, inode)| (major, minor, inode))
        .collect();
    if held.is_empty() {
        return None;
    }
    let mut found = std::fs::read_dir(presence).ok()?.flatten().filter_map(|entry| {
        let path = entry.path();
        let name = path.file_name()?.to_str()?.strip_suffix(".lock")?.to_string();
        let meta = entry.metadata().ok()?;
        let (major, minor) = device(meta.dev());
        held.contains(&(major, minor, meta.ino())).then_some(name)
    });
    let one = found.next()?;
    found.next().is_none().then_some(one)
}

/// `st_dev` split the way `/proc/locks` prints it, which is the kernel's own `major`/`minor`
/// encoding rather than the top and bottom bytes.
fn device(dev: u64) -> (u32, u32) {
    let major = ((dev >> 31 >> 5) & 0xffff_f000) | ((dev >> 8) & 0x0000_0fff);
    let minor = ((dev >> 12) & 0xffff_ff00) | (dev & 0x0000_00ff);
    (major as u32, minor as u32)
}
