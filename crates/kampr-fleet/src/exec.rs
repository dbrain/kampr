//! One command, on a pty this process owns.
//!
//! **A supervisor can only read a job it forked at its own privilege.** An unprivileged parent is
//! refused `/proc/<pid>/syscall` for its own child the moment that child is setuid, so a command
//! that escalates is exactly as opaque here as it is to the node (probe #334) — and this process
//! does not escalate to follow it. Such a run is reported blind and read off its screen instead.
//!
//! [`RunEvent::Readable`] is how a caller finds out which kind it got: an observation made over the
//! run rather than a sample taken the instant after a fork, when the child is still this process's
//! own un-`exec`ed copy and readable whatever it is about to become.

use crate::tail::Tail;
use crate::waiting::{Procfs, Waiting, mode_of};
use kampr_core::question::{self as prompt};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tokio::sync::mpsc;

/// How often the kernel is asked what the job is doing.
///
/// Four times a second: a person waiting for a host to answer notices a second, and a `syscall`
/// read is one small file per process in a tree that is rarely more than three deep.
const POLL: Duration = Duration::from_millis(250);

/// How long a job with no readable state and no output must be silent before it is called quiet.
///
/// This is the *fallback* rung, for the hosts probes #331/#332 describe — never the rung that
/// decides a host is asking something. A build that links for twelve seconds is quiet too.
const QUIET_AFTER: Duration = Duration::from_secs(10);

/// Context lines published beside a question.
const CONTEXT_ROWS: usize = 6;

/// How long a run whose state cannot be read must sit on an unterminated prompt before the screen
/// is allowed to speak for the kernel.
///
/// This is the rung that makes `sudo pacman -Syu` — the command this whole feature exists for —
/// show as a question rather than as [`State::Quiet`]. It is weaker evidence than [`Waiting`] and
/// the question it produces says so; two seconds is long enough that a program pausing mid-line
/// mid-work does not qualify, and short enough that nobody waits on the board for it.
const INFERRED_AFTER: Duration = Duration::from_secs(2);

pub use kampr_core::provider::FleetState as State;

#[derive(Debug)]
pub enum RunEvent {
    Bytes(Vec<u8>),
    State(State),
    /// The kernel has answered about this job — so the run is **not** blind and never was.
    ///
    /// Sent at most once, and only after two consecutive readings agree. One is not enough:
    /// `spawn` returns as soon as the fork succeeds, and for a moment the child is still this
    /// process's own un-`exec`ed copy, which is readable however privileged the binary it is about
    /// to become. Sampling once at that instant reports `sudo pacman` as readable and then never
    /// corrects itself.
    Readable,
}

pub struct Supervisor {
    master: Arc<OwnedFd>,
    child: Child,
    tail: Arc<Mutex<Tail>>,
    procfs: Procfs,
}

#[derive(Debug, Clone, Copy)]
pub struct Geometry {
    pub cols: u16,
    pub rows: u16,
}

impl Default for Geometry {
    /// The floor ADR 0012 sets for a deliberate resize, used here as the size of a pane nobody
    /// else is looking at. A fleet pty has no desk attached and no operator geometry to lose, so
    /// this is Kampr's to choose (rule 3).
    fn default() -> Self {
        Self { cols: 100, rows: 30 }
    }
}

