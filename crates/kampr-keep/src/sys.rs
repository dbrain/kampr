#[cfg(target_os = "linux")]
mod imp {
    use anyhow::{Result, bail};
    use rustix::event::{PollFd, PollFlags, Timespec, poll};
    use rustix::fs::inotify;
    use rustix::net::{RecvFlags, SendFlags};
    use rustix::process::{Pid, PidfdFlags, WaitId, pidfd_open};
    use std::io;
    use std::mem::MaybeUninit;
    use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd};
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::time::Duration;

    pub const RECV: RecvFlags = RecvFlags::CMSG_CLOEXEC;
    pub const SEND: SendFlags = SendFlags::NOSIGNAL;
    pub const TICK: Option<Duration> = None;

    pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
        Ok(rustix::net::sockopt::socket_peercred(stream)?.uid.as_raw())
    }

    /// `kampr update` replaces the binary in place, so `/proc/self/exe` names it ` (deleted)`
    /// until this process restarts, and a keeper started from that name would not start at all.
    pub fn own_exe() -> io::Result<PathBuf> {
        use std::os::unix::ffi::OsStrExt;
        let link = std::fs::read_link("/proc/self/exe")?;
        let bytes = link.as_os_str().as_bytes();
        Ok(match bytes.strip_suffix(b" (deleted)") {
            Some(stripped) => PathBuf::from(std::ffi::OsStr::from_bytes(stripped)),
            None => link,
        })
    }

    pub fn pid_alive(pid: u32) -> bool {
        let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
            return false;
        };
        stat.rsplit_once(')')
            .and_then(|(_, rest)| rest.split_ascii_whitespace().next())
            .is_some_and(|state| state != "Z" && state != "X")
    }

    pub fn child_pre_exec() {}

    pub struct Child {
        pidfd: OwnedFd,
    }

    impl Child {
        pub fn watch(pid: u32) -> Option<Self> {
            let pidfd = pidfd_open(Pid::from_raw(pid as i32)?, PidfdFlags::empty()).ok()?;
            Some(Self { pidfd })
        }

        pub fn fd(&self) -> Option<BorrowedFd<'_>> {
            Some(self.pidfd.as_fd())
        }

        pub fn id(&self) -> WaitId<'_> {
            WaitId::PidFd(self.pidfd.as_fd())
        }

        pub fn settle(&self, grace: Duration) {
            let timeout = Timespec::try_from(grace).ok();
            let mut fds = [PollFd::new(&self.pidfd, PollFlags::IN)];
            let _ = poll(&mut fds, timeout.as_ref());
        }
    }

    pub struct Usr1 {
        fd: OwnedFd,
    }

    impl Usr1 {
        pub fn open() -> Result<Self> {
            unsafe {
                let mut mask: libc::sigset_t = std::mem::zeroed();
                libc::sigemptyset(&mut mask);
                libc::sigaddset(&mut mask, libc::SIGUSR1);
                if libc::pthread_sigmask(libc::SIG_BLOCK, &mask, std::ptr::null_mut()) != 0 {
                    bail!("blocking SIGUSR1: {}", io::Error::last_os_error());
                }
                let fd = libc::signalfd(-1, &mask, libc::SFD_CLOEXEC | libc::SFD_NONBLOCK);
                if fd < 0 {
                    bail!("signalfd: {}", io::Error::last_os_error());
                }
                Ok(Self {
                    fd: OwnedFd::from_raw_fd(fd),
                })
            }
        }

        pub fn fd(&self) -> BorrowedFd<'_> {
            self.fd.as_fd()
        }

        pub fn take(&self) -> bool {
            let mut got = false;
            let mut info = [0u8; std::mem::size_of::<libc::signalfd_siginfo>()];
            while let Ok(n) = rustix::io::read(&self.fd, &mut info) {
                if n < info.len() {
                    break;
                }
                let signo = u32::from_ne_bytes([info[0], info[1], info[2], info[3]]);
                got |= signo == libc::SIGUSR1 as u32;
            }
            got
        }
    }

    pub struct DirWatch {
        inotify: OwnedFd,
    }

    impl DirWatch {
        pub fn open(dir: &Path) -> io::Result<Self> {
            let inotify = inotify::init(inotify::CreateFlags::CLOEXEC | inotify::CreateFlags::NONBLOCK)?;
            inotify::add_watch(
                &inotify,
                dir,
                inotify::WatchFlags::MODIFY
                    | inotify::WatchFlags::CREATE
                    | inotify::WatchFlags::MOVED_TO
                    | inotify::WatchFlags::DELETE_SELF,
            )?;
            Ok(Self { inotify })
        }

        pub fn wait(&self, left: Duration) -> io::Result<()> {
            let spec = Timespec::try_from(left).map_err(io::Error::other)?;
            let mut fds = [PollFd::new(&self.inotify, PollFlags::IN)];
            match poll(&mut fds, Some(&spec)) {
                Ok(_) | Err(rustix::io::Errno::INTR) => Ok(()),
                Err(e) => Err(e.into()),
            }
        }

        pub fn drain(&self) -> io::Result<()> {
            let mut buf = [MaybeUninit::<u8>::uninit(); 4096];
            let mut reader = inotify::Reader::new(&self.inotify, &mut buf);
            loop {
                match reader.next() {
                    Ok(_) => {}
                    Err(rustix::io::Errno::AGAIN) => return Ok(()),
                    Err(rustix::io::Errno::INTR) => {}
                    Err(e) => return Err(e.into()),
                }
            }
        }
    }
}

