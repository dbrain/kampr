use kampr_watch::Exit;
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::Mutex;
use std::time::Instant;

/// One thread's sleep until a deadline, a process's end or a ring from another thread, whichever
/// comes first; with none of them due it sleeps without waking.
pub(crate) struct Alarm {
    bell: UnixStream,
    ear: UnixStream,
    sleep: Mutex<Sleep>,
}

#[derive(Clone, Copy)]
enum Sleep {
    Awake,
    Until(Option<Instant>),
}

impl Alarm {
    pub fn new() -> io::Result<Self> {
        let (bell, ear) = UnixStream::pair()?;
        bell.set_nonblocking(true)?;
        ear.set_nonblocking(true)?;
        Ok(Self {
            bell,
            ear,
            sleep: Mutex::new(Sleep::Awake),
        })
    }

    /// Makes the sleeper look again by `at`. A sleeper that is awake is rung all the same: it may
    /// be about to sleep on a deadline worked out before this one existed.
    pub fn ring_by(&self, at: Instant) {
        let sleep = self.sleep.lock().unwrap();
        if let Sleep::Until(Some(until)) = *sleep
            && until <= at
        {
            return;
        }
        drop(sleep);
        self.ring();
    }

    pub fn ring(&self) {
        let _ = (&self.bell).write(&[0]);
    }

    /// True when `exit`'s process has ended.
    pub fn wait(&self, until: Option<Instant>, exit: Option<&Exit>) -> io::Result<bool> {
        *self.sleep.lock().unwrap() = Sleep::Until(until);
        let timeout = until
            .map(|at| Timespec::try_from(at.saturating_duration_since(Instant::now())))
            .transpose()
            .map_err(io::Error::other)?;
        let mut fds = vec![PollFd::new(&self.ear, PollFlags::IN)];
        if let Some(exit) = exit {
            fds.push(PollFd::new(exit, PollFlags::IN));
        }
        let polled = poll(&mut fds, timeout.as_ref());
        let ended = fds.get(1).is_some_and(|fd| !fd.revents().is_empty());
        *self.sleep.lock().unwrap() = Sleep::Awake;
        let mut drain = [0u8; 64];
        while (&self.ear).read(&mut drain).is_ok_and(|n| n > 0) {}
        match polled {
            Ok(_) | Err(rustix::io::Errno::INTR) => Ok(ended),
            Err(e) => Err(e.into()),
        }
    }
}
