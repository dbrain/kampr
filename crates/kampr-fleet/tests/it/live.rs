//! The supervisor against real processes on a real pty.
//!
//! Unit tests of `prompt::read` prove the wording is parsed; only this level proves the thing the
//! feature rests on — that the kernel actually says "parked in a read" for a command that has
//! stopped for somebody, and does not say it for one that is merely slow. Probes #334 and #335
//! measured that by hand; these keep it measured.
//!
//! Nothing here needs herdr and nothing here touches the operator's sessions.

use kampr_fleet::Job;
use kampr_fleet::exec::{Geometry, RunEvent, State, Supervisor};
use std::time::Duration;
use tokio::sync::mpsc;

const PATIENCE: Duration = Duration::from_secs(10);

struct Run {
    events: mpsc::Receiver<RunEvent>,
    handle: tokio::task::JoinHandle<std::io::Result<State>>,
    writer: kampr_fleet::exec::Writer,
    killer: kampr_fleet::exec::Killer,
}

fn start(script: &str) -> Run {
    spawn(&Job::Argv(vec![
        "sh".to_string(),
        "-c".to_string(),
        script.to_string(),
    ]))
}

/// The same, through the shell a real fleet run is given: the operator's own, from `$SHELL` or
/// their passwd entry, with neither `-l` nor `-i`. On this machine that is a `bash` whose
/// `.bashrc` sources ble.sh, which is the whole reason these assertions exist.
fn typed(line: &str) -> Run {
    spawn(&Job::Shell(line.to_string()))
}

fn spawn(job: &Job) -> Run {
    let argv = job.argv(&kampr_fleet::env::login_shell());
    let supervisor = Supervisor::spawn(&argv, None, Geometry::default(), None).expect("a pty and a child");
    let writer = supervisor.writer();
    let killer = supervisor.killer();
    let (tx, events) = mpsc::channel(256);
    let handle = tokio::spawn(supervisor.drive(tx));
    Run {
        events,
        handle,
        writer,
        killer,
    }
}

impl Run {
    /// The next state matching `want`, or a panic naming everything seen instead.
    async fn wait_for(&mut self, want: impl Fn(&State) -> bool) -> State {
        let mut seen = Vec::new();
        let found = tokio::time::timeout(PATIENCE, async {
            while let Some(event) = self.events.recv().await {
                if let RunEvent::State(state) = event {
                    if want(&state) {
                        return Some(state);
                    }
                    seen.push(state);
                }
            }
            None
        })
        .await;
        match found {
            Ok(Some(state)) => state,
            _ => panic!("never reached the state asked for; saw {seen:?}"),
        }
    }

    async fn finish(self) -> State {
        tokio::time::timeout(PATIENCE, self.handle)
            .await
            .expect("the run ended")
            .expect("the task did not panic")
            .expect("the supervisor did not fail")
    }
}

#[tokio::test]
async fn a_command_sitting_at_a_prompt_is_reported_as_waiting_with_what_it_asked() {
    let mut run = start("printf ':: Proceed with installation? [Y/n] '; read answer; echo \"got $answer\"");
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert_eq!(question.prompt, ":: Proceed with installation? [Y/n]");
    assert_eq!(question.options().len(), 2, "{:?}", question.shape);
    assert!(!question.secret());
    // Measured rather than guessed: a same-uid child is readable, so the screen never had to
    // speak for the kernel here.
    assert!(!question.inferred, "this one should have come from /proc");

    run.writer.write(b"n\n").expect("the answer went in");
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

#[tokio::test]
async fn a_command_doing_silent_work_is_never_reported_as_waiting() {
    // The false positive that would make every long build look like a question. `sleep` writes
    // nothing at all, which is exactly the case a quiescence-only detector gets wrong.
    let mut run = start("sleep 2; echo done");
    let end = run.wait_for(|s| matches!(s, State::Exited { .. })).await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

#[tokio::test]
async fn a_prompt_with_echo_turned_off_is_a_secret_and_offers_no_buttons() {
    // Probe #337: on a pty with no shell on it the termios bit ble.sh ruins (#333) is honest.
    let mut run = start("stty -echo; printf 'Password: '; read p; stty echo; echo");
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert!(question.secret(), "{:?}", question.shape);
    assert!(question.options().is_empty());

    run.writer.write(b"hunter2\n").expect("the answer went in");
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

#[tokio::test]
async fn a_prompt_nothing_recognises_is_still_reported_as_waiting_and_is_still_answerable() {
    // The fallback rung. Detection of the *wording* fails here on purpose; the host must still
    // surface as needing somebody, because that half comes from the kernel and not from a match.
    let mut run = start("printf 'wat '; read answer; exit 7");
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert_eq!(question.prompt, "wat");
    assert!(
        matches!(question.shape, kampr_fleet::Shape::Free),
        "{:?}",
        question.shape
    );

    run.writer.write(b"anything\n").expect("the answer went in");
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(7), .. }), "{end:?}");
}

