mod common;

use common::{Live, root, sh};
use kampr_keep::{Ending, Launch, SessionDir};
use std::process::{Child, Command};

struct Sleeper(Child);

impl Drop for Sleeper {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

fn sleeper() -> Sleeper {
    Sleeper(Command::new("sleep").arg("60").spawn().expect("sleep"))
}

fn start_of(pid: u32) -> u64 {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).expect("stat");
    let (_, rest) = stat.rsplit_once(')').expect("comm");
    rest.split_ascii_whitespace()
        .nth(19)
        .and_then(|t| t.parse().ok())
        .expect("starttime")
}

fn session_naming(pid: u32, starts: Option<u64>) -> (tempfile::TempDir, SessionDir) {
    let root = root();
    let dir = root.path().join("s1");
    std::fs::create_dir(&dir).expect("session dir");
    let mut info = serde_json::json!({
        "keeper": pid, "child": pid, "exe": "/nowhere", "started_unix": 0, "how": "spawn",
    });
    if let Some(start) = starts {
        info["keeper_start"] = start.into();
        info["child_start"] = start.into();
    }
    std::fs::write(dir.join("keeper.json"), info.to_string()).expect("keeper.json");
    (root, SessionDir::at(dir))
}

/// A restarted container hands its first pids out again, so a keeper.json from before the restart
/// names a live process that is somebody else.
#[test]
fn a_keeper_whose_pid_now_belongs_to_another_process_is_lost() {
    let other = sleeper();
    let pid = other.0.id();

    let (_root, reused) = session_naming(pid, Some(start_of(pid) + 1));
    assert!(!reused.alive());
    assert_eq!(reused.ending(), Some(Ending::Lost));

    let (_root, same) = session_naming(pid, Some(start_of(pid)));
    assert!(same.alive());

    let (_root, older) = session_naming(pid, None);
    assert!(
        older.alive(),
        "a keeper.json from before the field trusts the pid"
    );
}

#[test]
fn a_keeper_records_when_it_and_its_child_started() {
    let root = root();
    let live = Live::spawn(root.path(), sh("exec cat"), Launch::Detached);
    let info = live.session.keeper().expect("keeper.json");
    assert_eq!(
        (info.keeper_start, info.child_start),
        (Some(start_of(info.keeper)), Some(start_of(info.child))),
    );
    assert!(live.session.alive());
}