impl Supervisor {
    /// Forks `argv` onto a fresh pty, with this process as its parent and its session leader.
    ///
    /// `path` is the `PATH` the child is given — the operator's rather than the service manager's,
    /// see [`crate::env`]. `None` leaves this process's, which is what every run got before.
    pub fn spawn(
        argv: &[String],
        cwd: Option<&str>,
        geometry: Geometry,
        path: Option<&str>,
    ) -> io::Result<Self> {
        let (master, slave) = open_pty(geometry)?;

        let mut command = Command::new(
            argv.first()
                .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "a fleet run needs a command"))?,
        );
        command.args(&argv[1..]);
        if let Some(cwd) = cwd {
            command.current_dir(cwd);
        }
        command.stdin(Stdio::from(slave.try_clone()?));
        command.stdout(Stdio::from(slave.try_clone()?));
        command.stderr(Stdio::from(slave.try_clone()?));
        // A command that prompts needs a controlling terminal of its own, or it reads EOF and
        // "answers" itself.
        command.env("TERM", "xterm-256color");
        if let Some(path) = path {
            command.env("PATH", path);
        }
        let slave_raw = slave.as_raw_fd();
        unsafe {
            command.pre_exec(move || {
                if libc::setsid() < 0 {
                    return Err(io::Error::last_os_error());
                }
                if libc::ioctl(slave_raw, libc::TIOCSCTTY as _, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }

        // Named rather than propagated. `spawn` answers ENOENT as "No such file or directory (os
        // error 2)", which on a fan-out is the same sentence from every host in the herd and says
        // nothing about the thing that is actually wrong — that this is not the `PATH` the
        // operator installs into (#392).
        let child = command.spawn().map_err(|e| match e.kind() {
            io::ErrorKind::NotFound => io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "`{}` is not on this node's fleet PATH ({})",
                    argv[0],
                    path.unwrap_or("unset"),
                ),
            ),
            _ => e,
        })?;
        drop(slave);

        Ok(Self {
            master: Arc::new(master),
            child,
            tail: Arc::new(Mutex::new(Tail::default())),
            procfs: Procfs::default(),
        })
    }

    pub fn pid(&self) -> u32 {
        self.child.id()
    }

    /// A handle that can end the run from anywhere, including after [`Self::drive`] has taken
    /// ownership.
    pub fn killer(&self) -> Killer {
        Killer {
            pgid: self.child.id() as i32,
        }
    }

    pub fn writer(&self) -> Writer {
        Writer {
            master: Arc::clone(&self.master),
        }
    }

    /// Runs until the command exits, publishing its bytes and its state.
    pub async fn drive(mut self, events: mpsc::Sender<RunEvent>) -> io::Result<State> {
        let reader_master = Arc::clone(&self.master);
        let reader_tail = Arc::clone(&self.tail);
        let byte_sink = events.clone();
        let reader = tokio::task::spawn_blocking(move || {
            let mut buf = [0u8; 8192];
            loop {
                match rustix::io::read(reader_master.as_fd(), &mut buf) {
                    Ok(0) => break,
                    Ok(n) => {
                        reader_tail.lock().expect("tail").push(&buf[..n]);
                        if byte_sink
                            .blocking_send(RunEvent::Bytes(buf[..n].to_vec()))
                            .is_err()
                        {
                            break;
                        }
                    }
                    Err(rustix::io::Errno::INTR) => continue,
                    Err(_) => break,
                }
            }
        });

        let mut published = State::Running;
        // Two agreeing readings before the run is called readable — see [`RunEvent::Readable`].
        let mut readable_streak = 0u8;
        let mut announced_readable = false;
        let mut silent_since = Instant::now();
        let mut last_bytes = 0usize;
        let mut settled_bytes = usize::MAX;
        let mut was_waiting = false;
        let mut ticker = tokio::time::interval(POLL);

        let final_state = loop {
            ticker.tick().await;

            if let Some(status) = self.child.try_wait()? {
                break State::Exited {
                    code: status.code(),
                    signal: exit_signal(&status),
                };
            }

            let (unterminated, completed, written) = {
                let tail = self.tail.lock().expect("tail");
                (
                    tail.unterminated().to_string(),
                    tail.completed().to_vec(),
                    tail.received(),
                )
            };
            if written != last_bytes {
                last_bytes = written;
                silent_since = Instant::now();
            }

            let waiting = self.procfs.waiting(self.pid());
            if waiting == Waiting::Unknown {
                readable_streak = 0;
            } else {
                readable_streak = readable_streak.saturating_add(1);
                if readable_streak >= 2 && !announced_readable {
                    announced_readable = true;
                    if events.send(RunEvent::Readable).await.is_err() {
                        break State::Running;
                    }
                }
            }
            let mode = mode_of(self.master.as_fd()).unwrap_or_default();
            // **A question is only published once its own text has arrived.** The child parks in
            // `read(2)` the instant it asks, which is before the bytes it just wrote have reached
            // this process — so the first tick that sees `Waiting` would publish a prompt with an
            // empty question. One tick of agreement, with no new bytes in between, is what makes
            // the two views consistent.
            let settled = written == settled_bytes;
            settled_bytes = written;

            let next = match waiting {
                Waiting::Waiting if was_waiting && settled => {
                    was_waiting = true;
                    Some(State::Waiting(Box::new(prompt::read(
                        &unterminated,
                        &completed,
                        mode,
                        CONTEXT_ROWS,
                    ))))
                }
                Waiting::Waiting => {
                    was_waiting = true;
                    None
                }
                Waiting::Busy => {
                    was_waiting = false;
                    Some(State::Running)
                }
                // **What survives the privilege wall.** A command that escalates hides its
                // `/proc` from us entirely (#332), and these two rungs are what is left.
                //
                // termios belongs to the tty and we hold the master, so a `sudo` on its own
                // password prompt is still measured as one (#339). Failing that, a run that has
                // stopped writing mid-line on text that parses as a question is *inferred* to be
                // asking — weaker evidence, said so on the wire, and the difference between
                // `sudo pacman -Syu` being usable on the board and not.
                Waiting::Unknown if settled && mode.asking_for_a_secret() => {
                    was_waiting = true;
                    Some(State::Waiting(Box::new(prompt::read(
                        &unterminated,
                        &completed,
                        mode,
                        CONTEXT_ROWS,
                    ))))
                }
                Waiting::Unknown => {
                    let hush = silent_since.elapsed();
                    // **Termios is trustworthy exactly where `/proc` is, and this is not there.**
                    // `sudo` relays for a command that has a controlling terminal, and while it
                    // does, the pty's own ECHO and ICANON describe the relay rather than the job
                    // (#341) — both off, which reads as a full-screen program and drops the
                    // commonest fleet prompt there is off the board. So the inference goes on the
                    // text alone.
                    let guess = (settled && hush >= INFERRED_AFTER)
                        .then(|| {
                            prompt::read(
                                &unterminated,
                                &completed,
                                kampr_core::question::Mode::default(),
                                CONTEXT_ROWS,
                            )
                        })
                        .filter(|q| q.reads_as_a_question());
                    match guess {
                        Some(question) => {
                            was_waiting = true;
                            Some(State::Waiting(Box::new(question.inferred())))
                        }
                        None => {
                            was_waiting = false;
                            if hush >= QUIET_AFTER {
                                Some(State::Quiet {
                                    seconds: hush.as_secs(),
                                })
                            } else {
                                Some(State::Running)
                            }
                        }
                    }
                }
            };

            let Some(next) = next else { continue };
            if !same_rung(&published, &next) {
                published = next.clone();
                if events.send(RunEvent::State(next)).await.is_err() {
                    break State::Running;
                }
            }
        };

        self.killer().hangup();
        reader.abort();
        let _ = events.send(RunEvent::State(final_state.clone())).await;
        Ok(final_state)
    }
}

