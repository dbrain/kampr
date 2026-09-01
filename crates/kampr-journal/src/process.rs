use std::path::Path;
use std::time::{Duration, SystemTime};

/// The harness process a pane is running.
///
/// **A working directory is not an identity.** Every `claude` run in one writes its own
/// transcript, so "the newest transcript declaring this cwd" is somebody else's conversation as
/// often as it is this pane's — the operator's own desktop session, the run that was quit a
/// minute ago, the pane next door. The process is the identity, and this is what a node manages
/// to learn about one.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PaneProcess {
    pub pid: u32,
    /// Field 22 of `/proc/<pid>/stat`, which is what Claude records verbatim as `procStart`.
    /// Comparing it is what separates a live process from the dead one whose pid it inherited.
    pub start: Option<String>,
    /// When the process started. For the harnesses that publish no process-to-session map at all,
    /// this is still a bound worth having: a transcript whose last word was written before the
    /// process existed cannot be that process's.
    pub started: Option<SystemTime>,
}

impl PaneProcess {
    /// Reads what procfs knows about a live local pid. Everything is optional because a platform
    /// without procfs must degrade to a weaker rule rather than to no conversation at all.
    pub fn look_up(pid: u32) -> Self {
        Self {
            pid,
            start: start_ticks(pid),
            started: started_at(pid),
        }
    }

    /// Whether `recorded` — a `procStart` a harness wrote beside a session id — describes *this*
    /// process. Absent on either side is not a contradiction: nothing has been disproved, and the
    /// alternative is refusing to serve a conversation on a host with no procfs.
    pub fn owns(&self, recorded: Option<&str>) -> bool {
        match (recorded, self.start.as_deref()) {
            (Some(a), Some(b)) => a == b,
            _ => true,
        }
    }
}

/// Whether this host answers questions about a process at all.
///
/// A pid with no start time means two different things, and only one of them is a harness that
/// has gone: with procfs it is a pid that is not there, and without procfs it is every pid there
/// has ever been. Refusing them all on a host that can read none would be refusing every
/// conversation on it.
pub fn observable() -> bool {
    Path::new("/proc/self/stat").is_file()
}

/// Field 22 of `/proc/<pid>/stat`, read past the comm field so a process named `a) b` cannot
/// shift the count.
fn start_ticks(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let fields = &stat[stat.rfind(") ")? + 2..];
    fields.split_whitespace().nth(19).map(str::to_string)
}

/// The `/proc/<pid>` directory is stamped when the process is created and not touched again, so
/// its modification time is the process's start — to the second, rounded *down*, which is the
/// safe direction for a bound that must not hide a real conversation.
fn started_at(pid: u32) -> Option<SystemTime> {
    std::fs::metadata(Path::new("/proc").join(pid.to_string()))
        .and_then(|m| m.modified())
        .ok()
}

/// What a pane's host managed to learn about the harness running in it.
///
/// **The middle case is the one that matters.** Herdr detects a harness by scraping the screen,
/// so a pane can claim `claude` while nothing named `claude` is running in it — a detection that
/// has gone stale, or one that was never right. Treating that as "no information" falls back to
/// the working directory and serves whichever session in it was touched last, which is the exact
/// lie this all exists to stop. It is not no information: it is the host saying there is no
/// harness here, and a pane with no harness has no conversation.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub enum Harness {
    /// The process the pane is running.
    Running(PaneProcess),
    /// The host looked and found no harness process in the pane.
    Absent,
    /// Nothing looked — a host that cannot see processes at all. The working directory is then
    /// the only handle there is, exactly as it was before any of this.
    #[default]
    Unknown,
}

impl Harness {
    pub fn process(&self) -> Option<&PaneProcess> {
        match self {
            Self::Running(process) => Some(process),
            _ => None,
        }
    }

    /// Whether the working directory may be searched at all. An absent harness wrote nothing, so
    /// there is nothing in the directory for it to have written.
    pub fn may_search(&self) -> bool {
        !matches!(self, Self::Absent)
    }
}

/// The direct children of `pid`, from procfs.
///
/// **A harness is not always the process a pane reports.** `codex` ships a node wrapper that
/// spawns the native binary, and it is the native one that holds the thread's writer lock — so a
/// handle keyed on the pane's own process finds nothing without looking one level down. Empty on
/// a host that will not answer, which is the same as having no children as far as any caller is
/// concerned.
pub fn children(pid: u32) -> Vec<u32> {
    let tasks = match std::fs::read_dir(format!("/proc/{pid}/task")) {
        Ok(tasks) => tasks,
        Err(_) => return Vec::new(),
    };
    let mut found = Vec::new();
    for task in tasks.flatten() {
        let Ok(list) = std::fs::read_to_string(task.path().join("children")) else {
            continue;
        };
        for child in list.split_whitespace().filter_map(|c| c.parse().ok()) {
            if !found.contains(&child) {
                found.push(child);
            }
        }
    }
    found
}

/// Whether a transcript was still being written after `since`.
///
/// The transcript's own newest RFC 3339 stamp is what answers this, with the file's modification
/// time only as the fallback: a rollout copied onto this disk carries an mtime saying when the
/// copy happened, not when the conversation did.
pub fn active_since(stamp: Option<&str>, modified: SystemTime, since: SystemTime) -> bool {
    match stamp.and_then(parse_rfc3339) {
        Some(at) => at >= since,
        None => modified >= since,
    }
}

/// **The fraction is load-bearing.** `since` is a process start read from procfs at nanoseconds,
/// so a stamp rounded down to its second is compared against an instant that is not — and a record
/// claude wrote 462 ms *after* the harness started reads as one written 538 ms before it. That is
/// a pane refused its own transcript for as long as the process lives, which no later record can
/// clear once the file is only ever appended to within the same second (#415). Every harness in
/// tree writes the fraction (#285); throwing it away here was the whole of the miss.
fn parse_rfc3339(stamp: &str) -> Option<SystemTime> {
    let at = time::OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339).ok()?;
    let seconds = at.unix_timestamp();
    let whole = match seconds >= 0 {
        true => SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64),
        false => SystemTime::UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs()),
    };
    Some(whole + Duration::from_nanos(at.nanosecond() as u64))
}
