use crate::dir::{CTL, SessionDir};
use crate::procs::ProcState;
use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer, SendAncillaryMessage,
    SendFlags, recvmsg, sendmsg,
};
use rustix::process::{Pid, Signal, kill_process_group};
use rustix::termios::{Winsize, tcsetwinsize};
use std::io::{self, BufRead, BufReader, IoSlice, IoSliceMut, Write};
use std::mem::MaybeUninit;
use std::os::fd::{BorrowedFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

const RETRY: Duration = Duration::from_secs(2);
const REPLY: Duration = Duration::from_secs(5);
pub(crate) const HOLDER_MAX: usize = 256;

fn request(session: &SessionDir, line: &str) -> io::Result<UnixStream> {
    let path = session.path().join(CTL);
    let deadline = Instant::now() + RETRY;
    let mut stream = loop {
        match UnixStream::connect(&path) {
            Ok(stream) => break stream,
            // A keeper rebinds its socket after a re-exec or a crash restart.
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
                ) && Instant::now() < deadline
                    && session.exit().is_none() =>
            {
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(e) => return Err(e),
        }
    };
    if rustix::net::sockopt::socket_peercred(&stream)?.uid != rustix::process::getuid() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{} is served by another uid", path.display()),
        ));
    }
    stream.set_read_timeout(Some(REPLY))?;
    stream.write_all(line.as_bytes())?;
    Ok(stream)
}

pub fn borrow(session: &SessionDir) -> io::Result<OwnedFd> {
    let stream = request(session, "fd\n")?;
    let mut byte = [0u8; 1];
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = RecvAncillaryBuffer::new(&mut space);
    recvmsg(
        &stream,
        &mut [IoSliceMut::new(&mut byte)],
        &mut control,
        RecvFlags::CMSG_CLOEXEC,
    )?;
    for message in control.drain() {
        if let RecvAncillaryMessage::ScmRights(mut fds) = message
            && let Some(fd) = fds.next()
        {
            return Ok(fd);
        }
    }
    Err(io::Error::other("the keeper sent no master"))
}

pub fn state(session: &SessionDir) -> io::Result<ProcState> {
    let stream = request(session, "state\n")?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    serde_json::from_str(&line).map_err(io::Error::other)
}

pub(crate) fn set_winsize(fd: BorrowedFd<'_>, cols: u16, rows: u16) -> io::Result<()> {
    tcsetwinsize(
        fd,
        Winsize {
            ws_row: rows,
            ws_col: cols,
            ws_xpixel: 0,
            ws_ypixel: 0,
        },
    )?;
    Ok(())
}

pub fn claim_size(session: &SessionDir, cols: u16, rows: u16, holder: &str) -> io::Result<()> {
    if holder.is_empty() || holder.contains(['\n', '\r']) || holder.len() > HOLDER_MAX {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{holder:?} cannot hold a size"),
        ));
    }
    let stream = request(session, &format!("resize {cols} {rows} {holder}\n"))?;
    let mut line = String::new();
    BufReader::new(stream).read_line(&mut line)?;
    match line.trim_end() {
        "ok" => Ok(()),
        "" => Err(io::Error::other("the keeper hung up on a resize")),
        reply => Err(io::Error::other(
            reply.strip_prefix("error ").unwrap_or(reply).to_string(),
        )),
    }
}

pub fn close(session: &SessionDir) -> io::Result<()> {
    if session.exit().is_some() {
        return Ok(());
    }
    let keeper = session
        .keeper()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no keeper.json"))?;
    let pid =
        Pid::from_raw(keeper.child as i32).ok_or_else(|| io::Error::other("keeper.json names no child"))?;
    match kill_process_group(pid, Signal::HUP) {
        Ok(()) | Err(rustix::io::Errno::SRCH) => Ok(()),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn send_fd(stream: &UnixStream, fd: BorrowedFd<'_>) -> io::Result<()> {
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    let fds = [fd];
    control.push(SendAncillaryMessage::ScmRights(&fds));
    sendmsg(stream, &[IoSlice::new(&[0])], &mut control, SendFlags::NOSIGNAL)?;
    Ok(())
}
