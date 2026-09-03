//! Whether a job is parked waiting for somebody to type something.
//!
//! **This only ever works from the job's own parent, at the job's own privilege** (probe #334).
//! From outside — which is where the node stands for every herdr pane — `/proc/<pid>/syscall` is
//! EPERM under yama `ptrace_scope=1` (#331), and a job running as root refuses `wchan` and `fd/0`
//! as well, so the node can read *nothing at all* about a `sudo pacman` (#332). That is why a
//! fleet run is a pty this process owns rather than a pane herdr forked, and why the supervisor
//! cannot follow a command that escalates: such a run is reported blind and read off its screen.
//!
//! The three-state answer is the point. `Unknown` is a real answer and it is not `Busy`: the
//! caller renders it as quiet, never as a question, because a host that is merely unreadable must
//! not present itself as one that is asking (#233 is the same lesson from the other end).

use kampr_core::question::Mode;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Waiting {
    /// Measured: parked in a read-family call whose fd argument is 0.
    Waiting,
    /// Measured: doing something else — sleeping on a timer, on the CPU, writing.
    Busy,
    /// The kernel would not say, or this architecture has no table here. Fall back to quiescence
    /// and do not claim a question.
    Unknown,
}

/// Read-family calls **whose first argument is a file descriptor**.
///
/// `poll`, `ppoll` and `select` are deliberately absent: their first argument is a pointer to a
/// descriptor set, so testing it against 0 would be testing a pointer. `sudo` sits in `ppoll`
/// while it relays for the job underneath it and must read as busy (#334).
///
/// `splice` is here because coreutils `cat` parks there rather than in `read`, with fd 0 still in
/// the first argument (#335) — the syscall is the implementation's choice and the fd is the
/// invariant.
#[cfg(target_arch = "x86_64")]
const READS_FD0: &[u64] = &[
    0,   // read
    17,  // pread64
    19,  // readv
    275, // splice
    295, // preadv
    327, // preadv2
];

#[cfg(target_arch = "aarch64")]
const READS_FD0: &[u64] = &[
    63,  // read
    65,  // readv
    67,  // pread64
    69,  // preadv
    76,  // splice
    286, // preadv2
];

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const READS_FD0: &[u64] = &[];

#[derive(Debug, Clone)]
pub struct Procfs {
    root: PathBuf,
}

impl Default for Procfs {
    fn default() -> Self {
        Self::at("/proc")
    }
}

impl Procfs {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The state of `pid`, or of the deepest descendant of it that has an opinion.
    ///
    /// A command is often a tree — `sudo` forks `pacman`, a wrapper execs the real binary, a
    /// shell runs a pipeline — and the thing holding the terminal is the leaf, not the root. The
    /// root is usually in `ppoll` or `wait4`, which is honestly `Busy` and would mask the leaf's
    /// answer.
    ///
    /// `tty` is this run's own pty, and passing it is what keeps a **pipeline** honest. Measured:
    /// `bash -c 'sleep 30 | cat'` parks `cat` in `splice(2)` on **fd 0**, and fd 0 there is the
    /// pipe rather than the terminal — the exact shape rung 1 recognises, with nobody waiting for
    /// the operator at all. Requiring the descriptor to resolve to the run's own pty can only
    /// remove such an answer and never add one.
    pub fn waiting(&self, pid: u32, tty: Option<&str>) -> Waiting {
        if READS_FD0.is_empty() {
            return Waiting::Unknown;
        }
        let mut answer = Waiting::Unknown;
        for candidate in self.tree(pid, 0) {
            match self.one(candidate, tty) {
                Waiting::Waiting => return Waiting::Waiting,
                Waiting::Busy => answer = Waiting::Busy,
                Waiting::Unknown => {}
            }
        }
        answer
    }

    fn one(&self, pid: u32, tty: Option<&str>) -> Waiting {
        let Some(raw) = self.read(pid, "syscall") else {
            return Waiting::Unknown;
        };
        match parse_syscall(&raw) {
            Waiting::Waiting if !self.reads_the_terminal(pid, tty) => Waiting::Busy,
            answer => answer,
        }
    }

    /// Whether this job's fd 0 is the run's own pty.
    ///
    /// An unreadable link is taken as the terminal, which leaves the answer exactly where it was
    /// before this check existed: a job whose `syscall` can be read but whose `fd/0` cannot is a
    /// shape [#332](../../../docs/03-probe-log.md) says does not occur — an escalated job refuses
    /// both together — so treating it as a refusal would only ever lose a real signal.
    fn reads_the_terminal(&self, pid: u32, tty: Option<&str>) -> bool {
        let Some(tty) = tty else { return true };
        match std::fs::read_link(self.root.join(pid.to_string()).join("fd").join("0")) {
            Ok(target) => target == Path::new(tty),
            Err(_) => true,
        }
    }

