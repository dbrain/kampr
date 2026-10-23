use crate::ctl::{HOLDER_MAX, send_fd, set_winsize};
use crate::dir::{
    CTL, EXIT, Exit, How, Image, KEEPER, KeeperInfo, SIZE, SPEC, SessionDir, Size, Spec, now_unix, write_json,
};
use crate::log::{Appender, DEFAULT_CAP};
use crate::notify::notify;
use crate::procs;
use crate::sizes::write_size;
use crate::sys::{self, Child, Usr1};
use anyhow::{Context, Result, anyhow, bail};
use rustix::event::{PollFd, PollFlags, Timespec, poll};
use rustix::io::{Errno, FdFlags, fcntl_getfd, fcntl_setfd};
use rustix::process::{WaitIdOptions, getpid, getuid, waitid};
use std::ffi::{CString, OsString};
use std::io::{self, BufRead, BufReader, Read as _, Write};
use std::os::fd::{AsFd, AsRawFd, BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const HANDOVER: &str = "KAMPR_KEEP_HANDOVER";
const LISTEN_FDS_START: RawFd = 3;
const CTL_TIMEOUT: Duration = Duration::from_millis(250);
const REAP_GRACE: Duration = Duration::from_secs(2);
// A process left behind that ignores the hangup holds the slave open, and the session would
// outlive its program for as long as that process lives; a terminal closes when its program exits.
const DRAIN_AFTER_EXIT: Duration = Duration::from_secs(3);

const NOT_FOR_THE_CHILD: &[&str] = &[
    HANDOVER,
    "NOTIFY_SOCKET",
    "LISTEN_PID",
    "LISTEN_FDS",
    "LISTEN_FDNAMES",
    "INVOCATION_ID",
    "JOURNAL_STREAM",
    "SYSTEMD_EXEC_PID",
    "MANAGERPID",
    "MEMORY_PRESSURE_WATCH",
    "MEMORY_PRESSURE_WRITE",
    "WATCHDOG_PID",
    "WATCHDOG_USEC",
];

#[derive(Debug)]
struct Unstarted;

impl std::fmt::Display for Unstarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("the session's command did not start")
    }
}

struct Started {
    master: OwnedFd,
    child: u32,
    how: How,
}

enum Outcome {
    Running,
    Reaped { code: Option<i32>, signal: Option<i32> },
    Lost,
}

