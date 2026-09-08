use crossterm::event::{KeyCode, KeyEvent};
use kampr_core::wire::FindMatch;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph, Widget};

/// A search over the pane's whole scrollback, and the operator's position in what it found.
///
/// It is the node that searches, not this client: the ring held here is a window on what herdr
/// holds, and `pane.copy_search` covers the whole of it (probe #511). So the query goes out and the
/// answer comes back as a frame — which is why the prompt closes on Enter rather than waiting, and
/// why `n` before an answer has arrived has nothing to step through and says so.
#[derive(Debug, Default)]
pub struct Find {
    prompt: Option<Prompt>,
    /// What came back, for the pane it came back about. A result set belongs to one pane; stepping
    /// it after switching panes would scroll a pane to a row from somebody else's history.
    result: Option<Result>,
}

#[derive(Debug)]
struct Prompt {
    buf: String,
    backward: bool,
}

#[derive(Debug)]
struct Result {
    pane: String,
    query: String,
    matches: Vec<FindMatch>,
    total: u32,
    at: usize,
}

/// What the app should do about a key the find surface was offered.
pub enum Took {
    /// Not a find key; carry on down the chain.
    Ignored,
    /// Handled, nothing else to do.
    Consumed,
    /// Run this search on this pane.
    Search { query: String, backward: bool },
}

impl Find {
    pub fn open(&mut self, backward: bool) {
        self.prompt = Some(Prompt {
            buf: String::new(),
            backward,
        });
    }

    pub fn prompting(&self) -> bool {
        self.prompt.is_some()
    }

    pub fn key(&mut self, key: KeyEvent) -> Took {
        let Some(prompt) = self.prompt.as_mut() else {
            return Took::Ignored;
        };
        match key.code {
            KeyCode::Esc => {
                self.prompt = None;
                Took::Consumed
            }
            KeyCode::Enter => {
                let prompt = self.prompt.take().expect("a prompt");
                match prompt.buf.is_empty() {
                    true => Took::Consumed,
                    false => Took::Search {
                        query: prompt.buf,
                        backward: prompt.backward,
                    },
                }
            }
            KeyCode::Backspace => {
                prompt.buf.pop();
                Took::Consumed
            }
            KeyCode::Char(c) => {
                prompt.buf.push(c);
                Took::Consumed
            }
            _ => Took::Consumed,
        }
    }

    pub fn found(
        &mut self,
        pane: &str,
        query: &str,
        matches: Vec<FindMatch>,
        total: u32,
        current: Option<u32>,
    ) {
        let at = current.unwrap_or(0) as usize;
        self.result = Some(Result {
            pane: pane.into(),
            query: query.into(),
            at: at.min(matches.len().saturating_sub(1)),
            matches,
            total,
        });
    }

    /// Steps to the next match and answers where to scroll to, in rows from the live row.
    ///
    /// **`None` rather than a wrap-around when there is nothing to step**, because the two are
    /// different answers and the caller says so differently: a search that matched nothing and a
    /// search that has not come back yet both leave the operator pressing `n` at nothing.
    pub fn step(&mut self, pane: &str, forward: bool) -> Option<u32> {
        let result = self.result.as_mut().filter(|r| r.pane == pane)?;
        if result.matches.is_empty() {
            return None;
        }
        let n = result.matches.len();
        result.at = match forward {
            true => (result.at + 1) % n,
            false => (result.at + n - 1) % n,
        };
        Some(result.matches[result.at].from_bottom)
    }

    /// Where the current match is, without moving.
    pub fn at(&self, pane: &str) -> Option<u32> {
        let result = self.result.as_ref().filter(|r| r.pane == pane)?;
        result.matches.get(result.at).map(|m| m.from_bottom)
    }

    /// The one line this surface shows: the prompt while typing, the standing result after.
    pub fn line(&self, pane: Option<&str>) -> Option<String> {
        if let Some(prompt) = &self.prompt {
            let lead = if prompt.backward { '?' } else { '/' };
            return Some(format!("{lead}{}  ↵ search · esc cancel", prompt.buf));
        }
        let result = self.result.as_ref().filter(|r| Some(r.pane.as_str()) == pane)?;
        if result.matches.is_empty() {
            return Some(format!("{} · no match in the scrollback", result.query));
        }
        // `total` is every match herdr found; `matches` is the capped list this client was handed,
        // so a search of four hundred says four hundred and steps through the ones it has.
        let listed = result.matches.len();
        let of = match result.total as usize > listed {
            true => format!("{} of {} (first {listed})", result.at + 1, result.total),
            false => format!("{} of {}", result.at + 1, result.total),
        };
        let text = result.matches[result.at].text.trim();
        Some(format!("{} · {of} · {text}", result.query))
    }
}

impl Find {
    /// One row along the bottom, above nothing and below everything — the same shape the manage
    /// strip uses, because it is the same kind of thing: a line about what this client is doing
    /// rather than about what the pane is.
    pub fn render(&self, buf: &mut Buffer, area: Rect, pane: Option<&str>, t: &crate::theme::Theme) {
        let Some(text) = self.line(pane) else { return };
        if area.height == 0 || area.width == 0 {
            return;
        }
        let row = Rect {
            x: area.x,
            y: area.y + area.height - 1,
            width: area.width,
            height: 1,
        };
        Clear.render(row, buf);
        Paragraph::new(Line::from(Span::styled(
            format!(" find · {text}"),
            Style::default().fg(t.accent).bg(t.accent_soft),
        )))
        .style(Style::default().bg(t.accent_soft))
        .render(row, buf);
    }
}
