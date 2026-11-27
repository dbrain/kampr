use flate2::Compression;
use flate2::read::ZlibDecoder;
use flate2::write::ZlibEncoder;
use kampr_core::program::{BlockKind, ProgramState, ProgramStatus};
use kampr_native::Screen;
use std::io::{Read as _, Write as _};

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(format!("{}/tests/fixtures/{name}", env!("CARGO_MANIFEST_DIR"))).unwrap()
}

fn report(state: ProgramState, kind: Option<BlockKind>, message: Option<&str>) -> Option<ProgramStatus> {
    Some(ProgramStatus {
        state,
        kind,
        message: message.map(str::to_string),
    })
}

fn word(screen: &Screen) -> Option<ProgramState> {
    screen.program().fold().map(|s| s.state)
}

fn ids(screen: &Screen) -> Vec<String> {
    let mut ids: Vec<String> = screen.program().saved().into_iter().map(|r| r.id).collect();
    ids.sort();
    ids
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
            Some(BlockKind::Permission),
            Some("approve Bash: touch probe-made-this.txt"),
        ),
        report(Working, None, None),
        report(
            Blocked,
            Some(BlockKind::Question),
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
            let now = screen.program().fold();
            if seen.last() != Some(&now) && (now.is_some() || !seen.is_empty()) {
                seen.push(now);
            }
        }
        assert_eq!(seen, want, "chunks of {chunk}");
    }
}

#[test]
fn a_workers_reports_fold_into_the_session_and_a_word_nobody_measured_changes_nothing() {
    let mut screen = Screen::new(80, 24, 0);
    screen.feed(b"\x1b]7501;state=working:app=claude-code\x07");
    screen.feed(b"\x1b]7501;state=done:app=claude-code:id=agent-1\x07");
    assert_eq!(word(&screen), Some(ProgramState::Working));
    screen.feed(b"\x1b]7501;state=blocked:kind=permission:id=agent-2:msg=bGV0IG1l\x07");
    assert_eq!(
        screen.program().fold(),
        report(ProgramState::Blocked, Some(BlockKind::Permission), Some("let me"))
    );
    screen.feed(b"\x1b]7501;state=clear:id=agent-2\x07");
    screen.feed(b"\x1b]7501;state=compacting:app=claude-code\x1b\\");
    assert_eq!(word(&screen), Some(ProgramState::Working));
    assert_eq!(ids(&screen), ["", "agent-1"]);
}

#[test]
fn one_bad_pair_costs_a_report_nothing_and_a_bad_free_text_costs_it_everything() {
    let mut screen = Screen::new(80, 24, 0);
    screen.feed(b"\x1b]7501;state=working:garbage:app=x\x07");
    assert_eq!(word(&screen), Some(ProgramState::Working));
    screen.feed(b"\x1b]7501;state=done:msg=bGluZQpicmVhaw==\x07");
    screen.feed(b"\x1b]7501;state=done:msg=!!!\x07");
    screen.feed(b"\x1b]7501;state=done;msg=aGk=\x07");
    assert_eq!(word(&screen), Some(ProgramState::Working));
}

#[test]
fn several_reports_in_one_read_all_count_and_a_long_one_split_across_reads_does_too() {
    let mut screen = Screen::new(80, 24, 0);
    screen.feed(b"\x1b]7501;state=working\x07\x1b]7501;state=blocked:id=a\x07\x1b]7501;state=done:id=b\x07");
    assert_eq!(ids(&screen), ["", "a", "b"]);
    let text = "x".repeat(2000);
    let long = format!("\x1b]7501;state=blocked:id=long:msg={}\x07", base64_of(&text));
    for piece in long.as_bytes().chunks(97) {
        screen.feed(piece);
    }
    assert_eq!(
        screen.program().get("long").and_then(|r| r.msg.clone()),
        Some(text)
    );
}

fn base64_of(text: &str) -> String {
    use base64::Engine as _;
    base64::engine::general_purpose::STANDARD.encode(text)
}

const RUNNING: &[u8] = b"\x1b]7501;state=working\x07\x1b]7501;state=blocked:id=w\x07\
    \x1b]7501;state=done:id=d\x07\x1b]7501;state=error:id=e\x07";

