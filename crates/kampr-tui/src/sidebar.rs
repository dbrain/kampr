use crate::theme::Theme;
use kampr_client::Herd;
use kampr_core::provider::AgentStatus;
use kampr_core::wire::PaneEntry;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Widget};

pub const WIDTH: u16 = 30;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Header(&'static str, String),
    Node {
        name: String,
        online: bool,
        detail: Option<String>,
        rtt: String,
    },
    /// An offline node keeps its row: panes are not dropped for an outage (#70), so the pane
    /// count and the last-seen are what say the node exists and is unreachable.
    Absence(String),
    Workspace {
        name: String,
        subtitle: Option<String>,
        pane: String,
    },
    Pane {
        title: String,
        status: AgentStatus,
        agent: Option<String>,
        pane: String,
    },
    Blank,
}

impl Row {
    pub fn pane(&self) -> Option<&str> {
        match self {
            Self::Workspace { pane, .. } | Self::Pane { pane, .. } => Some(pane),
            _ => None,
        }
    }
}

fn glyph(status: AgentStatus) -> &'static str {
    match status {
        AgentStatus::Blocked => "⚑",
        AgentStatus::Working => "◐",
        AgentStatus::Done => "✓",
        AgentStatus::Idle => "○",
        AgentStatus::Unknown => "·",
    }
}

fn rank(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Blocked => 0,
        AgentStatus::Working => 1,
        AgentStatus::Done => 2,
        AgentStatus::Idle => 3,
        AgentStatus::Unknown => 4,
    }
}

fn clock(entry: &PaneEntry) -> Option<&str> {
    entry.updated_at.as_deref().and_then(|at| at.get(11..16))
}

fn title(entry: &PaneEntry) -> String {
    entry
        .label
        .clone()
        .or_else(|| entry.workspace.clone())
        .unwrap_or_else(|| {
            entry
                .id
                .split_once('/')
                .map_or(entry.id.clone(), |(_, p)| p.into())
        })
}

/// **Sorted by priority, not grouped**: blocked and working at the top, done and idle below.
/// Sorted **locally** — `agent.view.set` shapes herdr's own sidebar and leaves `agent.list`
/// alone, so a client cannot read back the view it set and must sort for itself either way
/// (#296).
pub fn triage(herd: &Herd) -> Vec<&PaneEntry> {
    let mut agents: Vec<&PaneEntry> = herd.panes.iter().filter(|p| p.agent.is_some()).collect();
    agents.sort_by(|a, b| {
        rank(a.agent_status)
            .cmp(&rank(b.agent_status))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
    agents
}

/// `spaces`, grouped by node — the structural change from herdr's own sidebar, and the thing a
/// herdr at one desk structurally cannot draw.
pub fn spaces(herd: &Herd) -> Vec<Row> {
    let mut rows = vec![Row::Header("spaces", String::new())];
    for group in herd.groups() {
        let node = group.node;
        let rtt = match (node.online, node.kind.as_str(), node.rtt_ms) {
            (false, _, _) => "offline".to_string(),
            (true, "local", _) => "local".to_string(),
            (true, _, Some(ms)) => format!("{ms:.0}ms"),
            (true, _, None) => "up".to_string(),
        };
        rows.push(Row::Node {
            name: node.name.clone(),
            online: node.online,
            detail: node.detail.clone(),
            rtt,
        });
        if !node.online {
            let seen = group.panes.iter().filter_map(|p| clock(p)).max().unwrap_or("—");
            let n = group.panes.len();
            let panes = if n == 1 { "pane" } else { "panes" };
            rows.push(Row::Absence(format!("{n} {panes} · seen {seen}")));
            if let Some(detail) = &node.detail {
                rows.push(Row::Absence(detail.clone()));
            }
            continue;
        }
        let mut workspace = None;
        for pane in &group.panes {
            let name = pane.workspace.clone().unwrap_or_else(|| "—".into());
            if workspace.as_deref() != Some(name.as_str()) {
                rows.push(Row::Workspace {
                    subtitle: pane.cwd.as_deref().map(short).map(str::to_string),
                    name: name.clone(),
                    pane: pane.id.clone(),
                });
                workspace = Some(name);
            }
            rows.push(Row::Pane {
                title: pane.tab.clone().unwrap_or_else(|| title(pane)),
                status: pane.agent_status,
                agent: pane.agent.clone(),
                pane: pane.id.clone(),
            });
        }
    }
    rows
}

fn short(path: &str) -> &str {
    path.rsplit('/').find(|p| !p.is_empty()).unwrap_or(path)
}

pub fn agents(herd: &Herd, questions: impl Fn(&str) -> Option<String>) -> Vec<Row> {
    let items = triage(herd);
    let mut rows = vec![Row::Header("agents", "priority".into())];
    for pane in items {
        rows.push(Row::Pane {
            title: title(pane),
            status: pane.agent_status,
            agent: pane.agent.clone(),
            pane: pane.id.clone(),
        });
        if let Some(question) = questions(&pane.id) {
            rows.push(Row::Absence(question));
        }
    }
    rows
}

pub struct Sidebar<'a> {
    pub rows: &'a [Row],
    pub theme: &'a Theme,
    pub selected: Option<usize>,
    pub top: usize,
}

impl Widget for Sidebar<'_> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let t = self.theme;
        let lines: Vec<Line> = self
            .rows
            .iter()
            .enumerate()
            .skip(self.top)
            .take(area.height as usize)
            .map(|(i, row)| {
                let picked = self.selected == Some(i);
                line(row, t, picked, area.width)
            })
            .collect();
        Paragraph::new(lines)
            .style(Style::default().fg(t.text).bg(t.bar))
            .render(area, buf);
    }
}

