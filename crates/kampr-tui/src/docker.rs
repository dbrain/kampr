use crate::sidebar::{ago, phrase};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent};
use kampr_core::wire::ContainerEntry;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(3);
const EVENTS: usize = 12;
const LOGS: usize = 20;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Shell,
    Restart,
    Power,
    Adopt,
}

fn running(entry: &ContainerEntry) -> bool {
    matches!(entry.state.as_str(), "running" | "restarting" | "paused")
}

impl Op {
    fn name(self, entry: &ContainerEntry) -> &'static str {
        match self {
            Self::Shell => "docker.shell",
            Self::Restart => "docker.restart",
            Self::Power if running(entry) => "docker.stop",
            Self::Power => "docker.start",
            Self::Adopt if entry.child.is_some() => "docker.release",
            Self::Adopt => "docker.adopt",
        }
    }

    pub fn body(self, node: &str, entry: &ContainerEntry) -> Value {
        json!({ "op": self.name(entry), "node": node, "id": entry.id })
    }
}

pub fn ask(node: &str, id: &str) -> Value {
    json!({ "op": "docker.detail", "node": node, "id": id })
}

pub enum Took {
    Stay,
    Close,
    Act(Op),
}

pub struct Detail {
    pub node: String,
    pub id: String,
    data: Option<Value>,
    error: Option<String>,
    asked: Option<Instant>,
    answered: Option<Instant>,
    top: usize,
}

impl Detail {
    pub fn new(node: &str, id: &str) -> Self {
        Self {
            node: node.into(),
            id: id.into(),
            data: None,
            error: None,
            asked: None,
            answered: None,
            top: 0,
        }
    }

    pub fn due(&self, now: Instant) -> bool {
        self.asked.is_none() && self.answered.is_none_or(|at| now >= at + REFRESH)
    }

    pub fn asked(&mut self, at: Instant) {
        self.asked = Some(at);
    }

    pub fn answered(&mut self, detail: &Value, at: Instant) {
        self.data = Some(detail.clone());
        self.error = None;
        self.asked = None;
        self.answered = Some(at);
    }

    pub fn refused(&mut self, why: String, at: Instant) {
        self.error = Some(why);
        self.asked = None;
        self.answered = Some(at);
    }

