//! The ladder against real processes on a real pty, kept by a real keeper.
//!
//! Unit tests of `prompt::read` prove the wording is parsed; only this level proves the thing the
//! feature rests on — that the kernel actually says "parked in a read" for a command that has
//! stopped for somebody, and does not say it for one that is merely slow. Probes #334 and #335
//! measured that by hand; these keep it measured.
//!
//! Nothing here needs herdr and nothing here touches the operator's sessions.

use crate::{Fleet, keeper_child, sh, typed};
use kampr_core::provider::{Input, Provider};
use kampr_fleet::{Job, State};
use std::time::Duration;

struct Run {
    fleet: Fleet,
    pane: String,
}

async fn start(script: &str) -> Run {
    spawn(&sh(script)).await
}

async fn spawn(job: &Job) -> Run {
    let fleet = Fleet::default();
    let pane = fleet.run(job).await;
    Run { fleet, pane }
}

impl Run {
    async fn wait_for(&self, want: impl Fn(&State) -> bool) -> State {
        self.fleet
            .wait_for(&self.pane, want)
            .await
            .fleet
            .expect("a fleet marker")
            .state
    }

    async fn answer(&self, bytes: &[u8]) {
        self.fleet
            .write_pane(&self.pane, Input::Bytes(bytes.to_vec()))
            .await
            .expect("the answer went in");
    }

    async fn finish(&self) -> State {
        self.wait_for(State::finished).await
    }

    async fn states_for(&self, within: Duration) -> Vec<State> {
        self.fleet.states_for(&self.pane, within).await
    }

    fn hangup(&self) {
        self.fleet.stop(&self.pane).expect("stopped");
    }
}

#[tokio::test]
async fn a_command_sitting_at_a_prompt_is_reported_as_waiting_with_what_it_asked() {
    let run = start("printf ':: Proceed with installation? [Y/n] '; read answer; echo \"got $answer\"").await;
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert_eq!(question.prompt, ":: Proceed with installation? [Y/n]");
    assert_eq!(question.options().len(), 2, "{:?}", question.shape);
    assert!(!question.secret());
    // Measured rather than guessed: a same-uid child is readable, so the screen never had to
    // speak for the kernel here. Only Linux says which call a process is parked in.
    if cfg!(target_os = "linux") {
        assert!(!question.inferred, "this one should have come from /proc");
    }

    run.answer(b"n\n").await;
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

#[tokio::test]
async fn a_command_doing_silent_work_is_never_reported_as_waiting() {
    // The false positive that would make every long build look like a question. `sleep` writes
    // nothing at all, which is exactly the case a quiescence-only detector gets wrong.
    let run = start("sleep 2; echo done").await;
    let end = run.wait_for(|s| matches!(s, State::Exited { .. })).await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

#[tokio::test]
async fn a_prompt_with_echo_turned_off_is_a_secret_and_offers_no_buttons() {
    // Probe #337: on a pty with no shell on it the termios bit ble.sh ruins (#333) is honest.
    let run = start("stty -echo; printf 'Password: '; read p; stty echo; echo").await;
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert!(question.secret(), "{:?}", question.shape);
    assert!(question.options().is_empty());

    run.answer(b"hunter2\n").await;
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

// The kernel's half, which only Linux has.
#[cfg(target_os = "linux")]
#[tokio::test]
async fn a_prompt_nothing_recognises_is_still_reported_as_waiting_and_is_still_answerable() {
    // The fallback rung. Detection of the *wording* fails here on purpose; the host must still
    // surface as needing somebody, because that half comes from the kernel and not from a match.
    let run = start("printf 'wat '; read answer; exit 7").await;
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

    run.answer(b"anything\n").await;
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(7), .. }), "{end:?}");
}

