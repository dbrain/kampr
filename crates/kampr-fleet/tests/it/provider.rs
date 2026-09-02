//! The fleet provider as the node sees it: panes, streams, answers and grouping.

use kampr_core::provider::{AgentStatus, Input, PaneEvent, PaneInfo, Provider};
use kampr_fleet::exec::Geometry;
use kampr_fleet::{FleetProvider, RunEvent, State, Supervisor};
use std::os::unix::fs::PermissionsExt;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;

const PATIENCE: Duration = Duration::from_secs(10);

fn provider() -> Arc<FleetProvider> {
    Arc::new(FleetProvider::new())
}

fn sh(script: &str) -> Vec<String> {
    vec!["sh".into(), "-c".into(), script.into()]
}

async fn wait_for(provider: &Arc<FleetProvider>, pane_id: &str, want: impl Fn(&State) -> bool) -> PaneInfo {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    let mut last = None;
    while tokio::time::Instant::now() < deadline {
        let panes = provider.list_panes().await.expect("a list");
        if let Some(pane) = panes.into_iter().find(|p| p.pane_id == pane_id) {
            let state = pane.fleet.as_ref().expect("a fleet marker").state.clone();
            if want(&state) {
                return pane;
            }
            last = Some(state);
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    panic!("{pane_id} never reached the state asked for; last was {last:?}");
}

#[tokio::test]
async fn a_started_run_is_served_as_a_pane_carrying_its_cohort_and_its_command() {
    let provider = provider();
    let pane_id = provider
        .start("cohort-a", &sh("echo hello; sleep 5"), None, Geometry::default())
        .expect("a run");

    let panes = provider.list_panes().await.expect("a list");
    let pane = panes.iter().find(|p| p.pane_id == pane_id).expect("the pane");
    let fleet = pane.fleet.as_ref().expect("a fleet marker");
    assert_eq!(fleet.cohort, "cohort-a");
    assert!(fleet.command.contains("echo hello"));
    assert!(
        pane.pane_id.starts_with("fleet:"),
        "a fleet pane id must not collide with herdr's `w3:p2` namespace"
    );
    provider.stop(&pane_id).expect("stopped");
}

#[tokio::test]
async fn a_herdr_pane_carries_no_fleet_marker_so_the_two_never_mix_in_one_list() {
    // The grouping is a property of the pane, not a filter a client has to remember.
    let plain = PaneInfo::default();
    assert!(plain.fleet.is_none());
}

#[tokio::test]
async fn watching_a_run_replays_what_it_already_printed_and_then_follows_it() {
    let provider = provider();
    let pane_id = provider
        .start(
            "c",
            &sh("echo first; sleep 4; echo second"),
            None,
            Geometry::default(),
        )
        .expect("a run");
    tokio::time::sleep(Duration::from_millis(500)).await;

    // A watcher that arrives late must still see the beginning.
    let mut stream = provider.watch_pane(&pane_id).await.expect("a stream");
    let first = stream.recv().await.expect("an event");
    assert!(matches!(first, PaneEvent::Reset { .. }), "{first:?}");

    let mut seen = String::new();
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while tokio::time::Instant::now() < deadline {
        let Ok(Some(PaneEvent::Bytes { bytes, .. })) =
            tokio::time::timeout(Duration::from_secs(6), stream.recv()).await
        else {
            break;
        };
        seen.push_str(&String::from_utf8_lossy(&bytes));
        if seen.contains("second") {
            break;
        }
    }
    assert!(seen.contains("first"), "the replay was lost: {seen:?}");
    assert!(seen.contains("second"), "the live follow stopped: {seen:?}");
    provider.stop(&pane_id).expect("stopped");
}

#[tokio::test]
async fn a_waiting_run_reports_blocked_so_the_existing_ordering_floats_it_to_the_top() {
    // Reuse rather than a second sort: the sidebar already puts blocked above working.
    let provider = provider();
    let pane_id = provider
        .start(
            "c",
            &sh("printf ':: Proceed with installation? [Y/n] '; read a; exit 0"),
            None,
            Geometry::default(),
        )
        .expect("a run");

    let pane = wait_for(&provider, &pane_id, |s| matches!(s, State::Waiting(_))).await;
    assert_eq!(pane.agent_status, AgentStatus::Blocked);
    let question = pane.fleet.unwrap().state.question().cloned().expect("a question");
    assert_eq!(question.prompt, ":: Proceed with installation? [Y/n]");
    assert_eq!(question.options().len(), 2);
    provider.stop(&pane_id).expect("stopped");
}

#[tokio::test]
async fn an_answer_written_to_the_pane_reaches_the_command_and_it_finishes() {
    let provider = provider();
    let pane_id = provider
        .start(
            "c",
            &sh("printf 'Continue? [y/N] '; read a; [ \"$a\" = y ] && exit 0 || exit 3"),
            None,
            Geometry::default(),
        )
        .expect("a run");
    wait_for(&provider, &pane_id, |s| matches!(s, State::Waiting(_))).await;

    provider
        .write_pane(&pane_id, Input::Bytes(b"y\n".to_vec()))
        .await
        .expect("the answer went in");

    let pane = wait_for(&provider, &pane_id, |s| s.finished()).await;
    let state = pane.fleet.unwrap().state;
    assert!(state.succeeded(), "{state:?}");
    assert_eq!(pane.agent_status, AgentStatus::Done);
}

#[tokio::test]
async fn a_run_that_fails_is_not_reported_as_one_that_succeeded() {
    let provider = provider();
    let pane_id = provider
        .start("c", &sh("exit 1"), None, Geometry::default())
        .expect("a run");
    let pane = wait_for(&provider, &pane_id, |s| s.finished()).await;
    let state = pane.fleet.unwrap().state;
    assert!(state.finished());
    assert!(!state.succeeded(), "exit 1 must not round to success: {state:?}");
}

#[tokio::test]
async fn keys_sent_as_names_reach_the_command_as_bytes() {
    let provider = provider();
    let pane_id = provider
        .start("c", &sh("read a; exit 0"), None, Geometry::default())
        .expect("a run");
    wait_for(&provider, &pane_id, |s| matches!(s, State::Waiting(_))).await;
    provider
        .write_pane(&pane_id, Input::Keys(vec!["enter".into()]))
        .await
        .expect("the key went in");
    let pane = wait_for(&provider, &pane_id, |s| s.finished()).await;
    assert!(pane.fleet.unwrap().state.succeeded());
}

#[tokio::test]
async fn a_cohort_gathers_its_own_runs_and_nothing_else() {
    let provider = provider();
    let a1 = provider
        .start("upgrade", &sh("sleep 4"), None, Geometry::default())
        .expect("run");
    let a2 = provider
        .start("upgrade", &sh("sleep 4"), None, Geometry::default())
        .expect("run");
    let b1 = provider
        .start("reboot", &sh("sleep 4"), None, Geometry::default())
        .expect("run");

    let upgrade: Vec<String> = provider
        .cohort("upgrade")
        .into_iter()
        .map(|p| p.pane_id)
        .collect();
    assert_eq!(upgrade.len(), 2);
    assert!(upgrade.contains(&a1) && upgrade.contains(&a2));
    assert!(!upgrade.contains(&b1));
    assert_eq!(provider.cohort("reboot").len(), 1);
    assert!(provider.cohort("nothing-ran-here").is_empty());

    for pane in [a1, a2, b1] {
        provider.stop(&pane).expect("stopped");
    }
}

#[tokio::test]
async fn a_run_that_is_still_going_cannot_be_forgotten() {
    // Forgetting a live run would leave nothing reading its pty and nobody able to answer it —
    // the fleet version of a node that looks healthy while a path is dead.
    let provider = provider();
    let pane_id = provider
        .start("c", &sh("sleep 5"), None, Geometry::default())
        .expect("a run");
    assert!(provider.forget(&pane_id).is_err(), "a live run was forgotten");

    provider.stop(&pane_id).expect("stopped");
    wait_for(&provider, &pane_id, |s| s.finished()).await;
    provider
        .forget(&pane_id)
        .expect("a finished run can be forgotten");
    assert!(
        !provider
            .list_panes()
            .await
            .expect("a list")
            .iter()
            .any(|p| p.pane_id == pane_id),
        "the pane outlived being forgotten"
    );
}

#[tokio::test]
async fn a_finished_run_stays_listed_until_it_is_forgotten() {
    // "How did they all go" is the other half of the board, and it is unanswerable if a pane
    // vanishes the moment it exits.
    let provider = provider();
    let pane_id = provider
        .start("c", &sh("exit 0"), None, Geometry::default())
        .expect("a run");
    wait_for(&provider, &pane_id, |s| s.finished()).await;
    tokio::time::sleep(Duration::from_millis(400)).await;
    assert!(
        provider
            .list_panes()
            .await
            .expect("a list")
            .iter()
            .any(|p| p.pane_id == pane_id),
        "a finished run disappeared before anybody could read its result"
    );
}

#[tokio::test]
async fn the_topology_bumps_when_a_run_starts_and_when_its_state_moves() {
    let provider = provider();
    let mut topology = provider.topology();
    let before = *topology.borrow_and_update();
    let pane_id = provider
        .start("c", &sh("exit 0"), None, Geometry::default())
        .expect("a run");
    tokio::time::timeout(PATIENCE, topology.changed())
        .await
        .expect("the topology bumped")
        .expect("the channel stayed open");
    assert!(*topology.borrow() > before);
    wait_for(&provider, &pane_id, |s| s.finished()).await;
}

#[tokio::test]
async fn writing_to_a_pane_that_is_not_a_fleet_run_is_an_error_rather_than_a_silent_success() {
    let provider = provider();
    assert!(
        provider
            .write_pane("w1:p1", Input::Bytes(b"y\n".to_vec()))
            .await
            .is_err(),
        "a herdr pane id must not be quietly accepted and dropped"
    );
    assert!(provider.watch_pane("fleet:nope").await.is_err());
}

#[tokio::test]
async fn a_run_whose_state_cannot_be_read_says_so_instead_of_looking_idle() {
    // Probe #334's privilege half, as a test. An unprivileged supervisor is refused
    // `/proc/<pid>/syscall` for its own child the moment that child is setuid, so `sudo` under a
    // plain supervisor is exactly as opaque as a herdr pane. The run still works; what must not
    // happen is the board quietly showing it as working.
    if std::process::Command::new("sudo")
        .args(["-n", "true"])
        .status()
        .map(|s| !s.success())
        .unwrap_or(true)
    {
        eprintln!("SKIPPED: needs passwordless sudo to produce a setuid child; this assertion was NOT made");
        return;
    }

    let provider = provider();
    let pane_id = provider
        .start(
            "c",
            &[
                "sudo".into(),
                "-n".into(),
                "sh".into(),
                "-c".into(),
                "read a".into(),
            ],
            None,
            Geometry::default(),
        )
        .expect("a run");
    tokio::time::sleep(Duration::from_millis(400)).await;

    let panes = provider.list_panes().await.expect("a list");
    let pane = panes.iter().find(|p| p.pane_id == pane_id).expect("the pane");
    let fleet = pane.fleet.as_ref().expect("a fleet marker");
    assert!(
        fleet.blind,
        "an unprivileged supervisor over a setuid child cannot read it, and must say so"
    );
    assert!(
        pane.detail
            .as_deref()
            .is_some_and(|d| d.contains("cannot read whether it is waiting")),
        "the operator has to be told what to do about it, not just that it happened: {:?}",
        pane.detail
    );
    provider.stop(&pane_id).expect("stopped");
}

/// The half of #419 that a table test cannot reach: whether a `PATH` this process does not have
/// actually resolves the child's program name.
///
/// It is not obvious that it does. `Command::new` takes a bare name and `execvp` searches the
/// **calling** process's environment — Rust only gets this right because a `pre_exec` (which this
/// supervisor has, for `setsid` and the controlling terminal) forces the fork/exec path, where the
/// child's `envp` is installed before the exec. Reasoning about that would have been a guess; this
/// runs it.
#[tokio::test]
async fn a_command_is_found_on_the_path_the_run_was_given_and_not_on_this_processs() {
    let dir = tempfile::tempdir().expect("a directory");
    let name = "kampr-fleet-path-probe";
    let script = dir.path().join(name);
    std::fs::write(&script, "#!/bin/sh\necho found-on-the-given-path\n").expect("a script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("executable");

    let argv = vec![name.to_string()];
    assert!(
        Supervisor::spawn(&argv, None, Geometry::default(), None).is_err(),
        "this process's own PATH must not already resolve {name}, or this proves nothing",
    );

    let given = format!("/nonexistent-for-this-test:{}", dir.path().display());
    let supervisor =
        Supervisor::spawn(&argv, None, Geometry::default(), Some(&given)).expect("a pty and a child");
    let (tx, mut events) = mpsc::channel(64);
    let driver = tokio::spawn(supervisor.drive(tx));

    let mut seen = String::new();
    while let Some(event) = events.recv().await {
        if let RunEvent::Bytes(bytes) = event {
            seen.push_str(&String::from_utf8_lossy(&bytes));
            if seen.contains("found-on-the-given-path") {
                break;
            }
        }
    }
    driver.abort();
    let _ = driver.await;
    assert!(seen.contains("found-on-the-given-path"), "{seen:?}");
}