pub fn serve(dir: &Path) -> Result<()> {
    let dir = std::path::absolute(dir)?;
    let signals = Usr1::open()?;
    let exe = sys::own_exe()?;
    let session = SessionDir::at(&dir);
    let spec = session.spec().context("reading spec.json")?;

    let started = match recover(&session).and_then(|found| match found {
        Some(started) => Ok(started),
        None => spawn_child(&spec).map_err(|e| e.context(Unstarted)),
    }) {
        Ok(started) => started,
        Err(e) => {
            write_json(
                &dir.join(EXIT),
                &Exit {
                    code: None,
                    signal: None,
                    status_lost: e.downcast_ref::<Unstarted>().is_none(),
                    at_unix: now_unix(),
                    error: Some(format!("{e:#}")),
                },
            )?;
            notify("READY=1", &[])?;
            notify("FDSTOREREMOVE=1\nFDNAME=pty", &[])?;
            return Ok(());
        }
    };
    let Started { master, child, how } = started;
    fcntl_setfd(&master, FdFlags::CLOEXEC)?;

    write_json(
        &dir.join(KEEPER),
        &KeeperInfo {
            keeper: getpid().as_raw_nonzero().get() as u32,
            child,
            exe: exe.to_string_lossy().into_owned(),
            started_unix: now_unix(),
            how,
            image: std::fs::metadata(&exe).ok().map(|meta| Image::of(&meta)),
        },
    )?;
    if how == How::Spawn {
        // Read once, by the spawn just made: a restarted or upgraded keeper takes the child over
        // and never spawns again, so the environment has no reason to stay on disk.
        let mut written = spec.clone();
        written.env.clear();
        write_json(&dir.join(SPEC), &written)?;
        write_json(
            &dir.join(SIZE),
            &Size {
                cols: spec.cols,
                rows: spec.rows,
                holder: "spawn".into(),
            },
        )?;
        notify("FDSTORE=1\nFDNAME=pty", &[master.as_fd()])?;
    }

    let mut log = Appender::open(&dir, spec.log_cap.unwrap_or(DEFAULT_CAP))?;
    let ctl_path = dir.join(CTL);
    let _ = std::fs::remove_file(&ctl_path);
    let listener = UnixListener::bind(&ctl_path)?;
    listener.set_nonblocking(true)?;
    let tty = rustix::pty::ptsname(&master, Vec::new())
        .ok()
        .and_then(|n| n.into_string().ok());
    let mut watched = Child::watch(child);
    let mut outcome = if watched.is_none() {
        Outcome::Lost
    } else {
        Outcome::Running
    };

    notify("READY=1", &[])?;

    let mut buf = vec![0u8; 64 * 1024];
    let mut ended: Option<Instant> = None;
    loop {
        let left = ended.map(|at| DRAIN_AFTER_EXIT.saturating_sub(at.elapsed()));
        if left.is_some_and(|left| left.is_zero()) {
            break;
        }
        let tick = sys::TICK.filter(|_| watched.is_some());
        let wake = match (left, tick) {
            (Some(left), Some(tick)) => Some(left.min(tick)),
            (left, tick) => left.or(tick),
        };
        let timeout = wake.and_then(|wake| Timespec::try_from(wake).ok());
        let (master_ready, ctl_ready, signal_ready, child_ready) = {
            let signal_fd = signals.fd();
            let mut fds = vec![
                PollFd::new(&master, PollFlags::IN),
                PollFd::new(&listener, PollFlags::IN),
                PollFd::new(&signal_fd, PollFlags::IN),
            ];
            let child_fd = watched.as_ref().and_then(Child::fd);
            if let Some(fd) = &child_fd {
                fds.push(PollFd::new(fd, PollFlags::IN));
            }
            match poll(&mut fds, timeout.as_ref()) {
                Ok(_) => {}
                Err(Errno::INTR) => continue,
                Err(e) => return Err(e.into()),
            }
            let ready = |i: usize| fds.get(i).is_some_and(|f| !f.revents().is_empty());
            (
                ready(0),
                ready(1),
                ready(2),
                ready(3) || (child_fd.is_none() && watched.is_some()),
            )
        };

        if child_ready && let Some(watch) = &watched {
            outcome = reap(watch);
            if !matches!(outcome, Outcome::Running) {
                watched = None;
                ended = Some(Instant::now());
            }
        }
        if signal_ready && signals.take() {
            let err = reexec(&exe, &dir, master.as_fd(), child);
            eprintln!("kampr keep: re-exec of {} failed: {err}", exe.display());
            fcntl_setfd(&master, FdFlags::CLOEXEC)?;
        }
        if ctl_ready {
            let keeper = Keeper {
                dir: &dir,
                master: master.as_fd(),
                child,
                tty: tty.as_deref(),
            };
            keeper.accept_all(&listener, &mut log, &mut buf);
        }
        if master_ready {
            match rustix::io::read(&master, &mut buf) {
                Ok(0) | Err(Errno::IO) => break,
                Ok(n) => log.write(&buf[..n])?,
                Err(Errno::INTR | Errno::AGAIN) => {}
                Err(e) => return Err(e.into()),
            }
        }
    }

    if let Some(watch) = watched.take() {
        watch.settle(REAP_GRACE);
        outcome = reap(&watch);
    }
    let (code, signal, status_lost) = match outcome {
        Outcome::Reaped { code, signal } => (code, signal, false),
        Outcome::Lost => (None, None, true),
        Outcome::Running => (None, None, false),
    };
    write_json(
        &dir.join(EXIT),
        &Exit {
            code,
            signal,
            status_lost,
            at_unix: now_unix(),
            error: None,
        },
    )?;
    notify("FDSTOREREMOVE=1\nFDNAME=pty", &[])?;
    let _ = std::fs::remove_file(&ctl_path);
    Ok(())
}

fn recover(session: &SessionDir) -> Result<Option<Started>> {
    if let Some(handover) = std::env::var_os(HANDOVER) {
        let (master, child) = parse_handover(&handover.to_string_lossy())?;
        return Ok(Some(Started {
            master: unsafe { OwnedFd::from_raw_fd(master) },
            child,
            how: How::ReExec,
        }));
    }
    let listening = std::env::var("LISTEN_PID")
        .ok()
        .and_then(|p| p.parse::<u32>().ok())
        == Some(getpid().as_raw_nonzero().get() as u32)
        && std::env::var("LISTEN_FDS")
            .ok()
            .and_then(|n| n.parse::<u32>().ok())
            .is_some_and(|n| n >= 1);
    let previous = session.keeper();
    match (listening, previous) {
        (true, Some(previous)) => Ok(Some(Started {
            master: unsafe { OwnedFd::from_raw_fd(LISTEN_FDS_START) },
            child: previous.child,
            how: How::FdStore,
        })),
        (true, None) => bail!("systemd handed over a master but keeper.json names no child"),
        (false, Some(_)) => {
            bail!("this session already had a keeper and nothing handed its master over")
        }
        (false, None) => Ok(None),
    }
}

