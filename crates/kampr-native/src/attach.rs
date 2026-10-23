mod paint;
mod term;

use crate::Screen;
use crate::replay;
use crate::screen::Feed;
use kampr_keep::{Ending, Exit, Host, Replay, SessionDir, SizedReader};
use rustix::event::{PollFd, PollFlags, poll};
use std::fs::File;
use std::io::{self, Read as _, Write};
use std::ops::Range;
use std::os::fd::AsFd;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use term::{Raw, Signals};

const SCROLLBACK: usize = 5_000;
const IDLE: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Default)]
pub struct AttachOptions {
    pub herdr_pane: Option<String>,
    pub herdr_socket: Option<String>,
}

impl AttachOptions {
    pub fn from_env() -> Self {
        let var = |k| std::env::var(k).ok().filter(|v| !v.is_empty());
        Self {
            herdr_pane: var("HERDR_PANE_ID"),
            herdr_socket: var("HERDR_SOCKET_PATH"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ended {
    Session(Exit),
    Detached,
}

pub fn exit_status(exit: &Exit) -> String {
    match (exit.code, exit.signal) {
        _ if exit.error.is_some() => format!("failed: {}", exit.error.as_deref().unwrap_or("")),
        (_, Some(signal)) => format!("killed by signal {signal}"),
        (Some(code), None) => format!("exited {code}"),
        (None, None) if exit.status_lost => "ended, status lost".to_string(),
        (None, None) => "ended".to_string(),
    }
}

pub fn attach(session: &SessionDir, opts: &AttachOptions) -> io::Result<Ended> {
    if let Some(exit) = session.exit() {
        return Ok(Ended::Session(exit));
    }
    let master = File::from(kampr_keep::borrow(session)?);
    let pid = std::process::id();
    let me = format!("attach:{pid}");
    let signals = Signals::install()?;
    let _raw = Raw::enter()?;
    let mine = term::tty_size();
    if let Some((cols, rows)) = mine {
        kampr_keep::claim_size(session, cols, rows, &me)?;
    }
    let (cols, rows) = mine.or_else(|| term::size(master.as_fd())).unwrap_or((80, 24));
    let mut host = kampr_keep::register_host(
        session,
        Host {
            pid,
            herdr_pane: opts.herdr_pane.clone(),
            herdr_socket: opts.herdr_socket.clone(),
            cols,
            rows,
            answers_from: None,
        },
    )?;
    let stop = &AtomicBool::new(false);
    let answers_from = host.host().answers_from.unwrap_or(0);
    let mut view = View::new(session, &master, &me, answers_from)?;
    std::thread::scope(|s| {
        let waker = signals.waker()?;
        let output = s.spawn(move || {
            let _waker = waker;
            view.run(stop)
        });
        let input = forward(&signals, &master, |claimed| {
            if let Some((cols, rows)) = claimed {
                kampr_keep::claim_size(session, cols, rows, &me)?;
                host.update(cols, rows)?;
            }
            Ok(())
        });
        stop.store(true, Ordering::Relaxed);
        let shown = output
            .join()
            .unwrap_or_else(|_| Err(io::Error::other("the attach's output thread panicked")));
        match (input, shown) {
            (Err(e), _) | (_, Err(e)) => Err(e),
            (Ok(()), Ok(Some(exit))) => Ok(Ended::Session(exit)),
            (Ok(()), Ok(None)) => Ok(Ended::Detached),
        }
    })
}

/// Returns when the output side has stopped (it wakes us) or when this terminal goes away.
fn forward(
    signals: &Signals,
    master: &File,
    mut on_winch: impl FnMut(Option<(u16, u16)>) -> io::Result<()>,
) -> io::Result<()> {
    let stdin = io::stdin();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let mut fds = [
            PollFd::new(&signals.read, PollFlags::IN),
            PollFd::new(&stdin, PollFlags::IN),
        ];
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
        let (woken, typed) = (fds[0].revents(), fds[1].revents());
        if woken.contains(PollFlags::IN) {
            let mut got = [0u8; 64];
            let n = (&signals.read).read(&mut got)?;
            for &byte in &got[..n] {
                match byte as libc::c_int {
                    libc::SIGWINCH => on_winch(term::tty_size())?,
                    _ => return Ok(()),
                }
            }
        }
        if typed.contains(PollFlags::IN) {
            let n = match rustix::io::read(&stdin, &mut buf) {
                Ok(0) => return Ok(()),
                Ok(n) => n,
                Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => continue,
                Err(e) => return Err(e.into()),
            };
            match (&*master).write_all(&buf[..n]) {
                Ok(()) => {}
                Err(e) if e.raw_os_error() == Some(libc::EIO) => return Ok(()),
                Err(e) => return Err(e),
            }
        } else if typed.intersects(PollFlags::HUP | PollFlags::ERR | PollFlags::NVAL) {
            return Ok(());
        }
    }
}

#[derive(PartialEq)]
struct Cropped {
    pty: (u16, u16),
    tty: (u16, u16),
    holder: String,
}

struct View<'a> {
    session: &'a SessionDir,
    master: &'a File,
    me: &'a str,
    out: File,
    reader: SizedReader,
    screen: Screen,
    passthrough: bool,
    host_alt: bool,
    shown: Option<Cropped>,
    answers_from: u64,
}

impl<'a> View<'a> {
    fn new(session: &'a SessionDir, master: &'a File, me: &'a str, answers_from: u64) -> io::Result<Self> {
        let out = File::from(io::stdout().as_fd().try_clone_to_owned()?);
        let resumed = replay::resume(session.path(), SCROLLBACK)?;
        Ok(Self {
            session,
            master,
            me,
            out,
            reader: resumed.reader,
            screen: resumed.screen,
            passthrough: true,
            host_alt: false,
            shown: None,
            answers_from,
        })
    }

    fn run(&mut self, stop: &AtomicBool) -> io::Result<Option<Exit>> {
        let result = self.serve(stop);
        let last = paint::last(&self.screen, &mut self.host_alt);
        let _ = self.out.write_all(&last);
        result
    }

    fn serve(&mut self, stop: &AtomicBool) -> io::Result<Option<Exit>> {
        let mut buf = vec![0u8; 256 * 1024];
        loop {
            let from = self.reader.offset();
            match self.reader.read(&mut buf)? {
                Replay::Data(n) => {
                    let fed = self.screen.feed(&buf[..n]);
                    self.answer(from, &fed, false)?;
                }
                Replay::Resize(to) => self.screen.resize(to.cols, to.rows),
                Replay::Truncated { .. } => self.resume()?,
                Replay::CaughtUp => break,
            }
        }
        let first = paint::first(&self.screen, &mut self.host_alt);
        self.out.write_all(&first)?;
        self.reconcile()?;
        let mut dirty = false;
        loop {
            let from = self.reader.offset();
            match self.reader.read(&mut buf)? {
                Replay::Data(n) => {
                    let bytes = &buf[..n];
                    let fed = self.screen.feed(bytes);
                    let hidden = self.answer(from, &fed, self.passthrough)?;
                    if self.passthrough {
                        let mut at = 0;
                        for cut in hidden {
                            if cut.start < at {
                                continue;
                            }
                            self.out.write_all(&bytes[at..cut.start])?;
                            at = cut.end;
                        }
                        self.out.write_all(&bytes[at..])?;
                        self.host_alt = self.screen.alternate_screen();
                    } else {
                        dirty = true;
                    }
                }
                Replay::Resize(to) => {
                    self.screen.resize(to.cols, to.rows);
                    self.reconcile()?;
                    dirty |= !self.passthrough;
                }
                Replay::Truncated { .. } => {
                    self.resume()?;
                    dirty = true;
                }
                Replay::CaughtUp => {
                    if dirty {
                        self.repaint()?;
                        dirty = false;
                    }
                    if let Some(exit) = self.ended()? {
                        return Ok(Some(exit));
                    }
                    if stop.load(Ordering::Relaxed) {
                        return Ok(None);
                    }
                    self.reader.wait(IDLE)?;
                    self.reconcile()?;
                }
            }
        }
    }

    /// herdr answers DA1 and not `OSC 7501;?` (#611), and Claude takes a 7501 answer behind its DA1
    /// answer for none at all (#600), so the answer goes to the master before the bytes that asked
    /// reach this terminal. A query is ours once it ends past the offset we registered at, and of
    /// several attaches the lowest pid answers; the node answers the rest (`live.rs`).
    ///
    /// Every other question is the terminal's, but only if it sees it: bytes read while catching up
    /// or painted cropped are never `forwarded`, and the node leaves a query past our offset to us,
    /// so those are answered here from the emulator, as the node would have. Passed through by an
    /// attach that is not the answerer, a question is cut out of what its terminal is shown, so
    /// that a session on two desks hears one answer, not two; those cuts are returned.
    fn answer(&self, from: u64, fed: &Feed, forwarded: bool) -> io::Result<Vec<Range<usize>>> {
        let mut hidden = Vec::new();
        if fed.asked.is_empty() {
            return Ok(hidden);
        }
        let Ok(hosts) = self.session.hosts_settled() else {
            return Ok(hidden);
        };
        for ask in &fed.asked {
            let end = from + ask.end as u64;
            if end <= self.answers_from {
                continue;
            }
            let Some(answerer) = hosts
                .iter()
                .find(|h| h.answers_from.is_some() && h.covers(end))
                .map(|h| h.pid)
            else {
                continue;
            };
            if answerer == std::process::id() {
                if ask.program || !forwarded {
                    match (&*self.master).write_all(&fed.replies[ask.reply.clone()]) {
                        Err(e) if e.raw_os_error() != Some(libc::EIO) => return Err(e),
                        _ => {}
                    }
                }
            } else if forwarded
                && !ask.program
                && let Some(span) = &ask.span
            {
                hidden.push(span.clone());
            }
        }
        Ok(hidden)
    }

    fn ended(&mut self) -> io::Result<Option<Exit>> {
        Ok(match self.session.ending() {
            // The keeper writes exit.json only after the master's last byte reached the log.
            Some(Ending::Exited(exit)) => (self.reader.end()? <= self.reader.offset()).then_some(exit),
            Some(Ending::Lost) => Some(Exit {
                code: None,
                signal: None,
                status_lost: true,
                at_unix: 0,
                error: Some("the keeper is gone".into()),
            }),
            None => None,
        })
    }

    fn holder(&self) -> String {
        self.session
            .size()
            .map_or_else(|| "nobody".to_string(), |s| s.holder)
    }

    /// A reader that fell behind the log's trimming starts again where `kampr serve` would.
    fn resume(&mut self) -> io::Result<()> {
        let resumed = replay::resume(self.session.path(), SCROLLBACK)?;
        self.screen = resumed.screen;
        self.reader = resumed.reader;
        Ok(())
    }

    fn reconcile(&mut self) -> io::Result<()> {
        let pty = self.screen.size();
        let tty = term::tty_size();
        let fits = tty.is_none_or(|t| t == pty);
        let holder = self.holder();
        if fits || holder == self.me {
            if !self.passthrough {
                self.passthrough = true;
                self.shown = None;
                let frame = paint::full(&self.screen, &mut self.host_alt);
                self.out.write_all(&frame)?;
            }
            return Ok(());
        }
        self.passthrough = false;
        let now = Cropped {
            pty,
            tty: tty.unwrap_or(pty),
            holder,
        };
        if self.shown.as_ref() != Some(&now) {
            self.shown = Some(now);
            self.repaint()?;
        }
        Ok(())
    }

    fn repaint(&mut self) -> io::Result<()> {
        let frame = if self.passthrough {
            paint::full(&self.screen, &mut self.host_alt)
        } else {
            let tty = term::tty_size().unwrap_or(self.screen.size());
            let holder = self.holder();
            paint::cropped(&self.screen, &mut self.host_alt, tty, &holder)
        };
        self.out.write_all(&frame)
    }
}
