use std::ffi::CString;

/// Whether the terminal's line discipline is assembling lines (ICANON), read from outside it the
/// way #333 and #667 read a pane's: `tty` as [`crate::Table::tty`] names it.
pub fn canonical(tty: &str) -> Option<bool> {
    let path = CString::new(format!("/dev/{tty}")).ok()?;
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY | libc::O_NOCTTY | libc::O_NONBLOCK) };
    if fd < 0 {
        return None;
    }
    let mut t = unsafe { std::mem::zeroed::<libc::termios>() };
    let read = unsafe { libc::tcgetattr(fd, &mut t) };
    unsafe { libc::close(fd) };
    (read == 0).then_some(t.c_lflag & libc::ICANON != 0)
}

#[cfg(test)]
mod tests {
    use super::canonical;

    #[test]
    fn a_terminal_reads_canonical_until_a_program_takes_it_raw() {
        let (mut master, mut slave) = (0, 0);
        let opened = unsafe {
            libc::openpty(
                &mut master,
                &mut slave,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
            )
        };
        assert_eq!(opened, 0);
        let name = unsafe { std::ffi::CStr::from_ptr(libc::ttyname(slave)) }
            .to_string_lossy()
            .trim_start_matches("/dev/")
            .to_string();
        assert_eq!(canonical(&name), Some(true), "{name}");
        let mut t = unsafe { std::mem::zeroed::<libc::termios>() };
        unsafe {
            libc::tcgetattr(slave, &mut t);
            libc::cfmakeraw(&mut t);
            libc::tcsetattr(slave, libc::TCSANOW, &t);
        }
        assert_eq!(canonical(&name), Some(false), "{name}");
        assert_eq!(canonical("pts/no-such"), None);
        unsafe {
            libc::close(slave);
            libc::close(master);
        }
    }
}