#[tokio::test]
async fn the_exit_code_is_the_real_one_and_is_not_scraped_off_the_screen() {
    for expected in [0, 1, 42] {
        let run = start(&format!("exit {expected}"));
        let end = run.finish().await;
        assert!(
            matches!(end, State::Exited { code: Some(c), .. } if c == expected),
            "expected {expected}, got {end:?}"
        );
    }
}

#[tokio::test]
async fn a_run_killed_by_a_signal_reports_the_signal_rather_than_a_plausible_exit_code() {
    // The `catch`/`unwrap_or` failure this project names as its most expensive: a run that died
    // must not look like one that finished.
    let run = start("kill -TERM $$");
    let end = run.finish().await;
    assert!(
        matches!(
            end,
            State::Exited {
                code: None,
                signal: Some(15)
            }
        ),
        "{end:?}"
    );
}

#[tokio::test]
async fn a_progress_bar_is_not_mistaken_for_a_prompt() {
    // pacman redraws downloads with \r. Every frame of the bar is unterminated text, so a naive
    // reader is looking at a "prompt" the whole time the download runs.
    let mut run = start(
        "printf 'linux-firmware 10%%\\rlinux-firmware 60%%\\r'; sleep 1; printf 'done\\n'; printf 'Continue? [y/N] '; read a",
    );
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert_eq!(
        question.prompt, "Continue? [y/N]",
        "the bar's frames must not be part of the question"
    );
    run.writer.write(b"\n").expect("the answer went in");
    let _ = run.finish().await;
}

#[tokio::test]
async fn dropping_a_supervisor_does_not_leave_the_command_running() {
    // Found by a mutation run that hung: `spawn_blocking` cannot be cancelled, so the reader stays
    // parked on the master fd and the child survives the task that was driving it. An orphaned
    // `sudo pacman` holding the package database is the version of this that matters.
    let argv = vec![
        "sh".to_string(),
        "-c".to_string(),
        "printf 'waiting '; read forever".to_string(),
    ];
    let supervisor = Supervisor::spawn(&argv, None, Geometry::default(), None).expect("a pty and a child");
    let pid = supervisor.pid();
    let (tx, _events) = mpsc::channel(16);
    let driver = tokio::spawn(supervisor.drive(tx));

    assert!(
        settles(|| alive(pid)).await,
        "the child should be running before we abort"
    );

    driver.abort();
    let _ = driver.await;
    assert!(
        settles(|| !alive(pid)).await,
        "pid {pid} outlived the supervisor that spawned it"
    );
}

#[tokio::test]
async fn a_run_can_be_ended_from_the_outside_while_it_sits_at_a_prompt() {
    let mut run = start("printf 'Continue? [Y/n] '; read a");
    let _ = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    run.killer.hangup();
    let end = run.finish().await;
    assert!(
        matches!(end, State::Exited { .. }),
        "a hung-up run must report an ending, not sit in Waiting: {end:?}"
    );
}

