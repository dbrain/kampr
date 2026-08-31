//! The reply box the conversation view never had.
//!
//! Without one, every key typed at a conversation fell straight through to the pane's PTY: it
//! worked, and there was no box, no echo and no send affordance to say so — and a single character
//! that happened to match a pending prompt's option key answered the prompt instead of being typed.
//! The Compose client has had `Composer.kt` since W8; this is the same contract for a keyboard.

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget, Wrap};
use std::collections::HashMap;

use crate::theme::Theme;

/// Tall enough for a paragraph, short enough that the transcript above it is still a transcript.
const MOST_ROWS: u16 = 6;

#[derive(Debug, Default, Clone)]
struct Draft {
    text: String,
    /// A byte offset into `text`, always on a character boundary.
    cursor: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Typed {
    /// Not the composer's key. The caller decides what else may have it.
    Ignored,
    Changed,
    /// The draft, and the carriage return that follows it as its own message.
    Send(String),
}

#[derive(Debug, Default)]
pub struct Composer {
    drafts: HashMap<String, Draft>,
}

impl Composer {
    pub fn text(&self, pane: &str) -> &str {
        self.drafts.get(pane).map(|d| d.text.as_str()).unwrap_or_default()
    }

    /// **Whether a bare keystroke still belongs to a pending prompt.** An empty box means the
    /// operator has not started writing, so `1` is the answer to the question on screen; once
    /// there is a draft, `1` is a character in it. This is the whole of the fix for a composer
    /// that used to swallow prompt keys and a prompt that used to swallow typed ones.
    pub fn empty(&self, pane: &str) -> bool {
        self.text(pane).is_empty()
    }

    pub fn key(&mut self, pane: &str, key: KeyEvent) -> Typed {
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        let shift = key.modifiers.contains(KeyModifiers::SHIFT);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let draft = self.drafts.entry(pane.to_string()).or_default();
        match key.code {
            // Return sends and a modifier with it writes the second line, which is what every
            // agent CLI this targets does at its own prompt.
            KeyCode::Enter if alt || shift => {
                draft.insert('\n');
                Typed::Changed
            }
            KeyCode::Enter => {
                let text = draft.text.trim_end().to_string();
                if text.is_empty() {
                    // Never synthesise a submit into an empty box: a bare Enter at a conversation
                    // is a keystroke the pane's own prompt may be waiting for, and #43 is that the
                    // node decides whether one follows.
                    return Typed::Ignored;
                }
                self.drafts.remove(pane);
                Typed::Send(text)
            }
            KeyCode::Char(c) if !ctrl => {
                draft.insert(c);
                Typed::Changed
            }
            KeyCode::Backspace => {
                draft.backspace();
                Typed::Changed
            }
            KeyCode::Delete => {
                draft.delete();
                Typed::Changed
            }
            KeyCode::Left => {
                draft.step(false);
                Typed::Changed
            }
            KeyCode::Right => {
                draft.step(true);
                Typed::Changed
            }
            KeyCode::Home => {
                draft.cursor = 0;
                Typed::Changed
            }
            KeyCode::End => {
                draft.cursor = draft.text.len();
                Typed::Changed
            }
            // An empty box has nothing to give up, so esc is the router's — it is how a mode is
            // left, and a composer that swallowed it would strand the operator in one.
            KeyCode::Esc if !draft.text.is_empty() => {
                self.drafts.remove(pane);
                Typed::Changed
            }
            _ => Typed::Ignored,
        }
    }

    /// How many rows the box wants, given the width it will be drawn at.
    ///
    /// **None at all on a device that cannot write.** There is nothing to compose, and the row it
    /// would take is the one the chrome borrows to say `readonly` — two answers to the same
    /// question, one of them painted over the other.
    pub fn height(&self, pane: &str, width: u16, writes: bool) -> u16 {
        if !writes {
            return 0;
        }
        let inner = width.saturating_sub(2).max(1);
        let text = self.text(pane);
        if text.is_empty() {
            return 1;
        }
        let rows: usize = text
            .split('\n')
            .map(|line| line.chars().count().div_ceil(inner as usize).max(1))
            .sum();
        (rows as u16).clamp(1, MOST_ROWS)
    }

    pub fn render(&self, buf: &mut Buffer, area: Rect, pane: &str, theme: &Theme, writes: bool) {
        if area.width == 0 || area.height == 0 {
            return;
        }
        let text = self.text(pane);
        let body = match (writes, text.is_empty()) {
            (false, _) => return,
            (true, true) => Line::from(Span::styled(
                " type a reply · enter sends · alt+enter for a new line",
                Style::default().fg(theme.mute).bg(theme.bar),
            )),
            (true, false) => Line::from(vec![
                Span::styled(
                    " \u{203a} ",
                    Style::default()
                        .fg(theme.accent)
                        .bg(theme.bar)
                        .add_modifier(Modifier::BOLD),
                ),
                Span::styled(text, Style::default().fg(theme.text).bg(theme.bar)),
            ]),
        };
        Paragraph::new(body)
            .wrap(Wrap { trim: false })
            .style(Style::default().bg(theme.bar))
            .render(area, buf);
    }
}

impl Draft {
    fn insert(&mut self, c: char) {
        self.text.insert(self.cursor, c);
        self.cursor += c.len_utf8();
    }

    fn backspace(&mut self) {
        let Some(prev) = self.text[..self.cursor].chars().next_back() else {
            return;
        };
        self.cursor -= prev.len_utf8();
        self.text.remove(self.cursor);
    }

    fn delete(&mut self) {
        if self.cursor < self.text.len() {
            self.text.remove(self.cursor);
        }
    }

    fn step(&mut self, forward: bool) {
        match forward {
            true => {
                if let Some(next) = self.text[self.cursor..].chars().next() {
                    self.cursor += next.len_utf8();
                }
            }
            false => {
                if let Some(prev) = self.text[..self.cursor].chars().next_back() {
                    self.cursor -= prev.len_utf8();
                }
            }
        }
    }
}
