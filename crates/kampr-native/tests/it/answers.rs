use kampr_native::Screen;

fn replies(setup: &[u8], query: &[u8]) -> String {
    let mut screen = Screen::new(80, 24, 0);
    screen.feed(setup);
    String::from_utf8(screen.feed(query).replies).unwrap()
}

#[test]
fn terminal_queries_are_answered_as_a_terminal_answers_them() {
    let version = format!("\x1bP>|kampr {}\x1b\\", env!("CARGO_PKG_VERSION"));
    let cases: &[(&str, &[u8], &[u8], &str)] = &[
        ("DA1, as herdr answers it (#88)", b"", b"\x1b[c", "\x1b[?62;22c"),
        ("DA1 with its parameter", b"", b"\x1b[0c", "\x1b[?62;22c"),
        ("DA2", b"", b"\x1b[>c", "\x1b[>0;0;0c"),
        ("XTVERSION", b"", b"\x1b[>0q", &version),
        ("DSR status", b"", b"\x1b[5n", "\x1b[0n"),
        ("DSR cursor", b"\x1b[3;7H", b"\x1b[6n", "\x1b[3;7R"),
        (
            "DSR cursor after text moved it",
            b"\x1b[2;1Hab",
            b"\x1b[6n",
            "\x1b[2;3R",
        ),
        (
            "DSR cursor in the pending-wrap column",
            b"\x1b[1;80Hx",
            b"\x1b[6n",
            "\x1b[1;80R",
        ),
        ("DECXCPR", b"\x1b[4;2H", b"\x1b[?6n", "\x1b[?4;2;1R"),
        ("DECRQM, a shown cursor", b"", b"\x1b[?25$p", "\x1b[?25;1$y"),
        (
            "DECRQM, a hidden cursor",
            b"\x1b[?25l",
            b"\x1b[?25$p",
            "\x1b[?25;2$y",
        ),
        (
            "DECRQM, bracketed paste",
            b"\x1b[?2004h",
            b"\x1b[?2004$p",
            "\x1b[?2004;1$y",
        ),
        (
            "DECRQM, the alternate screen",
            b"\x1b[?1049h",
            b"\x1b[?1049$p",
            "\x1b[?1049;1$y",
        ),
        (
            "DECRQM, SGR mouse",
            b"\x1b[?1006h",
            b"\x1b[?1006$p",
            "\x1b[?1006;1$y",
        ),
        (
            "DECRQM, synchronized output is not claimed",
            b"",
            b"\x1b[?2026$p",
            "\x1b[?2026;0$y",
        ),
        ("DECRQM, an ANSI mode", b"", b"\x1b[4$p", "\x1b[4;0$y"),
        ("text area size", b"", b"\x1b[18t", "\x1b[8;24;80t"),
        ("DECRQM, autowrap", b"\x1b[?7l", b"\x1b[?7$p", "\x1b[?7;2$y"),
        (
            "DECRQM, focus events",
            b"\x1b[?1004h",
            b"\x1b[?1004$p",
            "\x1b[?1004;1$y",
        ),
        ("kitty keyboard, nothing pushed", b"", b"\x1b[?u", "\x1b[?0u"),
        ("kitty keyboard, pushed", b"\x1b[>1u", b"\x1b[?u", "\x1b[?1u"),
        (
            "kitty keyboard, pushed twice",
            b"\x1b[>1u\x1b[>11u",
            b"\x1b[?u",
            "\x1b[?11u",
        ),
        (
            "kitty keyboard, popped",
            b"\x1b[>1u\x1b[>11u\x1b[<u",
            b"\x1b[?u",
            "\x1b[?1u",
        ),
        (
            "kitty keyboard, popped past empty",
            b"\x1b[>1u\x1b[<5u",
            b"\x1b[?u",
            "\x1b[?0u",
        ),
        (
            "kitty keyboard, set",
            b"\x1b[>1u\x1b[=5;1u",
            b"\x1b[?u",
            "\x1b[?5u",
        ),
        (
            "kitty keyboard, set as a union",
            b"\x1b[>1u\x1b[=8;2u",
            b"\x1b[?u",
            "\x1b[?9u",
        ),
        (
            "kitty keyboard, the alternate screen keeps its own",
            b"\x1b[>1u\x1b[?1049h\x1b[>3u\x1b[?1049l",
            b"\x1b[?u",
            "\x1b[?1u",
        ),
        (
            "OSC 10, the clients' ink, BEL for BEL",
            b"",
            b"\x1b]10;?\x07",
            "\x1b]10;rgb:dddd/e3e3/eeee\x07",
        ),
        (
            "OSC 11, the clients' ground, ST for ST",
            b"",
            b"\x1b]11;?\x1b\\",
            "\x1b]11;rgb:0b0b/0d0d/1212\x1b\\",
        ),
        (
            "OSC 12, the cursor, which the clients draw in the ink",
            b"",
            b"\x1b]12;?\x07",
            "\x1b]12;rgb:dddd/e3e3/eeee\x07",
        ),
        (
            "OSC 10 asking for the next one too",
            b"",
            b"\x1b]10;?;?\x07",
            "\x1b]10;rgb:dddd/e3e3/eeee\x07\x1b]11;rgb:0b0b/0d0d/1212\x07",
        ),
        (
            "OSC 11 after the program set it",
            b"\x1b]11;rgb:10/20/30\x07",
            b"\x1b]11;?\x07",
            "\x1b]11;rgb:1010/2020/3030\x07",
        ),
        (
            "OSC 4, a themed slot, the cube and the grey ramp",
            b"",
            b"\x1b]4;1;?;196;?\x07\x1b]4;244;?\x07",
            "\x1b]4;1;rgb:f2f2/7070/7a7a\x07\x1b]4;196;rgb:ffff/0000/0000\x07\x1b]4;244;rgb:8080/8080/8080\x07",
        ),
        (
            "OSC 7501, Claude's own, answered as the probe that found it did (#600)",
            b"",
            b"\x1b]7501;?\x07",
            "\x1b]7501;?\x1b\\",
        ),
        (
            "OSC 7501, a report is not a question",
            b"",
            b"\x1b]7501;state=idle:app=claude-code\x07",
            "",
        ),
        (
            "Claude's boot questions, in order, 7501's answer before DA1's (#587, #600)",
            b"",
            b"\x1b[>0q\x1b[?u\x1b]7501;?\x07\x1b[c",
            &format!("{version}\x1b[?0u\x1b]7501;?\x1b\\\x1b[?62;22c"),
        ),
        (
            "a question answered at the point it was asked",
            b"",
            b"\x1b[2;2H\x1b[6nab\x1b[6n",
            "\x1b[2;2R\x1b[2;4R",
        ),
        (
            "plain output asks nothing",
            b"",
            b"hello \x1b[31mworld\x1b[0m\r\n",
            "",
        ),
    ];
    for (name, setup, query, want) in cases {
        assert_eq!(replies(setup, query), *want, "{name}");
    }
}