#[tokio::test]
async fn the_exit_code_is_the_real_one_and_is_not_scraped_off_the_screen() {
    for expected in [0, 1, 42] {
        let run = start(&format!("exit {expected}")).await;
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
    let run = start("kill -TERM $$").await;
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
    let run = start(
        "printf 'linux-firmware 10%%\\rlinux-firmware 60%%\\r'; sleep 1; printf 'done\\n'; printf 'Continue? [y/N] '; read a",
    ).await;
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert_eq!(
        question.prompt, "Continue? [y/N]",
        "the bar's frames must not be part of the question"
    );
    run.answer(b"\n").await;
    let _ = run.finish().await;
}

#[tokio::test]
async fn a_run_outlives_the_node_that_started_it_and_the_next_one_can_still_answer_it() {
    // A node restart is a provider dropped and another built over the same root. The keeper holds
    // the pty, so the command never notices, and the new ladder reads its question again from the
    // log and from the keeper's `state`.
    let run = start("printf 'Continue? [y/N] '; read a; [ \"$a\" = y ] && exit 0 || exit 3").await;
    run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let child = keeper_child(&run.fleet, &run.pane).expect("a child");

    let Run { fleet, pane } = run;
    let run = Run {
        fleet: fleet.restart(),
        pane,
    };
    assert!(alive(child), "pid {child} died with the provider that started it");
    let pane = run.fleet.pane(&run.pane).await;
    assert_eq!(pane.fleet.as_ref().expect("a fleet marker").cohort, "c");
    let State::Waiting(question) = run.wait_for(|s| matches!(s, State::Waiting(_))).await else {
        unreachable!()
    };
    assert_eq!(question.prompt, "Continue? [y/N]");
    assert!(
        run.fleet
            .transcript_with(&run.pane, "Continue?")
            .await
            .contains("Continue?"),
        "the screen was not rebuilt from the log"
    );
    run.answer(b"y\n").await;
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

#[tokio::test]
async fn a_run_can_be_ended_from_the_outside_while_it_sits_at_a_prompt() {
    let run = start("printf 'Continue? [Y/n] '; read a").await;
    let _ = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    run.hangup();
    let end = run.finish().await;
    assert!(
        matches!(end, State::Exited { .. }),
        "a hung-up run must report an ending, not sit in Waiting: {end:?}"
    );
}

/// A pid that exists and has not become a zombie. A reaped-but-unwaited child keeps its `/proc`
/// entry, so existence alone would call every finished run alive.
fn alive(pid: u32) -> bool {
    use kampr_proc::Table;
    kampr_proc::here().argv(pid).is_some()
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
    let run = start("exec less /etc/hostname").await;
    let seen = run.states_for(Duration::from_secs(3)).await;
    for state in &seen {
        if let State::Waiting(question) = state {
            assert!(
                !question.secret(),
                "a pager in raw mode was reported as a password prompt: {:?}",
                question.shape
            );
        }
    }
    run.hangup();
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
    let run = start("exec su -").await;
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert!(question.secret(), "{:?}", question.shape);
    assert!(question.options().is_empty(), "never offer buttons for a secret");
    run.hangup();
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
    let run =
        start("exec sudo -n sh -c 'printf \":: Proceed with installation? [Y/n] \"; read a; exit 0'").await;
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

    run.answer(b"n\n").await;
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
    let run = start("exec sudo -n sh -c 'printf \"linking kampr \"; sleep 4; echo done'").await;
    let seen = run.states_for(Duration::from_millis(3500)).await;
    assert!(
        !seen.iter().any(|s| matches!(s, State::Waiting(_))),
        "a half-written progress line was read as a question: {seen:?}"
    );
    run.hangup();
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
    let run = spawn(&typed("stty -echo; printf 'Password: '; read p; stty echo; echo")).await;
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
    run.answer(b"hunter2\n").await;
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

/// And the other direction: a shell sitting on the pty doing ordinary work must not read as a
/// secret, or every fleet run would render as a password box. This is the assertion #333 would
/// break — an interactive shell fails it before it asks anything at all.
#[tokio::test]
async fn a_shell_merely_running_a_command_is_never_reported_as_a_secret() {
    let run = spawn(&typed(
        "printf 'ready\n'; sleep 1; printf 'Continue? [Y/n] '; read a; echo ok",
    ))
    .await;
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
    run.answer(b"y\n").await;
    let end = run.finish().await;
    assert!(matches!(end, State::Exited { code: Some(0), .. }), "{end:?}");
}

/// **A pipeline is not a question.** Measured: `bash -c 'sleep 30 | cat'` parks `cat` in
/// `splice(2)` on fd 0 — byte for byte the line a `cat` at a terminal produces — and fd 0 there is
/// a pipe. Rung 1 keyed on the syscall alone puts this host on the board as needing somebody, for
/// as long as the pipeline runs, with nothing to answer.
#[tokio::test]
async fn a_host_running_a_pipeline_is_not_reported_as_one_waiting_for_an_answer() {
    let run = spawn(&typed("sleep 2 | cat; echo done")).await;
    let seen = run.states_for(Duration::from_secs(3)).await;
    assert!(
        !seen.iter().any(|s| matches!(s, State::Waiting(_))),
        "the tail of a pipeline was reported as a host asking something: {seen:?}",
    );
    run.hangup();
}

/// And the front of a pipeline, which does hold the terminal, is still a question — so the
/// narrowing above removed a false answer rather than a true one.
#[tokio::test]
async fn the_end_of_a_pipeline_that_reads_the_terminal_is_still_a_question() {
    let run = spawn(&typed("printf 'Proceed? [Y/n] '; read a | cat; echo got")).await;
    let state = run.wait_for(|s| matches!(s, State::Waiting(_))).await;
    let State::Waiting(question) = state else {
        unreachable!()
    };
    assert_eq!(question.prompt, "Proceed? [Y/n]");
    run.hangup();
}
