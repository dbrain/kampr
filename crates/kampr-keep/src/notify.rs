use rustix::net::{
    AddressFamily, SendAncillaryBuffer, SendAncillaryMessage, SendFlags, SocketAddrUnix, SocketType,
    sendmsg_addr, socket,
};
use std::io::{self, IoSlice};
use std::mem::MaybeUninit;
use std::os::fd::BorrowedFd;
use std::os::unix::ffi::OsStrExt;

pub(crate) fn notify(message: &str, fds: &[BorrowedFd<'_>]) -> io::Result<()> {
    let Some(target) = std::env::var_os("NOTIFY_SOCKET") else {
        return Ok(());
    };
    let target = target.as_bytes();
    let addr = match target.strip_prefix(b"@") {
        Some(name) => abstract_name(name)?,
        None => SocketAddrUnix::new(target)?,
    };
    let sock = socket(AddressFamily::UNIX, SocketType::DGRAM, None)?;
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() && !control.push(SendAncillaryMessage::ScmRights(fds)) {
        return Err(io::Error::other("ancillary buffer too small"));
    }
    sendmsg_addr(
        &sock,
        &addr,
        &[IoSlice::new(message.as_bytes())],
        &mut control,
        SendFlags::empty(),
    )?;
    Ok(())
}

#[cfg(target_os = "linux")]
fn abstract_name(name: &[u8]) -> io::Result<SocketAddrUnix> {
    Ok(SocketAddrUnix::new_abstract_name(name)?)
}

#[cfg(not(target_os = "linux"))]
fn abstract_name(_: &[u8]) -> io::Result<SocketAddrUnix> {
    Err(io::Error::from(io::ErrorKind::Unsupported))
}
