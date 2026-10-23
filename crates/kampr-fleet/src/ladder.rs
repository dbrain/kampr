//! What one run is doing, read every [`POLL`] from its keeper's directory.
//!
//! **Only the run's own ancestor can read its state** (#331), and the keeper is that ancestor, so
//! the kernel's half comes from the keeper's `state` reply and the text's half from the log. A
//! command that escalates is as opaque to its keeper as to the node (#334): such a run is reported
//! blind and read off its screen.

use crate::tail::Tail;
use crate::waiting::{Waiting, mode_of, waiting};
use kampr_core::question::{self as prompt, Mode};
use kampr_keep::{Ending, Replay, SessionDir, SizedReader};
use std::os::fd::{AsFd, OwnedFd};
use std::time::{Duration, Instant};

pub use kampr_core::provider::FleetState as State;

/// Four times a second: a person waiting for a host to answer notices a second, and a `state`
/// request is one small reply about a tree that is rarely more than three deep.
pub(crate) const POLL: Duration = Duration::from_millis(250);

/// The *fallback* rung, for the hosts #331/#332 describe — never the rung that decides a host is
/// asking something. A build that links for twelve seconds is quiet too.
const QUIET_AFTER: Duration = Duration::from_secs(10);

const CONTEXT_ROWS: usize = 6;

/// How long a run whose state cannot be read must sit on an unterminated prompt before the screen
/// may speak for the kernel. Long enough that a program pausing mid-line does not qualify, short
/// enough that nobody waits on the board for `sudo pacman -Syu`'s `[Y/n]`.
const INFERRED_AFTER: Duration = Duration::from_secs(2);

#[derive(Debug)]
pub(crate) enum Seen {
    State(State),
    /// Sent at most once, after two consecutive readings agree. One is not enough: for a moment
    /// after the fork the child is still the keeper's own un-`exec`ed copy, readable however
    /// privileged the binary it is about to become.
    Readable,
}

pub(crate) struct Ladder {
    session: SessionDir,
    log: Option<SizedReader>,
    buf: Vec<u8>,
    tail: Tail,
    master: Option<OwnedFd>,
    published: State,
    readable_streak: u8,
    announced_readable: bool,
    silent_since: Instant,
    last_bytes: usize,
    settled_bytes: usize,
    was_waiting: bool,
}

impl Ladder {
    pub fn new(session: SessionDir) -> Self {
        Self {
            session,
            log: None,
            buf: vec![0u8; 64 * 1024],
            tail: Tail::default(),
            master: None,
            published: State::Running,
            readable_streak: 0,
            announced_readable: false,
            silent_since: Instant::now(),
            last_bytes: 0,
            settled_bytes: usize::MAX,
            was_waiting: false,
        }
    }

    /// One reading. A run that has ended yields its `Exited` last, after the whole log is read,
    /// because the keeper writes `exit.json` only once it has logged everything.
    pub fn step(&mut self) -> Vec<Seen> {
        if let Some(ended) = self.ended() {
            self.drain();
            return vec![Seen::State(ended)];
        }
        self.drain();
        let mut seen = Vec::new();

        let written = self.tail.received();
        if written != self.last_bytes {
            self.last_bytes = written;
            self.silent_since = Instant::now();
        }

        let waiting = kampr_keep::state(&self.session).map_or(Waiting::Unknown, |s| waiting(&s));
        if waiting == Waiting::Unknown {
            self.readable_streak = 0;
        } else {
            self.readable_streak = self.readable_streak.saturating_add(1);
            if self.readable_streak >= 2 && !self.announced_readable {
                self.announced_readable = true;
                seen.push(Seen::Readable);
            }
        }
        let mode = self.mode();
        // **A question is only published once its own text has arrived.** The child parks in
        // `read(2)` the instant it asks, which can be before the keeper has logged what it just
        // wrote, so one reading of agreement with no new bytes in between is what makes the two
        // views consistent.
        let settled = written == self.settled_bytes;
        self.settled_bytes = written;

        if let Some(next) = self.rung(waiting, settled, mode)
            && !same_rung(&self.published, &next)
        {
            self.published = next.clone();
            seen.push(Seen::State(next));
        }
        seen
    }

