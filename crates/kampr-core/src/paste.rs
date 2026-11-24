const START: &str = "\u{1b}[200~";
const END: &str = "\u{1b}[201~";

pub fn bracketed(text: &str) -> String {
    format!("{START}{text}{END}")
}

/// The text inside one bracketed paste, or `None` for anything else: a client marks a paste by
/// sending it framed, so a node that predates the mark still writes it framed (docs/04 `input`).
pub fn unbracketed(text: &str) -> Option<&str> {
    text.strip_prefix(START)?.strip_suffix(END)
}

/// What a terminal writes for a paste: framed for a program that turned 2004 on, and otherwise the
/// keys a person would have typed, every line ending as the Enter key does.
pub fn framed(text: &str, bracketed_paste: bool) -> Vec<u8> {
    if bracketed_paste {
        return bracketed(text).into_bytes();
    }
    text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_paste_is_framed_for_2004_and_typed_line_by_line_without_it() {
        assert_eq!(framed("a\nb", true), b"\x1b[200~a\nb\x1b[201~");
        assert_eq!(framed("a\r\nb\nc", false), b"a\rb\rc");
        assert_eq!(unbracketed(&bracketed("x\ny")), Some("x\ny"));
        for not_one in ["plain", "\u{1b}[200~open", "\u{1b}[200~a\u{1b}[201~tail"] {
            assert_eq!(unbracketed(not_one), None, "{not_one:?}");
        }
    }
}
