use super::{Detail, Op, Tab, alive};
use crate::sidebar::{ago, phrase};
use crate::theme::Theme;
use kampr_core::wire::ContainerEntry;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};
use serde_json::Value;

const WIDTH: u16 = 96;
const HEIGHT: u16 = 28;
const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
/// The action bar, the tab row and the rule under it.
const CHROME: u16 = 3;

/// One size whatever it holds, so a 3 s refresh never moves the box under the operator's eye.
fn placed(area: Rect) -> Rect {
    let width = area.width.saturating_sub(4).min(WIDTH);
    let height = area.height.saturating_sub(2).min(HEIGHT);
    Rect {
        x: area.x + (area.width - width) / 2,
        y: area.y + (area.height - height) / 2,
        width,
        height,
    }
}

fn badge(text: String, colour: Color, t: &Theme) -> [Span<'static>; 2] {
    [
        Span::raw(" "),
        Span::styled(
            format!(" {text} "),
            Style::default().fg(t.bg).bg(colour).add_modifier(Modifier::BOLD),
        ),
    ]
}

fn title(name: &str, entry: Option<&ContainerEntry>, t: &Theme) -> Line<'static> {
    let mut spans = vec![Span::styled(
        format!(" {name} "),
        Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
    )];
    if let Some(e) = entry {
        let state = match e.state.as_str() {
            "running" => t.done,
            "restarting" | "paused" => t.working,
            _ if e.exit_code.unwrap_or(0) != 0 => t.blocked,
            _ => t.mute,
        };
        spans.extend(badge(e.state.clone(), state, t));
        if let Some(health) = &e.health {
            let tone = match health.as_str() {
                "healthy" => t.done,
                "unhealthy" => t.blocked,
                _ => t.working,
            };
            spans.extend(badge(health.clone(), tone, t));
        }
        if e.oom {
            spans.extend(badge("OOM".into(), t.blocked, t));
        }
        if e.restarts > 0 {
            spans.extend(badge(format!("↻{}", e.restarts), t.working, t));
        }
        spans.push(Span::raw(" "));
    }
    Line::from(spans)
}

fn actions(entry: Option<&ContainerEntry>, t: &Theme) -> Line<'static> {
    let Some(e) = entry else {
        return Line::default();
    };
    let power = if alive(e) { "stop" } else { "start" };
    let adopt = if e.child.is_some() { "release" } else { "adopt" };
    let all = [
        (Op::Shell, "s", "shell"),
        (Op::Restart, "r", "restart"),
        (Op::Power, "x", power),
        (Op::Kill, "K", "kill"),
        (Op::Remove, "D", "remove"),
        (Op::Adopt, "a", adopt),
    ];
    let mut spans = Vec::new();
    for (_, key, word) in all.into_iter().filter(|(op, ..)| op.applies(e)) {
        if !spans.is_empty() {
            spans.push(Span::styled(" · ", Style::default().fg(t.mute)));
        }
        spans.push(Span::styled(
            key,
            Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
        ));
        spans.push(Span::styled(format!(" {word}"), Style::default().fg(t.text)));
    }
    Line::from(spans)
}

fn tabs(on: Tab, t: &Theme) -> Line<'static> {
    let mut spans = Vec::new();
    for (i, tab) in Tab::ALL.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("│", Style::default().fg(t.mute)));
        }
        let style = match tab == on {
            true => Style::default()
                .fg(t.on_accent)
                .bg(t.accent)
                .add_modifier(Modifier::BOLD),
            false => Style::default().fg(t.dim),
        };
        spans.push(Span::styled(format!(" {} {} ", i + 1, tab.word()), style));
    }
    Line::from(spans)
}

