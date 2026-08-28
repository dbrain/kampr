use crate::keymap::{self, Action, Bind, Mode};
use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Nothing,
    Do(Action),
    /// Bytes for the focused pane, over `input` as `text`. Never a terminal stream.
    ToPane(String),
    /// The keymap on top of the stack changed; the footer has to be redrawn.
    Redrew,
}

/// A **mode stack**, not a prefix flag.
///
/// [`Mode::Prefix`] is one-shot: it is popped before the key it captured is looked up.
/// [`Mode::Copy`], [`Mode::Resize`] and [`Mode::Navigate`] are modal — they stay until `esc` or
/// `q` closes them, they take no prefix, and while one is open every key in it is kampr's rather
/// than the pane's (#289, #290).
#[derive(Debug, Clone)]
pub struct Router {
    stack: Vec<Mode>,
    prefix: KeyEvent,
}

impl Default for Router {
    fn default() -> Self {
        Self::with_prefix(keymap::HERDR_PREFIX)
    }
}

impl Router {
    pub fn with_prefix(prefix: KeyEvent) -> Self {
        Self {
            stack: vec![Mode::Pane],
            prefix,
        }
    }

    pub fn prefix(&self) -> KeyEvent {
        self.prefix
    }
    pub fn mode(&self) -> Mode {
        *self.stack.last().unwrap_or(&Mode::Pane)
    }

    pub fn modal(&self) -> bool {
        matches!(self.mode(), Mode::Copy | Mode::Resize | Mode::Navigate)
    }

    pub fn footer(&self) -> Option<&'static str> {
        keymap::footer(self.mode())
    }

    pub fn enter(&mut self, mode: Mode) {
        self.stack.push(mode);
    }

    pub fn leave(&mut self) {
        if self.stack.len() > 1 {
            self.stack.pop();
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Outcome {
        if key.kind == KeyEventKind::Release {
            return Outcome::Nothing;
        }
        match self.mode() {
            Mode::Pane => {
                if keymap::same(key, self.prefix) {
                    self.enter(Mode::Prefix);
                    return Outcome::Redrew;
                }
                encode(key).map_or(Outcome::Nothing, Outcome::ToPane)
            }
            Mode::Prefix => {
                self.leave();
                // `cat -v` printed `^B` once for `ctrl+b ctrl+b`, so the second prefix is
                // delivered as a byte and is never looked up as a binding (#290).
                if keymap::same(key, self.prefix) {
                    return encode(key).map_or(Outcome::Nothing, Outcome::ToPane);
                }
                match keymap::prefix(key) {
                    Some(bind) => self.apply(bind),
                    // Anything that is not a binding after the prefix goes to the pane.
                    None => encode(key).map_or(Outcome::Redrew, Outcome::ToPane),
                }
            }
            modal => match keymap::lookup(modal, key) {
                Some(bind) => self.apply(bind),
                // A modal keymap swallows the keyboard; an unbound key is not the pane's.
                None => Outcome::Nothing,
            },
        }
    }

    fn apply(&mut self, bind: Bind) -> Outcome {
        match bind {
            Bind::Do(action) => Outcome::Do(action),
            Bind::Enter(mode) => {
                self.enter(mode);
                Outcome::Redrew
            }
            Bind::Leave => {
                self.leave();
                Outcome::Redrew
            }
        }
    }
}

/// A paste, with the bracketing it has to supply itself: `pane.send_text` writes raw bytes with
/// no framing (#9), so a multi-line paste without `ESC[200~`/`ESC[201~` executes line by line in
/// a shell instead of arriving as one block.
pub fn bracketed(data: &str) -> String {
    format!("\u{1b}[200~{data}\u{1b}[201~")
}

/// One key as the bytes a PTY would have seen.
///
/// Home, End, PageUp, PageDown, Insert and Delete are **not** in herdr's key grammar (#8) and go
/// as their escape sequences here rather than through `keys`.
pub fn encode(key: KeyEvent) -> Option<String> {
    let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
    let alt = key.modifiers.contains(KeyModifiers::ALT);
    let base = match key.code {
        KeyCode::Char(c) => {
            if ctrl {
                control(c)?
            } else {
                c.to_string()
            }
        }
        KeyCode::Enter => "\r".into(),
        KeyCode::Tab => "\t".into(),
        KeyCode::BackTab => "\u{1b}[Z".into(),
        KeyCode::Backspace => "\u{7f}".into(),
        KeyCode::Esc => "\u{1b}".into(),
        KeyCode::Up => "\u{1b}[A".into(),
        KeyCode::Down => "\u{1b}[B".into(),
        KeyCode::Right => "\u{1b}[C".into(),
        KeyCode::Left => "\u{1b}[D".into(),
        KeyCode::Home => "\u{1b}[H".into(),
        KeyCode::End => "\u{1b}[F".into(),
        KeyCode::Insert => "\u{1b}[2~".into(),
        KeyCode::Delete => "\u{1b}[3~".into(),
        KeyCode::PageUp => "\u{1b}[5~".into(),
        KeyCode::PageDown => "\u{1b}[6~".into(),
        KeyCode::F(n) => function(n)?,
        _ => return None,
    };
    Some(match alt {
        true => format!("\u{1b}{base}"),
        false => base,
    })
}

fn control(c: char) -> Option<String> {
    let byte = match c {
        'a'..='z' => c as u8 - b'a' + 1,
        'A'..='Z' => c as u8 - b'A' + 1,
        '@' | ' ' => 0,
        '[' => 27,
        '\\' => 28,
        ']' => 29,
        '^' => 30,
        '_' | '?' => 31,
        _ => return None,
    };
    Some((byte as char).to_string())
}

fn function(n: u8) -> Option<String> {
    let text = match n {
        1 => "\u{1b}OP",
        2 => "\u{1b}OQ",
        3 => "\u{1b}OR",
        4 => "\u{1b}OS",
        5 => "\u{1b}[15~",
        6 => "\u{1b}[17~",
        7 => "\u{1b}[18~",
        8 => "\u{1b}[19~",
        9 => "\u{1b}[20~",
        10 => "\u{1b}[21~",
        11 => "\u{1b}[23~",
        12 => "\u{1b}[24~",
        _ => return None,
    };
    Some(text.into())
}
