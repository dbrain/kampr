use crate::{Start, Table, pairs};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// Linux's table, at a root a test can lay out by hand.
#[derive(Debug, Clone)]
pub struct Procfs {
    root: PathBuf,
}

impl Default for Procfs {
    fn default() -> Self {
        Self::at("/proc")
    }
}

impl Procfs {
    pub fn at(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    fn dir(&self, pid: u32) -> PathBuf {
        self.root.join(pid.to_string())
    }

    /// The fields after `comm`, read past its closing paren so a process named `a) b` cannot
    /// shift the count: index 0 is field 3.
    fn stat(&self, pid: u32) -> Option<Vec<String>> {
        let stat = std::fs::read_to_string(self.dir(pid).join("stat")).ok()?;
        let fields = &stat[stat.rfind(") ")? + 2..];
        Some(fields.split_whitespace().map(str::to_string).collect())
    }

    /// Ticks since boot subtracted from `uptime` and from now, rather than added to `btime`,
    /// which carries a whole-second floor and the wall clock's drift from `CLOCK_BOOTTIME`; and
    /// never the `/proc/<pid>` directory's mtime, which is when the dentry was last instantiated.
    fn started(&self, ticks: &str) -> Option<SystemTime> {
        let ticks = ticks.parse::<f64>().ok()?;
        let uptime = std::fs::read_to_string(self.root.join("uptime")).ok()?;
        let uptime = uptime.split_whitespace().next()?.parse::<f64>().ok()?;
        let hz = match unsafe { libc::sysconf(libc::_SC_CLK_TCK) } {
            hz if hz > 0 => hz as f64,
            _ => return None,
        };
        let age = Duration::try_from_secs_f64(uptime - ticks / hz).ok()?;
        SystemTime::now().checked_sub(age)
    }
}

impl Table for Procfs {
    /// Gathered across every thread: a job forked from a shell's non-main thread is listed under
    /// that thread and nowhere else.
    fn children(&self, pid: u32) -> Vec<u32> {
        let Ok(threads) = std::fs::read_dir(self.dir(pid).join("task")) else {
            return Vec::new();
        };
        let mut pids: Vec<u32> = threads
            .flatten()
            .filter_map(|thread| std::fs::read_to_string(thread.path().join("children")).ok())
            .flat_map(|text| {
                text.split_ascii_whitespace()
                    .filter_map(|pid| pid.parse::<u32>().ok())
                    .collect::<Vec<_>>()
            })
            .collect();
        pids.sort_unstable();
        pids.dedup();
        pids
    }

    fn argv(&self, pid: u32) -> Option<Vec<String>> {
        let raw = std::fs::read(self.dir(pid).join("cmdline")).ok()?;
        let argv: Vec<String> = raw
            .split(|byte| *byte == 0)
            .filter(|arg| !arg.is_empty())
            .map(|arg| String::from_utf8_lossy(arg).into_owned())
            .collect();
        (!argv.is_empty()).then_some(argv)
    }

    fn comm(&self, pid: u32) -> Option<String> {
        let comm = std::fs::read_to_string(self.dir(pid).join("comm")).ok()?;
        Some(comm.trim_end().to_string())
    }

    fn cwd(&self, pid: u32) -> Option<PathBuf> {
        std::fs::read_link(self.dir(pid).join("cwd")).ok()
    }

    fn tpgid(&self, pid: u32) -> Option<u32> {
        self.stat(pid)?.get(5)?.parse::<i64>().ok()?.try_into().ok()
    }

    fn tty(&self, pid: u32) -> Option<String> {
        const PTS_MAJOR: u32 = 136;
        let tty: u32 = self.stat(pid)?.get(4)?.parse().ok()?;
        let major = (tty >> 8) & 0xfff;
        let minor = (tty & 0xff) | ((tty >> 12) & 0xfff00);
        (major == PTS_MAJOR).then(|| format!("pts/{minor}"))
    }

    fn start(&self, pid: u32) -> Option<Start> {
        let token = self.stat(pid)?.get(19)?.clone();
        let at = self.started(&token);
        Some(Start { token, at })
    }

    fn environ(&self, pid: u32) -> Option<Vec<(String, String)>> {
        let raw = std::fs::read(self.dir(pid).join("environ")).ok()?;
        Some(pairs(raw.split(|b| *b == 0).map(<[u8]>::to_vec)))
    }

    fn open_files(&self, pid: u32) -> Option<Vec<PathBuf>> {
        let fds = std::fs::read_dir(self.dir(pid).join("fd")).ok()?;
        Some(
            fds.flatten()
                .filter_map(|fd| std::fs::read_link(fd.path()).ok())
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with_stat(stat: &str) -> (tempfile::TempDir, Procfs) {
        let root = tempfile::tempdir().expect("a root");
        std::fs::create_dir_all(root.path().join("7")).expect("a pid");
        std::fs::write(root.path().join("7").join("stat"), stat).expect("stat");
        let procfs = Procfs::at(root.path());
        (root, procfs)
    }

    #[test]
    fn the_terminal_and_its_foreground_group_are_read_past_a_comm_that_would_shift_the_count() {
        for (stat, tty, tpgid) in [
            (
                "7 (omp) S 1423234 1423965 1423234 34819 1423965 4194304 242628 268753 0 2 1315 166",
                Some("pts/3"),
                Some(1423965),
            ),
            ("7 (a) b) S 1 7 7 0 -1 4194304 1 2 0 0 3 4", None, None),
        ] {
            let (_root, procfs) = with_stat(stat);
            assert_eq!(procfs.tty(7).as_deref(), tty, "{stat}");
            assert_eq!(procfs.tpgid(7), tpgid, "{stat}");
        }
    }
}