// No pidfd, signalfd or inotify here: the child is polled with waitid on a tick, SIGUSR1 is ignored
// (which, like Linux's blocked mask, survives the re-exec) and counted through kqueue, and a log
// follower re-reads the directory on a tick.
#[cfg(not(target_os = "linux"))]
mod imp {
    use anyhow::{Result, bail};
    use rustix::net::{RecvFlags, SendFlags};
    use rustix::process::{Pid, WaitId};
    use std::io;
    use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd};
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, Instant};

    pub const RECV: RecvFlags = RecvFlags::empty();
    pub const SEND: SendFlags = SendFlags::empty();
    pub const TICK: Option<Duration> = Some(Duration::from_millis(100));
    const FOLLOW_TICK: Duration = Duration::from_millis(50);

    pub fn peer_uid(stream: &UnixStream) -> io::Result<u32> {
        let (mut uid, mut gid) = (0, 0);
        if unsafe { libc::getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) } != 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(uid)
    }

    pub fn own_exe() -> io::Result<PathBuf> {
        std::env::current_exe()
    }

    pub fn pid_alive(pid: u32) -> bool {
        Pid::from_raw(pid as i32).is_some_and(|pid| {
            matches!(
                rustix::process::test_kill_process(pid),
                Ok(()) | Err(rustix::io::Errno::PERM)
            )
        })
    }

    pub fn child_pre_exec() {
        unsafe { libc::signal(libc::SIGUSR1, libc::SIG_DFL) };
    }

    pub struct Child {
        pid: Pid,
    }

    impl Child {
        pub fn watch(pid: u32) -> Option<Self> {
            Some(Self {
                pid: Pid::from_raw(pid as i32)?,
            })
        }

        pub fn fd(&self) -> Option<BorrowedFd<'_>> {
            None
        }

        pub fn id(&self) -> WaitId<'_> {
            WaitId::Pid(self.pid)
        }

        pub fn settle(&self, grace: Duration) {
            let deadline = Instant::now() + grace;
            while Instant::now() < deadline
                && rustix::process::waitid(
                    self.id(),
                    rustix::process::WaitIdOptions::EXITED
                        | rustix::process::WaitIdOptions::NOHANG
                        | rustix::process::WaitIdOptions::NOWAIT,
                )
                .is_ok_and(|s| s.is_none())
            {
                std::thread::sleep(TICK.unwrap_or_default());
            }
        }
    }

    pub struct Usr1 {
        kq: OwnedFd,
    }

    impl Usr1 {
        pub fn open() -> Result<Self> {
            unsafe {
                if libc::signal(libc::SIGUSR1, libc::SIG_IGN) == libc::SIG_ERR {
                    bail!("ignoring SIGUSR1: {}", io::Error::last_os_error());
                }
                let kq = libc::kqueue();
                if kq < 0 {
                    bail!("kqueue: {}", io::Error::last_os_error());
                }
                let kq = OwnedFd::from_raw_fd(kq);
                let mut change: libc::kevent = std::mem::zeroed();
                change.ident = libc::SIGUSR1 as _;
                change.filter = libc::EVFILT_SIGNAL;
                change.flags = libc::EV_ADD;
                if libc::kevent(
                    kq.as_raw_fd(),
                    &change,
                    1,
                    std::ptr::null_mut(),
                    0,
                    std::ptr::null(),
                ) < 0
                {
                    bail!("kevent: {}", io::Error::last_os_error());
                }
                Ok(Self { kq })
            }
        }

        pub fn fd(&self) -> BorrowedFd<'_> {
            self.kq.as_fd()
        }

        pub fn take(&self) -> bool {
            let zero = libc::timespec {
                tv_sec: 0,
                tv_nsec: 0,
            };
            let mut got = false;
            loop {
                let mut event: libc::kevent = unsafe { std::mem::zeroed() };
                let n =
                    unsafe { libc::kevent(self.kq.as_raw_fd(), std::ptr::null(), 0, &mut event, 1, &zero) };
                if n <= 0 {
                    return got;
                }
                got |= event.ident == libc::SIGUSR1 as libc::uintptr_t;
            }
        }
    }

    pub struct DirWatch;

    impl DirWatch {
        pub fn open(_dir: &Path) -> io::Result<Self> {
            Ok(Self)
        }

        pub fn wait(&self, left: Duration) -> io::Result<()> {
            std::thread::sleep(left.min(FOLLOW_TICK));
            Ok(())
        }

        pub fn drain(&self) -> io::Result<()> {
            Ok(())
        }
    }
}

pub(crate) use imp::*;