/// Whether two states are the same as far as a watcher is concerned.
///
/// `Quiet` ticks its own seconds every poll and would otherwise republish four times a second for
/// as long as a host stays silent; a question republishes only when its wording changes, which is
/// what a second prompt on the same run looks like.
fn same_rung(a: &State, b: &State) -> bool {
    match (a, b) {
        (State::Quiet { .. }, State::Quiet { .. }) => true,
        (State::Waiting(x), State::Waiting(y)) => x == y,
        _ => a == b,
    }
}

/// Ends a run, by process **group**.
///
/// The group and not the pid: the supervisor `setsid`s its child, so a `sudo` that forked the real
/// job is the group leader and signalling it alone leaves the job behind. An orphaned `pacman`
/// holding the package database is the failure this exists to prevent.
#[derive(Debug, Clone, Copy)]
pub struct Killer {
    pgid: i32,
}

impl Killer {
    /// What a real terminal sends when its window closes, and what a package manager knows how to
    /// unwind from.
    pub fn hangup(&self) {
        self.signal(libc::SIGHUP);
    }

    pub fn kill(&self) {
        self.signal(libc::SIGKILL);
    }

    fn signal(&self, sig: i32) {
        if self.pgid > 1 {
            unsafe { libc::kill(-self.pgid, sig) };
        }
    }
}

/// **A dropped supervisor must not leave its command running.**
///
/// `spawn_blocking` cannot be cancelled, so the reader stays parked on the master fd and keeps it
/// open however the driving task ended — which means closing the master is not on its own enough
/// to hang the child up. Killing the group is, and it closes the slave, which is what finally
/// releases the reader.
impl Drop for Supervisor {
    fn drop(&mut self) {
        if matches!(self.child.try_wait(), Ok(Some(_))) {
            return;
        }
        let killer = self.killer();
        killer.hangup();
        for _ in 0..20 {
            if matches!(self.child.try_wait(), Ok(Some(_))) {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        killer.kill();
        let _ = self.child.wait();
    }
}

pub struct Writer {
    master: Arc<OwnedFd>,
}

impl Writer {
    pub fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut sent = 0;
        while sent < bytes.len() {
            match rustix::io::write(self.master.as_fd(), &bytes[sent..]) {
                Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
                Ok(n) => sent += n,
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

fn open_pty(geometry: Geometry) -> io::Result<(OwnedFd, OwnedFd)> {
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let name = ptsname(&master, Vec::new())?;
    let slave = rustix::fs::open(
        name.as_c_str(),
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::NOCTTY,
        rustix::fs::Mode::empty(),
    )?;
    set_winsize(master.as_fd(), geometry)?;
    Ok((master, slave))
}

fn set_winsize(fd: BorrowedFd<'_>, geometry: Geometry) -> io::Result<()> {
    rustix::termios::tcsetwinsize(
        fd,
        rustix::termios::Winsize {
            ws_row: geometry.rows,
            ws_col: geometry.cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )?;
    Ok(())
}

fn exit_signal(status: &std::process::ExitStatus) -> Option<i32> {
    std::os::unix::process::ExitStatusExt::signal(status)
}
