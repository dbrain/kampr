use super::{Area, Hosts, Row, Tab, host, push_pane, spread};
use crate::theme::Theme;
use kampr_client::NodeGroup;
use kampr_core::wire::{ContainerEntry, NodeEntry, PaneEntry};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::{BTreeMap, HashMap, HashSet};

const DAY: i64 = 86_400;
const ALIVE: [&str; 3] = ["running", "restarting", "paused"];

pub(super) struct Look<'a, Q> {
    pub hosts: &'a Hosts,
    pub now: i64,
    pub questions: &'a Q,
}

pub(super) struct Branch<'a> {
    pub group: NodeGroup<'a>,
    pub children: Vec<Branch<'a>>,
}

/// A node a host adopted into one of its containers hangs under that host. A parent that is not in
/// the herd, or a loop, leaves the node at the top, which is where an older client draws it anyway.
pub(super) fn tree(groups: Vec<NodeGroup<'_>>) -> Vec<Branch<'_>> {
    let ids: HashSet<&str> = groups.iter().map(|g| g.node.id.as_str()).collect();
    let nests = |g: &NodeGroup<'_>| {
        g.node
            .parent
            .as_deref()
            .is_some_and(|p| ids.contains(p) && p != g.node.id)
    };
    let mut by_parent: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, g) in groups.iter().enumerate().filter(|(_, g)| nests(g)) {
        by_parent
            .entry(g.node.parent.clone().unwrap_or_default())
            .or_default()
            .push(i);
    }
    let roots: Vec<usize> = (0..groups.len()).filter(|&i| !nests(&groups[i])).collect();
    let mut slots: Vec<Option<NodeGroup<'_>>> = groups.into_iter().map(Some).collect();
    let mut built = Vec::new();
    for i in roots {
        built.extend(build(i, &mut slots, &by_parent));
    }
    for i in 0..slots.len() {
        built.extend(build(i, &mut slots, &by_parent));
    }
    built
}

fn build<'a>(
    i: usize,
    slots: &mut [Option<NodeGroup<'a>>],
    by_parent: &HashMap<String, Vec<usize>>,
) -> Option<Branch<'a>> {
    let group = slots[i].take()?;
    let children = by_parent
        .get(&group.node.id)
        .into_iter()
        .flatten()
        .filter_map(|&k| build(k, slots, by_parent))
        .collect();
    Some(Branch { group, children })
}

fn muffit(pane: &PaneEntry) -> bool {
    pane.provider.as_deref() == Some("muffit")
}

/// A shell into one of the host's containers is listed under that container instead.
pub(super) fn terminal(node: &NodeEntry, pane: &PaneEntry) -> bool {
    !muffit(pane)
        && !node
            .containers
            .iter()
            .any(|c| pane.container.as_deref() == Some(c.name.as_str()))
}

fn shells<'a>(branch: &Branch<'a>, container: &ContainerEntry) -> Vec<&'a PaneEntry> {
    branch
        .group
        .panes
        .iter()
        .copied()
        .filter(|p| !muffit(p) && p.container.as_deref() == Some(container.name.as_str()))
        .collect()
}

fn child<'b, 'a>(branch: &'b Branch<'a>, container: &ContainerEntry) -> Option<&'b Branch<'a>> {
    let id = container.child.as_deref()?;
    branch.children.iter().find(|c| c.group.node.id == id)
}

/// A container the operator has a shell in or a node inside is listed whatever its state or fold:
/// hiding it would hide what hangs off it.
fn pinned(branch: &Branch<'_>, container: &ContainerEntry) -> bool {
    !shells(branch, container).is_empty() || child(branch, container).is_some()
}

fn wanted(container: &ContainerEntry, now: i64) -> bool {
    ALIVE.contains(&container.state.as_str())
        || container.health.as_deref() == Some("unhealthy")
        || container.adopt_error.is_some()
        || container.last_crash.as_ref().is_some_and(|c| c.at >= now - DAY)
}

pub(super) fn shellable(container: &ContainerEntry) -> bool {
    container.state == "running"
}

/// The lists a host can show, terminals first; none at all when terminals are the only one.
pub(super) fn tabs(branch: &Branch<'_>) -> Vec<Tab> {
    let node = branch.group.node;
    let panes = &branch.group.panes;
    let mut tabs = vec![Tab {
        area: Area::Terminals,
        count: panes.iter().filter(|p| terminal(node, p)).count(),
    }];
    if !node.containers.is_empty() || node.docker == Some(false) {
        tabs.push(Tab {
            area: Area::Containers,
            count: node.containers.iter().filter(|c| shellable(c)).count(),
        });
    }
    let background = panes.iter().filter(|p| muffit(p)).count();
    if background > 0 {
        tabs.push(Tab {
            area: Area::Muffit,
            count: background,
        });
    }
    match tabs.len() {
        1 => Vec::new(),
        _ => tabs,
    }
}

pub(super) fn background<Q: Fn(&str) -> Option<String>>(
    rows: &mut Vec<Row>,
    branch: &Branch<'_>,
    look: &Look<'_, Q>,
) {
    for pane in branch.group.panes.iter().copied().filter(|p| muffit(p)) {
        push_pane(rows, pane, None, false, look.questions);
    }
}

