//! Grouping a fan-out, and keeping it off the operator's desk.

use kampr_client::herd::Herd;
use kampr_core::provider::AgentStatus;
use kampr_core::wire::{FleetEntry, NodeEntry, PaneEntry};

fn node(id: &str) -> NodeEntry {
    serde_json::from_value(serde_json::json!({
        "id": id, "name": id, "kind": "local", "online": true
    }))
    .expect("a node")
}

fn pane(node_id: &str, id: &str, fleet: Option<FleetEntry>) -> PaneEntry {
    let mut entry: PaneEntry = serde_json::from_value(serde_json::json!({
        "id": format!("{node_id}/{id}"), "node_id": node_id, "rows": 30
    }))
    .expect("a pane");
    entry.agent_status = match fleet.as_ref().map(|f| f.state.as_str()) {
        Some("waiting") => AgentStatus::Blocked,
        Some("running") => AgentStatus::Working,
        Some("exited") => AgentStatus::Done,
        _ => AgentStatus::Unknown,
    };
    entry.fleet = fleet;
    entry
}

fn fleet(cohort: &str, state: &str) -> FleetEntry {
    FleetEntry {
        cohort: cohort.to_string(),
        command: "pacman -Syu".to_string(),
        state: state.to_string(),
        question: None,
        exit_code: (state == "exited").then_some(0),
        signal: None,
        quiet_seconds: (state == "quiet").then_some(45),
        blind: false,
        started_unix: 1_700_000_000,
    }
}

fn herd(panes: Vec<PaneEntry>) -> Herd {
    let mut h = Herd::default();
    h.apply(vec![node("n1"), node("n2")], panes);
    h
}

#[test]
fn a_fleet_run_is_never_listed_beside_the_operators_own_panes() {
    // The whole reason a fleet run is not a herdr pane: it must not clutter the desk it runs on.
    let h = herd(vec![
        pane("n1", "w1:p1", None),
        pane("n1", "fleet:01A", Some(fleet("upgrade", "running"))),
    ]);
    let listed: Vec<&str> = h
        .groups()
        .iter()
        .flat_map(|g| g.panes.iter().map(|p| p.id.as_str()))
        .collect();
    assert_eq!(listed, vec!["n1/w1:p1"]);
    assert_eq!(h.cohorts().len(), 1, "but it is on the board");
}

#[test]
fn one_command_across_two_hosts_is_one_cohort() {
    let h = herd(vec![
        pane("n1", "fleet:01A", Some(fleet("upgrade", "running"))),
        pane("n2", "fleet:01B", Some(fleet("upgrade", "waiting"))),
        pane("n1", "fleet:01C", Some(fleet("reboot", "running"))),
    ]);
    let cohorts = h.cohorts();
    assert_eq!(cohorts.len(), 2);
    let upgrade = cohorts.iter().find(|c| c.id == "upgrade").expect("the cohort");
    assert_eq!(upgrade.panes.len(), 2);
    assert_eq!(upgrade.command, "pacman -Syu");
    assert_eq!(upgrade.waiting(), 1);
    assert_eq!(upgrade.running(), 1);
    assert!(!upgrade.finished());
}

#[test]
fn the_board_puts_what_needs_somebody_at_the_top_and_failures_above_successes() {
    let ok = fleet("c", "exited");
    let mut failed = fleet("c", "exited");
    failed.exit_code = Some(1);
    let mut killed = fleet("c", "exited");
    killed.exit_code = None;
    killed.signal = Some(9);

    let h = herd(vec![
        pane("n1", "fleet:01A", Some(ok)),
        pane("n1", "fleet:01B", Some(fleet("c", "quiet"))),
        pane("n1", "fleet:01C", Some(failed)),
        pane("n1", "fleet:01D", Some(fleet("c", "waiting"))),
        pane("n1", "fleet:01E", Some(fleet("c", "running"))),
        pane("n2", "fleet:01F", Some(killed)),
    ]);
    let cohort = &h.cohorts()[0];
    let states: Vec<&str> = cohort
        .panes
        .iter()
        .map(|p| p.fleet.as_ref().unwrap().state.as_str())
        .collect();
    assert_eq!(states[0], "waiting", "the point of the board is at the top");
    assert_eq!(states[1], "running");
    assert_eq!(states[2], "quiet");
    // Both failures before the one success.
    assert_eq!(&states[3..], &["exited", "exited", "exited"]);
    let tail: Vec<Option<i32>> = cohort.panes[3..]
        .iter()
        .map(|p| p.fleet.as_ref().unwrap().exit_code)
        .collect();
    assert_eq!(tail.last(), Some(&Some(0)), "the success sorts last");
}

#[test]
fn a_run_the_kernel_killed_is_finished_and_is_not_a_success() {
    // A signal has no exit code, and rounding it to zero would report a death as a clean upgrade.
    let mut killed = fleet("c", "exited");
    killed.exit_code = None;
    killed.signal = Some(15);
    let h = herd(vec![pane("n1", "fleet:01A", Some(killed))]);
    let cohort = &h.cohorts()[0];
    assert!(cohort.finished());
    assert_eq!(cohort.succeeded(), 0);
    assert_eq!(cohort.failed(), 1);
}

