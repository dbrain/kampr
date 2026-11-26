#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Step {
    Interrupt,
    Submit(String),
}

/// What a composer types into a conversation, gathered into the one message it is.
///
/// The composer sends its text and then `"\r"` as two inputs, so the line is held until the
/// carriage return. Escape sequences — arrows, bracketed paste framing — have no meaning here
/// and are dropped; ctrl-c and a lone escape interrupt.
#[derive(Debug, Clone, Default)]
pub struct Line {
    text: String,
}

impl Line {
    pub fn feed(&mut self, input: &str) -> Vec<Step> {
        let mut steps = Vec::new();
        let mut chars = input.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                '\u{3}' => {
                    self.text.clear();
                    steps.push(Step::Interrupt);
                }
                '\u{1b}' => match chars.next() {
                    None => {
                        self.text.clear();
                        steps.push(Step::Interrupt);
                    }
                    Some('[') => {
                        for c in chars.by_ref() {
                            if ('\u{40}'..='\u{7e}').contains(&c) {
                                break;
                            }
                        }
                    }
                    Some('O') => {
                        chars.next();
                    }
                    Some(_) => {}
                },
                '\r' => {
                    let text = std::mem::take(&mut self.text);
                    if !text.trim().is_empty() {
                        steps.push(Step::Submit(text));
                    }
                }
                '\u{7f}' | '\u{8}' => {
                    self.text.pop();
                }
                '\n' | '\t' => self.text.push(c),
                c if c.is_control() => {}
                c => self.text.push(c),
            }
        }
        steps
    }

    pub fn keys(&mut self, keys: &[String]) -> Vec<Step> {
        let mut steps = Vec::new();
        for key in keys {
            // The legacy bytes for the chord are a plain enter, and a plain enter is what the
            // composer reads as submit: the named key must carry the difference.
            if key == "shift+enter" {
                steps.extend(self.feed("\n"));
                continue;
            }
            if let Some(bytes) = crate::keys::encode(key) {
                steps.extend(self.feed(&String::from_utf8_lossy(&bytes)));
            }
        }
        steps
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_composer_line_is_one_message() {
        let cases: &[(&[&str], &[Step])] = &[
            (&["run the tests", "\r"], &[Step::Submit("run the tests".into())]),
            (&["ls\r"], &[Step::Submit("ls".into())]),
            (
                &["\u{1b}[200~two\nlines\u{1b}[201~", "\r"],
                &[Step::Submit("two\nlines".into())],
            ),
            (&["abc\u{1b}[D\u{7f}", "\r"], &[Step::Submit("ab".into())]),
            (&["\u{3}"], &[Step::Interrupt]),
            (&["half", "\u{1b}"], &[Step::Interrupt]),
            (&["\r"], &[]),
            (&["\u{1b}[A", "\u{1b}OB"], &[]),
        ];
        for (inputs, want) in cases {
            let mut line = Line::default();
            let got: Vec<Step> = inputs.iter().flat_map(|i| line.feed(i)).collect();
            assert_eq!(&got, want, "{inputs:?}");
        }
    }

    #[test]
    fn named_keys_are_read_as_the_bytes_they_send() {
        let mut line = Line::default();
        let keys = |names: &[&str]| names.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        assert_eq!(
            line.keys(&keys(&["h", "i", "Enter"])),
            [Step::Submit("hi".into())]
        );
        assert_eq!(line.keys(&keys(&["ctrl+c"])), [Step::Interrupt]);
        assert_eq!(line.keys(&keys(&["Escape"])), [Step::Interrupt]);
        assert_eq!(line.keys(&keys(&["Up", "Left"])), []);
    }

    #[test]
    fn a_shift_enter_is_a_line_break_and_not_a_submit() {
        // The legacy bytes for the chord are a plain enter, and a plain enter is what the
        // composer reads as submit: the named key must carry the difference.
        let mut line = Line::default();
        let keys = |names: &[&str]| names.iter().map(|k| k.to_string()).collect::<Vec<_>>();
        assert_eq!(
            line.keys(&keys(&["a", "shift+enter", "b", "Enter"])),
            [Step::Submit("a\nb".into())],
            "the line break ran the message"
        );
    }
}
