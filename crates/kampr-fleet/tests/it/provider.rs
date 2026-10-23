//! The fleet provider as the node sees it: panes, streams, answers and grouping.

use crate::{Fleet, PATIENCE, sh, typed};
use kampr_core::provider::{AgentStatus, Input, PaneEvent, PaneInfo, Provider};
use kampr_fleet::{Geometry, Job, State};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

#[tokio::test]
async fn a_started_run_is_served_as_a_pane_carrying_its_cohort_and_its_command() {
    let provider = Fleet::default();
    let pane_id = provider
        .start("cohort-a", &sh("echo hello; sleep 5"), None, Geometry::default())
        .await
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
    let provider = Fleet::default();
    let pane_id = provider
        .start(
            "c",
            &sh("echo first; sleep 4; echo second"),
            None,
            Geometry::default(),
        )
        .await
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
    let provider = Fleet::default();
    let pane_id = provider
        .start(
            "c",
            &sh("printf ':: Proceed with installation? [Y/n] '; read a; exit 0"),
            None,
            Geometry::default(),
        )
        .await
        .expect("a run");

    let pane = provider
        .wait_for(&pane_id, |s| matches!(s, State::Waiting(_)))
        .await;
    assert_eq!(pane.agent_status, AgentStatus::Blocked);
    assert!(
        !pane.fleet.as_ref().unwrap().blind && pane.detail.is_none(),
        "a run its keeper can read was reported blind"
    );
    let question = pane.fleet.unwrap().state.question().cloned().expect("a question");
    assert_eq!(question.prompt, ":: Proceed with installation? [Y/n]");
    assert_eq!(question.options().len(), 2);
    provider.stop(&pane_id).expect("stopped");
}

#[tokio::test]
async fn an_answer_written_to_the_pane_reaches_the_command_and_it_finishes() {
    let provider = Fleet::default();
    let pane_id = provider
        .start(
            "c",
            &sh("printf 'Continue? [y/N] '; read a; [ \"$a\" = y ] && exit 0 || exit 3"),
            None,
            Geometry::default(),
        )
        .await
        .expect("a run");
    provider
        .wait_for(&pane_id, |s| matches!(s, State::Waiting(_)))
        .await;

    provider
        .write_pane(&pane_id, Input::Bytes(b"y\n".to_vec()))
        .await
        .expect("the answer went in");

    let pane = provider.wait_for(&pane_id, |s| s.finished()).await;
    let state = pane.fleet.unwrap().state;
    assert!(state.succeeded(), "{state:?}");
    assert_eq!(pane.agent_status, AgentStatus::Done);
}

#[tokio::test]
async fn a_run_that_fails_is_not_reported_as_one_that_succeeded() {
    let provider = Fleet::default();
    let pane_id = provider
        .start("c", &sh("exit 1"), None, Geometry::default())
        .await
        .expect("a run");
    let pane = provider.wait_for(&pane_id, |s| s.finished()).await;
    let state = pane.fleet.unwrap().state;
    assert!(state.finished());
    assert!(!state.succeeded(), "exit 1 must not round to success: {state:?}");
}

#[tokio::test]
async fn keys_sent_as_names_reach_the_command_as_bytes() {
    let provider = Fleet::default();
    let pane_id = provider
        .start("c", &sh("read a; exit 0"), None, Geometry::default())
        .await
        .expect("a run");
    provider
        .wait_for(&pane_id, |s| matches!(s, State::Waiting(_)))
        .await;
    provider
        .write_pane(&pane_id, Input::Keys(vec!["enter".into()]))
        .await
        .expect("the key went in");
    let pane = provider.wait_for(&pane_id, |s| s.finished()).await;
    assert!(pane.fleet.unwrap().state.succeeded());
}

#[tokio::test]
async fn a_cohort_gathers_its_own_runs_and_nothing_else() {
    let provider = Fleet::default();
    let a1 = provider
        .start("upgrade", &sh("sleep 4"), None, Geometry::default())
        .await
        .expect("run");
    let a2 = provider
        .start("upgrade", &sh("sleep 4"), None, Geometry::default())
        .await
        .expect("run");
    let b1 = provider
        .start("reboot", &sh("sleep 4"), None, Geometry::default())
        .await
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
    let provider = Fleet::default();
    let pane_id = provider
        .start("c", &sh("sleep 5"), None, Geometry::default())
        .await
        .expect("a run");
    assert!(provider.forget(&pane_id).is_err(), "a live run was forgotten");

    provider.stop(&pane_id).expect("stopped");
    provider.wait_for(&pane_id, |s| s.finished()).await;
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
    let provider = Fleet::default();
    let pane_id = provider
        .start("c", &sh("exit 0"), None, Geometry::default())
        .await
        .expect("a run");
    provider.wait_for(&pane_id, |s| s.finished()).await;
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
    let provider = Fleet::default();
    let mut topology = provider.topology();
    let before = *topology.borrow_and_update();
    let pane_id = provider
        .start("c", &sh("exit 0"), None, Geometry::default())
        .await
        .expect("a run");
    tokio::time::timeout(PATIENCE, topology.changed())
        .await
        .expect("the topology bumped")
        .expect("the channel stayed open");
    assert!(*topology.borrow() > before);
    provider.wait_for(&pane_id, |s| s.finished()).await;
}

