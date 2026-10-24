//! What this machine says about one of its own processes, the same questions answered from
//! `/proc` on Linux and from libproc and `sysctl` on macOS. Every answer can be `None`: a process
//! that has gone, one the kernel will not describe to us, and a platform with neither look alike
//! to a caller, and each of them must degrade to what a node that never looked would do.

#[cfg(target_os = "macos")]
mod darwin;
mod procfs;
mod time;

pub use procfs::Procfs;

use std::path::PathBuf;
use std::time::SystemTime;

/// When a process started, as the harnesses that record it write it down and as an instant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Start {
    /// What Claude Code records as `procStart`: field 22 of `/proc/<pid>/stat` on Linux, and
    /// `LC_ALL=C TZ=UTC ps -o lstart=` elsewhere (2.1.308's `pKe`).
    pub token: String,
    pub at: Option<SystemTime>,
}

pub trait Table {
    /// Every direct child, in pid order, which is fork order.
    fn children(&self, pid: u32) -> Vec<u32>;
    /// `None` for a process that has gone and for one that has exited unreaped: the command line
    /// is the proof of life.
    fn argv(&self, pid: u32) -> Option<Vec<String>>;
    /// The kernel's short name, which herdr labels a pane with (#75).
    fn comm(&self, pid: u32) -> Option<String>;
    fn cwd(&self, pid: u32) -> Option<PathBuf>;
    /// The foreground process group of the terminal `pid` is attached to.
    fn tpgid(&self, pid: u32) -> Option<u32>;
    /// The pseudo-terminal it is attached to, without `/dev/`: `pts/3`, `ttys003`. Read from the
    /// device number rather than `fd/0`, which a job running as root refuses to anything that did
    /// not fork it (#332).
    fn tty(&self, pid: u32) -> Option<String>;
    fn start(&self, pid: u32) -> Option<Start>;
    /// The environment as the kernel keeps it, which moves with the process (#542).
    fn environ(&self, pid: u32) -> Option<Vec<(String, String)>>;
    /// The paths of the files it holds open.
    fn open_files(&self, pid: u32) -> Option<Vec<PathBuf>>;
}

/// This machine's own table.
pub fn here() -> impl Table {
    #[cfg(target_os = "macos")]
    return darwin::Darwin;
    #[cfg(not(target_os = "macos"))]
    Procfs::default()
}

fn pairs(entries: impl IntoIterator<Item = Vec<u8>>) -> Vec<(String, String)> {
    entries
        .into_iter()
        .filter_map(|entry| {
            let text = String::from_utf8(entry).ok()?;
            let (key, value) = text.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
struct Args {
    argv: Vec<Vec<u8>>,
    env: Vec<Vec<u8>>,
}

/// `KERN_PROCARGS2`: the argument count as a native `int`, the executable's path, NUL padding,
/// that many NUL-terminated arguments, then the environment up to an empty string.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn procargs(buf: &[u8]) -> Option<Args> {
    let (count, rest) = buf.split_at_checked(size_of::<libc::c_int>())?;
    let count = usize::try_from(libc::c_int::from_ne_bytes(count.try_into().ok()?)).ok()?;
    let rest = &rest[rest.iter().position(|&b| b == 0)?..];
    let rest = &rest[rest.iter().position(|&b| b != 0)?..];
    let mut strings = rest.split(|&b| b == 0).map(<[u8]>::to_vec);
    let args: Vec<Vec<u8>> = strings.by_ref().take(count).collect();
    if args.len() != count {
        return None;
    }
    let env = strings.take_while(|s| !s.is_empty()).collect();
    Some(Args { argv: args, env })
}

#[cfg(test)]
mod tests {
    use super::procargs;

    fn laid_out(count: i32, path: &[u8], pad: usize, rest: &[u8]) -> Vec<u8> {
        let mut buf = count.to_ne_bytes().to_vec();
        buf.extend_from_slice(path);
        buf.extend(std::iter::repeat_n(0, pad));
        buf.extend_from_slice(rest);
        buf
    }

    #[test]
    fn a_macs_process_arguments_are_read_past_its_path_and_padding_and_its_environment_after_them() {
        let keeper = laid_out(
            3,
            b"/opt/k/kampr",
            4,
            b"/opt/k/kampr\0keep\0/run/s/01ja\0HOME=/x\0PI=y\0\0\0junk",
        );
        let args = procargs(&keeper).expect("laid out");
        assert_eq!(args.argv, [&b"/opt/k/kampr"[..], b"keep", b"/run/s/01ja"]);
        assert_eq!(args.env, [&b"HOME=/x"[..], b"PI=y"]);
        for short in [
            laid_out(3, b"/k", 1, b"/k\0keep"),
            laid_out(1, b"/k", 0, b""),
            vec![1, 0],
        ] {
            assert_eq!(procargs(&short), None, "{short:?}");
        }
    }
}
