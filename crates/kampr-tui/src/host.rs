use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent};
use kampr_core::wire::NodeEntry;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

const WIDTH: u16 = 44;

pub enum Took {
    Stay,
    Close,
    New,
}

pub struct HostMenu {
    pub node: String,
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
        Self { node: node.into() }
    }

    pub fn key(&self, key: KeyEvent) -> Took {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') | KeyCode::Char('.') | KeyCode::Char('m') => Took::Close,
            KeyCode::Char('n') => Took::New,
            _ => Took::Stay,
        }
    }

    pub fn render(&self, node: Option<&NodeEntry>, area: Rect, buf: &mut Buffer, t: &Theme) {
        let width = area.width.min(WIDTH);
        let height = area.height.min(6);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        };
        let name = node.map_or(self.node.as_str(), |n| n.name.as_str());
        let lines = vec![
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