    fn tree(&self, pid: u32, depth: usize) -> Vec<u32> {
        let mut found = vec![pid];
        if depth >= 4 {
            return found;
        }
        for child in self.children(pid) {
            found.extend(self.tree(child, depth + 1));
        }
        found
    }

    fn children(&self, pid: u32) -> Vec<u32> {
        let dir = self.root.join(pid.to_string()).join("task");
        let Ok(threads) = std::fs::read_dir(&dir) else {
            return Vec::new();
        };
        let mut pids: Vec<u32> = threads
            .flatten()
            .filter_map(|thread| std::fs::read_to_string(thread.path().join("children")).ok())
            .flat_map(|text| {
                text.split_ascii_whitespace()
                    .filter_map(|p| p.parse::<u32>().ok())
                    .collect::<Vec<_>>()
            })
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    }

    fn read(&self, pid: u32, name: &str) -> Option<String> {
        std::fs::read_to_string(self.root.join(pid.to_string()).join(name)).ok()
    }
}

/// `"0 0x0 0x55d3… 0x400 …"` → the call number and its first argument; `"running"` and a refused
/// read are both `Unknown`.
fn parse_syscall(raw: &str) -> Waiting {
    let raw = raw.trim();
    if raw.is_empty() || raw == "running" {
        return Waiting::Unknown;
    }
    let mut fields = raw.split_ascii_whitespace();
    let Some(nr) = fields.next().and_then(|n| n.parse::<i64>().ok()) else {
        return Waiting::Unknown;
    };
    if nr < 0 {
        return Waiting::Unknown;
    }
    let Some(arg0) = fields.next().and_then(parse_hex) else {
        return Waiting::Unknown;
    };
    if READS_FD0.contains(&(nr as u64)) && arg0 == 0 {
        Waiting::Waiting
    } else {
        Waiting::Busy
    }
}

fn parse_hex(field: &str) -> Option<u64> {
    u64::from_str_radix(field.strip_prefix("0x").unwrap_or(field), 16).ok()
}

