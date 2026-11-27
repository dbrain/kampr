use kampr_core::{MouseTracking, PaneModes};
use kampr_native::Screen;

fn after(output: &[&[u8]]) -> PaneModes {
    let mut screen = Screen::new(80, 24, 100);
    for chunk in output {
        screen.feed(chunk);
    }
    screen.modes()
}

fn mode(mouse: MouseTracking, sgr: bool, alt_scroll: bool, app_cursor: bool, alt_screen: bool) -> PaneModes {
    PaneModes {
        mouse,
        mouse_sgr: sgr,
        alt_scroll,
        app_cursor,
        alt_screen,
    }
}

#[test]
fn what_a_program_set_is_what_a_client_is_told_and_what_it_reset_is_gone() {
    use MouseTracking::*;
    let claude = b"\x1b[?1049h\x1b[?1000h\x1b[?1006h".as_slice();
    let less = b"\x1b[?1049h\x1b[?1h\x1b[?1007h".as_slice();
    for (what, output, want) in [
        (
            "a shell",
            vec![b"$ ls\r\n".as_slice()],
            mode(None, false, true, false, false),
        ),
        ("claude", vec![claude], mode(Press, true, true, false, true)),
        (
            "claude, drag and motion too",
            vec![claude, b"\x1b[?1002h\x1b[?1003h"],
            mode(Any, true, true, false, true),
        ),
        (
            "claude, drag",
            vec![claude, b"\x1b[?1002h"],
            mode(Drag, true, true, false, true),
        ),
        ("less", vec![less], mode(None, false, true, true, true)),
        (
            "less that turned alternate scroll off",
            vec![less, b"\x1b[?1007l"],
            mode(None, false, false, true, true),
        ),
        (
            "claude, quit",
            vec![claude, b"\x1b[?1006l\x1b[?1000l\x1b[?1049l"],
            mode(None, false, true, false, false),
        ),
        (
            "claude, mouse off but still on the screen",
            vec![claude, b"\x1b[?1000l"],
            mode(None, true, true, false, true),
        ),
    ] {
        assert_eq!(after(&output), want, "{what}");
    }
}

/// A real Claude Code 2.1.306 session (probe #587): its fullscreen boot sets every mouse mode, and a
/// turn drawn after it leaves them set.
#[test]
fn a_real_claude_sets_every_mouse_mode_at_boot_and_keeps_them_through_a_turn() {
    const TURN: &[u8] = include_bytes!("../fixtures/claude-fullscreen-turn.bin");
    let mut screen = Screen::new(100, 30, 100);
    screen.feed(TURN);
    assert_eq!(screen.modes(), mode(MouseTracking::Any, true, true, false, true));
}
