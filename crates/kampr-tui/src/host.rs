use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent};
use kampr_core::wire::NodeEntry;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

const WIDTH: u16 = 72;

pub enum Took {
    Stay,
    Close,
    New,
    Claude,
    Resume(String),
    Dismiss(String),
}

/// What a host offers beyond `New…`: a Claude with no terminal where the node runs headless
/// Claude, and the programs it kept to resume after a reboot.
pub struct Offers<'a> {
    pub claude: bool,
    pub records: &'a [crate::resume::Record],
}

pub struct HostMenu {
    pub node: String,
    at: usize,
}

pub fn reach(node: &NodeEntry) -> String {
    let mut parts = vec![match (node.is_serving(), node.kind.as_str()) {
        (false, _) => "offline".to_string(),
        (true, "local") => "this machine".to_string(),
        (true, kind) => format!("{kind} node"),
    }];
    if let Some(ms) = node.rtt_ms.filter(|_| node.is_serving()) {
        parts.push(format!("{ms:.0} ms ping"));
    }
    if let Some(detail) = &node.detail {
        parts.push(detail.clone());
    }
    parts.join(" · ")
}

impl HostMenu {
    pub fn new(node: &str) -> Self {
        Self {
            node: node.into(),
            at: 0,
        }
    }

    pub fn key(&mut self, key: KeyEvent, offers: &Offers<'_>) -> Took {
        let picked = offers.records.get(self.at).map(|r| r.id.clone());
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('.') | KeyCode::Char('m') => Took::Close,
            KeyCode::Char('n') => Took::New,
            KeyCode::Char('c') if offers.claude => Took::Claude,
            KeyCode::Up => {
                self.at = self.at.saturating_sub(1);
                Took::Stay
            }
            KeyCode::Down => {
                self.at = (self.at + 1).min(offers.records.len().saturating_sub(1));
                Took::Stay
            }
            KeyCode::Enter => picked.map_or(Took::Stay, Took::Resume),
            KeyCode::Char('x') => {
                let took = picked.map_or(Took::Stay, Took::Dismiss);
                self.at = self.at.saturating_sub(1);
                took
            }
            _ => Took::Stay,
        }
    }

    pub fn render(
        &self,
        node: Option<&NodeEntry>,
        offers: &Offers<'_>,
        area: Rect,
        buf: &mut Buffer,
        t: &Theme,
    ) {
        let name = node.map_or(self.node.as_str(), |n| n.name.as_str());
        let mut lines = vec![
            Line::from(Span::styled(
                node.map(reach).unwrap_or_else(|| "not in the herd".into()),
                Style::default().fg(t.dim),
            )),
            Line::default(),
            Line::from(vec![
                Span::styled(" n ", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
                Span::styled("New…", Style::default().fg(t.text)),
                Span::styled("  a session, tab or workspace", Style::default().fg(t.mute)),
            ]),
        ];
        if offers.claude {
            lines.push(Line::from(vec![
                Span::styled(" c ", Style::default().fg(t.accent).add_modifier(Modifier::BOLD)),
                Span::styled("Claude without a terminal", Style::default().fg(t.text)),
            ]));
        }
        if !offers.records.is_empty() {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                "resume after reboot · enter resume · x dismiss",
                Style::default().fg(t.dim),
            )));
            for (i, record) in offers.records.iter().enumerate() {
                let on = i == self.at;
                lines.push(Line::from(Span::styled(
                    format!("{} {}", if on { "▸" } else { " " }, record.line()),
                    match on {
                        true => Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                        false => Style::default().fg(t.text),
                    },
                )));
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
        Clear.render(rect, buf);
        Paragraph::new(lines)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(Span::styled(
                        format!(" {name} "),
                        Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                    ))
                    .title_bottom(Span::styled(" n new · esc close ", Style::default().fg(t.mute))),
            )
            .style(Style::default().bg(t.surface))
            .render(rect, buf);
    }
}
