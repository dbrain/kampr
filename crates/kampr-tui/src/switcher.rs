use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kampr_core::provider::AgentStatus;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

const MAX_SHOWN: usize = 12;

#[derive(Debug, Clone)]
pub struct Item {
    pub pane: String,
    pub name: String,
    pub place: String,
    pub status: AgentStatus,
}

/// herdr's `goto` (`prefix g`, #289): a query over every pane on every node, matched against its
/// name and where it lives.
#[derive(Debug, Default)]
pub struct Switcher {
    query: Option<String>,
    at: usize,
}

pub enum Went {
    Stayed,
    Closed,
    To(String),
}

impl Switcher {
    pub fn open(&mut self) {
        self.query = Some(String::new());
        self.at = 0;
    }

    pub fn active(&self) -> bool {
        self.query.is_some()
    }

    pub fn close(&mut self) {
        self.query = None;
    }

    pub fn key(&mut self, key: KeyEvent, items: &[Item]) -> Went {
        let Some(query) = self.query.as_mut() else {
            return Went::Stayed;
        };
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let alt = key.modifiers.contains(KeyModifiers::ALT);
        // Bare while nothing is typed, because then a digit cannot be the start of a name; behind
        // alt once something is.
        if let KeyCode::Char(c @ '1'..='9') = key.code
            && (alt || query.is_empty())
        {
            let n = c as usize - '1' as usize;
            let picked = matches(query, items).get(n).map(|item| item.pane.clone());
            if picked.is_some() {
                self.query = None;
            }
            return picked.map_or(Went::Stayed, Went::To);
        }
        match key.code {
            KeyCode::Esc => {
                self.query = None;
                return Went::Closed;
            }
            KeyCode::Enter => {
                let picked = matches(query, items).get(self.at).map(|item| item.pane.clone());
                self.query = None;
                return picked.map_or(Went::Closed, Went::To);
            }
            KeyCode::Up => self.at = self.at.saturating_sub(1),
            KeyCode::Char('p') if ctrl => self.at = self.at.saturating_sub(1),
            KeyCode::Down | KeyCode::Tab => self.at += 1,
            KeyCode::Char('n') if ctrl => self.at += 1,
            KeyCode::Backspace => {
                query.pop();
                self.at = 0;
            }
            KeyCode::Char(c) if !ctrl && !alt => {
                query.push(c);
                self.at = 0;
            }
            _ => {}
        }
        let found = matches(self.query.as_deref().unwrap_or_default(), items).len();
        self.at = self.at.min(found.saturating_sub(1));
        Went::Stayed
    }

    /// The box it drew and the row each pane is on, for the pointer.
    pub fn render(
        &self,
        buf: &mut Buffer,
        area: Rect,
        items: &[Item],
        t: &crate::theme::Theme,
    ) -> Option<(Rect, Vec<(Rect, String)>)> {
        let query = self.query.as_ref()?;
        let found = matches(query, items);
        let width = area.width.saturating_sub(4).min(72);
        let shown = found.len().clamp(1, MAX_SHOWN);
        let height = (shown as u16 + 3).min(area.height);
        if width < 20 || height < 4 {
            return None;
        }
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        };
        let inner = width.saturating_sub(2) as usize;
        let top = self.at.saturating_sub(shown - 1);
        let mut rows = Vec::new();
        let mut lines = vec![Line::from(vec![
            Span::styled(" › ", Style::default().fg(t.accent).bg(t.surface)),
            Span::styled(
                format!("{query}▏"),
                Style::default()
                    .fg(t.text)
                    .bg(t.surface)
                    .add_modifier(Modifier::BOLD),
            ),
        ])];
        if found.is_empty() {
            lines.push(Line::from(Span::styled(
                "   no pane matches",
                Style::default().fg(t.mute).bg(t.surface),
            )));
        }
        for (i, item) in found.iter().enumerate().skip(top).take(shown) {
            let ground = match i == self.at {
                true => t.accent_soft,
                false => t.surface,
            };
            rows.push((
                Rect {
                    x: popup.x + 1,
                    y: popup.y + 1 + lines.len() as u16,
                    width: inner as u16,
                    height: 1,
                },
                item.pane.clone(),
            ));
            let digit = match i < 9 {
                true => format!(" {}", i + 1),
                false => "  ".to_string(),
            };
            let place = format!("{} ", item.place);
            let room = inner.saturating_sub(place.chars().count() + 7);
            let name: String = item.name.chars().take(room).collect();
            let pad = inner.saturating_sub(7 + name.chars().count() + place.chars().count());
            let body = match i == self.at {
                true => Style::default()
                    .fg(t.accent_hi)
                    .bg(ground)
                    .add_modifier(Modifier::BOLD),
                false => Style::default().fg(t.text).bg(ground),
            };
            lines.push(
                Line::from(vec![
                    Span::styled(digit, Style::default().fg(t.mute).bg(ground)),
                    Span::styled(
                        format!(" {} ", crate::sidebar::glyph(item.status)),
                        Style::default().fg(t.status(item.status)).bg(ground),
                    ),
                    Span::styled(name, body),
                    Span::styled(" ".repeat(pad + 1), Style::default().bg(ground)),
                    Span::styled(place, Style::default().fg(t.mute).bg(ground)),
                ])
                .style(Style::default().bg(ground)),
            );
        }
        Clear.render(popup, buf);
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(Span::styled(
                        " go to ",
                        Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                    ))
                    .title_bottom(Span::styled(
                        " ↑↓ pick · ↵ go · 1-9 · esc ",
                        Style::default().fg(t.mute),
                    )),
            )
            .style(Style::default().bg(t.surface))
            .render(popup, buf);
        Some((popup, rows))
    }
}

/// Best first. An empty query is every pane in the order it was handed over.
pub fn matches<'a>(query: &str, items: &'a [Item]) -> Vec<&'a Item> {
    let mut scored: Vec<(i32, usize, &Item)> = items
        .iter()
        .enumerate()
        .filter_map(|(i, item)| {
            let hay = format!("{} {}", item.name, item.place);
            score(query, &hay).map(|s| (s, i, item))
        })
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, _, item)| item).collect()
}

/// Every query character in order, case-insensitively, scored up for runs and for landing on the
/// start of a word, so `wheel` ranks "fix the wheel" above "who held everything else".
fn score(query: &str, hay: &str) -> Option<i32> {
    let hay: Vec<char> = hay.chars().flat_map(char::to_lowercase).collect();
    let mut at = 0;
    let mut total = 0;
    let mut last: Option<usize> = None;
    for q in query
        .chars()
        .flat_map(char::to_lowercase)
        .filter(|c| !c.is_whitespace())
    {
        let found = (at..hay.len()).find(|&i| hay[i] == q)?;
        total += 1;
        if last.is_some_and(|l| l + 1 == found) {
            total += 4;
        }
        if found == 0 || !hay[found - 1].is_alphanumeric() {
            total += 3;
        }
        last = Some(found);
        at = found + 1;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_run_and_a_word_start_outrank_letters_scattered_through_the_text() {
        let close = score("wheel", "fix the wheel").expect("a match");
        let spread = score("wheel", "who held everything else").expect("a match");
        assert!(close > spread, "{close} vs {spread}");
        assert_eq!(score("zz", "fix the wheel"), None);
    }
}
