mod common;

use common::{COUNTER, Live, count, proc_alive, root, sh, until};
use kampr_keep::{How, Launch, close};
use std::collections::HashMap;
use std::process::Command;
use std::time::Duration;

const SOON: Duration = Duration::from_secs(10);

fn show(unit: &str) -> HashMap<String, String> {
    let out = Command::new("systemctl")
        .args(["--user", "show", unit])
        .args([
            "-p",
            "MainPID,NRestarts,NFileDescriptorStore,ActiveState,LoadState",
        ])
        .output()
        .expect("systemctl show");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| l.split_once('='))
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

fn signal_unit(unit: &str, signal: &str) {
    let status = Command::new("systemctl")
        .args(["--user", "kill", "-s", signal, "--kill-whom=main", unit])
        .status()
        .expect("systemctl kill");
    assert!(status.success(), "systemctl kill -s {signal} {unit}");
}

fn gone(unit: &str) -> bool {
    let props = show(unit);
    props.get("LoadState").map(String::as_str) == Some("not-found")
        || props.get("ActiveState").map(String::as_str) == Some("inactive")
}

#[test]
fn a_killed_keeper_comes_back_with_the_same_master_and_says_the_status_is_lost() {
    let root = root();
    let mut spec = sh(COUNTER);
    spec.log_cap = Some(1 << 30);
    spec.cwd = Some(root.path().join(&spec.id).to_string_lossy().into_owned());
    let live = Live::spawn(root.path(), spec, Launch::Systemd);
    let unit = live.unit();
    let (first, child) = (live.keeper_pid(), live.child());
    let props = show(&unit);
    assert_eq!(props["MainPID"], first.to_string(), "{props:?}");
    assert_eq!(props["NFileDescriptorStore"], "1", "{props:?}");
    live.wait_for_log("SEQ 1000\n", SOON);

    let before = live.log().1.len();
    signal_unit(&unit, "USR1");
    assert!(until(SOON, || live.log().1.len() > before + 100_000));
    let info = live.session.keeper().expect("keeper.json");
    assert_eq!((info.how, info.keeper, info.child), (How::ReExec, first, child));
    assert_eq!(show(&unit)["MainPID"], first.to_string());

    signal_unit(&unit, "KILL");
    assert!(
        until(SOON, || live
            .session
            .keeper()
            .is_some_and(|k| k.how == How::FdStore && k.keeper != first)),
        "no restart: {:?} {:?}",
        show(&unit),
        live.session.keeper()
    );
    let second = live.keeper_pid();
    assert_eq!(live.child(), child);
    assert!(proc_alive(child));
    let props = show(&unit);
    assert_eq!(
        (props["MainPID"].as_str(), props["NRestarts"].as_str()),
        (second.to_string().as_str(), "1"),
        "{props:?}"
    );
    let before = live.log().1.len();
    assert!(
        until(SOON, || live.log().1.len() > before + 100_000),
        "the restarted keeper is not draining the master"
    );
    kampr_keep::borrow(&live.session).expect("the restarted keeper answers ctl.sock");

    signal_unit(&unit, "USR1");
    assert!(until(SOON, || live
        .session
        .keeper()
        .is_some_and(|k| k.how == How::ReExec && k.keeper == second)));
    assert!(!live.session.path().join("hup").exists(), "the child was hung up");

    close(&live.session).expect("close");
    let exit = live.exit(SOON);
    assert!(exit.status_lost, "{exit:?}");
    assert_eq!((exit.code, exit.signal), (None, None));
    assert!(until(SOON, || gone(&unit)), "{:?}", show(&unit));

    let counted = count(&live.log().1);
    assert!(
        counted.gaps <= 1,
        "{} gaps in {} lines",
        counted.gaps,
        counted.lines
    );
}

#[test]
fn a_session_that_ends_is_not_restarted() {
    let root = root();
    let live = Live::spawn(root.path(), sh("sleep 0.3; exit 3"), Launch::Systemd);
    let unit = live.unit();
    let exit = live.exit(SOON);
    assert_eq!((exit.code, exit.status_lost), (Some(3), false));
    assert!(until(SOON, || gone(&unit)), "{:?}", show(&unit));
    std::thread::sleep(Duration::from_millis(1500));
    assert!(gone(&unit), "restarted: {:?}", show(&unit));
}

#[test]
fn a_keeper_restarted_from_the_fd_store_carries_on_the_same_journal() {
    let root = root();
    let live = Live::spawn(
        root.path(),
        sh("stty -echo -opost; echo READY; while read l; do echo \"SIZE $(stty size)\"; done"),
        Launch::Systemd,
    );
    live.wait_for_log("READY\n", SOON);
    let unit = live.unit();
    let first = live.keeper_pid();
    let master = kampr_keep::borrow(&live.session).expect("borrow");
    let ask = |cols: u16, rows: u16, holder: &str| {
        kampr_keep::claim_size(&live.session, cols, rows, holder).expect("claim");
        rustix::io::write(&master, b"x\n").expect("write");
        live.wait_for_log(&format!("SIZE {rows} {cols}\n"), SOON);
    };
    ask(101, 31, "before");
    signal_unit(&unit, "KILL");
    assert!(until(SOON, || live
        .session
        .keeper()
        .is_some_and(|k| k.how == How::FdStore && k.keeper != first)));
    ask(77, 22, "after");

    let journal = kampr_keep::sizes(live.session.path()).expect("journal");
    let got: Vec<(u64, u16, &str)> = journal
        .iter()
        .map(|r| (r.seq, r.cols, r.holder.as_str()))
        .collect();
    assert_eq!(got, vec![(0, 101, "before"), (1, 77, "after")]);
    assert!(journal[0].offset < journal[1].offset);
    assert_eq!(
        kampr_keep::size_at(live.session.path(), journal[1].offset)
            .expect("size_at")
            .holder,
        "after"
    );
}