pub(super) fn containers<Q: Fn(&str) -> Option<String>>(
    rows: &mut Vec<Row>,
    branch: &Branch<'_>,
    look: &Look<'_, Q>,
) {
    let node = branch.group.node;
    let stopped_open = look.hosts.stopped(&node.id);
    if node.docker == Some(false) {
        rows.push(Row::Note("docker is not answering here".into()));
    }
    let (shown, stopped): (Vec<&ContainerEntry>, Vec<&ContainerEntry>) = node
        .containers
        .iter()
        .partition(|c| wanted(c, look.now) || pinned(branch, c));
    let listed: Vec<&ContainerEntry> = match stopped_open {
        true => shown.into_iter().chain(stopped.iter().copied()).collect(),
        false => shown,
    };
    for (project, members) in by_project(listed) {
        if let Some(project) = project {
            rows.push(Row::Project(project.to_string()));
        }
        for container in members {
            rows.push(Row::Container {
                node: node.id.clone(),
                entry: Box::new(container.clone()),
            });
            rows.push(Row::Note(phrase(container, look.now)));
            let mut under = Vec::new();
            for pane in shells(branch, container) {
                push_pane(&mut under, pane, None, false, look.questions);
            }
            if let Some(child) = child(branch, container) {
                host(&mut under, child, look);
            }
            rows.extend(under.into_iter().map(|r| Row::Nested(Box::new(r))));
        }
    }
    if !stopped.is_empty() {
        rows.push(Row::Stopped {
            node: node.id.clone(),
            open: stopped_open,
            count: stopped.len(),
        });
    }
}

/// An adopted node whose container has no row to sit under: the container went, or the host is too
/// old to say which one it is. A host that is not serving draws no containers, so all of them.
pub(super) fn strays<Q: Fn(&str) -> Option<String>>(
    rows: &mut Vec<Row>,
    branch: &Branch<'_>,
    look: &Look<'_, Q>,
    all: bool,
) {
    let node = branch.group.node;
    for child in &branch.children {
        let id = child.group.node.id.as_str();
        if !all && node.containers.iter().any(|c| c.child.as_deref() == Some(id)) {
            continue;
        }
        let mut under = Vec::new();
        host(&mut under, child, look);
        rows.extend(under.into_iter().map(|r| Row::Nested(Box::new(r))));
    }
}

fn by_project(containers: Vec<&ContainerEntry>) -> Vec<(Option<&str>, Vec<&ContainerEntry>)> {
    let mut groups: BTreeMap<Option<&str>, Vec<&ContainerEntry>> = BTreeMap::new();
    for c in containers {
        groups.entry(c.project.as_deref()).or_default().push(c);
    }
    groups
        .into_iter()
        .map(|(project, mut members)| {
            members.sort_by(|a, b| a.name.cmp(&b.name));
            (project, members)
        })
        .collect()
}

/// `up 3h`, or `exited 137 · OOM 3m ago`: what the Compose client's `containerPhrase` says.
pub fn phrase(container: &ContainerEntry, now: i64) -> String {
    match container.state.as_str() {
        "running" => container
            .started_at
            .map_or("running".into(), |at| match elapsed(now - at) {
                None => "up <1m".into(),
                Some(e) => format!("up {e}"),
            }),
        "exited" | "dead" => {
            let mut how = container.state.clone();
            if let Some(code) = container.exit_code {
                how = format!("{how} {code}");
            }
            if container.oom {
                how = format!("{how} · OOM");
            }
            match container.finished_at {
                Some(at) => format!("{how} {}", ago(at, now)),
                None => how,
            }
        }
        "missing" => "gone".into(),
        "" => "unknown".into(),
        other => other.into(),
    }
}

pub fn ago(at: i64, now: i64) -> String {
    match elapsed(now - at) {
        None => "just now".into(),
        Some(e) => format!("{e} ago"),
    }
}

fn elapsed(seconds: i64) -> Option<String> {
    match seconds {
        s if s < 45 => None,
        s if s < 3600 => Some(format!("{}m", s / 60)),
        s if s < DAY => Some(format!("{}h", s / 3600)),
        s => Some(format!("{}d", s / DAY)),
    }
}

pub(super) fn stopped_line<'a>(
    open: bool,
    count: usize,
    t: &Theme,
    base: Style,
    lifted: Style,
    width: u16,
) -> Line<'a> {
    let text = match open {
        true => format!("    hide {count} stopped"),
        false => format!("    +{count} stopped"),
    };
    Line::from(spread((text, lifted.fg(t.dim)), None, width, base)).style(base)
}

pub(super) fn container_line<'a>(
    entry: &ContainerEntry,
    t: &Theme,
    base: Style,
    lifted: Style,
    width: u16,
) -> Line<'a> {
    let sick = entry.health.as_deref() == Some("unhealthy");
    let alarm = sick || entry.adopt_error.is_some();
    let (glyph, colour) = match (alarm, entry.state.as_str()) {
        (true, _) => ("■", t.blocked),
        (_, "running") => ("●", t.done),
        (_, "restarting") => ("◐", t.working),
        (_, "exited" | "dead") if entry.exit_code.unwrap_or(0) != 0 => ("○", t.blocked),
        _ => ("○", t.idle),
    };
    let mut badges = Vec::new();
    if entry.restarts > 0 {
        badges.push(format!("↻{}", entry.restarts));
    }
    if sick {
        badges.push("unhealthy".into());
    }
    if entry.adopt_error.is_some() {
        badges.push("adoption failing".into());
    }
    let tone = if alarm { t.blocked } else { t.working };
    let right = (!badges.is_empty()).then(|| (format!("{} ", badges.join(" ")), base.fg(tone)));
    let mut spans = vec![
        Span::styled("  ", base),
        Span::styled(format!("{glyph} "), base.fg(colour)),
    ];
    let plus = shellable(entry);
    spans.extend(spread(
        (entry.name.clone(), lifted),
        right,
        width.saturating_sub(if plus { 7 } else { 4 }),
        base,
    ));
    if plus {
        spans.push(Span::styled(
            " + ",
            base.fg(t.accent).add_modifier(Modifier::BOLD),
        ));
    }
    Line::from(spans).style(base)
}