/// Waits for a condition rather than guessing how long it takes, so a loaded machine reports a
/// slow test and never a failing one.
async fn settles(mut done: impl FnMut() -> bool) -> bool {
    let deadline = tokio::time::Instant::now() + PATIENCE;
    while tokio::time::Instant::now() < deadline {
        if done() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    false
}

/// A pid that exists and has not become a zombie. A reaped-but-unwaited child keeps its `/proc`
/// entry, so existence alone would call every finished run alive.
fn alive(pid: u32) -> bool {
    let Ok(stat) = std::fs::read_to_string(format!("/proc/{pid}/stat")) else {
        return false;
    };
    let Some(after_comm) = stat.rsplit_once(") ") else {
        return false;
    };
    !matches!(after_comm.1.split_whitespace().next(), Some("Z") | None)
}

#[tokio::test]
async fn a_full_screen_program_is_never_reported_as_a_secret() {
    // Probe #340: `less` turns ECHO off exactly as `getpass` does. Testing ECHO alone would call
    // it a password prompt and hide a screen the operator needed to see. (It also never reads fd 0
    // directly — it polls — so it stays `Running`, which is the honest answer for a program whose
    // screen *is* the interface.)
    if which("less").is_none() {
        eprintln!("SKIPPED: no `less` on PATH; this assertion was NOT made");
        return;
    }
    let mut run = start("exec less /etc/hostname");
    let mut seen = Vec::new();
    let collect = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = run.events.recv().await {
            if let RunEvent::State(state) = event {
                seen.push(state);
            }
        }
    });
    let _ = collect.await;
    for state in &seen {
        if let State::Waiting(question) = state {
            assert!(
                !question.secret(),
                "a pager in raw mode was reported as a password prompt: {:?}",
                question.shape
            );
        }
    }
    run.killer.kill();
    let _ = run.finish().await;
}

#[tokio::test]
async fn a_command_that_escalates_still_shows_its_password_prompt_as_a_question() {
    // Probe #339 as a test. `su` hides its whole `/proc` from us, so the syscall ladder answers
    // Unknown and the board would say merely "quiet" — but termios belongs to the tty and we hold
    // the master, so the one thing that matters still gets through.
    if which("su").is_none() {
        eprintln!("SKIPPED: no `su` on PATH; this assertion was NOT made");
        return;
    }
    let mut run = start("exec su -");
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert!(question.secret(), "{:?}", question.shape);
    assert!(question.options().is_empty(), "never offer buttons for a secret");
    run.killer.kill();
    let _ = run.finish().await;
}

fn which(binary: &str) -> Option<()> {
    std::env::var_os("PATH")?
        .to_str()?
        .split(':')
        .any(|dir| std::path::Path::new(dir).join(binary).exists())
        .then_some(())
}