    fn rung(&mut self, waiting: Waiting, settled: bool, mode: Mode) -> Option<State> {
        let question =
            |tail: &Tail, mode: Mode| prompt::read(tail.unterminated(), tail.completed(), mode, CONTEXT_ROWS);
        match waiting {
            Waiting::Waiting if self.was_waiting && settled => {
                Some(State::Waiting(Box::new(question(&self.tail, mode))))
            }
            Waiting::Waiting => {
                self.was_waiting = true;
                None
            }
            Waiting::Busy => {
                self.was_waiting = false;
                Some(State::Running)
            }
            // **What survives the privilege wall.** termios belongs to the tty, so a `sudo` on its
            // own password prompt is still measured as one (#339). Failing that, a run that has
            // stopped writing mid-line on text that parses as a question is *inferred* to be
            // asking — weaker evidence, said so on the wire.
            Waiting::Unknown if settled && mode.asking_for_a_secret() => {
                self.was_waiting = true;
                Some(State::Waiting(Box::new(question(&self.tail, mode))))
            }
            Waiting::Unknown => {
                let hush = self.silent_since.elapsed();
                // **Termios is trustworthy exactly where `/proc` is, and this is not there.**
                // While `sudo` relays, the pty's ECHO and ICANON describe the relay rather than
                // the job (#341), so the inference goes on the text alone.
                let guess = (settled && hush >= INFERRED_AFTER)
                    .then(|| question(&self.tail, Mode::default()))
                    .filter(|q| q.reads_as_a_question());
                match guess {
                    Some(asked) => {
                        self.was_waiting = true;
                        Some(State::Waiting(Box::new(asked.inferred())))
                    }
                    None => {
                        self.was_waiting = false;
                        Some(match hush >= QUIET_AFTER {
                            true => State::Quiet {
                                seconds: hush.as_secs(),
                            },
                            false => State::Running,
                        })
                    }
                }
            }
        }
    }

    fn ended(&self) -> Option<State> {
        Some(match self.session.ending()? {
            Ending::Exited(exit) => State::Exited {
                code: exit.code,
                signal: exit.signal,
            },
            Ending::Lost => State::Exited {
                code: None,
                signal: None,
            },
        })
    }

    fn drain(&mut self) {
        if self.log.is_none() {
            self.log = SizedReader::at(self.session.path(), 0).ok();
        }
        let Some(log) = self.log.as_mut() else {
            return;
        };
        loop {
            match log.read(&mut self.buf) {
                Ok(Replay::Data(n)) => self.tail.push(&self.buf[..n]),
                Ok(Replay::Resize(_) | Replay::Truncated { .. }) => {}
                Ok(Replay::CaughtUp) | Err(_) => return,
            }
        }
    }

    /// A borrowed master is for `TIOCSWINSZ`, writes and, here, `tcgetattr`; never for reading,
    /// which is the keeper's alone.
    fn mode(&mut self) -> Mode {
        if self.master.is_none() {
            self.master = kampr_keep::borrow(&self.session).ok();
        }
        self.master
            .as_ref()
            .and_then(|fd| mode_of(fd.as_fd()))
            .unwrap_or_default()
    }
}

/// `Quiet` ticks its own seconds every poll and would otherwise republish four times a second for
/// as long as a host stays silent; a question republishes only when its wording changes.
fn same_rung(a: &State, b: &State) -> bool {
    match (a, b) {
        (State::Quiet { .. }, State::Quiet { .. }) => true,
        (State::Waiting(x), State::Waiting(y)) => x == y,
        _ => a == b,
    }
}