fn parse_handover(value: &str) -> Result<(RawFd, u32)> {
    let mut parts = value.split(':');
    match (parts.next(), parts.next(), parts.next(), parts.next()) {
        (Some("v1"), Some(fd), Some(child), None) => Ok((fd.parse()?, child.parse()?)),
        _ => Err(anyhow!("unrecognised {HANDOVER}={value}")),
    }
}

fn spawn_child(spec: &Spec) -> Result<Started> {
    use rustix::pty::{OpenptFlags, grantpt, openpt, ptsname, unlockpt};
    let program = spec.argv.first().context("the session has no command")?;
    let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    fcntl_setfd(&master, FdFlags::CLOEXEC)?;
    grantpt(&master)?;
    unlockpt(&master)?;
    let name = ptsname(&master, Vec::new())?;
    let slave: OwnedFd = rustix::fs::open(
        name.as_c_str(),
        rustix::fs::OFlags::RDWR | rustix::fs::OFlags::NOCTTY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?;
    crate::ctl::set_winsize(master.as_fd(), spec.cols, spec.rows)?;

    let mut command = Command::new(program);
    command
        .args(&spec.argv[1..])
        .env_clear()
        .envs(child_env(spec))
        .stdin(Stdio::from(slave.try_clone()?))
        .stdout(Stdio::from(slave.try_clone()?))
        .stderr(Stdio::from(slave.try_clone()?));
    if let Some(cwd) = &spec.cwd {
        command.current_dir(cwd);
    }
    let slave_raw = slave.as_raw_fd();
    unsafe {
        command.pre_exec(move || {
            sys::child_pre_exec();
            if libc::setsid() < 0 || libc::ioctl(slave_raw, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = command.spawn().with_context(|| format!("starting `{program}`"))?;
    drop(slave);
    let pid = child.id();
    drop(child);
    Ok(Started {
        master,
        child: pid,
        how: How::Spawn,
    })
}

fn child_env(spec: &Spec) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = std::env::vars_os().collect();
    for (k, v) in &spec.env {
        env.retain(|(have, _)| have != k.as_str());
        env.push((k.into(), v.into()));
    }
    env.retain(|(k, _)| {
        let k = k.to_string_lossy();
        !k.starts_with("HERDR_") && !NOT_FOR_THE_CHILD.contains(&k.as_ref())
    });
    for (k, v) in [
        ("KAMPR_SESSION_ID", spec.id.as_str()),
        ("TERM", "xterm-256color"),
        ("COLORTERM", "truecolor"),
    ] {
        env.retain(|(have, _)| have != k);
        env.push((k.into(), v.into()));
    }
    env
}

fn reap(child: &Child) -> Outcome {
    match waitid(child.id(), WaitIdOptions::EXITED | WaitIdOptions::NOHANG) {
        Ok(Some(status)) => Outcome::Reaped {
            code: status.exit_status(),
            signal: status.terminating_signal(),
        },
        Ok(None) => Outcome::Running,
        Err(_) => Outcome::Lost,
    }
}

const DRAIN_MAX: usize = 1 << 20;

struct Keeper<'a> {
    dir: &'a Path,
    master: BorrowedFd<'a>,
    child: u32,
    tty: Option<&'a str>,
}

impl Keeper<'_> {
    fn accept_all(&self, listener: &UnixListener, log: &mut Appender, buf: &mut [u8]) {
        loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    if let Err(e) = self.answer(stream, log, buf) {
                        eprintln!("kampr keep: ctl request: {e}");
                    }
                }
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                Err(e) => {
                    eprintln!("kampr keep: accept: {e}");
                    return;
                }
            }
        }
    }

    fn answer(&self, stream: UnixStream, log: &mut Appender, buf: &mut [u8]) -> io::Result<()> {
        stream.set_nonblocking(false)?;
        stream.set_read_timeout(Some(CTL_TIMEOUT))?;
        stream.set_write_timeout(Some(CTL_TIMEOUT))?;
        if sys::peer_uid(&stream)? != getuid().as_raw() {
            return Ok(());
        }
        let mut line = String::new();
        BufReader::new((&stream).take(64 + HOLDER_MAX as u64)).read_line(&mut line)?;
        let line = line.trim_end();
        match line.split_once(' ').unwrap_or((line, "")) {
            ("fd", "") => send_fd(&stream, self.master),
            ("state", "") => {
                let tty = self.tty.map(str::to_string);
                let mut reply =
                    serde_json::to_vec(&procs::read(self.child, tty)).map_err(io::Error::other)?;
                reply.push(b'\n');
                (&stream).write_all(&reply)
            }
            ("resize", args) => {
                let reply = match self.resize(args, log, buf) {
                    Ok(()) => "ok\n".to_string(),
                    Err(e) => format!("error {}\n", e.to_string().replace('\n', " ")),
                };
                (&stream).write_all(reply.as_bytes())
            }
            _ => Ok(()),
        }
    }

    // Draining first puts every byte the program wrote before the resize ahead of its record;
    // nothing else reads the master, so nothing else can order the two.
    fn resize(&self, args: &str, log: &mut Appender, buf: &mut [u8]) -> io::Result<()> {
        let mut parts = args.splitn(3, ' ');
        let parsed = (
            parts.next().and_then(|c| c.parse::<u16>().ok()),
            parts.next().and_then(|r| r.parse::<u16>().ok()),
            parts.next().filter(|h| !h.is_empty()),
        );
        let (Some(cols), Some(rows), Some(holder)) = parsed else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("`resize {args}` is not `resize <cols> <rows> <holder>`"),
            ));
        };
        if cols == 0 || rows == 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{cols}x{rows} is not a size"),
            ));
        }
        self.drain(log, buf)?;
        log.resize(cols, rows, holder)?;
        set_winsize(self.master, cols, rows)?;
        write_size(self.dir, cols, rows, holder)
    }

    fn drain(&self, log: &mut Appender, buf: &mut [u8]) -> io::Result<()> {
        let now = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        let mut drained = 0;
        while drained < DRAIN_MAX {
            let mut fds = [PollFd::new(&self.master, PollFlags::IN)];
            match poll(&mut fds, Some(&now)) {
                Ok(_) if !fds[0].revents().contains(PollFlags::IN) => return Ok(()),
                Ok(_) => {}
                Err(Errno::INTR) => continue,
                Err(e) => return Err(e.into()),
            }
            match rustix::io::read(self.master, &mut *buf) {
                Ok(0) | Err(Errno::IO) => return Ok(()),
                Ok(n) => {
                    log.write(&buf[..n])?;
                    drained += n;
                }
                Err(Errno::INTR | Errno::AGAIN) => {}
                Err(e) => return Err(e.into()),
            }
        }
        Ok(())
    }
}

