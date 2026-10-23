//! herdr's key grammar (#7) turned into the bytes a terminal would send, for the providers that
//! write to a PTY themselves rather than handing the names to herdr.

const SHIFT: u8 = 1;
const ALT: u8 = 2;
const CTRL: u8 = 4;

const DISAMBIGUATE: u8 = 1;
const ALL_AS_ESCAPES: u8 = 8;

pub fn encode(key: &str) -> Option<Vec<u8>> {
    encode_for(key, 0)
}

/// As a terminal sends `key` to a program that has pushed these kitty keyboard flags. Only the
/// first and fourth change what a press sends: the event types, alternate keys and associated
/// text the others ask for are optional fields of the same sequences.
pub fn encode_for(key: &str, kitty: u8) -> Option<Vec<u8>> {
    let (mods, name) = split(key)?;
    let lower = name.to_ascii_lowercase();
    if kitty & (DISAMBIGUATE | ALL_AS_ESCAPES) != 0
        && let Some(bytes) = kitty_key(mods, &lower, name, kitty)
    {
        return Some(bytes);
    }
    let csi_mod = |tail: &str| match mods {
        0 => format!("\x1b[{tail}").into_bytes(),
        m => {
            let (num, end) = tail.split_at(tail.len() - 1);
            let num = if num.is_empty() { "1" } else { num };
            format!("\x1b[{num};{}{end}", m + 1).into_bytes()
        }
    };
    let plain = |bytes: &[u8]| -> Vec<u8> {
        match mods & ALT {
            0 => bytes.to_vec(),
            _ => [&[0x1b], bytes].concat(),
        }
    };
    Some(match lower.as_str() {
        "up" => csi_mod("A"),
        "down" => csi_mod("B"),
        "right" => csi_mod("C"),
        "left" => csi_mod("D"),
        "home" => csi_mod("H"),
        "end" => csi_mod("F"),
        "insert" => csi_mod("2~"),
        "delete" | "del" => csi_mod("3~"),
        "pageup" | "pgup" => csi_mod("5~"),
        "pagedown" | "pgdn" => csi_mod("6~"),
        "tab" if mods & SHIFT != 0 => b"\x1b[Z".to_vec(),
        "tab" => plain(b"\t"),
        "enter" | "return" => plain(b"\r"),
        "escape" | "esc" => plain(b"\x1b"),
        "space" if mods & CTRL != 0 => plain(&[0]),
        "space" => plain(b" "),
        "backspace" | "bs" if mods & CTRL != 0 => plain(&[0x08]),
        "backspace" | "bs" => plain(&[0x7f]),
        f if f.len() > 1 && f.starts_with('f') => function(f[1..].parse().ok()?, mods)?,
        _ => {
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            plain(&character(c, mods)?)
        }
    })
}

/// `None` where the legacy bytes are also the kitty ones: unmodified text, Enter, Tab and
/// Backspace under flag 1 alone, and the cursor and editing keys under any flags.
fn kitty_key(mods: u8, lower: &str, name: &str, kitty: u8) -> Option<Vec<u8>> {
    let all = kitty & ALL_AS_ESCAPES != 0;
    let csi_u = |code: u32| match mods {
        0 => format!("\x1b[{code}u").into_bytes(),
        m => format!("\x1b[{code};{}u", m + 1).into_bytes(),
    };
    let control = |code: u32| (mods != 0 || all).then(|| csi_u(code));
    match lower {
        "escape" | "esc" => Some(csi_u(27)),
        "enter" | "return" => control(13),
        "tab" => control(9),
        "backspace" | "bs" => control(127),
        "space" => (mods & !SHIFT != 0 || all).then(|| csi_u(32)),
        // F3's CSI form would be read as a cursor position report.
        "f1" | "f2" | "f3" | "f4" => {
            let (num, end) = match lower {
                "f1" => ("1", "P"),
                "f2" => ("1", "Q"),
                "f3" => ("13", "~"),
                _ => ("1", "S"),
            };
            Some(match mods {
                0 if end == "~" => format!("\x1b[{num}~").into_bytes(),
                0 => format!("\x1b[{end}").into_bytes(),
                m => format!("\x1b[{num};{}{end}", m + 1).into_bytes(),
            })
        }
        _ => {
            let mut chars = name.chars();
            let c = chars.next()?;
            if chars.next().is_some() {
                return None;
            }
            (mods & !SHIFT != 0 || all).then(|| csi_u(c.to_ascii_lowercase() as u32))
        }
    }
}

fn split(key: &str) -> Option<(u8, &str)> {
    if key.chars().count() == 1 {
        return Some((0, key));
    }
    if let Some(head) = key.strip_suffix("++") {
        return Some((modifiers(head)?, "+"));
    }
    let Some((head, name)) = key.rsplit_once('+') else {
        return Some((0, key));
    };
    if name.is_empty() {
        return None;
    }
    Some((modifiers(head)?, name))
}

fn modifiers(head: &str) -> Option<u8> {
    head.split('+').filter(|m| !m.is_empty()).try_fold(0, |acc, m| {
        Some(
            acc | match m.to_ascii_lowercase().as_str() {
                "shift" => SHIFT,
                "alt" | "meta" | "option" => ALT,
                "ctrl" | "control" => CTRL,
                _ => return None,
            },
        )
    })
}

