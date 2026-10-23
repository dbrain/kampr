use kampr_core::provider::{ProgramState, ProgramStatus};
use kampr_native::Screen;

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn report(state: ProgramState, kind: Option<&str>, message: Option<&str>) -> Option<ProgramStatus> {
    Some(ProgramStatus {
        state,
        kind: kind.map(str::to_string),
        message: message.map(str::to_string),
    })
}

/// Claude Code 2.1.308's own log of one scripted session with `OSC 7501;?` answered (#600): a turn
/// that asks for a Bash permission and an AskUserQuestion, `/compact`, a turn answered HTTP 400,
/// an Esc interrupt and `/exit`. Every report is read whatever the reads were cut at.
#[test]
fn every_state_claude_reports_is_read_off_its_log() {
    use ProgramState::*;
    let log = fixture("claude-osc7501-turn.bin");
    let want = vec![
        report(Idle, None, None),
        report(Working, None, None),
        report(
            Blocked,
            Some("permission"),
            Some("approve Bash: touch probe-made-this.txt"),
        ),
        report(Working, None, None),
        report(
            Blocked,
            Some("question"),
            Some("answer: Which probe colour should win? (Teal · Amber)"),
        ),
        report(Working, None, None),
        report(Done, None, None),
        report(Working, None, None),
        report(Done, None, None),
        report(Working, None, None),
        report(Error, None, Some("API Error: 400 probe failure")),
        report(Working, None, None),
        report(Idle, None, None),
        None,
    ];
    for chunk in [1, 61, 512] {
        let mut screen = Screen::new(110, 34, 0);
        let mut seen: Vec<Option<ProgramStatus>> = Vec::new();
        for piece in log.chunks(chunk) {
            screen.feed(piece);
            let now = screen.program().cloned();
            if seen.last() != Some(&now) && (now.is_some() || !seen.is_empty()) {
                seen.push(now);
            }
        }
        assert_eq!(seen, want, "chunks of {chunk}");
    }
}

#[test]
fn a_subagents_report_and_a_word_nobody_measured_leave_the_sessions_alone() {
    let mut screen = Screen::new(80, 24, 0);
    screen.feed(b"\x1b]7501;state=working:app=claude-code\x07");
    screen.feed(b"\x1b]7501;state=done:app=claude-code:id=agent-1\x07");
    screen.feed(b"\x1b]7501;state=clear:id=agent-1\x07");
    screen.feed(b"\x1b]7501;state=compacting:app=claude-code\x1b\\");
    assert_eq!(
        screen.program(),
        report(ProgramState::Working, None, None).as_ref()
    );
}

/// A node restart starts from the newest checkpoint, so the report must ride in it: the bytes
/// after it may say nothing for minutes.
#[test]
fn a_checkpoint_keeps_the_last_report() {
    let log = fixture("claude-osc7501-turn.bin");
    let blocked = log
        .windows(28)
        .position(|w| w.starts_with(b"\x1b]7501;state=blocked"))
        .expect("a block in the log");
    let mut screen = Screen::new(110, 34, 0);
    for cut in (blocked..log.len()).step_by(7) {
        screen.feed(&log[..cut]);
        if let Some(saved) = screen.checkpoint()
            && screen.program().is_some_and(|p| p.state == ProgramState::Blocked)
        {
            let restored = Screen::from_checkpoint(&saved, 0).expect("a restored screen");
            assert_eq!(restored.program(), screen.program());
            return;
        }
        screen = Screen::new(110, 34, 0);
    }
    panic!("no checkpoint was taken while blocked");
}