#[tokio::test]
async fn a_privileged_command_at_a_yes_no_prompt_is_inferred_rather_than_called_quiet() {
    // The command this whole feature exists for. `sudo` hides its `/proc` (probe #332), so the
    // kernel ladder answers Unknown forever and the board would say only "quiet" — for the single
    // likeliest fleet command there is. The screen is allowed to speak here, and says so.
    if !passwordless_sudo() {
        eprintln!("SKIPPED: needs passwordless sudo; this assertion was NOT made");
        return;
    }
    let mut run =
        start("exec sudo -n sh -c 'printf \":: Proceed with installation? [Y/n] \"; read a; exit 0'");
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert_eq!(question.prompt, ":: Proceed with installation? [Y/n]");
    assert_eq!(question.options().len(), 2);
    // `inferred` is only reachable through the `Unknown` rung, so this is also the assertion that
    // the child really was unreadable — with no sample-once race to lose.
    assert!(
        question.inferred,
        "evidence this weak must travel labelled, not as a measurement"
    );

    run.writer.write(b"n\n").expect("the answer went in");
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

#[tokio::test]
async fn a_privileged_command_merely_working_is_not_inferred_to_be_asking() {
    // The false positive the shape guard exists to stop: a run whose state cannot be read, sitting
    // silent mid-line on text that is not a question, must stay off the board's top.
    if !passwordless_sudo() {
        eprintln!("SKIPPED: needs passwordless sudo; this assertion was NOT made");
        return;
    }
    let mut run = start("exec sudo -n sh -c 'printf \"linking kampr \"; sleep 4; echo done'");
    let mut seen = Vec::new();
    let _ = tokio::time::timeout(Duration::from_millis(3500), async {
        while let Some(event) = run.events.recv().await {
            if let RunEvent::State(state) = event {
                seen.push(state);
            }
        }
    })
    .await;
    assert!(
        !seen.iter().any(|s| matches!(s, State::Waiting(_))),
        "a half-written progress line was read as a question: {seen:?}"
    );
    run.killer.kill();
    let _ = run.finish().await;
}

fn passwordless_sudo() -> bool {
    std::process::Command::new("sudo")
        .args(["-n", "true"])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// **The dangerous half of putting a shell on a fleet pty, and it holds.**
///
/// #337 says ECHO going off is an honest password signal here *because there is no shell on the
/// pty*; #333 says ble.sh leaves a shell's tty with ECHO already off before anything asks for
/// anything. A run that understands `&&` has a shell on that pty, so this asserts the signal
/// against the operator's real shell on the machine where ble.sh is installed.
///
/// The measurement behind it: a `bash -c` fleet pty reads `BLE_VERSION` unset and `$-` as `hBc` —
/// no `i`, so `.bashrc` returns at its own guard — and reads `ECHO on, ICANON on` at idle against
/// `ECHO OFF, ICANON on` at a prompt, which is what a pty with nothing on it reads. `bash -i` on
/// the same pty reads `ECHO OFF, ICANON OFF` while merely sitting there, which is #333.
#[tokio::test]
async fn a_password_prompt_through_the_operators_own_shell_is_still_a_secret() {
    let mut run = typed("stty -echo; printf 'Password: '; read p; stty echo; echo");
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert!(
        question.secret(),
        "a shell on the pty put #333's ECHO confound back: {:?}",
        question.shape
    );
    assert!(question.options().is_empty());
    run.writer.write(b"hunter2\n").expect("the answer went in");
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

/// And the other direction: a shell sitting on the pty doing ordinary work must not read as a
/// secret, or every fleet run would render as a password box. This is the assertion #333 would
/// break — an interactive shell fails it before it asks anything at all.
#[tokio::test]
async fn a_shell_merely_running_a_command_is_never_reported_as_a_secret() {
    let mut run = typed("printf 'ready\n'; sleep 1; printf 'Continue? [Y/n] '; read a; echo ok");
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert!(
        !question.secret(),
        "an ordinary prompt read as a password: {:?}",
        question.shape
    );
    assert_eq!(question.options().len(), 2, "{:?}", question.shape);
    run.writer.write(b"y\n").expect("the answer went in");
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

/// **A pipeline is not a question.** Measured: `bash -c 'sleep 30 | cat'` parks `cat` in
/// `splice(2)` on fd 0 — byte for byte the line a `cat` at a terminal produces — and fd 0 there is
/// a pipe. Rung 1 keyed on the syscall alone puts this host on the board as needing somebody, for
/// as long as the pipeline runs, with nothing to answer.
#[tokio::test]
async fn a_host_running_a_pipeline_is_not_reported_as_one_waiting_for_an_answer() {
    let mut run = typed("sleep 2 | cat; echo done");
    let mut seen = Vec::new();
    let collect = tokio::time::timeout(Duration::from_secs(3), async {
        while let Some(event) = run.events.recv().await {
            if let RunEvent::State(state) = event {
                let finished = state.finished();
                seen.push(state);
                if finished {
                    return;
                }
            }
        }
    });
    let _ = collect.await;
    assert!(
        !seen.iter().any(|s| matches!(s, State::Waiting(_))),
        "the tail of a pipeline was reported as a host asking something: {seen:?}",
    );
    run.killer.kill();
}

/// And the front of a pipeline, which does hold the terminal, is still a question — so the
/// narrowing above removed a false answer rather than a true one.
#[tokio::test]
async fn the_end_of_a_pipeline_that_reads_the_terminal_is_still_a_question() {
    let mut run = typed("printf 'Proceed? [Y/n] '; read a | cat; echo got");
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert_eq!(question.prompt, "Proceed? [Y/n]");
    run.killer.kill();
}