/// A shell that ran a program reporting `working` and then killed it leaves `working` for ever
/// unless the prompt coming back ends it; `done` and `error` are what somebody comes back to.
#[test]
fn a_new_shell_prompt_ends_what_was_running_and_keeps_what_finished() {
    for (what, bytes, left) in [
        ("OSC 133 A with BEL", &b"\x1b]133;A\x07"[..], vec!["d", "e"]),
        (
            "OSC 133 A with ST and options",
            b"\x1b]133;A;aid=7;cl=m\x1b\\",
            vec!["d", "e"],
        ),
        ("OSC 133 B", b"\x1b]133;B\x07", vec!["", "d", "e", "w"]),
        ("OSC 133 C", b"\x1b]133;C\x07", vec!["", "d", "e", "w"]),
        ("OSC 133 D", b"\x1b]133;D;0\x07", vec!["", "d", "e", "w"]),
        ("DECSTR", b"\x1b[!p", vec!["", "d", "e", "w"]),
        (
            "the alternate screen",
            b"\x1b[?1049h\x1b[?1049l\x1b[?47h\x1b[?47l",
            vec!["", "d", "e", "w"],
        ),
        ("RIS", b"\x1bc", vec![]),
    ] {
        let mut screen = Screen::new(80, 24, 0);
        screen.feed(RUNNING);
        screen.feed(bytes);
        assert_eq!(ids(&screen), left, "{what}");
    }
}

#[test]
fn the_session_ending_ends_what_was_running() {
    let mut screen = Screen::new(80, 24, 0);
    screen.feed(RUNNING);
    let before = screen.program_reports();
    screen.process_ended();
    assert_eq!(ids(&screen), ["d", "e"]);
    assert!(
        screen.program_reports() > before,
        "a change a status watcher must see"
    );
    let after = screen.program_reports();
    screen.process_ended();
    assert_eq!(screen.program_reports(), after, "nothing ended the second time");
}

/// A node restart starts from the newest checkpoint, so every record must ride in it: the bytes
/// after it may say nothing for minutes.
#[test]
fn a_checkpoint_keeps_every_record_and_their_order() {
    let mut screen = Screen::new(80, 24, 0);
    screen.feed(b"\x1b]7501;state=working:app=ci\x07\x1b]7501;state=blocked:id=a/b:kind=auth:progress=3:title=VA==:msg=bQ==\x07");
    screen.feed(
        b"\x1b]7501;state=done:id=c\x07\x1b]7501;state=done:id=a/b\x07\x1b]7501;state=working:id=z\x07",
    );
    let saved = screen.checkpoint().expect("a checkpoint");
    let restored = Screen::from_checkpoint(&saved, 0).expect("a restored screen");
    assert_eq!(restored.program(), screen.program());
    assert_eq!(restored.program().rows(), screen.program().rows());
    assert_eq!(restored.program().fold(), screen.program().fold());
    assert_eq!(restored.program_reports(), screen.program_reports());
}

#[test]
fn a_checkpoint_taken_while_blocked_survives() {
    let log = fixture("claude-osc7501-turn.bin");
    let blocked = log
        .windows(28)
        .position(|w| w.starts_with(b"\x1b]7501;state=blocked"))
        .expect("a block in the log");
    let mut screen = Screen::new(110, 34, 0);
    for cut in (blocked..log.len()).step_by(7) {
        screen.feed(&log[..cut]);
        if let Some(saved) = screen.checkpoint()
            && word(&screen) == Some(ProgramState::Blocked)
        {
            let restored = Screen::from_checkpoint(&saved, 0).expect("a restored screen");
            assert_eq!(restored.program(), screen.program());
            return;
        }
        screen = Screen::new(110, 34, 0);
    }
    panic!("no checkpoint was taken while blocked");
}

/// Version 1 held the last report only, as a root record, after `Saved` and in the other order.
#[test]
fn a_version_one_checkpoint_is_still_read_as_one_root_record() {
    let mut empty = Screen::new(80, 24, 0);
    let current = empty.checkpoint().expect("a checkpoint");
    let mut body = Vec::new();
    ZlibDecoder::new(&current[b"kampr-screen\n".len()..])
        .read_to_end(&mut body)
        .unwrap();
    let tail = postcard::to_stdvec(&(0u64, Vec::<u8>::new())).unwrap();
    assert!(body.ends_with(&tail), "the tail is the last two bytes");
    body.truncate(body.len() - tail.len());
    assert_eq!(body[0], 2, "the version leads");
    body[0] = 1;
    let program: Option<(u8, Option<String>, Option<String>)> =
        Some((2, Some("permission".into()), Some("approve Bash".into())));
    body.extend(postcard::to_stdvec(&(program, 5u64)).unwrap());
    let mut out = ZlibEncoder::new(b"kampr-screen\n".to_vec(), Compression::fast());
    out.write_all(&body).unwrap();
    let old = out.finish().unwrap();
    let restored = Screen::from_checkpoint(&old, 0).expect("version 1 still reads");
    assert_eq!(
        restored.program().fold(),
        report(
            ProgramState::Blocked,
            Some(BlockKind::Permission),
            Some("approve Bash")
        )
    );
    assert_eq!(restored.program_reports(), 5);
}