/// What the pty's line discipline is set to.
///
/// **Only trustworthy on a pty with no *interactive* shell on it**: ble.sh leaves an ordinary
/// interactive pane with ECHO already off (#333), so this is a question the supervisor may ask of
/// its own pty and nothing may ask of a herdr pane. The confound needs the `i` in `$-` to exist —
/// measured, with `bash -c` on a fleet pty reading ECHO on at idle and ECHO off with ICANON on at
/// a real password prompt, exactly as a pty with nothing on it does — which is why the shell a
/// fleet run is given is never a login or interactive one. Both bits matter — see [`Mode`] and
/// probe #340.
pub fn mode_of(fd: std::os::fd::BorrowedFd<'_>) -> Option<Mode> {
    let t = rustix::termios::tcgetattr(fd).ok()?;
    Some(Mode {
        echo: t.local_modes.contains(rustix::termios::LocalModes::ECHO),
        canonical: t.local_modes.contains(rustix::termios::LocalModes::ICANON),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn a_job_parked_in_read_on_fd_zero_is_waiting() {
        // The exact line probe #334 read off a `pacman` sitting at `Proceed with installation?`.
        assert_eq!(
            parse_syscall("0 0x0 0x55dfebd638d0 0x400 0x0 0x0 0x0 0x7ffe36159da0 0x7f43e5d3ebbd"),
            Waiting::Waiting
        );
    }

    #[test]
    fn cat_parks_in_splice_and_is_still_waiting() {
        // #335: coreutils picks the syscall, the fd is the invariant. A check keyed on `read`
        // alone calls this busy.
        assert_eq!(
            parse_syscall("275 0x0 0x0 0x4 0x0 0x80000 0x0 0x7ffc960b7460 0x7f2809d4ebff"),
            Waiting::Waiting
        );
    }

    #[test]
    fn sleeping_on_a_timer_is_busy_not_waiting() {
        // clock_nanosleep. The false positive that would make every silent build look like a
        // question.
        assert_eq!(
            parse_syscall("230 0x0 0x0 0x7ffe6c7a6fd0 0x7ffe6c7a7020 0x0 0x0 0x0 0x0"),
            Waiting::Busy
        );
    }

    #[test]
    fn a_poll_with_a_null_first_argument_is_busy_because_that_argument_is_not_an_fd() {
        // `sudo` relaying for the job underneath it sits in ppoll (#334). Its first argument is a
        // pointer, and a table that included ppoll would read a null one as fd 0 and call the
        // relay a question.
        assert_eq!(parse_syscall("271 0x0 0x2 0x0 0x0 0x8"), Waiting::Busy);
    }

    #[test]
    fn a_read_on_some_other_descriptor_is_busy() {
        assert_eq!(parse_syscall("0 0x3 0x55dfebd638d0 0x400"), Waiting::Busy);
    }

    #[test]
    fn running_and_refused_both_answer_unknown_rather_than_busy() {
        // #331 and #332: `running` is a real state and a refused read is not. Neither may be
        // reported as a question, and neither may be reported as certainty that it is not one.
        assert_eq!(parse_syscall("running"), Waiting::Unknown);
        assert_eq!(parse_syscall(""), Waiting::Unknown);
        assert_eq!(parse_syscall("-1 0x0 0x0"), Waiting::Unknown);
        assert_eq!(parse_syscall("garbage"), Waiting::Unknown);
    }

    #[test]
    fn the_leaf_of_the_tree_decides_not_the_relay_above_it() {
        let temp = tempdir();
        let root = temp.path();
        // sudo(100) in ppoll, pacman(101) parked in read(0) — the shape probe #334 measured.
        write_proc(root, 100, "271 0x0 0x2 0x0", &[101]);
        write_proc(root, 101, "0 0x0 0x55d3 0x400", &[]);
        assert_eq!(Procfs::at(root).waiting(100, None), Waiting::Waiting);
    }

    #[test]
    fn a_tree_that_is_entirely_busy_says_busy() {
        let temp = tempdir();
        let root = temp.path();
        write_proc(root, 200, "271 0x0 0x2 0x0", &[201]);
        write_proc(root, 201, "230 0x0 0x0 0x0", &[]);
        assert_eq!(Procfs::at(root).waiting(200, None), Waiting::Busy);
    }

    #[test]
    fn a_tree_nothing_can_be_read_from_stays_unknown() {
        let temp = tempdir();
        let root = temp.path();
        assert_eq!(Procfs::at(root).waiting(999, None), Waiting::Unknown);
    }

    /// A shell running a pipeline parks the tail of it in a read on fd 0 that is a **pipe**, and
    /// nobody is waiting for the operator. Measured: `bash -c 'sleep 30 | cat'` reads
    /// `275 0x0 …` — the same line `cat` at a terminal produces — with `fd/0` pointing at
    /// `pipe:[690464455]`. Rung 1 keyed on the syscall alone calls that host a question.
    #[test]
    fn a_pipeline_reading_a_pipe_is_not_a_host_waiting_for_the_operator() {
        let temp = tempdir();
        let root = temp.path();
        write_proc(root, 300, "61 0xffffffffffffffff 0x0", &[301, 302]);
        write_proc(root, 301, "230 0x0 0x0 0x0", &[]);
        write_proc(root, 302, "275 0x0 0x0 0x4", &[]);
        point_fd0(root, 302, "pipe:[690464455]");
        assert_eq!(Procfs::at(root).waiting(300, Some("/dev/pts/8")), Waiting::Busy);
    }

    /// And the front of the same pipeline, which does hold the terminal, still is one.
    #[test]
    fn the_end_of_a_pipeline_that_holds_the_terminal_is_still_a_question() {
        let temp = tempdir();
        let root = temp.path();
        write_proc(root, 400, "61 0xffffffffffffffff 0x0", &[401, 402]);
        write_proc(root, 401, "275 0x0 0x0 0x4", &[]);
        point_fd0(root, 401, "/dev/pts/8");
        write_proc(root, 402, "275 0x0 0x0 0x4", &[]);
        point_fd0(root, 402, "pipe:[690475056]");
        assert_eq!(
            Procfs::at(root).waiting(400, Some("/dev/pts/8")),
            Waiting::Waiting
        );
    }

    /// A job whose `fd/0` cannot be read at all is left exactly where it was before the check
    /// existed. #332 measured that an escalated job refuses `syscall` and `fd/0` together, so
    /// this combination does not arise — and reading it as a refusal would only lose a signal.
    #[test]
    fn a_descriptor_that_cannot_be_read_leaves_the_answer_where_it_was() {
        let temp = tempdir();
        let root = temp.path();
        write_proc(root, 500, "0 0x0 0x55d3 0x400", &[]);
        assert_eq!(
            Procfs::at(root).waiting(500, Some("/dev/pts/8")),
            Waiting::Waiting
        );
    }

    /// Removed when it goes out of scope. A test that leaves a directory in `/tmp` for every run
    /// is a test that litters the machine it is measuring.
    fn tempdir() -> tempfile::TempDir {
        tempfile::tempdir().expect("temp dir")
    }

    fn write_proc(root: &Path, pid: u32, syscall: &str, children: &[u32]) {
        let dir = root.join(pid.to_string());
        std::fs::create_dir_all(dir.join("task").join(pid.to_string())).expect("proc dir");
        std::fs::write(dir.join("syscall"), syscall).expect("syscall");
        let kids = children
            .iter()
            .map(|c| c.to_string())
            .collect::<Vec<_>>()
            .join(" ");
        std::fs::write(dir.join("task").join(pid.to_string()).join("children"), kids).expect("children");
    }

    fn point_fd0(root: &Path, pid: u32, at: &str) {
        let fds = root.join(pid.to_string()).join("fd");
        std::fs::create_dir_all(&fds).expect("fd dir");
        std::os::unix::fs::symlink(at, fds.join("0")).expect("fd 0");
    }
}
