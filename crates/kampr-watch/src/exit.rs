use std::io;
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, OwnedFd, RawFd};
use std::time::Duration;

/// A process's end, as an fd that turns readable when it comes: a pidfd on Linux, a kqueue holding
/// `EVFILT_PROC` on macOS. Any process this user may see, child or not.
///
/// A pid names whoever holds it when [`Exit::open`] runs, so a caller that learned the pid a while
/// ago checks it is still the same process (start time) before trusting what this says.
pub struct Exit(OwnedFd);

impl Exit {
    /// `None` when the process has already gone.
    pub fn open(pid: u32) -> io::Result<Option<Self>> {
        let Some(pid) = i32::try_from(pid).ok().and_then(rustix::process::Pid::from_raw) else {
            return Ok(None);
        };
        match imp::open(pid) {
            Ok(fd) => Ok(Some(Self(fd))),
            Err(rustix::io::Errno::SRCH) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// True once the process has exited, false if `timeout` passed first. `None` waits for it.
    pub fn wait(&self, timeout: Option<Duration>) -> io::Result<bool> {
        use rustix::event::{PollFd, PollFlags, Timespec, poll};
        let spec = timeout.and_then(|t| Timespec::try_from(t).ok());
        let mut fds = [PollFd::new(&self.0, PollFlags::IN)];
        loop {
            match poll(&mut fds, spec.as_ref()) {
                Ok(n) => return Ok(n > 0),
                Err(rustix::io::Errno::INTR) => continue,
                Err(e) => return Err(e.into()),
            }
        }
    }
}

impl AsFd for Exit {
    fn as_fd(&self) -> BorrowedFd<'_> {
        self.0.as_fd()
    }
}

impl AsRawFd for Exit {
    fn as_raw_fd(&self) -> RawFd {
        self.0.as_raw_fd()
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use rustix::process::{Pid, PidfdFlags, pidfd_open};
    use std::os::fd::OwnedFd;

    pub(super) fn open(pid: Pid) -> rustix::io::Result<OwnedFd> {
        pidfd_open(pid, PidfdFlags::empty())
    }
}

#[cfg(not(target_os = "linux"))]
mod imp {
    use rustix::event::kqueue::{Event, EventFilter, EventFlags, ProcessEvents, kevent, kqueue};
    use rustix::process::Pid;
    use std::mem::MaybeUninit;
    use std::os::fd::OwnedFd;
    use std::time::Duration;

    pub(super) fn open(pid: Pid) -> rustix::io::Result<OwnedFd> {
        let kq = kqueue()?;
        let exit = Event::new(
            EventFilter::Proc {
                pid,
                flags: ProcessEvents::EXIT,
            },
            EventFlags::ADD | EventFlags::ONESHOT,
            std::ptr::null_mut(),
        );
        let mut none: [MaybeUninit<Event>; 0] = [];
        // Safety: a process filter refers to no fd.
        unsafe { kevent(&kq, &[exit], &mut none, Some(Duration::ZERO))? };
        Ok(kq)
    }
}
