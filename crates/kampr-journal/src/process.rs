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
    pub started: Started,
}

/// When a process started, and the answer that is neither an instant nor "it has gone".
///
/// **`Unknown` is not "long ago" and it is not "just now".** A host with no procfs — every darwin
/// build ships without one — a pid that has already been reaped, a read the kernel refuses: each
/// of them means *nothing has been learned*, and a caller that folds that into an instant answers
/// a question it cannot answer ([#233](#)). Every branch has to be named at the call site, which
/// is the point of it being an enum rather than an `Option` somebody can `unwrap_or`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Started {
    At(SystemTime),
    #[default]
    Unknown,
}

impl Started {
    pub fn at(self) -> Option<SystemTime> {
        match self {
            Self::At(at) => Some(at),
            Self::Unknown => None,
        }
    }

    /// Whether `stamp` names an instant before this process existed — which is a thing the process
    /// cannot have done, and therefore the work of a process that is no longer here.
    ///
    /// **`Unknown` answers `false` to everything, and so does a stamp that will not parse.** The
    /// alternative is dropping a launch that is genuinely running because a read failed, which is
    /// a worse bug than the one this exists to fix.
    pub fn predates(self, stamp: Option<&str>) -> bool {
        let (Self::At(started), Some(at)) = (self, stamp.and_then(parse_rfc3339)) else {
            return false;
        };
        at < started
    }
}

impl PaneProcess {
    /// Reads what procfs knows about a live local pid. Every field can come back unanswered
    /// because a platform without procfs must degrade to a weaker rule rather than to no
    /// conversation at all.
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

/// What `/proc/<pid>/comm` calls a process, which is the same name herdr labels a pane with
/// (probe #75) and is read here rather than taken from herdr — a pane herdr can only describe as
/// `bash` (#297) still has its own harness named honestly in procfs.
pub fn comm(pid: u32) -> Option<String> {
    let comm = std::fs::read_to_string(format!("/proc/{pid}/comm")).ok()?;
    Some(comm.trim_end().to_string())
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

/// The process's age subtracted from now: field 22 of `/proc/<pid>/stat` against `/proc/uptime`,
/// both of which are counted from the same boot clock.
///
/// **It is emphatically not the `/proc/<pid>` directory's mtime, which is what this used to read.**
/// procfs stamps that inode when the *dentry* is instantiated, not when the process forks, and it
/// is instantiated again after every eviction — so it is the process's start only for a process
/// nobody has evicted since. Measured on this machine at 14 days of uptime: **426 of 545 live pids
/// disagreed with their own start by more than five seconds**, and the disagreeing ones clustered
/// onto a handful of microsecond-identical stamps (141 pids sharing one, eight days after the boot
/// they all started at). `init` read as eleven hours younger than the machine, `herdr` as six days
/// younger. Always younger, which is the direction that takes a live launch off the running list.
///
/// `btime` + ticks — what `ps` does — is the other way to a wall clock and it carries a constant
/// error: 0.92 s here, being the whole-second floor `btime` is recorded at plus fourteen days of
/// the wall clock drifting away from `CLOCK_BOOTTIME`. Subtracting the age from `now` carries
/// neither, and lands inside **0.5 ms** of a fork bracketed by two `clock_gettime` calls.
fn started_at(pid: u32) -> Started {
    match age_of(pid).and_then(|age| SystemTime::now().checked_sub(age)) {
        Some(at) => Started::At(at),
        None => Started::Unknown,
    }
}

/// `/proc/<pid>/stat` is read before anything else so that a host with no procfs at all stops here
/// rather than in the arithmetic. A negative age — a `/proc/uptime` read that lost a race with the
/// fork it is being subtracted from — fails `try_from_secs_f64` and is therefore unknown.
fn age_of(pid: u32) -> Option<Duration> {
    let ticks = start_ticks(pid)?.parse::<f64>().ok()?;
    let uptime = std::fs::read_to_string("/proc/uptime").ok()?;
    let uptime = uptime.split_whitespace().next()?.parse::<f64>().ok()?;
    Duration::try_from_secs_f64(uptime - ticks / clock_ticks()?).ok()
}

/// Field 22 is in `USER_HZ`, which is 100 everywhere this ships and is still not a thing to
/// assume: a wrong divisor is a start time wrong by a factor, silently.
fn clock_ticks() -> Option<f64> {
    match unsafe { libc::sysconf(libc::_SC_CLK_TCK) } {
        hz if hz > 0 => Some(hz as f64),
        _ => None,
    }
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
