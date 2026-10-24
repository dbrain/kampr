use kampr_proc::Table;
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
    /// What Claude records verbatim as `procStart` ([`kampr_proc::Start::token`]). Comparing it
    /// is what separates a live process from the dead one whose pid it inherited.
    pub start: Option<String>,
    /// When the process started. For the harnesses that publish no process-to-session map at all,
    /// this is still a bound worth having: a transcript whose last word was written before the
    /// process existed cannot be that process's.
    pub started: Started,
}

/// When a process started, and the answer that is neither an instant nor "it has gone".
///
/// **`Unknown` is not "long ago" and it is not "just now".** A host that cannot describe its
/// processes, a pid that has already been reaped, a read the kernel refuses: each
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
    /// Reads what this host knows about a live local pid. Every field can come back unanswered
    /// because a host that cannot see must degrade to a weaker rule rather than to no
    /// conversation at all.
    pub fn look_up(pid: u32) -> Self {
        let start = kampr_proc::here().start(pid);
        Self {
            pid,
            started: match start.as_ref().and_then(|s| s.at) {
                Some(at) => Started::At(at),
                None => Started::Unknown,
            },
            start: start.map(|s| s.token),
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

/// What the kernel calls a process, which is the same name herdr labels a pane with (probe #75)
/// and is read here rather than taken from herdr — a pane herdr can only describe as `bash`
/// (#297) still has its own harness named honestly here.
pub fn comm(pid: u32) -> Option<String> {
    kampr_proc::here().comm(pid)
}

/// Whether this host answers questions about a process at all.
///
/// A pid with no start time means two different things, and only one of them is a harness that
/// has gone: on a host that can see it is a pid that is not there, and on one that cannot it is
/// every pid there has ever been. Refusing them all on a host that can read none would be
/// refusing every conversation on it.
pub fn observable() -> bool {
    kampr_proc::here().start(std::process::id()).is_some()
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

/// The direct children of `pid`.
///
/// **A harness is not always the process a pane reports.** `codex` ships a node wrapper that
/// spawns the native binary, and it is the native one that holds the thread's writer lock — so a
/// handle keyed on the pane's own process finds nothing without looking one level down. Empty on
/// a host that will not answer, which is the same as having no children as far as any caller is
/// concerned.
pub fn children(pid: u32) -> Vec<u32> {
    kampr_proc::here().children(pid)
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
pub(crate) fn parse_rfc3339(stamp: &str) -> Option<SystemTime> {
    let at = time::OffsetDateTime::parse(stamp, &time::format_description::well_known::Rfc3339).ok()?;
    let seconds = at.unix_timestamp();
    let whole = match seconds >= 0 {
        true => SystemTime::UNIX_EPOCH + Duration::from_secs(seconds as u64),
        false => SystemTime::UNIX_EPOCH - Duration::from_secs(seconds.unsigned_abs()),
    };
    Some(whole + Duration::from_nanos(at.nanosecond() as u64))
}
