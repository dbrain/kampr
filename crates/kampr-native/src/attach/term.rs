use rustix::termios::{self, OptionalActions, Termios};
use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicI32, Ordering};

const ENDED: u8 = 0;

pub(super) fn size(fd: BorrowedFd<'_>) -> Option<(u16, u16)> {
    let ws = termios::tcgetwinsize(fd).ok()?;
    (ws.ws_col > 0 && ws.ws_row > 0).then_some((ws.ws_col, ws.ws_row))
}

pub(super) fn tty_size() -> Option<(u16, u16)> {
    size(io::stdin().as_fd()).or_else(|| size(io::stdout().as_fd()))
}

pub(super) struct Raw {
    saved: Option<Termios>,
}

impl Raw {
    pub(super) fn enter() -> io::Result<Self> {
        let stdin = io::stdin();
        if !termios::isatty(stdin.as_fd()) {
            return Ok(Self { saved: None });
        }
        let saved = termios::tcgetattr(stdin.as_fd())?;
        let mut raw = saved.clone();
        raw.make_raw();
        // TCSAFLUSH would drop what was typed while the attach started (#580).
        termios::tcsetattr(stdin.as_fd(), OptionalActions::Now, &raw)?;
        Ok(Self { saved: Some(saved) })
    }
}

impl Drop for Raw {
    fn drop(&mut self) {
        if let Some(saved) = &self.saved {
            let _ = termios::tcsetattr(io::stdin().as_fd(), OptionalActions::Now, saved);
        }
    }
}

static WAKE: AtomicI32 = AtomicI32::new(-1);

const CAUGHT: [libc::c_int; 5] = [
    libc::SIGWINCH,
    libc::SIGHUP,
    libc::SIGTERM,
    libc::SIGINT,
    libc::SIGQUIT,
];

extern "C" fn on_signal(signal: libc::c_int) {
    let fd = WAKE.load(Ordering::Relaxed);
    if fd >= 0 {
        let byte = signal as u8;
        unsafe { libc::write(fd, (&raw const byte).cast(), 1) };
    }
}

pub(super) struct Signals {
    pub(super) read: UnixStream,
    write: UnixStream,
    old: Vec<(libc::c_int, libc::sigaction)>,
}

impl Signals {
    pub(super) fn install() -> io::Result<Self> {
        let (read, write) = UnixStream::pair()?;
        write.set_nonblocking(true)?;
        WAKE.store(write.as_raw_fd(), Ordering::Relaxed);
        let mut old = Vec::with_capacity(CAUGHT.len());
        for signal in CAUGHT {
            unsafe {
                let mut action: libc::sigaction = std::mem::zeroed();
                action.sa_sigaction = on_signal as *const () as usize;
                action.sa_flags = libc::SA_RESTART;
                libc::sigemptyset(&mut action.sa_mask);
                let mut was: libc::sigaction = std::mem::zeroed();
                if libc::sigaction(signal, &action, &mut was) < 0 {
                    return Err(io::Error::last_os_error());
                }
                old.push((signal, was));
            }
        }
        Ok(Self { read, write, old })
    }

    pub(super) fn waker(&self) -> io::Result<Waker> {
        Ok(Waker(self.write.try_clone()?))
    }
}

impl Drop for Signals {
    fn drop(&mut self) {
        for (signal, was) in &self.old {
            unsafe { libc::sigaction(*signal, was, std::ptr::null_mut()) };
        }
        WAKE.store(-1, Ordering::Relaxed);
    }
}

/// Wakes the input loop when the output side stops, however it stops, panics included.
pub(super) struct Waker(UnixStream);

impl Drop for Waker {
    fn drop(&mut self) {
        let _ = io::Write::write(&mut &self.0, &[ENDED]);
    }
}
