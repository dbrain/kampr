mod common;

use common::{COUNTER, Live, contains, count, proc_alive, root, sh, until};
use kampr_keep::{
    Host, How, Launch, Replay, SEGMENT, Size, SizedReader, borrow, claim_size, close, list, register_host,
    segments, state,
};
use std::time::Duration;

const SOON: Duration = Duration::from_secs(10);

#[test]
fn exit_json_records_how_the_child_ended() {
    let root = root();
    for (script, code, signal) in [
        ("exit 0", Some(0), None),
        ("exit 3", Some(3), None),
        ("kill -TERM $$", None, Some(15)),
    ] {
        let live = Live::spawn(root.path(), sh(script), Launch::Detached);
        let exit = live.exit(SOON);
        assert_eq!((exit.code, exit.signal), (code, signal), "{script}");
        assert!(!exit.status_lost, "{script}");
        assert!(!live.session.path().join("ctl.sock").exists(), "{script}");
        assert!(!live.session.alive(), "{script}");
    }
}

#[test]
fn the_log_keeps_every_byte_at_its_absolute_offset_across_rollover_and_the_cap() {
    let root = root();
    let mut spec = sh("stty -opost; seq 1 700000");
    spec.log_cap = Some(2 * SEGMENT);
    let live = Live::spawn(root.path(), spec, Launch::Detached);
    assert_eq!(live.exit(SOON).code, Some(0));

    let expected: Vec<u8> = (1..=700000).flat_map(|n| format!("{n}\n").into_bytes()).collect();
    let dir = live.session.path();
    let offsets = segments(dir).expect("segments");
    assert!(offsets.iter().all(|o| o % SEGMENT == 0), "{offsets:?}");
    assert!(offsets.len() <= 3, "the cap kept {offsets:?}");

    let reader = SizedReader::at(dir, 0).expect("reader");
    assert_eq!(reader.end().expect("end"), expected.len() as u64);
    let oldest = reader.start().expect("start");
    assert_eq!(oldest, offsets[0]);
    assert!(oldest > 0, "nothing was deleted past the cap");
    assert!(expected.len() as u64 - oldest <= 2 * SEGMENT);

    let (start, bytes) = live.log();
    assert_eq!(start, oldest, "a reader at 0 is told where the log now starts");
    assert_eq!(bytes, expected[oldest as usize..]);

    for at in [
        oldest - 1,
        oldest,
        oldest + SEGMENT - 3,
        expected.len() as u64 - 1,
    ] {
        let mut reader = SizedReader::at(dir, at).expect("reader");
        let mut buf = vec![0u8; 8];
        let mut got = Vec::new();
        let mut truncated = None;
        while got.len() < 6 {
            match reader.read(&mut buf).expect("read") {
                Replay::Data(n) => got.extend_from_slice(&buf[..n]),
                Replay::Truncated { oldest, .. } => truncated = Some(oldest),
                Replay::Resize(_) => {}
                Replay::CaughtUp => break,
            }
        }
        let from = truncated.unwrap_or(at) as usize;
        assert_eq!(truncated.is_some(), at < oldest, "at {at}");
        let want = &expected[from..(from + 6).min(expected.len())];
        assert_eq!(&got[..want.len()], want, "at {at}");
    }
}

#[test]
fn a_borrowed_master_types_into_the_child_and_its_echo_lands_in_the_log() {
    let root = root();
    let live = Live::spawn(root.path(), sh("exec cat"), Launch::Detached);
    let master = borrow(&live.session).expect("borrow");
    rustix::io::write(&master, b"kampr-hello\n").expect("write");
    let log = live.wait_for_log("kampr-hello\r\nkampr-hello\r\n", SOON);
    assert!(contains(&log, b"kampr-hello"));
}

#[test]
fn a_resize_reaches_the_child_and_names_its_holder() {
    let root = root();
    let mut spec = sh("stty size; while read l; do stty size; done");
    spec.cols = 90;
    spec.rows = 30;
    let live = Live::spawn(root.path(), spec, Launch::Detached);
    live.wait_for_log("30 90", SOON);
    assert_eq!(
        live.session.size(),
        Some(Size {
            cols: 90,
            rows: 30,
            holder: "spawn".into()
        })
    );

    let master = borrow(&live.session).expect("borrow");
    claim_size(&live.session, 100, 40, "phone:test").expect("claim");
    rustix::io::write(&master, b"\n").expect("write");
    live.wait_for_log("40 100", SOON);
    assert_eq!(live.session.size().expect("size").holder, "phone:test");
}