impl Detail {
    pub fn render(
        &mut self,
        herd: Option<&ContainerEntry>,
        now: i64,
        area: Rect,
        buf: &mut Buffer,
        t: &Theme,
    ) {
        let entry = self.entry(herd);
        let rect = placed(area);
        let inner_w = rect.width.saturating_sub(2) as usize;
        let body_h = rect.height.saturating_sub(2 + CHROME) as usize;
        let name = entry.as_ref().map_or(self.id.clone(), |e| e.name.clone());
        let mut shown = vec![
            actions(entry.as_ref(), t),
            tabs(self.tab, t),
            Line::from(Span::styled("─".repeat(inner_w), Style::default().fg(t.line))),
        ];
        shown.extend(self.body(entry.as_ref(), now, inner_w, body_h, t));
        Clear.render(rect, buf);
        Paragraph::new(shown)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(title(&name, entry.as_ref(), t))
                    .title_bottom(Span::styled(
                        " 1-5 h/l tab · j/k pgup/pgdn scroll · ←/→ pan logs · g/G top/tail · esc close ",
                        Style::default().fg(t.mute),
                    )),
            )
            .style(Style::default().fg(t.text).bg(t.surface))
            .render(rect, buf);
    }

    fn body(
        &mut self,
        entry: Option<&ContainerEntry>,
        now: i64,
        width: usize,
        height: usize,
        t: &Theme,
    ) -> Vec<Line<'static>> {
        let mut lines = Vec::new();
        if let Some(why) = &self.error {
            lines.push(Line::from(Span::styled(
                format!("docker.detail was refused · {why}"),
                Style::default().fg(t.blocked),
            )));
        }
        if self.data.is_none() && self.error.is_none() {
            let frame = (self.opened.elapsed().as_millis() / 80) as usize % SPINNER.len();
            let mut out = vec![Line::default(); height / 2];
            out.push(Line::from(Span::styled(
                format!("{:>w$}", SPINNER[frame], w = width / 2),
                Style::default().fg(t.accent),
            )));
            return out;
        }
        let d = self.data.clone().unwrap_or(Value::Null);
        match (self.tab, entry) {
            (Tab::Info, Some(e)) => lines.extend(info(e, &d, now, t)),
            (Tab::Logs, _) => lines.extend(logs(&d, t)),
            (Tab::Stats, _) => lines.extend(super::stats::lines(&d, width, t)),
            (Tab::Env, _) => lines.extend(
                list(&d, "env")
                    .iter()
                    .filter_map(|e| e.as_str())
                    .map(|e| Line::from(Span::styled(e.to_string(), Style::default().fg(t.text)))),
            ),
            (Tab::Mounts, _) => lines.extend(list(&d, "mounts").iter().map(|m| {
                let mode = m["mode"]
                    .as_str()
                    .filter(|m| !m.is_empty())
                    .map(|m| format!(" ({m})"))
                    .unwrap_or_default();
                Line::from(Span::styled(
                    format!(
                        "{} → {}{mode}",
                        m["src"].as_str().unwrap_or_default(),
                        m["dst"].as_str().unwrap_or_default()
                    ),
                    Style::default().fg(t.text),
                ))
            })),
            (Tab::Info, None) => {}
        }
        if lines.is_empty() {
            lines.push(Line::from(Span::styled(
                "nothing here",
                Style::default().fg(t.mute),
            )));
        }
        self.bottom = lines.len().saturating_sub(height);
        if self.tab == Tab::Logs {
            if self.follow || self.top >= self.bottom {
                self.follow = true;
                self.top = self.bottom;
            }
            let widest = lines.iter().map(Line::width).max().unwrap_or(0);
            self.left = self.left.min(widest.saturating_sub(width));
        }
        self.top = self.top.min(self.bottom);
        lines
            .into_iter()
            .skip(self.top)
            .take(height)
            .map(|line| pan(line, self.left))
            .collect()
    }
}

/// No wrap: a log line is cut at the box's edge and `left` columns are scrolled off its start.
fn pan(line: Line<'static>, left: usize) -> Line<'static> {
    if left == 0 {
        return line;
    }
    let mut skip = left;
    let spans = line
        .spans
        .into_iter()
        .filter_map(|span| {
            let n = span.content.chars().count();
            if skip >= n {
                skip -= n;
                return None;
            }
            let rest: String = span.content.chars().skip(skip).collect();
            skip = 0;
            Some(Span::styled(rest, span.style))
        })
        .collect::<Vec<_>>();
    Line::from(spans)
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

fn logs(d: &Value, t: &Theme) -> Vec<Line<'static>> {
    list(d, "logs")
        .iter()
        .map(|l| {
            let tone = match l["stream"].as_str() {
                Some("stderr") => t.blocked,
                _ => t.text,
            };
            Line::from(Span::styled(
                l["line"].as_str().unwrap_or_default().to_string(),
                Style::default().fg(tone),
            ))
        })
        .collect()
}

fn info(e: &ContainerEntry, d: &Value, now: i64, t: &Theme) -> Vec<Line<'static>> {
    let head = Style::default().fg(t.accent).add_modifier(Modifier::BOLD);
    let mute = Style::default().fg(t.mute);
    let body = Style::default().fg(t.text);
    let field = |label: &str, value: String| {
        Line::from(vec![
            Span::styled(format!("{label:<9} "), mute),
            Span::styled(value, body),
        ])
    };
    let mut out = Vec::new();
    let mut status = vec![phrase(e, now)];
    status.extend(e.health.clone());
    out.push(field("status", status.join(" · ")));
    out.push(field("image", e.image.clone()));
    if let Some(command) = d["command"].as_str().filter(|c| !c.is_empty()) {
        out.push(field("command", command.to_string()));
    }
    if let Some(policy) = d["restart_policy"].as_str().filter(|p| !p.is_empty()) {
        out.push(field("restart", policy.to_string()));
    }
    if !e.ports.is_empty() {
        out.push(field("ports", e.ports.join(", ")));
    }
    let networks: Vec<&str> = list(d, "networks").iter().filter_map(Value::as_str).collect();
    if !networks.is_empty() {
        out.push(field("networks", networks.join(", ")));
    }
    let compose: Vec<String> = [e.project.clone(), e.service.clone()]
        .into_iter()
        .flatten()
        .collect();
    if !compose.is_empty() {
        out.push(field("compose", compose.join(" · ")));
    }
    if let Some(child) = &e.child {
        out.push(field("node", format!("adopted · {child}")));
    }
    if let Some(why) = &e.adopt_error {
        out.push(Line::from(Span::styled(
            format!("adoption failing · {why}"),
            Style::default().fg(t.blocked),
        )));
    }
    if let Some(crash) = &e.last_crash {
        let oom = if crash.oom { " · OOM" } else { "" };
        out.push(field(
            "crashed",
            format!("exit {}{oom} {}", crash.exit_code, ago(crash.at, now)),
        ));
    }
    if e.crashes > 0 {
        out.push(field("crashes", format!("{} in 24h", e.crashes)));
    }
    let events = list(d, "events");
    if !events.is_empty() {
        out.push(Line::default());
        out.push(Line::from(Span::styled("crash timeline", head)));
        for event in events.iter().rev() {
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
    if let Some(check) = list(d, "health_log").last() {
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
    out
}