#[test]
fn a_query_split_across_reads_is_answered_once_it_is_whole() {
    let mut screen = Screen::new(80, 24, 0);
    assert!(screen.feed(b"\x1b[5;9H\x1b[").replies.is_empty());
    assert_eq!(screen.feed(b"6n").replies, b"\x1b[5;9R");
    assert!(
        screen.feed(b"").replies.is_empty(),
        "an answer is handed out once"
    );
}

#[test]
fn the_title_is_the_last_one_set() {
    let mut screen = Screen::new(80, 24, 0);
    assert_eq!(screen.title(), "");
    screen.feed(b"\x1b]0;first\x07\x1b]2;second\x1b\\");
    assert_eq!(screen.title(), "second");
}

#[test]
fn the_input_modes_handed_to_a_host_terminal_put_it_in_the_programs_modes() {
    let mut program = Screen::new(80, 24, 0);
    program.feed(b"\x1b[?1h\x1b=\x1b[?2004h\x1b[?1002h\x1b[?1006h\x1b[?1004h\x1b[>1u\x1b[5 q");
    let mut host = Screen::new(80, 24, 0);
    host.feed(b"\x1b[?1000h\x1b[?1005h\x1b[>9u");
    host.feed(&program.input_modes());
    assert_eq!(host.input_modes(), program.input_modes());
    assert_eq!(host.keyboard_flags(), 1);
    let ask = |s: &mut Screen, mode: &str| {
        String::from_utf8(s.feed(format!("\x1b[?{mode}$p").as_bytes()).replies).unwrap()
    };
    for (mode, want) in [
        ("1", 1),
        ("2004", 1),
        ("1002", 1),
        ("1006", 1),
        ("1004", 1),
        ("1000", 2),
        ("1005", 2),
    ] {
        assert_eq!(
            ask(&mut host, mode),
            format!("\x1b[?{mode};{want}$y"),
            "mode {mode}"
        );
    }

    host.feed(&Screen::new(80, 24, 0).input_modes());
    assert_eq!(
        host.keyboard_flags(),
        0,
        "a program with no flags clears the host's"
    );
    assert_eq!(ask(&mut host, "1002"), "\x1b[?1002;2$y");
}
