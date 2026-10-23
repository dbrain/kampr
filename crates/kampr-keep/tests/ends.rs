mod common;

use common::{Live, root, sh};
use kampr_keep::Launch;
use std::time::{Duration, Instant};

/// A terminal closes when its program exits, not when the last process holding the slave lets
/// go: a fleet run's `sleep 30 &` that ignores the hangup must not keep the session alive.
#[test]
fn a_session_ends_with_its_program_even_when_a_leftover_child_holds_the_terminal() {
    let root = root();
    let started = Instant::now();
    let live = Live::spawn(
        root.path(),
        sh("trap '' HUP; sleep 30 & echo left-behind $!; exit 3"),
        Launch::Detached,
    );
    let exit = live.exit(Duration::from_secs(10));
    let waited = started.elapsed();
    let log = String::from_utf8_lossy(&live.log().1).into_owned();
    let leftover = log
        .split_whitespace()
        .skip_while(|w| *w != "left-behind")
        .nth(1)
        .and_then(|p| p.parse::<i32>().ok());
    if let Some(pid) = leftover {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert_eq!(exit.code, Some(3), "{exit:?}");
    assert!(
        waited < Duration::from_secs(8),
        "the session took {waited:?} to end"
    );
    assert!(leftover.is_some(), "{log:?}");
}