fn reexec(exe: &Path, dir: &Path, master: BorrowedFd<'_>, child: u32) -> io::Error {
    let flags = match fcntl_getfd(master) {
        Ok(flags) => flags,
        Err(e) => return e.into(),
    };
    if let Err(e) = fcntl_setfd(master, flags - FdFlags::CLOEXEC) {
        return e.into();
    }
    let cstr = |b: &[u8]| CString::new(b).map_err(io::Error::other);
    let build = || -> io::Result<(CString, Vec<CString>, Vec<CString>)> {
        let path = cstr(exe.as_os_str().as_bytes())?;
        let argv = vec![path.clone(), cstr(b"keep")?, cstr(dir.as_os_str().as_bytes())?];
        let mut envp = Vec::new();
        for (k, v) in std::env::vars_os() {
            if [HANDOVER, "LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"]
                .iter()
                .any(|drop| k == *drop)
            {
                continue;
            }
            let mut pair = k.as_bytes().to_vec();
            pair.push(b'=');
            pair.extend_from_slice(v.as_bytes());
            envp.push(cstr(&pair)?);
        }
        envp.push(cstr(
            format!("{HANDOVER}=v1:{}:{child}", master.as_raw_fd()).as_bytes(),
        )?);
        Ok((path, argv, envp))
    };
    let (path, argv, envp) = match build() {
        Ok(built) => built,
        Err(e) => return e,
    };
    let mut argv_ptrs: Vec<*const libc::c_char> = argv.iter().map(|a| a.as_ptr()).collect();
    argv_ptrs.push(std::ptr::null());
    let mut envp_ptrs: Vec<*const libc::c_char> = envp.iter().map(|e| e.as_ptr()).collect();
    envp_ptrs.push(std::ptr::null());
    // execve, not Command::exec: std resets the signal mask before exec, which would leave a
    // second SIGUSR1 able to kill the new image before it blocks the signal again.
    unsafe { libc::execve(path.as_ptr(), argv_ptrs.as_ptr(), envp_ptrs.as_ptr()) };
    io::Error::last_os_error()
}