#[test]
fn a_quiet_host_is_counted_apart_from_one_that_is_asking() {
    // Probes #331/#332: a host whose state cannot be read is not a host with a question, and a
    // board that added them together would send somebody to a machine that is only slow.
    let h = herd(vec![
        pane("n1", "fleet:01A", Some(fleet("c", "quiet"))),
        pane("n2", "fleet:01B", Some(fleet("c", "waiting"))),
    ]);
    let cohort = &h.cohorts()[0];
    assert_eq!(cohort.waiting(), 1);
    assert_eq!(cohort.quiet(), 1);
}

#[test]
fn a_herd_with_no_fleet_runs_has_no_board() {
    let h = herd(vec![pane("n1", "w1:p1", None)]);
    assert!(h.cohorts().is_empty());
}

#[test]
fn the_newest_fan_out_is_first() {
    let mut older = fleet("older", "running");
    older.started_unix = 1_700_000_000;
    let mut newer = fleet("newer", "running");
    newer.started_unix = 1_700_009_999;
    let h = herd(vec![
        pane("n1", "fleet:01A", Some(older)),
        pane("n1", "fleet:01B", Some(newer)),
    ]);
    let ids: Vec<String> = h.cohorts().into_iter().map(|c| c.id).collect();
    assert_eq!(ids, vec!["newer", "older"]);
}

fn asking(cohort: &str, prompt: &str) -> FleetEntry {
    let mut f = fleet(cohort, "waiting");
    f.question = Some(kampr_core::question::read(
        prompt,
        &[],
        kampr_core::question::Mode::default(),
        0,
    ));
    f
}

fn a_secret(cohort: &str) -> FleetEntry {
    let mut f = fleet(cohort, "waiting");
    f.question = Some(kampr_core::question::read(
        "Password: ",
        &[],
        kampr_core::question::Mode {
            echo: false,
            canonical: true,
        },
        0,
    ));
    f
}

#[test]
fn one_answer_reaches_every_host_asking_byte_identically() {
    let h = herd(vec![
        pane(
            "n1",
            "fleet:01A",
            Some(asking("c", ":: Proceed with installation? [Y/n] ")),
        ),
        pane(
            "n2",
            "fleet:01B",
            Some(asking("c", ":: Proceed with installation? [Y/n] ")),
        ),
    ]);
    let m = kampr_client::fleet::matching(&h, "n1/fleet:01A").expect("a match");
    assert_eq!(m.reach(), 2);
    assert!(m.differing.is_empty());

    assert_eq!(m.others[0].id, "n2/fleet:01B");
}

#[test]
fn a_host_asking_something_else_is_named_and_not_answered() {
    // The silent third of the fleet is what bites you: it must be visible, and it must not be sent
    // an answer to a question it did not ask.
    let h = herd(vec![
        pane("n1", "fleet:01A", Some(asking("c", "Proceed? [Y/n] "))),
        pane("n2", "fleet:01B", Some(asking("c", "Proceed? [Y/n] "))),
        pane(
            "n3",
            "fleet:01C",
            Some(asking("c", "Remove kdelibs4support-git? [y/N] ")),
        ),
    ]);
    let m = kampr_client::fleet::matching(&h, "n1/fleet:01A").expect("a match");
    assert_eq!(m.reach(), 2);
    assert_eq!(m.differing.len(), 1);
    assert_eq!(m.differing[0].id, "n3/fleet:01C");

    assert!(!m.others.iter().any(|p| p.id == "n3/fleet:01C"));
}

#[test]
fn a_different_cohort_is_never_swept_in_however_alike_its_question() {
    let h = herd(vec![
        pane("n1", "fleet:01A", Some(asking("upgrade", "Proceed? [Y/n] "))),
        pane(
            "n2",
            "fleet:01B",
            Some(asking("something-else", "Proceed? [Y/n] ")),
        ),
    ]);
    let m = kampr_client::fleet::matching(&h, "n1/fleet:01A").expect("a match");
    assert_eq!(m.reach(), 1, "another run's identical prompt is not this run's");
    assert!(m.differing.is_empty());
}

#[test]
fn a_password_is_answered_one_host_at_a_time() {
    // Every password prompt in the world says "Password:", so a text match is no evidence that two
    // hosts want the same secret — and being wrong means handing it to the one that did not.
    let h = herd(vec![
        pane("n1", "fleet:01A", Some(a_secret("c"))),
        pane("n2", "fleet:01B", Some(a_secret("c"))),
    ]);
    assert!(matches!(
        kampr_client::fleet::matching(&h, "n1/fleet:01A"),
        Err(kampr_client::fleet::AnswerError::Secret)
    ));
}

#[test]
fn a_host_that_is_not_waiting_has_nothing_to_answer() {
    let h = herd(vec![
        pane("n1", "fleet:01A", Some(fleet("c", "running"))),
        pane("n2", "w1:p1", None),
    ]);
    assert!(matches!(
        kampr_client::fleet::matching(&h, "n1/fleet:01A"),
        Err(kampr_client::fleet::AnswerError::NotWaiting)
    ));
    assert!(matches!(
        kampr_client::fleet::matching(&h, "n2/w1:p1"),
        Err(kampr_client::fleet::AnswerError::NotAFleetRun)
    ));
}
