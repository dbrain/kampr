use crate::{Args, Start, Table, pairs, procargs, time};
use std::ffi::OsStr;
use std::mem::{MaybeUninit, size_of};
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;
use std::time::{Duration, Instant, SystemTime};

pub(crate) struct Darwin;

// `<sys/proc_info.h>`; libc has the path half and not the whole.
const PROC_PIDFDVNODEPATHINFO: libc::c_int = 2;

// Read only for its size: the path comes after it.
#[allow(dead_code)]
#[repr(C)]
struct ProcFileInfo {
    fi_openflags: u32,
    fi_status: u32,
    fi_offset: libc::off_t,
    fi_type: i32,
    fi_guardflags: u32,
}

#[repr(C)]
struct VnodeFdInfoWithPath {
    pfi: ProcFileInfo,
    pvip: libc::vnode_info_path,
}

/// `proc_pidinfo` answers with exactly the size asked for or with nothing usable.
fn info<T>(pid: u32, flavor: libc::c_int) -> Option<T> {
    let mut out = MaybeUninit::<T>::zeroed();
    let size = size_of::<T>() as libc::c_int;
    let got = unsafe { libc::proc_pidinfo(pid as libc::c_int, flavor, 0, out.as_mut_ptr().cast(), size) };
    (got == size).then(|| unsafe { out.assume_init() })
}

fn bsd(pid: u32) -> Option<libc::proc_bsdinfo> {
    info::<libc::proc_bsdinfo>(pid, libc::PROC_PIDTBSDINFO)
}

fn text(chars: &[libc::c_char]) -> String {
    let bytes: Vec<u8> = chars.iter().map(|&c| c as u8).take_while(|&b| b != 0).collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn path(chars: &[[libc::c_char; 32]; 32]) -> Option<PathBuf> {
    let bytes: Vec<u8> = chars
        .iter()
        .flatten()
        .map(|&c| c as u8)
        .take_while(|&b| b != 0)
        .collect();
    (!bytes.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(&bytes)))
}

fn image(pid: u32) -> Option<String> {
    let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
    let n = unsafe { libc::proc_pidpath(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    let n = usize::try_from(n).ok().filter(|&n| n > 0)?;
    let path = String::from_utf8_lossy(&buf[..n]).into_owned();
    Some(path.rsplit('/').next().unwrap_or_default().to_string())
}

// macOS's /bin/sh is a shim that execs the selected shell, and while a process execs the kernel
// answers EIO or EINVAL for its arguments, so a `#!/bin/sh` script named for a harness read for a
// moment as having none. The window is under a millisecond; a read that ought to work waits it out.
const EXEC_GRACE: Duration = Duration::from_millis(50);

/// Readable only for a process of our own uid, which every process a node watches is.
fn args(pid: u32) -> Option<Args> {
    let deadline = Instant::now() + EXEC_GRACE;
    loop {
        if let Some(args) = procargs2(pid) {
            return Some(args);
        }
        if Instant::now() >= deadline || !readable(pid) {
            return None;
        }
        std::thread::sleep(Duration::from_micros(100));
    }
}

fn readable(pid: u32) -> bool {
    let euid = unsafe { libc::geteuid() };
    bsd(pid).is_some_and(|i| i.pbi_status != libc::SZOMB && (euid == 0 || i.pbi_uid == euid))
}

fn procargs2(pid: u32) -> Option<Args> {
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    let mut sysctl = |buf: *mut libc::c_void, len: &mut libc::size_t| unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            mib.len() as _,
            buf,
            len,
            std::ptr::null_mut(),
            0,
        )
    };
    let mut len: libc::size_t = 0;
    if sysctl(std::ptr::null_mut(), &mut len) != 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    if sysctl(buf.as_mut_ptr().cast(), &mut len) != 0 {
        return None;
    }
    buf.truncate(len);
    procargs(&buf)
}

impl Table for Darwin {
    fn children(&self, pid: u32) -> Vec<u32> {
        let mut pids = vec![0 as libc::pid_t; 4096];
        let bytes = (pids.len() * size_of::<libc::pid_t>()) as libc::c_int;
        let n = unsafe { libc::proc_listchildpids(pid as libc::pid_t, pids.as_mut_ptr().cast(), bytes) };
        let Ok(n) = usize::try_from(n) else {
            return Vec::new();
        };
        let mut pids: Vec<u32> = pids[..n.min(pids.len())]
            .iter()
            .filter_map(|&p| u32::try_from(p).ok().filter(|&p| p > 0))
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    }

