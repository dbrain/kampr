//! A Claude with no terminal (docs/21 "Starting a conversation without a terminal"): a directory,
//! then a first prompt to start one or a stored conversation from that directory to resume.

use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use serde::Deserialize;
use serde_json::{Value, json};

const WIDTH: u16 = 72;

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Stored {
    pub id: String,
    pub cwd: Option<String>,
    pub title: Option<String>,
}

pub enum Went {
    Stay,
    Close,
    /// An op whose ack this modal is waiting on.
    Ask(Value),
    /// An op that ends the modal.
    Send(Value),
}

pub struct NewClaude {
    pub node: String,
    cwd: String,
    prompt: String,
    asked: bool,
    found: Option<Result<Vec<Stored>, String>>,
    at: Option<usize>,
}

impl NewClaude {
    pub fn new(node: &str) -> Self {
        Self {
            node: node.to_string(),
            cwd: String::new(),
            prompt: String::new(),
            asked: false,
            found: None,
            at: None,
        }
    }

    fn with_cwd(&self, mut op: Value) -> Value {
        if !self.cwd.trim().is_empty() {
            op["cwd"] = json!(self.cwd.trim());
        }
        op
    }

    pub fn found(&mut self, found: Result<Vec<Stored>, String>) {
        self.found = Some(found);
    }

    fn stored(&self) -> &[Stored] {
        match &self.found {
            Some(Ok(list)) => list,
            _ => &[],
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Went {
        if key.code == KeyCode::Esc {
            return Went::Close;
        }
        let typing = !key.modifiers.contains(KeyModifiers::CONTROL);
        if !self.asked {
            match key.code {
                KeyCode::Char(c) if typing => self.cwd.push(c),
                KeyCode::Backspace => {
                    self.cwd.pop();
                }
                KeyCode::Enter => {
                    self.asked = true;
                    return Went::Ask(self.with_cwd(json!({ "op": "claude.resumable", "node": self.node })));
                }
                _ => {}
            }
            return Went::Stay;
        }
        let count = self.stored().len();
        match (self.at, key.code) {
            (None, KeyCode::Down) if count > 0 => self.at = Some(0),
            (Some(i), KeyCode::Down) => self.at = Some((i + 1).min(count - 1)),
            (Some(0), KeyCode::Up) => self.at = None,
            (Some(i), KeyCode::Up) => self.at = Some(i - 1),
            (Some(i), KeyCode::Enter) => {
                return Went::Send(
                    json!({ "op": "claude.resume", "node": self.node, "id": self.stored()[i].id }),
                );
            }
            (None, KeyCode::Enter) => {
                let mut op = self.with_cwd(json!({ "op": "claude.create", "node": self.node }));
                if !self.prompt.trim().is_empty() {
                    op["prompt"] = json!(self.prompt.trim());
                }
                return Went::Send(op);
            }
            (None, KeyCode::Char(c)) if typing => self.prompt.push(c),
            (None, KeyCode::Backspace) => {
                self.prompt.pop();
            }
            _ => {}
        }
        Went::Stay
    }

    pub fn render(&self, area: Rect, buf: &mut Buffer, t: &Theme) {
        let field = |label: &str, text: &str, on: bool| {
            Line::from(vec![
                Span::styled(format!(" {label:<13}"), Style::default().fg(t.accent)),
                Span::styled(
                    format!("{text}{}", if on { "▏" } else { "" }),
                    Style::default().fg(t.text),
                ),
            ])
        };
        let dim = Style::default().fg(t.dim);
        let mut lines = vec![field("directory", &self.cwd, !self.asked)];
        if self.asked {
            lines.push(field("first prompt", &self.prompt, self.at.is_none()));
            lines.push(Line::default());
            match &self.found {
                None => lines.push(Line::from(Span::styled(
                    " looking for conversations to resume…",
                    dim,
                ))),
                Some(Err(why)) => lines.push(Line::from(Span::styled(format!(" {why}"), dim))),
                Some(Ok(list)) if list.is_empty() => {
                    lines.push(Line::from(Span::styled(" no conversation here to resume", dim)))
                }
                Some(Ok(list)) => {
                    lines.push(Line::from(Span::styled(" or resume", dim)));
                    for (i, s) in list.iter().enumerate() {
                        let on = self.at == Some(i);
                        let style = match on {
                            true => Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                            false => Style::default().fg(t.text),
                        };
                        lines.push(Line::from(Span::styled(
                            format!(
                                " {} {}",
                                if on { "▸" } else { " " },
                                s.title.as_deref().unwrap_or(&s.id)
                            ),
                            style,
                        )));
                    }
                }
            }
        }
        let width = area.width.min(WIDTH);
        let height = (lines.len() as u16 + 2).min(area.height);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        };
        let keys = match self.asked {
            false => " enter next · esc cancel ",
            true => " enter start · ↓ resume one · esc cancel ",
        };
        Clear.render(rect, buf);
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(Span::styled(
                        " Claude without a terminal ",
                        Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                    ))
                    .title_bottom(Span::styled(keys, Style::default().fg(t.mute))),
            )
            .style(Style::default().bg(t.surface))
            .render(rect, buf);
    }
}