#[tokio::test]
async fn writing_to_a_pane_that_is_not_a_fleet_run_is_an_error_rather_than_a_silent_success() {
    let provider = Fleet::default();
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

    let provider = Fleet::default();
    let pane_id = provider
        .start(
            "c",
            &Job::Argv(vec![
                "sudo".into(),
                "-n".into(),
                "sh".into(),
                "-c".into(),
                "read a".into(),
            ]),
            None,
            Geometry::default(),
        )
        .await
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
/// actually resolves the child's program name, and whether one that does not resolve it says so in
/// a sentence naming the `PATH` rather than in `execvp`'s (#392).
#[tokio::test]
async fn a_command_is_found_on_the_path_the_run_was_given_and_not_on_this_processs() {
    let dir = tempfile::tempdir().expect("a directory");
    let name = "kampr-fleet-path-probe";
    let script = dir.path().join(name);
    std::fs::write(&script, "#!/bin/sh\necho found-on-the-given-path\n").expect("a script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("executable");
    let job = Job::Argv(vec![name.to_string()]);

    let elsewhere = Fleet::with_path(Some("/nonexistent-for-this-test".into()));
    let refused = elsewhere
        .provider
        .start("c", &job, None, Geometry::default())
        .await
        .expect_err("a PATH without the directory must not resolve it");
    assert!(
        refused
            .to_string()
            .contains("is not on this node's fleet PATH (/nonexistent-for-this-test"),
        "{refused}"
    );

    let given = format!("/nonexistent-for-this-test:{}", dir.path().display());
    let provider = Fleet::with_path(Some(given));
    let pane_id = provider.run(&job).await;
    provider.wait_for(&pane_id, |s| s.finished()).await;
    let text = provider
        .transcript_with(&pane_id, "found-on-the-given-path")
        .await;
    assert!(text.contains("found-on-the-given-path"), "{text:?}");
}

/// What the operator typed, run the way their own terminal would run it.
///
/// Each of these is a shape the fan-out used to hand to `execvp` as words, where `|` was an
/// argument to `find` and `&&` was an argument to `make`. They are one table because the mechanism
/// is one: [`Job::Shell`] is `<their shell> -c <line>`, and the shell does everything after that.
///
/// `~` is deliberately in the table rather than reasoned about: tilde expansion is the *shell's*,
/// not `execvp`'s and not `getenv`'s, so it works here for the same reason `*` does and for no
/// other.
#[tokio::test]
async fn a_line_the_operator_would_type_in_their_own_shell_runs_as_one() {
    let dir = tempfile::tempdir().expect("a directory");
    std::fs::write(dir.path().join("one.rs"), "").expect("a file");
    std::fs::write(dir.path().join("two.rs"), "").expect("a file");
    std::fs::write(dir.path().join("three.txt"), "").expect("a file");
    let cwd = dir.path().to_str().expect("a utf-8 path").to_string();

    let cases: &[(&str, &str, &str)] = &[
        ("a pipeline", "printf 'a\\nb\\nc\\n' | wc -l", "3"),
        ("an && chain", "true && echo chained", "chained"),
        ("a ; sequence", "echo first; echo second", "second"),
        (
            "a quoted argument containing spaces",
            r#"printf '%s\n' "one argument""#,
            "one argument",
        ),
        ("a glob", "ls *.rs | wc -l", "2"),
        (
            "a redirection",
            "echo redirected > out.txt && cat out.txt",
            "redirected",
        ),
        ("a substitution", "echo \"count $(ls *.rs | wc -l)\"", "count 2"),
    ];

    for (what, line, expect) in cases {
        let provider = Fleet::default();
        let pane_id = provider
            .start("typed", &typed(line), Some(&cwd), Geometry::default())
            .await
            .unwrap_or_else(|e| panic!("{what}: {line} did not start: {e}"));
        provider.wait_for(&pane_id, |s| s.finished()).await;
        let text = provider.transcript_with(&pane_id, expect).await;
        assert!(
            text.contains(expect),
            "{what}: `{line}` should have printed {expect:?}; it printed {text:?}",
        );
    }

    // `~` on its own, because it must expand against the shell's `HOME` rather than against the
    // run's cwd, and a temp directory cannot stand in for it.
    let provider = Fleet::default();
    let pane_id = provider
        .start("typed", &typed("cd ~ && pwd"), Some(&cwd), Geometry::default())
        .await
        .expect("a run");
    provider.wait_for(&pane_id, |s| s.finished()).await;
    let home = std::env::var("HOME").expect("a home directory");
    let text = provider.transcript_with(&pane_id, &home).await;
    assert!(text.contains(&home), "`cd ~ && pwd` printed {text:?}, not {home}");
}

/// A chain that stops reports the failing command's status, not the last one's and not a zero.
///
/// The mechanism is the shell's `&&` and this is here because the *consequence* is the board's:
/// `Exited { code: 1 }` is what puts a red host in front of the operator, and a run that swallowed
/// a mid-chain failure would show five green machines that had done half the work.
#[tokio::test]
async fn a_chain_that_fails_in_the_middle_reports_the_failure_and_does_not_run_the_rest() {
    let provider = Fleet::default();
    let pane_id = provider
        .start(
            "typed",
            &typed("echo starting && false && echo never-printed"),
            None,
            Geometry::default(),
        )
        .await
        .expect("a run");
    let pane = provider.wait_for(&pane_id, |s| s.finished()).await;
    let state = pane.fleet.expect("a fleet marker").state;
    assert!(
        matches!(state, State::Exited { code: Some(1), .. }),
        "a chain that stopped at `false` reported {state:?}",
    );
    let text = provider.transcript_with(&pane_id, "starting").await;
    assert!(text.contains("starting"), "the chain never began: {text:?}");
    assert!(
        !text.contains("never-printed"),
        "the chain kept going past `false`: {text:?}"
    );
}

/// **The half of the operator's request that is not the shell.** A tool installed into
/// `~/.local/bin` works in their terminal and was invisible to a fleet run, because the login
/// shell that was read does not read `.bashrc` and `.bashrc` is what adds that directory (#419).
///
/// This runs it through the shell the way a run does, on a `PATH` that has the directory and one
/// that does not, so the assertion is about resolution rather than about a string.
#[tokio::test]
async fn a_binary_that_only_the_operators_own_path_resolves_is_still_found() {
    let dir = tempfile::tempdir().expect("a directory");
    let name = "kampr-fleet-local-bin-probe";
    let script = dir.path().join(name);
    std::fs::write(&script, "#!/bin/sh\necho found-in-local-bin\n").expect("a script");
    std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).expect("executable");

    let line = typed(&format!("{name} && echo after"));

    let without = Fleet::with_path(Some("/nonexistent-for-this-test".into()));
    let pane_id = without.run(&line).await;
    let state = without
        .wait_for(&pane_id, |s| s.finished())
        .await
        .fleet
        .expect("a fleet marker")
        .state;
    assert!(
        matches!(state, State::Exited { code: Some(127), .. }),
        "this process's PATH already resolves {name}, so the other half proves nothing: {state:?}",
    );

    let given = format!("/nonexistent-for-this-test:{}", dir.path().display());
    let with = Fleet::with_path(Some(given));
    let pane_id = with.run(&line).await;
    with.wait_for(&pane_id, |s| s.finished()).await;
    let text = with.transcript_with(&pane_id, "after").await;
    assert!(text.contains("found-in-local-bin"), "the run printed {text:?}");
    assert!(text.contains("after"), "the chain after it did not run: {text:?}");
}

/// The operator's own sentence, and never the shell that runs it. A board reading
/// `/usr/bin/bash -c pacman -Syu` has put an implementation detail in front of them, and the book
/// would remember it that way for ever.
#[tokio::test]
async fn a_pane_is_labelled_with_what_was_typed_and_not_with_the_shell_wrapper() {
    let provider = Fleet::default();
    let pane_id = provider
        .start(
            "typed",
            &typed("uptime | tee /dev/null"),
            None,
            Geometry::default(),
        )
        .await
        .expect("a run");
    let panes = provider.list_panes().await.expect("a list");
    let pane = panes
        .into_iter()
        .find(|p| p.pane_id == pane_id)
        .expect("the pane");
    assert_eq!(
        pane.fleet.expect("a fleet marker").command,
        "uptime | tee /dev/null"
    );
    assert_eq!(pane.label.as_deref(), Some("uptime | tee /dev/null"));
    assert_eq!(pane.cmd.as_deref(), Some("uptime"));
    provider.stop(&pane_id).expect("stopped");
}

/// A run ends with its command: a child left behind that ignores the hangup holds the pty open, and
/// would otherwise keep the run unfinished for as long as it lives.
#[tokio::test]
async fn a_run_whose_leftover_child_holds_the_terminal_still_finishes() {
    let provider = Fleet::default();
    let pane_id = provider
        .start(
            "stubborn",
            &sh("trap '' HUP; sleep 30 & echo left-behind $!"),
            None,
            Geometry::default(),
        )
        .await
        .expect("a run");
    let started = tokio::time::Instant::now();
    provider.wait_for(&pane_id, |s| s.finished()).await;
    let waited = started.elapsed();
    let text = provider.transcript_with(&pane_id, "left-behind").await;
    let pid = text.split_whitespace().skip_while(|w| *w != "left-behind").nth(1);
    if let Some(pid) = pid.and_then(|p| p.parse::<i32>().ok()) {
        unsafe { libc::kill(pid, libc::SIGKILL) };
    }
    assert!(
        waited < Duration::from_secs(8),
        "an exited run took {waited:?} to be published"
    );
    assert!(pid.is_some(), "{text:?}");
}
