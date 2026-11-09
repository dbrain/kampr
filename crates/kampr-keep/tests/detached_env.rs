#![cfg(target_os = "linux")]

mod common;

use common::{Live, root, sh};
use kampr_keep::Launch;

/// A node started by a `Type=notify` unit, or by a socket-activated one, hands its environment to
/// every keeper it launches detached. Those variables address the node's own manager, and a keeper
/// that kept them would speak to it as if it were the unit's main process.
#[test]
fn a_detached_keeper_does_not_inherit_its_launchers_service_manager_variables() {
    // Its own test binary, so the variables reach no other test's keeper.
    for (name, value) in [
        ("NOTIFY_SOCKET", "/nowhere/notify"),
        ("LISTEN_PID", "1"),
        ("LISTEN_FDS", "1"),
        ("LISTEN_FDNAMES", "pty"),
    ] {
        unsafe { std::env::set_var(name, value) };
    }
    let root = root();
    let live = Live::spawn(root.path(), sh("exec cat"), Launch::Detached);
    let environ = std::fs::read(format!("/proc/{}/environ", live.keeper_pid())).expect("environ");
    let names: Vec<String> = environ
        .split(|b| *b == 0)
        .filter_map(|entry| {
            String::from_utf8_lossy(entry)
                .split_once('=')
                .map(|(k, _)| k.to_string())
        })
        .collect();
    assert!(names.iter().any(|n| n == "PATH"), "{names:?}");
    for gone in ["NOTIFY_SOCKET", "LISTEN_PID", "LISTEN_FDS", "LISTEN_FDNAMES"] {
        assert!(
            !names.iter().any(|n| n == gone),
            "{gone} reached the keeper: {names:?}"
        );
    }
}
