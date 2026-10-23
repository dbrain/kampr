use crate::Screen;
use std::io::Write as _;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

pub(super) const RESET_MODES: &[u8] = b"\x1b[0m\x1b[?25h\x1b[?1l\x1b>\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1004l\x1b[?1005l\x1b[?1006l\x1b[<99u\x1b[0 q";

fn screen_rows(screen: &Screen) -> Vec<String> {
    let text = screen.scrollback(Some(u32::MAX)).text;
    text.lines().map(str::to_string).collect()
}

fn switch(out: &mut Vec<u8>, host_alt: &mut bool, want: bool) {
    if *host_alt != want {
        out.extend_from_slice(if want { b"\x1b[?1049h" } else { b"\x1b[?1049l" });
        *host_alt = want;
    }
}

/// Pushes the session's history through the host first, so a terminal with its own scrollback
/// (herdr's, #577) holds what the program printed before this attach.
pub(super) fn first(screen: &Screen, host_alt: &mut bool) -> Vec<u8> {
    let mut out = b"\x1b[?25l\x1b[0m\x1b[H\x1b[2J".to_vec();
    let back = screen.scrollback(None);
    let lines: Vec<&str> = back.text.lines().collect();
    let history = screen.history_rows() as usize;
    let shown = if screen.alternate_screen() {
        &lines[..history.min(lines.len())]
    } else {
        &lines[..]
    };
    if !shown.is_empty() {
        out.extend_from_slice(shown.join("\r\n").as_bytes());
        out.extend_from_slice(b"\r\n");
    }
    out.extend(full(screen, host_alt));
    out
}

pub(super) fn full(screen: &Screen, host_alt: &mut bool) -> Vec<u8> {
    let mut out = Vec::new();
    switch(&mut out, host_alt, screen.alternate_screen());
    out.extend(screen.frame_full());
    out.extend_from_slice(b"\x1b[0m");
    out.extend(screen.input_modes());
    out
}

pub(super) fn cropped(screen: &Screen, host_alt: &mut bool, tty: (u16, u16), holder: &str) -> Vec<u8> {
    let (tc, tr) = tty;
    let (pc, pr) = screen.size();
    let mut out = Vec::new();
    out.extend_from_slice(b"\x1b[?25l");
    switch(&mut out, host_alt, screen.alternate_screen());
    let banner = format!(" kampr: {pc}x{pr} is held by {holder}; this terminal is {tc}x{tr} ");
    out.extend_from_slice(b"\x1b[H\x1b[0;7m\x1b[2K");
    out.extend_from_slice(crop(&banner, tc).as_bytes());
    out.extend_from_slice(b"\x1b[0m");
    let rows = tr.saturating_sub(1).min(pr);
    for (r, line) in screen_rows(screen).iter().take(rows as usize).enumerate() {
        let _ = write!(out, "\x1b[{};1H\x1b[2K", r + 2);
        out.extend_from_slice(crop(line, tc).as_bytes());
    }
    if rows + 1 < tr {
        let _ = write!(out, "\x1b[{};1H\x1b[J", rows + 2);
    }
    let (col, row, visible) = screen.cursor();
    if row < rows && col < tc {
        let _ = write!(out, "\x1b[{};{}H", row + 2, col + 1);
        if visible {
            out.extend_from_slice(b"\x1b[?25h");
        }
    }
    out
}

pub(super) fn last(screen: &Screen, host_alt: &mut bool) -> Vec<u8> {
    let mut out = Vec::new();
    switch(&mut out, host_alt, false);
    out.extend_from_slice(RESET_MODES);
    if screen.alternate_screen() || screen.cursor().0 > 0 {
        out.extend_from_slice(b"\r\n");
    }
    out
}

/// Keeps every SGR and link and drops what falls past `cols` columns: `styled_row` writes nothing
/// else.
fn crop(styled: &str, cols: u16) -> String {
    let mut out = String::with_capacity(styled.len());
    let mut left = cols as usize;
    let mut rest = styled;
    let mut linked = false;
    while !rest.is_empty() {
        if let Some(link) = rest.strip_prefix("\x1b]8;;") {
            let end = link.find("\x1b\\").map_or(link.len(), |i| i + 2);
            linked = !link.starts_with('\x1b');
            out.push_str(&rest[..5 + end]);
            rest = &link[end..];
            continue;
        }
        if let Some(seq) = rest.strip_prefix("\x1b[") {
            let end = seq
                .find(|c: char| ('\x40'..='\x7e').contains(&c))
                .map_or(seq.len(), |i| i + 1);
            out.push_str(&rest[..2 + end]);
            rest = &seq[end..];
            continue;
        }
        let text_end = rest.find('\x1b').unwrap_or(rest.len());
        for cluster in rest[..text_end].graphemes(true) {
            let width = cluster.width();
            if width > left {
                left = 0;
                break;
            }
            left -= width;
            out.push_str(cluster);
        }
        rest = &rest[text_end..];
    }
    out.push_str("\x1b[0m");
    if linked {
        out.push_str("\x1b]8;;\x1b\\");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::crop;

    #[test]
    fn crop_counts_columns_not_bytes_and_keeps_styles() {
        for (styled, cols, want) in [
            ("hello world", 5, "hello\x1b[0m"),
            ("\x1b[1mab\x1b[0mcd", 3, "\x1b[1mab\x1b[0mc\x1b[0m"),
            ("漢字です", 5, "漢字\x1b[0m"),
            ("short", 80, "short\x1b[0m"),
            (
                "\x1b]8;;https://a.example\x1b\\link\x1b]8;;\x1b\\ after",
                6,
                "\x1b]8;;https://a.example\x1b\\link\x1b]8;;\x1b\\ a\x1b[0m",
            ),
            (
                "\x1b]8;;https://a.example\x1b\\a long link\x1b]8;;\x1b\\",
                3,
                "\x1b]8;;https://a.example\x1b\\a l\x1b]8;;\x1b\\\x1b[0m",
            ),
            (
                "\x1b]8;;https://a.example\x1b\\cut",
                2,
                "\x1b]8;;https://a.example\x1b\\cu\x1b[0m\x1b]8;;\x1b\\",
            ),
        ] {
            assert_eq!(crop(styled, cols), want, "{styled:?} at {cols}");
        }
    }
}