fn line<'a>(row: &'a Row, t: &Theme, picked: bool, width: u16) -> Line<'a> {
    let pick = |style: Style| match picked {
        true => style.bg(t.accent_soft).add_modifier(Modifier::BOLD),
        false => style,
    };
    match row {
        Row::Header(name, note) => {
            let head = Style::default()
                .fg(t.accent)
                .bg(t.bar)
                .add_modifier(Modifier::BOLD);
            let pad = width.saturating_sub(name.len() as u16 + note.len() as u16 + 2) as usize;
            Line::from(vec![
                Span::styled(format!(" {name}"), head),
                Span::raw(" ".repeat(pad)),
                Span::styled(note.clone(), Style::default().fg(t.mute).bg(t.bar)),
            ])
        }
        Row::Node {
            name,
            online,
            rtt,
            detail,
        } => {
            let dot = if *online { "●" } else { "○" };
            let colour = match (online, detail) {
                (false, _) => t.mute,
                (true, Some(_)) => t.blocked,
                (true, None) => t.done,
            };
            let pad = width.saturating_sub(name.len() as u16 + rtt.len() as u16 + 5) as usize;
            Line::from(vec![
                Span::styled(
                    format!(" {name}"),
                    pick(Style::default().fg(t.text).bg(t.bar).add_modifier(Modifier::BOLD)),
                ),
                Span::raw(" ".repeat(pad)),
                Span::styled(format!("{dot} "), Style::default().fg(colour).bg(t.bar)),
                Span::styled(rtt.clone(), Style::default().fg(t.dim).bg(t.bar)),
                Span::raw(" "),
            ])
        }
        Row::Absence(text) => Line::from(Span::styled(
            format!("     {text}"),
            Style::default()
                .fg(t.mute)
                .bg(t.bar)
                .add_modifier(Modifier::ITALIC),
        )),
        Row::Workspace { name, subtitle, .. } => {
            let mut spans = vec![Span::styled(
                format!("   ▸ {name}"),
                pick(Style::default().fg(t.text).bg(t.bar)),
            )];
            if let Some(sub) = subtitle {
                spans.push(Span::styled(
                    format!("  {sub}"),
                    Style::default().fg(t.mute).bg(t.bar),
                ));
            }
            Line::from(spans)
        }
        Row::Pane {
            title, status, agent, ..
        } => {
            let mark = Style::default().fg(t.status(*status)).bg(t.bar);
            let mut spans = vec![
                Span::styled(format!("   {} ", glyph(*status)), mark),
                Span::styled(title.clone(), pick(Style::default().fg(t.dim).bg(t.bar))),
            ];
            if let Some(agent) = agent {
                spans.push(Span::styled(
                    format!(" · {agent}"),
                    Style::default().fg(t.mute).bg(t.bar),
                ));
            }
            Line::from(spans)
        }
        Row::Blank => Line::from(Span::styled(" ", Style::default().bg(t.bar))),
    }
}