#[test]
fn close_hangs_the_session_up() {
    let root = root();
    let live = Live::spawn(root.path(), sh("exec cat"), Launch::Detached);
    assert!(live.session.alive());
    close(&live.session).expect("close");
    let exit = live.exit(SOON);
    assert_eq!((exit.code, exit.signal), (None, Some(libc::SIGHUP)));
    assert!(!proc_alive(live.child()));
    close(&live.session).expect("closing an ended session is not an error");
}

#[test]
fn the_child_gets_its_session_id_and_none_of_herdrs_variables() {
    let root = root();
    let mut spec = sh("stty -opost; env; echo END");
    spec.env.push(("HERDR_PANE_ID".into(), "w1:p1".into()));
    spec.env.push(("KAMPR_TEST_KEPT".into(), "yes".into()));
    spec.env.push(("TERM".into(), "dumb".into()));
    let id = spec.id.clone();
    let live = Live::spawn(root.path(), spec, Launch::Detached);
    live.exit(SOON);
    let env = String::from_utf8(live.log().1).expect("utf8");
    let lines: Vec<&str> = env.lines().collect();
    for want in [
        format!("KAMPR_SESSION_ID={id}"),
        "TERM=xterm-256color".into(),
        "COLORTERM=truecolor".into(),
        "KAMPR_TEST_KEPT=yes".into(),
    ] {
        assert!(lines.contains(&want.as_str()), "{want} missing from {env}");
    }
    for gone in ["HERDR_", "KAMPR_KEEP_HANDOVER", "NOTIFY_SOCKET", "TERM=dumb"] {
        assert!(!lines.iter().any(|l| l.starts_with(gone)), "{gone} leaked: {env}");
    }
}

#[test]
fn usr1_re_execs_the_keeper_with_the_same_child_and_no_lost_bytes() {
    let root = root();
    let mut spec = sh(COUNTER);
    spec.log_cap = Some(1 << 30);
    spec.cwd = Some(root.path().join(&spec.id).to_string_lossy().into_owned());
    let live = Live::spawn(root.path(), spec, Launch::Detached);
    let (keeper, child) = (live.keeper_pid(), live.child());
    live.wait_for_log("SEQ 1000\n", SOON);

    for round in 1..=3 {
        let before = live.log().1.len();
        unsafe { libc::kill(keeper as i32, libc::SIGUSR1) };
        assert!(
            until(SOON, || live.log().1.len() > before + 100_000),
            "round {round}: output stopped"
        );
        let info = live.session.keeper().expect("keeper.json");
        assert_eq!((info.how, info.keeper, info.child), (How::ReExec, keeper, child));
    }
    borrow(&live.session).expect("the re-exec'd keeper answers ctl.sock");
    assert!(!live.session.path().join("hup").exists(), "the child was hung up");

    close(&live.session).expect("close");
    live.exit(SOON);
    let counted = count(&live.log().1);
    assert_eq!(
        counted.gaps, 0,
        "{} lines, last {:?}",
        counted.lines, counted.last
    );
    assert!(counted.lines > 30_000, "{} lines", counted.lines);
}

#[test]
fn state_reads_the_childs_proc_files_as_its_ancestor() {
    let root = root();
    let live = Live::spawn(root.path(), sh("exec cat"), Launch::Detached);
    let child = live.child();
    let mut got = None;
    assert!(
        until(SOON, || {
            got = state(&live.session).ok();
            got.as_ref()
                .and_then(|s| s.procs.first())
                .and_then(|p| p.syscall.as_deref())
                .is_some_and(|s| s.split(' ').nth(1) == Some("0x0"))
        }),
        "{got:?}"
    );
    let got = got.expect("state");
    let me = &got.procs[0];
    assert_eq!(me.pid, child);
    assert!(me.stat.as_deref().is_some_and(|s| s.contains("(cat)")));
    assert_eq!(me.fd0, got.tty);
    assert!(got.tty.as_deref().is_some_and(|t| t.starts_with("/dev/pts/")));
}

#[test]
fn hosts_are_listed_while_registered_and_a_dead_ones_file_is_ignored() {
    let root = root();
    let live = Live::spawn(root.path(), sh("exec cat"), Launch::Detached);
    let host = |pid| Host {
        pid,
        herdr_pane: Some("w1:p2".into()),
        herdr_socket: None,
        cols: 80,
        rows: 24,
        answers_from: Some(0),
    };
    let mut guard = register_host(&live.session, host(std::process::id())).expect("register");
    let _stale = register_host(&live.session, host(i32::MAX as u32)).expect("register stale");
    assert_eq!(live.session.hosts(), vec![host(std::process::id())]);
    guard.update(120, 50).expect("update");
    assert_eq!(live.session.hosts()[0].cols, 120);
    let path = guard.path().to_path_buf();
    drop(guard);
    assert!(!path.exists());
    assert!(live.session.hosts().is_empty());
}

