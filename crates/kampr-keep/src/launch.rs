use crate::dir::{CTL, SPEC, SessionDir, Spec, make_private_dir, write_json};
use anyhow::{Context, Result, bail};
use std::fs::{File, OpenOptions};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const UNIT_PREFIX: &str = "kampr-keep-";
const READY: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Launch {
    Systemd,
    Detached,
}

impl Launch {
    pub fn auto() -> Self {
        match std::env::var("KAMPR_KEEP_LAUNCH").as_deref() {
            Ok("detached") => return Self::Detached,
            Ok("systemd") => return Self::Systemd,
            _ => {}
        }
        let reachable = Command::new("systemctl")
            .args(["--user", "show-environment"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|s| s.success());
        if reachable { Self::Systemd } else { Self::Detached }
    }
}

pub fn unit_name(id: &str) -> String {
    format!("{UNIT_PREFIX}{id}")
}

pub fn spawn(root: &Path, spec: Spec, launch: Launch, exe: &Path) -> Result<SessionDir> {
    let path = root.join(&spec.id);
    make_private_dir(root)?;
    std::fs::create_dir(&path).with_context(|| format!("creating {}", path.display()))?;
    let session = SessionDir::at(&path);
    match start(&session, &spec, launch, exe) {
        Ok(()) => Ok(session),
        Err(e) => {
            let _ = std::fs::remove_dir_all(&path);
            Err(e)
        }
    }
}

fn start(session: &SessionDir, spec: &Spec, launch: Launch, exe: &Path) -> Result<()> {
    std::fs::set_permissions(
        session.path(),
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    )?;
    write_json(&session.path().join(SPEC), spec)?;
    match launch {
        Launch::Systemd => systemd(session, &spec.id, exe)?,
        Launch::Detached => detached(session, exe)?,
    }
    let deadline = Instant::now() + READY;
    loop {
        if let Some(exit) = session.exit() {
            return match exit.error {
                Some(error) => bail!("{error}"),
                None => Ok(()),
            };
        }
        if UnixStream::connect(session.path().join(CTL)).is_ok() {
            return Ok(());
        }
        if Instant::now() >= deadline {
            bail!("the keeper did not answer within {READY:?}");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn systemd(session: &SessionDir, id: &str, exe: &Path) -> Result<()> {
    let output = Command::new("systemd-run")
        .args(["--user", "--quiet", "--unit", &unit_name(id)])
        .args([
            "-p",
            "Type=notify",
            "-p",
            "NotifyAccess=main",
            "-p",
            "Restart=on-failure",
            "-p",
            "RestartSec=1",
            "-p",
            "KillMode=process",
            "-p",
            "FileDescriptorStoreMax=1",
            "-p",
            "CollectMode=inactive-or-failed",
        ])
        .arg(exe)
        .arg("keep")
        .arg(session.path())
        .stdin(Stdio::null())
        .output()
        .context("running systemd-run")?;
    if !output.status.success() {
        bail!("systemd-run: {}", String::from_utf8_lossy(&output.stderr).trim());
    }
    Ok(())
}

fn detached(session: &SessionDir, exe: &Path) -> Result<()> {
    let err: File = OpenOptions::new()
        .create(true)
        .append(true)
        .open(session.path().join("keeper.err"))?;
    let mut command = Command::new(exe);
    command
        .arg("keep")
        .arg(session.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(err);
    // The second fork: the intermediate exits at once so the keeper is nobody's child and
    // outlives its creator; std's exec-error pipe still reaches us through the grandchild.
    unsafe {
        command.pre_exec(|| {
            match libc::fork() {
                -1 => return Err(std::io::Error::last_os_error()),
                0 => {}
                _ => libc::_exit(0),
            }
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let mut intermediate = command.spawn().context("starting the keeper")?;
    intermediate.wait()?;
    Ok(())
}
