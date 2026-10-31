#[cfg(target_os = "linux")]
mod imp {
    use anyhow::{Result, bail};
    use rustix::net::{RecvFlags, SendFlags};
    use std::io;
    use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    pub const RECV: RecvFlags = RecvFlags::CMSG_CLOEXEC;
    pub const SEND: SendFlags = SendFlags::NOSIGNAL;

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
}

// No signalfd here: SIGUSR1 is ignored (which, like Linux's blocked mask, survives the re-exec) and
// counted through kqueue.
#[cfg(not(target_os = "linux"))]
mod imp {
    use anyhow::{Result, bail};
    use rustix::net::{RecvFlags, SendFlags};
    use rustix::process::Pid;
    use std::io;
    use std::os::fd::{AsFd, BorrowedFd, FromRawFd, OwnedFd};
    use std::os::unix::io::AsRawFd;
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;

    pub const RECV: RecvFlags = RecvFlags::empty();
    pub const SEND: SendFlags = SendFlags::empty();

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
}

pub(crate) use imp::*;
