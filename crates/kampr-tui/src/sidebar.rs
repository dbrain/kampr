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

/// The header's switch, drawn right-aligned: both words, the live one lit.
const SWITCH: [Order; 2] = [Order::Grouped, Order::Priority];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Order {
    /// Node, then workspace, then pane: the herd's own shape.
    #[default]
    Grouped,
    /// Every pane in herdr's rollup order, blocked first — what the `agents` section was, without
    /// listing each agent a second time.
    Priority,
}

impl Order {
    pub fn flip(self) -> Self {
        match self {
            Self::Grouped => Self::Priority,
            Self::Priority => Self::Grouped,
        }
    }

    fn word(self) -> &'static str {
        match self {
            Self::Grouped => "grouped",
            Self::Priority => "priority",
        }
    }
}

/// The columns the header's switch takes at the right edge of a sidebar `width` wide.
pub fn switch(width: u16) -> std::ops::Range<u16> {
    let words: u16 = SWITCH.iter().map(|o| o.word().len() as u16 + 2).sum();
    width.saturating_sub(words + 1)..width.saturating_sub(1)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Row {
    Header(Order),
    /// This machine's node is a mesh peer and this client has no device on its hub, so the herd on
    /// screen is one machine of several.
    Pair(String),
    /// The hub this client opens is not answering, and this machine's own herd is standing in.
    Away(String),
    Node {
        name: String,
        serving: bool,
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
    /// Only in a workspace with more than one tab: one tab is the workspace, and saying so again is
    /// a row spent on nothing.
    Tab {
        name: String,
        pane: String,
    },
    Pane {
        name: String,
        status: AgentStatus,
        pane: String,
        /// The node, where the order has stopped saying it.
        place: Option<String>,
    },
    Question(String),
    Blank,
}

impl Row {
    pub fn pane(&self) -> Option<&str> {
        match self {
            Self::Workspace { pane, .. } | Self::Tab { pane, .. } | Self::Pane { pane, .. } => Some(pane),
            _ => None,
        }
    }

    pub fn selectable(&self) -> bool {
        self.pane().is_some() || matches!(self, Self::Pair(_))
    }
}

pub fn glyph(status: AgentStatus) -> &'static str {
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
/// Parsed once and not per pane per frame: the rows are rebuilt on every draw, so a
/// `Template::default()` here would re-scan the template string a few hundred times a second for
/// an answer that cannot change.
pub fn name(entry: &PaneEntry) -> String {
    static DEFAULT: std::sync::OnceLock<Template> = std::sync::OnceLock::new();
    DEFAULT
        .get_or_init(Template::default)
        .render(&Fields::from_entry(entry))
}

/// The default template's two halves, apart: what the pane is called, and what it is running.
fn halves() -> &'static (Template, Template) {
    static HALVES: std::sync::OnceLock<(Template, Template)> = std::sync::OnceLock::new();
    HALVES.get_or_init(|| {
        let (head, tail) = kampr_core::naming::DEFAULT_TEMPLATE
            .split_once(" · ")
            .expect("the default template has two halves");
        (
            Template::parse(head).expect("the default template's first half parses"),
            Template::parse(tail).expect("the default template's second half parses"),
        )
    })
}

/// A sidebar row's name: the template's, less whatever the row's surroundings already say.
///
/// An agent's row drops `· claude`, because the status glyph already says it is an agent and thirty
/// columns are better spent on the title. Under its workspace's header (`grouped`), a pane named
/// only after that workspace drops the name, which the header is already saying.
pub fn row_name(entry: &PaneEntry, grouped: bool) -> String {
    let fields = Fields::from_entry(entry);
    let (head, tail) = halves();
    let named = head.render(&fields);
    let echoes = grouped && entry.workspace.as_deref() == Some(named.as_str());
    match (entry.agent.is_some(), echoes) {
        (_, true) => tail.render(&fields),
        (true, false) => named,
        (false, false) => name(entry),
    }
}

/// **Sorted by priority, not grouped**: blocked first, then what finished unwatched, then
/// what is still running, and the quiet below that — herdr's own rollup order.
/// Sorted **locally** — `agent.view.set` shapes herdr's own sidebar and leaves `agent.list`
/// alone, so a client cannot read back the view it set and must sort for itself either way
/// (#296).
pub fn triage(herd: &Herd) -> Vec<&PaneEntry> {
    let mut agents: Vec<&PaneEntry> = herd.panes.iter().filter(|p| p.agent.is_some()).collect();
    sort(&mut agents);
    agents
}

fn sort(panes: &mut [&PaneEntry]) {
    panes.sort_by(|a, b| {
        rank(a.agent_status)
            .cmp(&rank(b.agent_status))
            .then_with(|| b.updated_at.cmp(&a.updated_at))
            .then_with(|| a.id.cmp(&b.id))
    });
}

#[derive(Debug, Clone, Copy)]
pub enum Hub<'a> {
    Pair(&'a str),
    Away(&'a str),
}

/// The whole sidebar: the header and its switch, the hub this machine could be showing instead,
/// and the panes in whichever order the switch says.
pub fn rows(
    herd: &Herd,
    order: Order,
    hub: Option<Hub<'_>>,
    questions: impl Fn(&str) -> Option<String>,
) -> Vec<Row> {
    let mut rows = vec![Row::Header(order)];
    match hub {
        Some(Hub::Pair(hub)) => rows.push(Row::Pair(hub.to_string())),
        Some(Hub::Away(hub)) => rows.push(Row::Away(hub.to_string())),
        None => {}
    }
    match order {
        Order::Grouped => rows.extend(spaces(herd, &questions)),
        Order::Priority => rows.extend(priority(herd, &questions)),
    }
    rows
}

/// Grouped by node — the structural change from herdr's own sidebar, and the thing a herdr at one
/// desk structurally cannot draw.
pub fn spaces(herd: &Herd, questions: &impl Fn(&str) -> Option<String>) -> Vec<Row> {
    let mut rows = Vec::new();
    for group in herd.groups() {
        let node = group.node;
        let serving = node.is_serving();
        let rtt = match (serving, node.kind.as_str(), node.rtt_ms) {
            (false, _, _) => "offline".to_string(),
            (true, "local", _) => "local".to_string(),
            (true, _, Some(ms)) => format!("{ms:.0}ms"),
            (true, _, None) => "up".to_string(),
        };
        if !rows.is_empty() {
            rows.push(Row::Blank);
        }
        rows.push(Row::Node {
            name: node.name.clone(),
            serving,
            detail: node.detail.clone().filter(|_| node.online || !serving),
            rtt,
        });
        if !serving {
            rows.push(Row::Absence(absence(&group.panes)));
            if let Some(detail) = &node.detail {
                rows.push(Row::Absence(detail.clone()));
            }
            continue;
        }
        let (mut live, remembered): (Vec<&PaneEntry>, Vec<&PaneEntry>) = group
            .panes
            .iter()
            .partition(|p| node.online || !crate::manage::herdr_on(node) || p.provider.is_some());
        // `groups()` ranks by status first, and a workspace whose panes are not all in one state
        // was drawn as a header per run of them. Priority is the other order; this one is shape.
        live.sort_by(|a, b| {
            space(a)
                .cmp(space(b))
                .then_with(|| a.tab_id.cmp(&b.tab_id))
                .then_with(|| a.id.cmp(&b.id))
        });
        if !node.online && crate::manage::herdr_on(node) {
            rows.push(Row::Absence(match remembered.is_empty() {
                true => "no herdr".to_string(),
                false => format!("no herdr · {}", absence(&remembered)),
            }));
        }
        let mut workspace = None;
        let mut tab = None;
        for pane in live.iter().copied() {
            let here = space(pane);
            if workspace != Some(here) {
                tab = None;
                rows.push(Row::Workspace {
                    subtitle: pane
                        .workspace
                        .as_ref()
                        .and(pane.cwd.as_deref())
                        .map(short)
                        .map(str::to_string),
                    name: here.to_string(),
                    pane: pane.id.clone(),
                });
                workspace = Some(here);
            }
            let tabs = live
                .iter()
                .filter(|p| space(p) == here)
                .filter_map(|p| p.tab_id.as_deref())
                .collect::<std::collections::BTreeSet<_>>()
                .len();
            if tabs > 1 && tab != pane.tab_id.as_deref() {
                tab = pane.tab_id.as_deref();
                rows.push(Row::Tab {
                    name: pane
                        .tab
                        .clone()
                        .or_else(|| pane.tab_id.clone())
                        .unwrap_or_default(),
                    pane: pane.id.clone(),
                });
            }
            push_pane(&mut rows, pane, None, true, questions);
        }
    }
    rows
}

/// A session kampr keeps or muffit runs has no herdr workspace, and its directory is the nearest
/// thing to one.
fn space(pane: &PaneEntry) -> &str {
    pane.workspace
        .as_deref()
        .or(pane.cwd.as_deref().map(short))
        .unwrap_or("—")
}

fn priority(herd: &Herd, questions: &impl Fn(&str) -> Option<String>) -> Vec<Row> {
    let mut panes: Vec<&PaneEntry> = herd
        .panes
        .iter()
        .filter(|p| herd.node(&p.node_id).is_none_or(|n| n.is_serving()))
        .collect();
    sort(&mut panes);
    // The machine is worth its columns only when there is more than one to tell apart.
    let several = herd.nodes.iter().filter(|n| n.is_serving()).count() > 1;
    let mut rows = Vec::new();
    for pane in panes {
        let place = herd
            .node(&pane.node_id)
            .filter(|_| several)
            .map(|n| n.name.clone());
        push_pane(&mut rows, pane, place, false, questions);
    }
    rows
}

fn push_pane(
    rows: &mut Vec<Row>,
    pane: &PaneEntry,
    place: Option<String>,
    grouped: bool,
    questions: &impl Fn(&str) -> Option<String>,
) {
    rows.push(Row::Pane {
        name: row_name(pane, grouped),
        status: pane.agent_status,
        pane: pane.id.clone(),
        place,
    });
    if let Some(question) = questions(&pane.id) {
        rows.push(Row::Question(question));
    }
}

fn absence(panes: &[&PaneEntry]) -> String {
    let seen = panes.iter().filter_map(|p| clock(p)).max().unwrap_or("—");
    let n = panes.len();
    let noun = if n == 1 { "pane" } else { "panes" };
    format!("{n} {noun} · seen {seen}")
}

fn short(path: &str) -> &str {
    path.rsplit('/').find(|p| !p.is_empty()).unwrap_or(path)
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
                let here = matches!(row, Row::Pane { .. }) && row.pane() == self.focused;
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

/// Cut at the end: a machine is recognised by how its name starts.
fn clip(text: &str, width: usize) -> String {
    match text.chars().count() > width {
        true => text.chars().take(width.saturating_sub(1)).chain(['…']).collect(),
        false => text.to_string(),
    }
}

fn len(text: &str) -> u16 {
    text.chars().count() as u16
}

/// `left` at the left edge and `right` flush right, with `left` cut to make room.
fn spread<'a>(
    left: (String, Style),
    right: Option<(String, Style)>,
    width: u16,
    fill: Style,
) -> Vec<Span<'a>> {
    let right_w = right.as_ref().map_or(0, |(text, _)| len(text));
    let room = width.saturating_sub(right_w + 1) as usize;
    let lead = elide(&left.0, room);
    let pad = width.saturating_sub(len(&lead) + right_w) as usize;
    let mut spans = vec![Span::styled(lead, left.1), Span::styled(" ".repeat(pad), fill)];
    if let Some((text, style)) = right {
        spans.push(Span::styled(text, style));
    }
    spans
}

fn line<'a>(row: &'a Row, t: &Theme, picked: bool, here: bool, width: u16) -> Line<'a> {
    let ground = match (picked, here) {
        (true, _) => t.accent_soft,
        (false, true) => t.raise,
        (false, false) => t.bar,
    };
    let base = Style::default().bg(ground);
    let lift = |style: Style| match picked {
        true => style.add_modifier(Modifier::BOLD),
        false => style,
    };
    match row {
        Row::Header(order) => {
            let head = base.fg(t.accent).add_modifier(Modifier::BOLD);
            let at = switch(width);
            let lead = " spaces";
            let mut spans = vec![
                Span::styled(lead, head),
                Span::styled(" ".repeat(at.start.saturating_sub(len(lead)) as usize), base),
            ];
            for each in SWITCH {
                let style = match each == *order {
                    true => Style::default()
                        .fg(t.on_accent)
                        .bg(t.accent)
                        .add_modifier(Modifier::BOLD),
                    false => base.fg(t.mute),
                };
                spans.push(Span::styled(format!(" {} ", each.word()), style));
            }
            Line::from(spans).style(base)
        }
        Row::Pair(hub) => Line::from(Span::styled(
            elide(&format!("  ⇄ pair with {hub}"), width as usize),
            lift(base.fg(t.accent)),
        ))
        .style(base),
        Row::Away(hub) => Line::from(Span::styled(
            format!(
                "  ⇄ {} unreachable",
                elide(hub, (width as usize).saturating_sub(16))
            ),
            base.fg(t.mute).add_modifier(Modifier::ITALIC),
        ))
        .style(base),
        Row::Node {
            name,
            serving,
            rtt,
            detail,
        } => {
            let band = Style::default().bg(t.surface);
            let colour = match (serving, detail) {
                (false, _) => t.mute,
                (true, Some(_)) => t.blocked,
                (true, None) => t.done,
            };
            let dot = if *serving { "●" } else { "○" };
            let rtt = format!(" {rtt} ");
            let lead = elide(&format!(" {name}"), width.saturating_sub(len(&rtt) + 2) as usize);
            let pad = width.saturating_sub(len(&lead) + 1 + len(&rtt)) as usize;
            let spans = vec![
                Span::styled(lead, band.fg(t.text).add_modifier(Modifier::BOLD)),
                Span::styled(" ".repeat(pad), band),
                Span::styled(dot, band.fg(colour)),
                Span::styled(rtt, band.fg(t.dim)),
            ];
            Line::from(spans).style(band)
        }
        Row::Absence(text) => Line::from(Span::styled(
            format!("  {}", elide(text, width.saturating_sub(3) as usize)),
            base.fg(t.mute).add_modifier(Modifier::ITALIC),
        ))
        .style(base),
        Row::Question(text) => Line::from(Span::styled(
            format!("      {}", elide(text, width.saturating_sub(7) as usize)),
            base.fg(t.blocked).add_modifier(Modifier::ITALIC),
        ))
        .style(base),
        Row::Workspace { name, subtitle, .. } => Line::from(spread(
            (format!("  {name}"), lift(base.fg(t.text))),
            subtitle
                .as_deref()
                .filter(|sub| *sub != name.as_str())
                .map(|sub| (format!("{} ", elide(sub, 12)), base.fg(t.mute))),
            width,
            base,
        ))
        .style(base),
        Row::Tab { name, .. } => Line::from(Span::styled(
            format!("   ▸ {}", elide(name, width.saturating_sub(5) as usize)),
            lift(base.fg(t.mute)),
        ))
        .style(base),
        Row::Pane {
            name, status, place, ..
        } => {
            let body = match here {
                true => base.fg(t.accent_hi).add_modifier(Modifier::BOLD),
                false => lift(base.fg(t.dim)),
            };
            let lead = match (here, place.is_some()) {
                (true, true) => "▌ ",
                (true, false) => "▌   ",
                (false, true) => "  ",
                (false, false) => "    ",
            };
            let mut spans = vec![
                Span::styled(lead, base.fg(t.accent)),
                Span::styled(format!("{} ", glyph(*status)), base.fg(t.status(*status))),
            ];
            spans.extend(spread(
                (name.clone(), body),
                place
                    .as_deref()
                    .map(|p| (format!("{} ", clip(p, 9)), base.fg(t.mute))),
                width.saturating_sub(len(lead) + 2),
                base,
            ));
            Line::from(spans).style(base)
        }
        Row::Blank => Line::from(Span::styled(" ", base)).style(base),
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

    fn node(json: serde_json::Value) -> kampr_core::wire::NodeEntry {
        serde_json::from_value(json).unwrap()
    }

    const NO_HERDR: &str = "herdr socket /run/user/1000/herdr/default.sock: No such file";

    fn herd_on(node_entry: kampr_core::wire::NodeEntry) -> Herd {
        let mut native = pane("n:01ja", AgentStatus::Unknown);
        native.provider = Some("native".into());
        Herd {
            nodes: vec![node_entry],
            panes: vec![pane("w1:p1", AgentStatus::Idle), native],
            ..Herd::default()
        }
    }

    fn node_rows(herd: &Herd) -> Vec<Row> {
        spaces(herd, &|_| None)
            .into_iter()
            .filter(|r| !matches!(r, Row::Workspace { .. }))
            .collect()
    }

    /// A machine with no herdr and native sessions was drawn as a machine that is gone, though its
    /// sessions worked. herdr's absence is still true, so it is said once, quietly, and the herdr
    /// panes it still remembers are folded into that line rather than offered as live.
    #[test]
    fn a_machine_with_no_herdr_serving_native_sessions_reads_as_serving() {
        let herd = herd_on(node(serde_json::json!({
            "id": "n", "name": "shed", "kind": "peer", "online": false, "reachable": true,
            "native": true, "rtt_ms": 12.0, "detail": NO_HERDR,
        })));

        assert_eq!(
            node_rows(&herd),
            [
                Row::Node {
                    name: "shed".into(),
                    serving: true,
                    detail: None,
                    rtt: "12ms".into(),
                },
                Row::Absence("no herdr · 1 pane · seen —".into()),
                Row::Pane {
                    name: row_name(&herd.panes[1], true),
                    status: AgentStatus::Unknown,
                    pane: "n/n:01ja".into(),
                    place: None,
                },
            ]
        );
    }

    #[test]
    fn a_herdr_only_machine_losing_herdr_reads_exactly_as_it_did() {
        let herd = herd_on(node(serde_json::json!({
            "id": "n", "name": "coldbox", "kind": "peer", "online": false, "reachable": true,
            "rtt_ms": 12.0, "detail": NO_HERDR,
        })));

        assert_eq!(
            node_rows(&herd),
            [
                Row::Node {
                    name: "coldbox".into(),
                    serving: false,
                    detail: Some(NO_HERDR.into()),
                    rtt: "offline".into(),
                },
                Row::Absence("2 panes · seen —".into()),
                Row::Absence(NO_HERDR.into()),
            ]
        );
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
