#![allow(dead_code)]

use kampr_keep::{Exit, Launch, Replay, SessionDir, SizedReader, Spec};
use std::path::Path;
use std::process::Command;
use std::time::{Duration, Instant};

pub const EXE: &str = env!("CARGO_BIN_EXE_kampr-keep");

pub const COUNTER: &str =
    r#"stty -opost; trap 'echo HUP >> hup; exit 1' HUP; i=0; while :; do echo "SEQ $i"; i=$((i+1)); done"#;

pub fn root() -> tempfile::TempDir {
    tempfile::Builder::new()
        .prefix("kk")
        .tempdir_in("/tmp")
        .expect("temp root")
}

pub fn sh(script: &str) -> Spec {
    let mut spec = Spec::new(vec!["sh".into(), "-c".into(), script.into()]);
    spec.id = format!("kt{}", kampr_keep::new_id());
    spec
}

pub struct Live {
    pub session: SessionDir,
    launch: Launch,
}

impl Live {
    pub fn spawn(root: &Path, spec: Spec, launch: Launch) -> Self {
        let session = kampr_keep::spawn(root, spec, launch, Path::new(EXE)).expect("spawn");
        Self::adopt(session, launch)
    }

    pub fn adopt(session: SessionDir, launch: Launch) -> Self {
        Self { session, launch }
    }

    pub fn child(&self) -> u32 {
        self.session.keeper().expect("keeper.json").child
    }

    pub fn keeper_pid(&self) -> u32 {
        self.session.keeper().expect("keeper.json").keeper
    }

    pub fn unit(&self) -> String {
        kampr_keep::unit_name(self.session.id())
    }

    pub fn exit(&self, within: Duration) -> Exit {
        let mut found = None;
        assert!(
            until(within, || {
                found = self.session.exit();
                found.is_some()
            }),
            "no exit.json in {}",
            self.session.path().display()
        );
        found.expect("exit")
    }

    pub fn log(&self) -> (u64, Vec<u8>) {
        read_all(self.session.path())
    }

    pub fn wait_for_log(&self, needle: &str, within: Duration) -> Vec<u8> {
        let mut log = Vec::new();
        let found = until(within, || {
            log = self.log().1;
            contains(&log, needle.as_bytes())
        });
        assert!(
            found,
            "{needle:?} never reached the log: {:?}",
            String::from_utf8_lossy(&log)
        );
        log
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Some(keeper) = self.session.keeper()
            && self.session.exit().is_none()
        {
            unsafe { libc::kill(-(keeper.child as i32), libc::SIGKILL) };
        }
        if self.launch == Launch::Systemd {
            let unit = self.unit();
            let _ = Command::new("systemctl")
                .args(["--user", "stop", &unit])
                .stderr(std::process::Stdio::null())
                .status();
            let _ = Command::new("systemctl")
                .args(["--user", "reset-failed", &unit])
                .stderr(std::process::Stdio::null())
                .status();
        }
        if let Some(keeper) = self.session.keeper() {
            unsafe { libc::kill(keeper.keeper as i32, libc::SIGKILL) };
        }
    }
}

pub fn read_all(dir: &Path) -> (u64, Vec<u8>) {
    let mut reader = SizedReader::at(dir, 0).expect("reader");
    let mut start = 0;
    let mut out = Vec::new();
    let mut buf = vec![0u8; 256 * 1024];
    loop {
        match reader.read(&mut buf).expect("read") {
            Replay::Data(n) => out.extend_from_slice(&buf[..n]),
            Replay::Truncated { oldest, .. } => {
                assert!(out.is_empty(), "truncated mid-read");
                start = oldest;
            }
            Replay::Resize(_) => {}
            Replay::CaughtUp => return (start, out),
        }
    }
}

pub fn until(within: Duration, mut f: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + within;
    loop {
        if f() {
            return true;
        }
        if Instant::now() >= deadline {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

pub fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

pub struct Count {
    pub lines: usize,
    pub gaps: usize,
    pub last: Option<u64>,
}

pub fn count(log: &[u8]) -> Count {
    let seqs: Vec<u64> = log
        .split(|&b| b == b'\n')
        .filter_map(|l| std::str::from_utf8(l).ok()?.strip_prefix("SEQ ")?.parse().ok())
        .collect();
    let gaps = seqs.windows(2).filter(|w| w[1] != w[0] + 1).count();
    Count {
        lines: seqs.len(),
        gaps,
        last: seqs.last().copied(),
    }
}

pub fn proc_alive(pid: u32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|s| s.rsplit_once(')').map(|(_, r)| !r.trim_start().starts_with('Z')))
        .unwrap_or(false)
}
