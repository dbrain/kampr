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
    holder_from(presence, pid, || std::fs::read_to_string(LOCK_TABLE).ok())
}

pub fn holder_from(presence: &Path, pid: u32, table: impl FnMut() -> Option<String>) -> Option<String> {
    let mut held = held_from(presence, pid, table).into_iter();
    let one = held.next()?;
    held.next().is_none().then_some(one)
}

/// The newest lock file in `dir` that `pid` holds.
///
/// **For a harness that keeps the lock it had.** `codex` takes a second writer lock on `/new`
/// and never releases the first, so [`holder`]'s "exactly one or nothing" would refuse every
/// session that has used it; the newest file is the thread it moved to. `agy` moves its lock
/// instead and wants [`holder`], which refuses a shape it has never been measured producing.
pub fn newest_holder(dir: &Path, pid: u32) -> Option<String> {
    held(dir, pid).into_iter().next()
}

/// Every lock file in `dir` that `pid` holds, newest file first.
pub fn held(dir: &Path, pid: u32) -> Vec<String> {
    held_from(dir, pid, || std::fs::read_to_string(LOCK_TABLE).ok())
}

pub fn held_from(dir: &Path, pid: u32, table: impl FnMut() -> Option<String>) -> Vec<String> {
    let Some(locks) = seen(table, pid) else {
        return Vec::new();
    };
    if locks.is_empty() {
        return Vec::new();
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut found: Vec<(std::time::SystemTime, String)> = entries
        .flatten()
        .filter_map(|entry| {
            let path = entry.path();
            let name = path.file_name()?.to_str()?.strip_suffix(".lock")?.to_string();
            let meta = entry.metadata().ok()?;
            let (major, minor) = device(meta.dev());
            locks
                .contains(&(major, minor, meta.ino()))
                .then(|| Some((meta.modified().ok()?, name)))
                .flatten()
        })
        .collect();
    found.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    found.into_iter().map(|(_, name)| name).collect()
}

/// How many reads of the lock table a lock has to stay out of before it is believed absent. Two
/// reads is not enough: the record a read drops is the one sitting on the buffer boundary, and
/// that position does not move between reads, so consecutive reads drop the *same* record far
/// more often than independence would predict — measured at one run in twenty-four hundred with
/// two reads, against one in a hundred and fifty with one (#279).
const READS: usize = 4;

/// Every file `pid` was seen holding an `flock` on across [`READS`] reads of the table.
///
/// **One read is not evidence** (#279). `/proc/locks` is a `seq_file`, and its iteration restarts by
/// index every time the kernel's ~4 KiB buffer is drained, so a lock released *before* that
/// boundary between two of the `read` calls behind one `read_to_string` shifts every later record
/// up one and the record sitting on the boundary is never printed at all. It leaves no trace
/// beyond the records after it printing with indices one lower.
///
/// The union rather than the intersection, because a dropped record is the dangerous direction
/// and a stale one is not. A pid holding two presence locks whose second lock falls out of the
/// view resolves to the *first*, which is precisely the wrong-conversation answer [`holder`]
/// exists to refuse; a lock released between two reads only makes it refuse an answer it would
/// have given, and the next poll asks again.
fn seen(mut table: impl FnMut() -> Option<String>, pid: u32) -> Option<Vec<(u32, u32, u64)>> {
    let mut held: Vec<(u32, u32, u64)> = Vec::new();
    for _ in 0..READS {
        for lock in flocks(&table()?)
            .into_iter()
            .filter(|(owner, ..)| *owner == pid)
            .map(|(_, major, minor, inode)| (major, minor, inode))
        {
            if !held.contains(&lock) {
                held.push(lock);
            }
        }
    }
    Some(held)
}

/// `st_dev` split the way `/proc/locks` prints it, which is the kernel's own `major`/`minor`
/// encoding rather than the top and bottom bytes.
fn device(dev: u64) -> (u32, u32) {
    let major = ((dev >> 31 >> 5) & 0xffff_f000) | ((dev >> 8) & 0x0000_0fff);
    let minor = ((dev >> 12) & 0xffff_ff00) | (dev & 0x0000_00ff);
    (major as u32, minor as u32)
}