/// A host answers the queries that end past the log's end at the moment it registered and none
/// before, so a question the program asked before anybody attached stays the node's (docs/17 §4).
#[test]
fn a_host_takes_the_questions_asked_after_it_registered_and_not_before() {
    let root = root();
    let live = Live::spawn(root.path(), sh("printf abc; exec cat"), Launch::Detached);
    assert!(until(SOON, || live.log().1 == b"abc"));
    let guard = register_host(
        &live.session,
        Host {
            pid: std::process::id(),
            herdr_pane: None,
            herdr_socket: None,
            cols: 80,
            rows: 24,
            answers_from: None,
        },
    )
    .expect("register");
    assert_eq!(guard.host().answers_from, Some(3));
    let settled = live.session.hosts_settled().expect("hosts");
    assert_eq!(settled, vec![guard.host().clone()]);
    assert!(!settled[0].covers(3), "a query that ended before it registered");
    assert!(settled[0].covers(4));
    let legacy = Host {
        answers_from: None,
        ..settled[0].clone()
    };
    assert!(
        legacy.covers(0),
        "an attach older than the field answers everything"
    );
}

#[test]
fn a_reader_waits_for_output_and_times_out_when_there_is_none() {
    let root = root();
    let live = Live::spawn(root.path(), sh("exec cat"), Launch::Detached);
    let mut reader = SizedReader::at(live.session.path(), 0).expect("reader");
    let t = std::time::Instant::now();
    assert!(!reader.wait(Duration::from_millis(300)).expect("wait"));
    assert!(t.elapsed() >= Duration::from_millis(250));

    let master = borrow(&live.session).expect("borrow");
    let writer = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        rustix::io::write(&master, b"x\n").expect("write");
    });
    let t = std::time::Instant::now();
    assert!(reader.wait(SOON).expect("wait"));
    assert!(t.elapsed() < Duration::from_secs(5));
    writer.join().expect("writer");
}

#[test]
fn list_finds_every_session_under_the_root() {
    let root = root();
    let a = Live::spawn(root.path(), sh("exec cat"), Launch::Detached);
    let b = Live::spawn(root.path(), sh("exit 0"), Launch::Detached);
    b.exit(SOON);
    let found = list(root.path()).expect("list");
    let mut want = vec![a.session.id().to_string(), b.session.id().to_string()];
    want.sort();
    assert_eq!(found.iter().map(|s| s.id().to_string()).collect::<Vec<_>>(), want);
    let alive = |id: &str| found.iter().find(|s| s.id() == id).map(|s| s.alive());
    assert_eq!(alive(a.session.id()), Some(true));
    assert_eq!(alive(b.session.id()), Some(false));
}

#[test]
fn a_command_that_cannot_start_fails_the_spawn_and_leaves_no_directory() {
    let root = root();
    let mut spec = sh("");
    spec.argv = vec!["/nonexistent/kampr-keep-test".into()];
    let path = root.path().join(&spec.id);
    let err = kampr_keep::spawn(
        root.path(),
        spec,
        Launch::Detached,
        std::path::Path::new(common::EXE),
    )
    .expect_err("spawn of a missing command");
    assert!(format!("{err:#}").contains("nonexistent"), "{err:#}");
    assert!(!path.exists());
}

#[test]
fn a_root_another_uid_could_replace_is_refused() {
    use std::os::unix::fs::PermissionsExt;
    let base = root();
    let open = base.path().join("open");
    std::fs::create_dir(&open).expect("open dir");
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o777)).expect("chmod");
    let inside = open.join("s");
    let spawned = kampr_keep::spawn(
        &inside,
        sh("exec cat"),
        Launch::Detached,
        std::path::Path::new(common::EXE),
    );
    assert!(spawned.is_err(), "spawned under a world-writable parent");
    assert!(list(&inside).is_err(), "listed under a world-writable parent");

    let sticky = base.path().join("sticky");
    std::fs::create_dir(&sticky).expect("sticky dir");
    std::fs::set_permissions(&sticky, std::fs::Permissions::from_mode(0o1777)).expect("chmod");
    let live = Live::spawn(&sticky.join("s"), sh("exec cat"), Launch::Detached);
    assert!(live.session.alive());

    let link = base.path().join("link");
    std::os::unix::fs::symlink(sticky.join("s"), &link).expect("symlink");
    assert!(list(&link).is_err(), "followed a symlinked root");
}
