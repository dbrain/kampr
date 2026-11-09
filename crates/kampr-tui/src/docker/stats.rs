use super::bytes;
use crate::theme::Theme;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

const LABEL: usize = 10;
const VALUE: usize = 22;

fn tone(fraction: f64, t: &Theme) -> ratatui::style::Color {
    match fraction {
        f if f > 0.9 => t.blocked,
        f if f > 0.7 => t.working,
        _ => t.done,
    }
}

fn label(text: &str, t: &Theme) -> Span<'static> {
    Span::styled(
        format!("{text:<LABEL$}"),
        Style::default().fg(t.mute).add_modifier(Modifier::BOLD),
    )
}

fn meter(name: &str, value: String, fraction: Option<f64>, width: usize, t: &Theme) -> Line<'static> {
    let mut spans = vec![label(name, t)];
    if let Some(fraction) = fraction {
        let fraction = fraction.clamp(0.0, 1.0);
        let room = width.saturating_sub(LABEL + VALUE + 2).max(4);
        let filled = ((fraction * room as f64).round() as usize).min(room);
        spans.push(Span::styled(
            "█".repeat(filled),
            Style::default().fg(tone(fraction, t)),
        ));
        spans.push(Span::styled(
            "░".repeat(room - filled),
            Style::default().fg(t.line),
        ));
        spans.push(Span::raw("  "));
    }
    spans.push(Span::styled(value, Style::default().fg(t.text)));
    Line::from(spans)
}

fn fact(name: &str, value: String, t: &Theme) -> Line<'static> {
    Line::from(vec![
        label(name, t),
        Span::styled(value, Style::default().fg(t.text)),
    ])
}

pub(super) fn lines(d: &Value, width: usize, t: &Theme) -> Vec<Line<'static>> {
    let Some(stats) = d.get("stats").filter(|s| s.is_object()) else {
        return vec![Line::from(Span::styled(
            "usage is read only while the container runs",
            Style::default().fg(t.mute),
        ))];
    };
    let mut out = Vec::new();
    if let Some(cpu) = stats["cpu_pct"].as_f64() {
        out.push(meter("cpu", format!("{cpu:.1}%"), Some(cpu / 100.0), width, t));
    }
    if let Some(used) = stats["mem_used"].as_u64() {
        let limit = stats["mem_limit"]
            .as_u64()
            .filter(|l| *l > 0)
            .or_else(|| d["limits"]["memory"].as_u64().filter(|l| *l > 0));
        out.push(match limit {
            Some(limit) => meter(
                "memory",
                format!("{} / {}", bytes(used), bytes(limit)),
                Some(used as f64 / limit as f64),
                width,
                t,
            ),
            None => meter("memory", format!("{}, no limit", bytes(used)), None, width, t),
        });
    }
    out.push(Line::default());
    if let Some(pids) = stats["pids"].as_u64() {
        out.push(fact("processes", pids.to_string(), t));
    }
    let n = |key: &str| stats[key].as_u64().unwrap_or(0);
    out.push(fact(
        "network",
        format!("↓ {}  ↑ {}", bytes(n("net_rx")), bytes(n("net_tx"))),
        t,
    ));
    out.push(fact(
        "disk",
        format!("read {}  written {}", bytes(n("blk_read")), bytes(n("blk_write"))),
        t,
    ));
    out
}
