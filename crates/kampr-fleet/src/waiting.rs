//! Whether a job is parked waiting for somebody to type something.
//!
//! **This only ever works from the job's own ancestor, at the job's own privilege** (probe #334).
//! From outside — which is where the node stands for every herdr pane — `/proc/<pid>/syscall` is
//! EPERM under yama `ptrace_scope=1` (#331), and a job running as root refuses `wchan` and `fd/0`
//! as well, so the node can read *nothing at all* about a `sudo pacman` (#332). That is why a
//! fleet run is a native session whose keeper forked it (docs/17), and why the keeper's `state`
//! reads these files rather than the node: it is the ancestor. It cannot follow a command that
//! escalates either, so such a run is reported blind and read off its screen.
//!
//! The three-state answer is the point. `Unknown` is a real answer and it is not `Busy`: the
//! caller renders it as quiet, never as a question, because a host that is merely unreadable must
//! not present itself as one that is asking (#233 is the same lesson from the other end).

use kampr_core::question::Mode;
use kampr_keep::{Proc, ProcState};

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
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const READS_FD0: &[u64] = &[
    0,   // read
    17,  // pread64
    19,  // readv
    275, // splice
    295, // preadv
    327, // preadv2
];

#[cfg(all(target_os = "linux", target_arch = "aarch64"))]
const READS_FD0: &[u64] = &[
    63,  // read
    65,  // readv
    67,  // pread64
    69,  // preadv
    76,  // splice
    286, // preadv2
];

// No kernel but Linux says which call a process is parked in, so elsewhere a run is read off its
// screen and called blind.
#[cfg(not(all(target_os = "linux", any(target_arch = "x86_64", target_arch = "aarch64"))))]
const READS_FD0: &[u64] = &[];

/// The state of the run's child, or of the deepest descendant of it that has an opinion, from the
/// keeper's `state` reply: the keeper is the child's parent, and only an ancestor may read these
/// files (#331).
///
/// A command is often a tree — `sudo` forks `pacman`, a wrapper execs the real binary, a shell runs
/// a pipeline — and the thing holding the terminal is the leaf, not the root. The root is usually
/// in `ppoll` or `wait4`, which is honestly `Busy` and would mask the leaf's answer.
///
/// `tty` is the run's own pty, and checking it is what keeps a **pipeline** honest. Measured:
/// `bash -c 'sleep 30 | cat'` parks `cat` in `splice(2)` on **fd 0**, and fd 0 there is the pipe
/// rather than the terminal — the exact shape rung 1 recognises, with nobody waiting for the
/// operator at all. Requiring the descriptor to resolve to the run's own pty can only remove such
/// an answer and never add one.
pub fn waiting(state: &ProcState) -> Waiting {
    if READS_FD0.is_empty() {
        return Waiting::Unknown;
    }
    let mut answer = Waiting::Unknown;
    for proc in &state.procs {
        match one(proc, state.tty.as_deref()) {
            Waiting::Waiting => return Waiting::Waiting,
            Waiting::Busy => answer = Waiting::Busy,
            Waiting::Unknown => {}
        }
    }
    answer
}

fn one(proc: &Proc, tty: Option<&str>) -> Waiting {
    let Some(raw) = proc.syscall.as_deref() else {
        return Waiting::Unknown;
    };
    match parse_syscall(raw) {
        Waiting::Waiting if !reads_the_terminal(proc, tty) => Waiting::Busy,
        answer => answer,
    }
}

/// An unreadable link is taken as the terminal, which leaves the answer exactly where it was
/// before this check existed: a job whose `syscall` can be read but whose `fd/0` cannot is a shape
/// #332 says does not occur — an escalated job refuses both together — so treating it as a
/// refusal would only ever lose a real signal.
fn reads_the_terminal(proc: &Proc, tty: Option<&str>) -> bool {
    match (tty, proc.fd0.as_deref()) {
        (Some(tty), Some(fd0)) => fd0 == tty,
        _ => true,
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
/// interactive pane with ECHO already off (#333), so this is a question a fleet run may ask of
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

// Lines captured on x86_64 Linux, numbered by its syscall table.
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::*;

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

    fn proc(pid: u32, syscall: Option<&str>, fd0: Option<&str>) -> Proc {
        Proc {
            pid,
            children: Vec::new(),
            stat: None,
            syscall: syscall.map(str::to_string),
            wchan: None,
            fd0: fd0.map(str::to_string),
        }
    }

    /// Each tree is the keeper's walk of a real shape: `sudo` over `pacman` (#334), a pipeline
    /// whose tail reads a pipe, its front reading the terminal, and a job whose `fd/0` was
    /// unreadable (#332 says that cannot happen alone, so it must not lose the answer).
    #[test]
    fn the_tree_is_read_leaf_first_and_only_a_read_of_the_terminal_is_a_question() {
        let tty = Some("/dev/pts/8");
        let cases: &[(&str, Vec<Proc>, Waiting)] = &[
            (
                "sudo relaying for pacman parked in read(0)",
                vec![
                    proc(100, Some("271 0x0 0x2 0x0"), None),
                    proc(101, Some("0 0x0 0x55d3 0x400"), None),
                ],
                Waiting::Waiting,
            ),
            (
                "a tree that is entirely busy",
                vec![
                    proc(200, Some("271 0x0 0x2 0x0"), None),
                    proc(201, Some("230 0x0 0x0 0x0"), None),
                ],
                Waiting::Busy,
            ),
            (
                "nothing readable at all",
                vec![proc(999, None, None)],
                Waiting::Unknown,
            ),
            ("no processes left", Vec::new(), Waiting::Unknown),
            (
                "`sleep 30 | cat`: cat in splice on a pipe",
                vec![
                    proc(300, Some("61 0xffffffffffffffff 0x0"), Some("/dev/pts/8")),
                    proc(301, Some("230 0x0 0x0 0x0"), Some("/dev/pts/8")),
                    proc(302, Some("275 0x0 0x0 0x4"), Some("pipe:[690464455]")),
                ],
                Waiting::Busy,
            ),
            (
                "the front of a pipeline holding the terminal",
                vec![
                    proc(400, Some("61 0xffffffffffffffff 0x0"), Some("/dev/pts/8")),
                    proc(401, Some("275 0x0 0x0 0x4"), Some("/dev/pts/8")),
                    proc(402, Some("275 0x0 0x0 0x4"), Some("pipe:[690475056]")),
                ],
                Waiting::Waiting,
            ),
            (
                "a descriptor that could not be read",
                vec![proc(500, Some("0 0x0 0x55d3 0x400"), None)],
                Waiting::Waiting,
            ),
        ];
        for (what, procs, want) in cases {
            let state = ProcState {
                tty: tty.map(str::to_string),
                procs: procs.clone(),
            };
            assert_eq!(waiting(&state), *want, "{what}");
        }
    }
}
