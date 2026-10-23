use kampr_native::Screen;
use std::time::Instant;

const DEFAULT_TURN: &[u8] = include_bytes!("../fixtures/claude-default-turn.bin");
const FULLSCREEN_TURN: &[u8] = include_bytes!("../fixtures/claude-fullscreen-turn.bin");
const SCROLLBACK: usize = 10_000;

/// Every piece of state a later byte can see, set before the cut and each exercised after it.
fn rich() -> Vec<u8> {
    let mut out = Vec::new();
    for i in 0..120 {
        out.extend(format!("\x1b[3{}mhistory {i} {}\x1b[0m\r\n", i % 8, "w".repeat(i % 50)).as_bytes());
        if i % 7 == 0 {
            out.extend(format!("\x1b[4;58;5;{i}munderlined {i}\x1b[0m\r\n").as_bytes());
        }
    }
    out.extend_from_slice(
        "\x1b]2;rich title\x07\x1b]4;3;rgb:12/34/56\x07\x1b]11;rgb:01/02/03\x07\x1b[5 q\
         \x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b=\x1b[?1h\x1b[>5u\
         \x1b[3g\x1b[5G\x1bH\x1b[17G\x1bH\x1b[1;1H\
         \x1b)0\x0eqqq\x1b]8;id=x;https://k.example\x1b\\link\x1b]8;;\x1b\\ 漢字 e\u{301}\
         \x1b[1;4;38;2;9;8;7;48;5;200;58:2::4:5:6m\x1b[5;7H\x1b7\x1b[2;3H\x1b[3;9r\x1b[4h\x1b[20h\x1b[?6h"
            .as_bytes(),
    );
    out.extend_from_slice(b"wrapped wrapped wrapped wrapped wrapped wrapped wrapped\r\n");
    out.extend_from_slice(
        b"\x1b[?1049h\x1b[>1u\x1b[Halternate screen \x1b[1;31mred\x1b[2;2r\x1b[4;4H\x1b7\x1b[6;6H",
    );
    out
}

/// What each carried piece of state changes, so a restored screen that lost any of it differs.
const AFTER: &[u8] = b"after\tTAB\x1b[I\x1bM\n\n\n\nscrolled\x1b8saved\x1b[<u\x1b[?1049l\
    \x0eqq\x1b8back\x1b[<u\x1b[?6l\x1b[4l\x1b[20l\x1b[r\x1b[22t\x1b]2;next\x07\x1b[23t\r\n\
    tab\tstops\there\r\nmore\x1b[0m\r\n";

fn rows(screen: &Screen) -> String {
    screen.scrollback(None).text
}

fn same(a: &mut Screen, b: &mut Screen, at: &str) {
    assert_eq!(rows(a), rows(b), "{at}: history and screen");
    assert_eq!(a.frame_full(), b.frame_full(), "{at}: the frame");
    assert_eq!(a.input_modes(), b.input_modes(), "{at}: the modes");
    assert_eq!(
        (a.title(), a.cursor(), a.size()),
        (b.title(), b.cursor(), b.size()),
        "{at}"
    );
    assert!(a.checkpoint() == b.checkpoint(), "{at}: the whole state");
}

fn cuts(stream: &[u8]) -> impl Iterator<Item = usize> + '_ {
    let step = (stream.len() / 40).max(1);
    (1..stream.len()).step_by(step).chain([stream.len()])
}