fn function(n: u8, mods: u8) -> Option<Vec<u8>> {
    let tail = match n {
        1 => "P",
        2 => "Q",
        3 => "R",
        4 => "S",
        5 => "15~",
        6 => "17~",
        7 => "18~",
        8 => "19~",
        9 => "20~",
        10 => "21~",
        11 => "23~",
        12 => "24~",
        13 => "25~",
        _ => return None,
    };
    Some(match (n, mods) {
        (1..=4, 0) => format!("\x1bO{tail}").into_bytes(),
        (1..=4, m) => format!("\x1b[1;{}{tail}", m + 1).into_bytes(),
        (_, 0) => format!("\x1b[{tail}").into_bytes(),
        (_, m) => {
            let num = &tail[..tail.len() - 1];
            format!("\x1b[{num};{}~", m + 1).into_bytes()
        }
    })
}

fn character(c: char, mods: u8) -> Option<Vec<u8>> {
    let c = match mods & SHIFT {
        0 => c,
        _ => c.to_ascii_uppercase(),
    };
    if mods & CTRL == 0 {
        return Some(c.to_string().into_bytes());
    }
    let b = match c.to_ascii_lowercase() {
        l @ 'a'..='z' => l as u8 & 0x1f,
        '@' | '2' => 0,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    };
    Some(vec![b])
}

#[cfg(test)]
mod tests {
    use super::{encode, encode_for};

    #[test]
    fn under_kitty_flags_keys_are_sent_as_a_kitty_terminal_sends_them() {
        let disambiguate = 1;
        let all = 1 | 8;
        for (key, flags, want) in [
            ("Escape", disambiguate, &b"\x1b[27u"[..]),
            ("alt+escape", disambiguate, b"\x1b[27;3u"),
            ("Enter", disambiguate, b"\r"),
            ("shift+Enter", disambiguate, b"\x1b[13;2u"),
            ("alt+Enter", disambiguate, b"\x1b[13;3u"),
            ("ctrl+Enter", disambiguate, b"\x1b[13;5u"),
            ("Tab", disambiguate, b"\t"),
            ("shift+Tab", disambiguate, b"\x1b[9;2u"),
            ("Backspace", disambiguate, b"\x7f"),
            ("ctrl+Backspace", disambiguate, b"\x1b[127;5u"),
            ("a", disambiguate, b"a"),
            ("shift+a", disambiguate, b"A"),
            ("ctrl+c", disambiguate, b"\x1b[99;5u"),
            ("ctrl+shift+c", disambiguate, b"\x1b[99;6u"),
            ("alt+x", disambiguate, b"\x1b[120;3u"),
            ("ctrl+[", disambiguate, b"\x1b[91;5u"),
            ("ctrl+space", disambiguate, b"\x1b[32;5u"),
            ("Space", disambiguate, b" "),
            ("Up", disambiguate, b"\x1b[A"),
            ("ctrl+left", disambiguate, b"\x1b[1;5D"),
            ("PageUp", disambiguate, b"\x1b[5~"),
            ("F1", disambiguate, b"\x1b[P"),
            ("F3", disambiguate, b"\x1b[13~"),
            ("shift+F3", disambiguate, b"\x1b[13;2~"),
            ("ctrl+F1", disambiguate, b"\x1b[1;5P"),
            ("F5", disambiguate, b"\x1b[15~"),
            ("Enter", all, b"\x1b[13u"),
            ("Tab", all, b"\x1b[9u"),
            ("Backspace", all, b"\x1b[127u"),
            ("a", all, b"\x1b[97u"),
            ("shift+a", all, b"\x1b[97;2u"),
            ("Space", all, b"\x1b[32u"),
            ("shift+Enter", 5, b"\x1b[13;2u"),
            ("ctrl+c", 5, b"\x1b[99;5u"),
            ("Escape", 0, b"\x1b"),
            ("shift+Enter", 0, b"\r"),
            ("ctrl+c", 0, b"\x03"),
        ] {
            assert_eq!(
                encode_for(key, flags).as_deref(),
                Some(want),
                "{key} under {flags}"
            );
        }
        assert_eq!(encode_for("nosuchkey", 1), None);
    }

    #[test]
    fn herdrs_key_names_become_what_a_terminal_sends() {
        for (key, want) in [
            ("Enter", &b"\r"[..]),
            ("enter", b"\r"),
            ("return", b"\r"),
            ("Escape", b"\x1b"),
            ("esc", b"\x1b"),
            ("Tab", b"\t"),
            ("shift+tab", b"\x1b[Z"),
            ("Space", b" "),
            ("ctrl+space", b"\x00"),
            ("Backspace", b"\x7f"),
            ("BS", b"\x7f"),
            ("Up", b"\x1b[A"),
            ("down", b"\x1b[B"),
            ("Right", b"\x1b[C"),
            ("left", b"\x1b[D"),
            ("ctrl+left", b"\x1b[1;5D"),
            ("shift+alt+up", b"\x1b[1;4A"),
            ("PageUp", b"\x1b[5~"),
            ("ctrl+pagedown", b"\x1b[6;5~"),
            ("F1", b"\x1bOP"),
            ("shift+F2", b"\x1b[1;2Q"),
            ("F5", b"\x1b[15~"),
            ("F13", b"\x1b[25~"),
            ("ctrl+F12", b"\x1b[24;5~"),
            ("a", b"a"),
            ("shift+a", b"A"),
            ("ctrl+c", b"\x03"),
            ("ctrl+d", b"\x04"),
            ("ctrl+[", b"\x1b"),
            ("alt+x", b"\x1bx"),
            ("alt+enter", b"\x1b\r"),
            ("ctrl+alt+c", b"\x1b\x03"),
            ("+", b"+"),
            ("alt++", b"\x1b+"),
            ("1", b"1"),
        ] {
            assert_eq!(encode(key).as_deref(), Some(want), "{key}");
        }
        for key in ["", "F14", "hyper+a", "ctrl+", "nosuchkey", "ctrl+é"] {
            assert_eq!(encode(key), None, "{key}");
        }
    }
}
