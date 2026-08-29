use crate::theme::Theme;
use kampr_client::Herd;
use kampr_core::naming::{Fields, Template};
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
        name: String,
        status: AgentStatus,
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

/// Herdr's own order, measured off its tab rollup: `blocked > done > working > idle >
/// unknown`. `done` outranks `working` because herdr only ever synthesises it for a pane
/// that went `working`→`idle` while **unfocused** — it is an unread marker, and a finished
/// turn nobody has seen wants the operator more than one still running does.
fn rank(status: AgentStatus) -> u8 {
    match status {
        AgentStatus::Blocked => 0,
        AgentStatus::Done => 1,
        AgentStatus::Working => 2,
        AgentStatus::Idle => 3,
        AgentStatus::Unknown => 4,
    }
}

fn clock(entry: &PaneEntry) -> Option<&str> {
    entry.updated_at.as_deref().and_then(|at| at.get(11..16))
}

/// **The shared engine, not a fourth spelling of it.** This file used to fall back to the pane's
/// `tab`, which herdr numbers 1, 2, 3 — so every row in the sidebar was called `1` while the same
/// pane was named properly in the app. `kampr_core::naming` is what the node and the Compose
/// client already render, and the point of it being in `kampr-core` is that all three agree.
///
/// Parsed once and not per pane per frame: [`spaces`] and [`agents`] are rebuilt on every draw, so
/// a `Template::default()` here would re-scan the template string a few hundred times a second for
/// an answer that cannot change.
pub fn name(entry: &PaneEntry) -> String {
    static DEFAULT: std::sync::OnceLock<Template> = std::sync::OnceLock::new();
    DEFAULT
        .get_or_init(Template::default)
        .render(&Fields::from_entry(entry))
}

/// **Sorted by priority, not grouped**: blocked first, then what finished unwatched, then
/// what is still running, and the quiet below that — herdr's own rollup order.
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
            let space = pane.workspace.clone().unwrap_or_else(|| "—".into());
            if workspace.as_deref() != Some(space.as_str()) {
                rows.push(Row::Workspace {
                    subtitle: pane.cwd.as_deref().map(short).map(str::to_string),
                    name: space.clone(),
                    pane: pane.id.clone(),
                });
                workspace = Some(space);
            }
            rows.push(Row::Pane {
                name: name(pane),
                status: pane.agent_status,
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
            name: name(pane),
            status: pane.agent_status,
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
    /// Where the navigator's cursor is, while it is open.
    pub selected: Option<usize>,
    /// Where the operator actually is. **Two different questions**, and the sidebar used to answer
    /// only the first — so outside the navigator nothing on it said which pane the frame was
    /// showing.
    pub focused: Option<&'a str>,
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
                let here = row.pane().is_some() && row.pane() == self.focused;
                line(row, t, picked, here, area.width)
            })
            .collect();
        Paragraph::new(lines)
            .style(Style::default().fg(t.text).bg(t.bar))
            .render(area, buf);
    }
}

/// Cut in the middle, never at the end. A sidebar 30 columns wide cuts something on most rows, and
/// the tail is the half that tells `kampr · cargo test -p kampr-tui` from `-p kampr-core`.
fn elide(text: &str, width: usize) -> String {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() <= width {
        return text.to_string();
    }
    if width <= 1 {
        return chars.into_iter().take(width).collect();
    }
    let keep = width - 1;
    let head = keep.div_ceil(2);
    let tail = keep - head;
    chars[..head]
        .iter()
        .chain(['…'].iter())
        .chain(chars[chars.len() - tail..].iter())
        .collect()
}

fn line<'a>(row: &'a Row, t: &Theme, picked: bool, here: bool, width: u16) -> Line<'a> {
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
            format!("     {}", elide(text, width.saturating_sub(5) as usize)),
            Style::default()
                .fg(t.mute)
                .bg(t.bar)
                .add_modifier(Modifier::ITALIC),
        )),
        Row::Workspace { name, subtitle, .. } => {
            let room = width.saturating_sub(5) as usize;
            let sub = subtitle.as_deref().unwrap_or_default();
            let split = room.saturating_sub(sub.chars().count() + 2);
            let mut spans = vec![Span::styled(
                format!("   ▸ {}", elide(name, split)),
                pick(Style::default().fg(t.text).bg(t.bar)),
            )];
            if !sub.is_empty() {
                spans.push(Span::styled(
                    format!("  {sub}"),
                    Style::default().fg(t.mute).bg(t.bar),
                ));
            }
            Line::from(spans)
        }
        Row::Pane { name, status, .. } => {
            let body = match here {
                true => Style::default()
                    .fg(t.accent_hi)
                    .bg(t.bar)
                    .add_modifier(Modifier::BOLD),
                false => Style::default().fg(t.dim).bg(t.bar),
            };
            Line::from(vec![
                Span::styled(
                    match here {
                        true => " ▌ ",
                        false => "   ",
                    },
                    Style::default().fg(t.accent).bg(t.bar),
                ),
                Span::styled(
                    format!("{} ", glyph(*status)),
                    Style::default().fg(t.status(*status)).bg(t.bar),
                ),
                Span::styled(elide(name, width.saturating_sub(5) as usize), pick(body)),
            ])
        }
        Row::Blank => Line::from(Span::styled(" ", Style::default().bg(t.bar))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kampr_core::provider::PaneInfo;

    fn pane(id: &str, agent_status: AgentStatus) -> PaneEntry {
        let info = PaneInfo {
            pane_id: id.to_string(),
            agent: Some("claude".into()),
            agent_status,
            ..PaneInfo::default()
        };
        PaneEntry::new("n", &info, false)
    }

    /// Herdr rolls a tab up as `blocked > done > working > idle > unknown`, measured by driving
    /// two panes through every pair and reading `tab.get`. `done` is what a pane becomes when it
    /// finished while nobody was looking, so it is news and `working` is not — and a sidebar that
    /// disagrees with herdr's own attention queue sorts one herd two ways.
    #[test]
    fn a_pane_that_finished_unwatched_sits_above_one_that_is_still_working() {
        let herd = Herd {
            panes: vec![
                pane("w1:p1", AgentStatus::Working),
                pane("w1:p2", AgentStatus::Done),
                pane("w1:p3", AgentStatus::Blocked),
                pane("w1:p4", AgentStatus::Idle),
                pane("w1:p5", AgentStatus::Unknown),
            ],
            ..Herd::default()
        };

        let order: Vec<AgentStatus> = triage(&herd).iter().map(|p| p.agent_status).collect();

        assert_eq!(
            order,
            [
                AgentStatus::Blocked,
                AgentStatus::Done,
                AgentStatus::Working,
                AgentStatus::Idle,
                AgentStatus::Unknown,
            ]
        );
    }
}
