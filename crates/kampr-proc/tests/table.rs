//! This machine's own table, read about processes the test starts. CI runs these on macOS too,
//! which is the only place the darwin backend is ever exercised.

use kampr_proc::{Table, here};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

struct Reaped(Child);

impl Drop for Reaped {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn until(mut ok: impl FnMut() -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if ok() {
            return true;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn a_child_is_found_with_its_arguments_directory_environment_and_start() {
    let dir = tempfile::tempdir().expect("a directory");
    let child = Reaped(
        Command::new("sleep")
            .arg("30")
            .current_dir(dir.path())
            .env("KAMPR_PROC_PROBE", "yes")
            .spawn()
            .expect("sleep"),
    );
    let pid = child.0.id();
    let table = here();
    assert!(until(|| table
        .argv(pid)
        .is_some_and(|a| a.last().is_some_and(|l| l == "30"))));
    assert!(table.children(std::process::id()).contains(&pid));
    assert_eq!(table.argv(pid).expect("argv"), ["sleep", "30"]);
    assert_eq!(table.comm(pid).as_deref(), Some("sleep"));
    assert_eq!(
        table.cwd(pid).and_then(|p| p.canonicalize().ok()),
        Some(dir.path().canonicalize().expect("canonical"))
    );
    let env = table.environ(pid).expect("environ");
    assert!(
        env.contains(&("KAMPR_PROC_PROBE".into(), "yes".into())),
        "{env:?}"
    );

    let start = table.start(pid).expect("a start");
    assert!(!start.token.is_empty());
    let age = SystemTime::now()
        .duration_since(start.at.expect("an instant"))
        .expect("started in the past");
    assert!(age < Duration::from_secs(10), "{age:?}");
}

#[test]
fn a_child_that_has_exited_unreaped_is_not_alive() {
    let child = Reaped(Command::new("true").spawn().expect("true"));
    let pid = child.0.id();
    assert!(until(|| here().argv(pid).is_none()), "{:?}", here().argv(pid));
}

#[test]
fn a_file_this_process_holds_open_is_listed() {
    let file = tempfile::NamedTempFile::new().expect("a file");
    let open = here().open_files(std::process::id()).expect("open files");
    let want = file.path().canonicalize().expect("canonical");
    assert!(
        open.iter().any(|p| p.canonicalize().ok().as_ref() == Some(&want)),
        "{want:?} in {open:?}"
    );
}

#[test]
fn a_session_leader_on_a_terminal_is_its_foreground_group() {
    let (mut master, mut slave) = (0, 0);
    let opened = unsafe {
        libc::openpty(
            &mut master,
            &mut slave,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            std::ptr::null_mut(),
        )
    };
    assert_eq!(opened, 0, "{}", std::io::Error::last_os_error());
    let mut command = Command::new("sleep");
    command.arg("30").stdin(Stdio::null()).stdout(Stdio::null());
    unsafe {
        command.pre_exec(move || {
            if libc::setsid() < 0 || libc::ioctl(slave, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = Reaped(command.spawn().expect("sleep"));
    let pid = child.0.id();
    assert!(
        until(|| here().tpgid(pid) == Some(pid)),
        "{:?}",
        here().tpgid(pid)
    );
    unsafe {
        libc::close(master);
        libc::close(slave);
    }
}

/// Claude Code compares its own `procStart` with ours byte for byte, so on a Mac ours has to be
/// what `ps` prints.
#[cfg(target_os = "macos")]
#[test]
fn a_start_reads_exactly_as_ps_prints_it_for_claude_to_compare() {
    let child = Reaped(Command::new("sleep").arg("30").spawn().expect("sleep"));
    let pid = child.0.id();
    let ps = Command::new("ps")
        .args(["-o", "lstart=", "-p", &pid.to_string()])
        .env("LC_ALL", "C")
        .env("TZ", "UTC")
        .output()
        .expect("ps");
    let ps = String::from_utf8_lossy(&ps.stdout).trim().to_string();
    assert_eq!(here().start(pid).expect("a start").token, ps);
}
