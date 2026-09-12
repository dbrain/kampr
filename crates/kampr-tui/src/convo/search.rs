//! Searching the transcript, which is not the scrollback.
//!
//! `find` searches what herdr retains of what was *drawn* on a pane and answers in rows; this
//! searches what the harness *recorded* and answers in turns, which outlives any number of
//! `clear`s. The node is asked because a conversation opens on a page of the newest turns and
//! pages backwards only as the reader reaches the top — so a client searching what it holds is
//! searching that page, and cannot know by how much its count is short.

use crate::find::{Prompt, Typing};
use crate::theme::Theme;
use kampr_core::wire::ConvoMatch;

#[derive(Debug, Default)]
pub struct Search {
    prompt: Option<Prompt>,
    result: Option<Result>,
}

#[derive(Debug)]
struct Result {
    pane: String,
    query: String,
    matches: Vec<ConvoMatch>,
    total: u32,
    at: usize,
    /// Whether the node answered. A search this client ran over the turns it holds says `so far`,
    /// because a count that covers the page the reader opened on must never read as though it
    /// covered the conversation.
    whole: bool,
    /// Nothing has come back yet, which is the one state silence cannot be told from.
    waiting: bool,
}

pub enum Took {
    Ignored,
    Consumed,
    /// Run this over the pane's transcript.
    Search(String),
}

impl Search {
    pub fn open(&mut self) {
        self.prompt = Some(Prompt::new(false));
    }

    pub fn key(&mut self, key: crossterm::event::KeyEvent) -> Took {
        let Some(prompt) = self.prompt.as_mut() else {
            return Took::Ignored;
        };
        match prompt.key(key) {
            Typing::Changed => Took::Consumed,
            Typing::Cancelled => {
                self.prompt = None;
                Took::Consumed
            }
            Typing::Run(query) => {
                self.prompt = None;
                Took::Search(query)
            }
        }
    }

    /// The question is out and the answer is not in. The row says so rather than reading as a
    /// search that found nothing.
    pub fn asking(&mut self, pane: &str, query: &str) {
        self.result = Some(Result {
            pane: pane.into(),
            query: query.into(),
            matches: Vec::new(),
            total: 0,
            at: 0,
            whole: true,
            waiting: true,
        });
    }

    pub fn found(&mut self, pane: &str, query: &str, matches: Vec<ConvoMatch>, total: u32, whole: bool) {
        self.result = Some(Result {
            pane: pane.into(),
            query: query.into(),
            total,
            at: 0,
            matches,
            whole,
            waiting: false,
        });
    }

    pub fn close(&mut self) {
        self.prompt = None;
        self.result = None;
    }

    /// Where the search is standing, without moving it.
    pub fn at(&self, pane: &str) -> Option<&ConvoMatch> {
        let result = self.result.as_ref().filter(|r| r.pane == pane)?;
        result.matches.get(result.at)
    }

    /// **`None` rather than a wrap-around when there is nothing to step.** A search that matched
    /// nothing and a search that has not come back yet both leave the operator pressing `n` at
    /// nothing, and the row is what tells them apart.
    pub fn step(&mut self, pane: &str, forward: bool) -> Option<&ConvoMatch> {
        let result = self.result.as_mut().filter(|r| r.pane == pane)?;
        let n = result.matches.len();
        if n == 0 {
            return None;
        }
        result.at = match forward {
            true => (result.at + 1) % n,
            false => (result.at + n - 1) % n,
        };
        result.matches.get(result.at)
    }

    /// The one row this surface has: the prompt while a query is being typed, the standing result
    /// after. `None` is a surface with nothing to say, which leaves the row to whatever else the
    /// chrome wanted it for.
    pub fn line(&self, pane: Option<&str>) -> Option<String> {
        if let Some(prompt) = &self.prompt {
            return Some(prompt.line());
        }
        let result = self.result.as_ref().filter(|r| Some(r.pane.as_str()) == pane)?;
        if result.waiting {
            return Some(format!("{} · searching the transcript…", result.query));
        }
        if result.matches.is_empty() {
            return Some(match result.whole {
                true => format!("{} · nothing in this transcript", result.query),
                false => format!("{} · nothing in the turns held here", result.query),
            });
        }
        // `total` is every matching turn the node counted; `matches` is the capped list it sent,
        // so a search of four hundred says four hundred and steps through the ones it has.
        let listed = result.matches.len();
        let of = match (result.whole, result.total as usize > listed) {
            (true, true) => format!("{} of {} (first {listed})", result.at + 1, result.total),
            (true, false) => format!("{} of {}", result.at + 1, result.total),
            (false, _) => format!("{} of {} so far", result.at + 1, result.total),
        };
        let text = result.matches[result.at].text.trim();
        Some(format!("{} · {of} · n/N step · esc done · {text}", result.query))
    }

    pub fn render(
        &self,
        buf: &mut ratatui::buffer::Buffer,
        area: ratatui::layout::Rect,
        pane: Option<&str>,
        t: &Theme,
    ) {
        use ratatui::style::Style;
        use ratatui::text::{Line, Span};
        use ratatui::widgets::{Clear, Paragraph, Widget};
        let Some(text) = self.line(pane) else { return };
        if area.height == 0 || area.width == 0 {
            return;
        }
        let row = ratatui::layout::Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        };
        Clear.render(row, buf);
        Paragraph::new(Line::from(Span::styled(
            format!(" transcript · {text}"),
            Style::default().fg(t.accent).bg(t.accent_soft),
        )))
        .style(Style::default().bg(t.accent_soft))
        .render(row, buf);
    }
}