/// **A checkpoint is the state, not a picture of it.** At cuts through each stream the restored
/// screen equals the one it was taken from, and keeps equalling it through what follows: more
/// output, resizes either way, and sequences whose effect depends on private state (tab stops,
/// the scroll region, the saved cursors, the kitty stacks, G1, the pen, the alternate screen).
/// Taking one changes nothing about the screen it was taken from.
#[test]
fn a_restored_checkpoint_carries_on_exactly_as_the_screen_it_was_taken_from() {
    for (name, stream, cols, rows) in [
        ("rich", rich(), 60, 12),
        ("claude default", DEFAULT_TURN.to_vec(), 100, 30),
        ("claude fullscreen", FULLSCREEN_TURN.to_vec(), 100, 30),
    ] {
        let mut taken = 0;
        for cut in cuts(&stream) {
            let mut live = Screen::new(cols, rows, SCROLLBACK);
            live.feed(&stream[..cut]);
            let Some(saved) = live.checkpoint() else {
                continue;
            };
            taken += 1;
            let at = format!("{name} cut at {cut}");
            let mut restored = Screen::from_checkpoint(&saved, SCROLLBACK).expect("a checkpoint that loads");
            same(&mut live, &mut restored, &at);

            let mut untouched = Screen::new(cols, rows, SCROLLBACK);
            untouched.feed(&stream[..cut]);
            for screen in [&mut live, &mut restored, &mut untouched] {
                screen.feed(&stream[cut..]);
                screen.feed(AFTER);
                screen.resize(cols - 17, rows - 3);
                screen.feed(AFTER);
                screen.resize(cols + 9, rows + 4);
                screen.feed(AFTER);
            }
            same(&mut live, &mut restored, &format!("{at}, then on"));
            same(
                &mut live,
                &mut untouched,
                &format!("{at}, against a screen never checkpointed"),
            );
        }
        assert!(taken >= 10, "{name}: only {taken} cuts were in ground");
    }
    let mut live = Screen::new(60, 12, SCROLLBACK);
    live.feed(&rich());
    let restored =
        Screen::from_checkpoint(&live.checkpoint().expect("a checkpoint"), SCROLLBACK).expect("loads");
    assert!(
        rows(&restored).contains(";58;5;42m"),
        "an underline colour in history did not survive the checkpoint"
    );
}

#[test]
fn a_cut_inside_a_sequence_or_a_character_is_not_taken() {
    for partial in [
        &b"ab\x1b"[..],
        b"\x1b[3",
        b"\x1b]2;tit",
        b"\x1bP1$q",
        "x\u{20ac}".as_bytes().split_last().unwrap().1,
    ] {
        let mut screen = Screen::new(20, 5, 100);
        screen.feed(partial);
        assert_eq!(screen.checkpoint(), None, "{partial:?}");
    }
    let mut screen = Screen::new(20, 5, 100);
    screen.feed(b"\x1b]2;title\x1b\\\x1b[31m\x1bP1$q\x1b\\done \xe2\x82\xac");
    assert!(screen.checkpoint().is_some());
}

#[test]
fn bytes_that_are_not_a_checkpoint_load_as_nothing() {
    let mut screen = Screen::new(40, 10, 100);
    screen.feed(&rich());
    let good = screen.checkpoint().expect("a checkpoint");
    let mut flipped = good.clone();
    let mid = flipped.len() / 2;
    flipped[mid] ^= 0x55;
    for (what, bytes) in [
        ("empty", Vec::new()),
        ("garbage", b"not a checkpoint at all".to_vec()),
        ("truncated", good[..good.len() / 2].to_vec()),
        ("a flipped byte", flipped),
        ("no magic", good[13..].to_vec()),
    ] {
        assert!(Screen::from_checkpoint(&bytes, 100).is_none(), "{what}");
    }
    assert!(Screen::from_checkpoint(&good, 100).is_some());
}

/// Not an assertion: the size and cost a checkpoint has for a Claude turn, printed for the report.
#[test]
fn what_a_checkpoint_costs_for_a_claude_turn() {
    for (name, stream) in [("default", DEFAULT_TURN), ("fullscreen", FULLSCREEN_TURN)] {
        let mut screen = Screen::new(100, 30, SCROLLBACK);
        for _ in 0..40 {
            screen.feed(stream);
        }
        let started = Instant::now();
        let saved = screen.checkpoint().expect("a checkpoint");
        let took = started.elapsed();
        let started = Instant::now();
        Screen::from_checkpoint(&saved, SCROLLBACK).expect("loads");
        eprintln!(
            "{name}: {} history rows, {} bytes, written in {took:?}, loaded in {:?}",
            screen.history_rows(),
            saved.len(),
            started.elapsed()
        );
    }
}
