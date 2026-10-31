mod common;

use common::{Live, root, sh};
use kampr_keep::{Launch, borrow};
use std::time::Duration;

const SOON: Duration = Duration::from_secs(10);

/// A keeper that cannot get a kernel watch on its child (kqueue refuses some, docs on
/// `Exit::open`) still answers at once and still records how the child ended, whether it ended
/// before the keeper looked or long after.
#[test]
fn a_child_the_keeper_cannot_watch_still_has_its_ending_recorded() {
    // Its own test binary, so the variable reaches no other test's keeper.
    unsafe { std::env::set_var(kampr_keep::UNWATCHED, "1") };
    let root = root();

    let started = std::time::Instant::now();
    let live = Live::spawn(root.path(), sh("read line; exit 7"), Launch::Detached);
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "the launch waited {:?} on a child that was still running",
        started.elapsed()
    );
    let master = borrow(&live.session).expect("borrow");
    rustix::io::write(&master, b"go\n").expect("write");
    let exit = live.exit(SOON);
    assert_eq!((exit.code, exit.signal, exit.status_lost), (Some(7), None, false));

    let quick = Live::spawn(root.path(), sh("exit 3"), Launch::Detached);
    let exit = quick.exit(SOON);
    assert_eq!((exit.code, exit.signal, exit.status_lost), (Some(3), None, false));
}