    fn argv(&self, pid: u32) -> Option<Vec<String>> {
        if bsd(pid)?.pbi_status == libc::SZOMB {
            return None;
        }
        let argv: Vec<String> = args(pid)?
            .argv
            .into_iter()
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8_lossy(&arg).into_owned())
            .collect();
        (!argv.is_empty()).then_some(argv)
    }

    fn comm(&self, pid: u32) -> Option<String> {
        let info = bsd(pid)?;
        let name = text(&info.pbi_name);
        Some(if name.is_empty() {
            text(&info.pbi_comm)
        } else {
            name
        })
    }

    // macOS takes `comm` from the file a symlink resolved to, so Claude's installer link
    // `~/.local/bin/claude -> versions/2.1.308` runs as `2.1.308`. argv[0] is the link's own name
    // when the image really is `comm`'s file and argv[0] was not rewritten into a title.
    fn name(&self, pid: u32) -> Option<String> {
        let comm = self.comm(pid)?;
        let typed = args(pid)
            .and_then(|a| a.argv.into_iter().next())
            .map(|a| String::from_utf8_lossy(&a).into_owned())
            .unwrap_or_default();
        let typed = typed.rsplit('/').next().unwrap_or_default();
        let linked = !typed.is_empty()
            && typed != comm
            && !typed.starts_with('-')
            && !typed.contains([' ', ':'])
            && image(pid).is_some_and(|image| image.starts_with(&comm));
        Some(if linked { typed.to_string() } else { comm })
    }

    fn cwd(&self, pid: u32) -> Option<PathBuf> {
        path(
            &info::<libc::proc_vnodepathinfo>(pid, libc::PROC_PIDVNODEPATHINFO)?
                .pvi_cdir
                .vip_path,
        )
    }

    fn tpgid(&self, pid: u32) -> Option<u32> {
        Some(bsd(pid)?.e_tpgid).filter(|&g| g > 0)
    }

    fn tty(&self, pid: u32) -> Option<String> {
        let dev = bsd(pid)?.e_tdev;
        if dev == u32::MAX || dev == 0 {
            return None;
        }
        let name = unsafe { libc::devname(dev as libc::dev_t, libc::S_IFCHR) };
        if name.is_null() {
            return None;
        }
        let name = unsafe { std::ffi::CStr::from_ptr(name) }
            .to_string_lossy()
            .into_owned();
        name.starts_with("ttys").then_some(name)
    }

    fn start(&self, pid: u32) -> Option<Start> {
        let info = bsd(pid)?;
        let secs = i64::try_from(info.pbi_start_tvsec).ok()?;
        Some(Start {
            token: time::lstart(secs),
            at: SystemTime::UNIX_EPOCH.checked_add(
                Duration::from_secs(info.pbi_start_tvsec) + Duration::from_micros(info.pbi_start_tvusec),
            ),
        })
    }

    fn environ(&self, pid: u32) -> Option<Vec<(String, String)>> {
        Some(pairs(args(pid)?.env))
    }

    fn open_files(&self, pid: u32) -> Option<Vec<PathBuf>> {
        let count = bsd(pid)?.pbi_nfiles as usize + 16;
        let mut fds: Vec<libc::proc_fdinfo> = Vec::with_capacity(count);
        let bytes = (count * size_of::<libc::proc_fdinfo>()) as libc::c_int;
        let got = unsafe {
            libc::proc_pidinfo(
                pid as libc::c_int,
                libc::PROC_PIDLISTFDS,
                0,
                fds.as_mut_ptr().cast(),
                bytes,
            )
        };
        let listed = usize::try_from(got).ok()? / size_of::<libc::proc_fdinfo>();
        unsafe { fds.set_len(listed.min(count)) };
        Some(
            fds.iter()
                .filter(|fd| fd.proc_fdtype == libc::PROX_FDTYPE_VNODE as u32)
                .filter_map(|fd| {
                    let mut out = MaybeUninit::<VnodeFdInfoWithPath>::zeroed();
                    let size = size_of::<VnodeFdInfoWithPath>() as libc::c_int;
                    let got = unsafe {
                        libc::proc_pidfdinfo(
                            pid as libc::c_int,
                            fd.proc_fd,
                            PROC_PIDFDVNODEPATHINFO,
                            out.as_mut_ptr().cast(),
                            size,
                        )
                    };
                    (got == size).then(|| unsafe { out.assume_init() })
                })
                .filter_map(|info| path(&info.pvip.vip_path))
                .collect(),
        )
    }
}