    pub fn key(&mut self, key: KeyEvent) -> Took {
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Took::Close,
            KeyCode::Char('s') => return Took::Act(Op::Shell),
            KeyCode::Char('r') => return Took::Act(Op::Restart),
            KeyCode::Char('x') => return Took::Act(Op::Power),
            KeyCode::Char('a') => return Took::Act(Op::Adopt),
            KeyCode::Down | KeyCode::Char('j') => self.top = self.top.saturating_add(1),
            KeyCode::Up | KeyCode::Char('k') => self.top = self.top.saturating_sub(1),
            KeyCode::PageDown => self.top = self.top.saturating_add(10),
            KeyCode::PageUp => self.top = self.top.saturating_sub(10),
            _ => {}
        }
        Took::Stay
    }

    /// What the node said last, else what the herd says: the herd's entry is the floor while the
    /// first answer is on its way.
    pub fn entry(&self, herd: Option<&ContainerEntry>) -> Option<ContainerEntry> {
        self.data
            .as_ref()
            .and_then(|d| serde_json::from_value(d.clone()).ok())
            .or_else(|| herd.cloned())
    }

    pub fn render(
        &mut self,
        herd: Option<&ContainerEntry>,
        now: i64,
        area: Rect,
        buf: &mut Buffer,
        t: &Theme,
    ) {
        let entry = self.entry(herd);
        let lines = self.lines(entry.as_ref(), now, t);
        let height = area.height.saturating_sub(3) as usize;
        self.top = self.top.min(lines.len().saturating_sub(height));
        let name = entry.as_ref().map_or(self.id.as_str(), |e| e.name.as_str());
        let mut shown: Vec<Line> = lines.into_iter().skip(self.top).take(height).collect();
        while shown.len() < height {
            shown.push(Line::default());
        }
        shown.push(Line::from(Span::styled(
            footer(entry.as_ref()),
            Style::default().fg(t.mute),
        )));
        Clear.render(area, buf);
        Paragraph::new(shown)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(Span::styled(
                        format!(" {name} · docker "),
                        Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                    )),
            )
            .style(Style::default().fg(t.text).bg(t.surface))
            .render(area, buf);
    }

    fn lines(&self, entry: Option<&ContainerEntry>, now: i64, t: &Theme) -> Vec<Line<'static>> {
        let head = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
        let mute = Style::default().fg(t.mute);
        let body = Style::default().fg(t.text);
        let field = |label: &str, value: String| {
            Line::from(vec![
                Span::styled(format!("{label:<8} "), mute),
                Span::styled(value, body),
            ])
        };
        let mut out = Vec::new();
        if let Some(why) = &self.error {
            out.push(Line::from(Span::styled(
                format!("docker.detail was refused · {why}"),
                Style::default().fg(t.blocked),
            )));
        }
        let Some(entry) = entry else {
            out.push(Line::from(Span::styled("asking the node…", mute)));
            return out;
        };
        let d = self.data.clone().unwrap_or(Value::Null);
        let mut state = vec![phrase(entry, now)];
        state.extend(entry.health.clone());
        if entry.restarts > 0 {
            state.push(format!("↻{} restarts", entry.restarts));
        }
        if entry.crashes > 0 {
            state.push(format!("{} crashes in 24h", entry.crashes));
        }
        out.push(field("state", state.join(" · ")));
        out.push(field("image", entry.image.clone()));
        if let Some(command) = d["command"].as_str() {
            out.push(field("command", command.to_string()));
        }
        if let Some(stats) = d.get("stats").filter(|s| s.is_object()) {
            out.push(field("stats", stats_line(stats)));
        }
        if !entry.ports.is_empty() {
            out.push(field("ports", entry.ports.join(", ")));
        }
        if let Some(why) = &entry.adopt_error {
            out.push(Line::from(Span::styled(
                format!("adoption failing · {why}"),
                Style::default().fg(t.blocked),
            )));
        }
        if let Some(child) = &entry.child {
            out.push(field("node", format!("adopted · {child}")));
        }
        if let Some(crash) = &entry.last_crash {
            let oom = if crash.oom { " · OOM" } else { "" };
            out.push(field(
                "crashed",
                format!("exit {}{oom} {}", crash.exit_code, ago(crash.at, now)),
            ));
        }
        let events = list(&d, "events");
        if !events.is_empty() {
            out.push(Line::default());
            out.push(Line::from(Span::styled("events", head)));
            for event in events.iter().rev().take(EVENTS) {
                let mut what = vec![event["action"].as_str().unwrap_or("?").to_string()];
                if let Some(code) = event["exit_code"].as_i64() {
                    what.push(format!("exit {code}"));
                }
                if let Some(signal) = scalar(&event["signal"]) {
                    what.push(format!("signal {signal}"));
                }
                if let Some(health) = event["health"].as_str() {
                    what.push(health.to_string());
                }
                let when = event["at"].as_i64().map(|at| ago(at, now)).unwrap_or_default();
                out.push(Line::from(vec![
                    Span::styled(format!("  {when:<9} "), mute),
                    Span::styled(what.join(" · "), body),
                ]));
            }
        }
        if let Some(check) = list(&d, "health_log").last() {
            out.push(Line::default());
            let code = check["exit_code"]
                .as_i64()
                .map(|c| format!(" · exit {c}"))
                .unwrap_or_default();
            out.push(Line::from(Span::styled(format!("health{code}"), head)));
            for line in check["output"].as_str().unwrap_or_default().lines() {
                out.push(Line::from(Span::styled(format!("  {line}"), body)));
            }
        }
        section(
            &mut out,
            "logs",
            head,
            list(&d, "logs").iter().rev().take(LOGS).rev().map(|l| {
                let style = match l["stream"].as_str() {
                    Some("stderr") => Style::default().fg(t.blocked),
                    _ => body,
                };
                Line::from(Span::styled(
                    format!("  {}", l["line"].as_str().unwrap_or_default()),
                    style,
                ))
            }),
        );
        section(
            &mut out,
            "env",
            head,
            list(&d, "env")
                .iter()
                .filter_map(|e| e.as_str())
                .map(|e| Line::from(Span::styled(format!("  {e}"), body))),
        );
        section(
            &mut out,
            "mounts",
            head,
            list(&d, "mounts").iter().map(|m| {
                let mode = m["mode"]
                    .as_str()
                    .filter(|m| !m.is_empty())
                    .map(|m| format!(" ({m})"))
                    .unwrap_or_default();
                Line::from(Span::styled(
                    format!(
                        "  {} → {}{mode}",
                        m["src"].as_str().unwrap_or_default(),
                        m["dst"].as_str().unwrap_or_default()
                    ),
                    body,
                ))
            }),
        );
        out
    }
}

fn section(
    out: &mut Vec<Line<'static>>,
    title: &str,
    head: Style,
    lines: impl Iterator<Item = Line<'static>>,
) {
    let lines: Vec<Line> = lines.collect();
    if lines.is_empty() {
        return;
    }
    out.push(Line::default());
    out.push(Line::from(Span::styled(title.to_string(), head)));
    out.extend(lines);
}

fn list<'a>(d: &'a Value, key: &str) -> &'a [Value] {
    d[key].as_array().map(Vec::as_slice).unwrap_or_default()
}

fn scalar(v: &Value) -> Option<String> {
    match v {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        _ => None,
    }
}

fn stats_line(stats: &Value) -> String {
    let mut parts = Vec::new();
    if let Some(cpu) = stats["cpu_pct"].as_f64() {
        parts.push(format!("cpu {cpu:.1}%"));
    }
    if let Some(used) = stats["mem_used"].as_u64() {
        match stats["mem_limit"].as_u64().filter(|l| *l > 0) {
            Some(limit) => parts.push(format!(
                "mem {} / {} ({}%)",
                bytes(used),
                bytes(limit),
                used * 100 / limit
            )),
            None => parts.push(format!("mem {}", bytes(used))),
        }
    }
    if let Some(pids) = stats["pids"].as_u64() {
        parts.push(format!("{pids} pids"));
    }
    parts.join(" · ")
}

fn bytes(n: u64) -> String {
    let mib = n as f64 / 1_048_576.0;
    match mib {
        m if m >= 1024.0 => format!("{:.1} GB", m / 1024.0),
        m if m >= 1.0 => format!("{} MB", m as u64),
        _ => format!("{} KB", n / 1024),
    }
}

fn footer(entry: Option<&ContainerEntry>) -> String {
    let power = match entry.is_some_and(running) {
        true => "stop",
        false => "start",
    };
    let adopt = match entry.is_some_and(|e| e.child.is_some()) {
        true => "release",
        false => "adopt",
    };
    format!("s shell in · r restart · x {power} · a {adopt} · j/k scroll · esc close")
}
