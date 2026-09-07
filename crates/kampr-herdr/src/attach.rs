use crate::locate::{self, Search};
use anyhow::{Context, Result, anyhow};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use tokio::process::{Child, Command};
use tokio::task::JoinHandle;

/// A herdr **client**, attached on a pty of its own, held for as long as a viewer wants the
/// session at its size.
///
/// **This is the instrument [`crate::Controller`] cannot be.** A controller takes the PTY with no
/// way to decline (#17), overrides the desk while it is held (#18), and the size dies with the
/// hold: herdr restores the desktop's geometry within a second of a release *or* a kill (#19), so
/// nothing a controller sets can outlive it. A client's size is herdr's own — the newest client
/// sizes the session, any interaction promotes whoever made it, and when a client leaves the size
/// falls back to whoever remains rather than being taken back from it (#477, #478). Measured from
/// nothing on a throwaway session (#507): a pane at 40 rows read **52 while a client was attached
/// at 100x52, and 52 after that client exited**.
///
/// It renders, so its master has to be drained — 11 407 bytes over 16 s of an ordinary session
/// (#507) — or the child blocks on a full pty and stops answering herdr at all.
pub struct Attached {
    child: Child,
    master: OwnedFd,
    drain: JoinHandle<()>,
}

impl Attached {
    /// Attaches to `session` on a pty of `cols`x`rows`.
    ///
    /// **Every `HERDR_*` variable comes out of the environment**, and that is not tidiness: herdr
    /// refuses to run inside its own pane — `error: nested herdr is disabled by default` — and a
    /// node spawned from anywhere but a service inherits the whole set (#507). Stripping them is
    /// what makes this work from a pane, a test and a shell alike.
    pub async fn open(herdr_bin: &str, session: &str, cols: u16, rows: u16) -> Result<Self> {
        let herdr = locate::locate(herdr_bin, &Search::from_env())?;
        let (master, slave) = open_pty(cols, rows)?;
        let stdin = slave.try_clone().context("a pty for the client's input")?;
        let stderr = slave.try_clone().context("a pty for the client's errors")?;
        let mut command = Command::new(&herdr.path);
        command
            .args(["--session", session])
            .env("TERM", "xterm-256color")
            .stdin(std::process::Stdio::from(stdin))
            .stdout(std::process::Stdio::from(slave))
            .stderr(std::process::Stdio::from(stderr))
            .kill_on_drop(true);
        for (name, _) in std::env::vars_os() {
            if name.to_string_lossy().starts_with("HERDR_") {
                command.env_remove(name);
            }
        }
        // Its own session, so the pty is the child's controlling terminal and never this process's.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
        let child = command.spawn().map_err(|e| {
            anyhow!(
                "spawning `{} --session {session}` to hold a size: {e}",
                herdr.path.display()
            )
        })?;
        let drain = tokio::spawn(drain(master.try_clone().context("a pty to drain")?));
        Ok(Self { child, master, drain })
    }

    /// Moves the size the way a person dragging a window does, which is the only thing herdr reads
    /// a client's geometry from.
    pub fn resize(&self, cols: u16, rows: u16) -> Result<()> {
        set_winsize(self.master.as_raw_fd(), cols, rows)
    }

    /// Lets go. The size stays where this client left it until something else moves it — a desk
    /// keystroke, another client, the operator — which is the whole reason this exists.
    pub async fn close(mut self) {
        self.drain.abort();
        let _ = self.child.kill().await;
    }
}

fn open_pty(cols: u16, rows: u16) -> Result<(OwnedFd, OwnedFd)> {
    let mut master = 0;
    let mut slave = 0;
    let size = winsize(cols, rows);
    // SAFETY: both fds are written by `openpty` and owned from here; `size` outlives the call.
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null(),
            &size,
        )
    };
    if opened != 0 {
        return Err(anyhow!(
            "opening a pty for a herdr client: {}",
            std::io::Error::last_os_error()
        ));
    }
    // SAFETY: `openpty` succeeded, so both are fresh fds nothing else owns.
    Ok(unsafe { (OwnedFd::from_raw_fd(master), OwnedFd::from_raw_fd(slave)) })
}

fn winsize(cols: u16, rows: u16) -> libc::winsize {
    libc::winsize {
        ws_row: rows,
        ws_col: cols,
        ws_xpixel: 0,
        ws_ypixel: 0,
    }
}

fn set_winsize(fd: std::os::fd::RawFd, cols: u16, rows: u16) -> Result<()> {
    let size = winsize(cols, rows);
    // SAFETY: `fd` is a pty master this process owns and `size` outlives the call.
    if unsafe { libc::ioctl(fd, libc::TIOCSWINSZ, &size) } == -1 {
        return Err(anyhow!(
            "resizing a herdr client's pty: {}",
            std::io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Reads and throws away, for as long as the client lives. What it draws is a session Kampr is not
/// looking at — the frames come from `observe` — but a pty nobody empties fills, and a client that
/// cannot write is a client that has stopped talking to herdr.
async fn drain(master: OwnedFd) {
    let mut file = tokio::fs::File::from_std(std::fs::File::from(master));
    let mut buffer = [0u8; 8192];
    loop {
        use tokio::io::AsyncReadExt;
        match file.read(&mut buffer).await {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
    }
}
