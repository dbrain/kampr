use super::{Area, Hit, Tab, elide, len};
use crate::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

pub(super) struct Heading<'a> {
    pub name: &'a str,
    pub serving: bool,
    pub local: bool,
    pub alarm: bool,
    pub tabs: &'a [Tab],
    pub area: Area,
}

fn count(n: usize) -> String {
    match n {
        0..=9 => n.to_string(),
        _ => "9+".into(),
    }
}

fn cells(tabs: &[Tab]) -> Vec<(Area, String)> {
    tabs.iter()
        .map(|tab| {
            let text = match tab.area {
                Area::Terminals => format!(" {} ", tab.area.glyph()),
                area => format!(" {} {} ", area.glyph(), count(tab.count)),
            };
            (tab.area, text)
        })
        .collect()
}

fn right(cells: &[(Area, String)]) -> u16 {
    let tabs: u16 = cells.iter().map(|(_, text)| len(text)).sum();
    tabs + cells.len().saturating_sub(1) as u16 + 3
}

/// Where each cell of the switch sits on a heading drawn at `rect`, so a click lands on what the
/// frame drew.
pub(super) fn spots(node: &str, tabs: &[Tab], rect: Rect) -> Vec<(Hit, Rect)> {
    let cells = cells(tabs);
    let mut x = rect.x + rect.width.saturating_sub(right(&cells));
    let mut out = Vec::new();
    for (area, text) in cells {
        out.push((
            Hit::Area {
                node: node.to_string(),
                area,
            },
            Rect {
                x,
                width: len(&text),
                ..rect
            },
        ));
        x += len(&text) + 1;
    }
    out
}

pub(super) fn line<'a>(h: Heading<'_>, t: &Theme, picked: bool, width: u16) -> Line<'a> {
    let band = Style::default().bg(if picked { t.accent_soft } else { t.surface });
    let dot = match (h.serving, h.alarm) {
        (false, _) => ("○", t.mute),
        (true, true) => ("●", t.blocked),
        (true, false) => ("●", t.done),
    };
    let cells = cells(h.tabs);
    let room = width.saturating_sub(right(&cells) + 1) as usize;
    let lead = elide(&format!(" {}", h.name), room);
    let pad = width.saturating_sub(len(&lead) + right(&cells)) as usize;
    let name = match h.local {
        true => band.fg(t.accent),
        false => band.fg(t.text),
    };
    let mut spans = vec![
        Span::styled(lead, name.add_modifier(Modifier::BOLD)),
        Span::styled(" ".repeat(pad), band),
    ];
    for (i, (area, text)) in cells.into_iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("│", band.fg(t.mute)));
        }
        let style = match area == h.area {
            true => band.fg(t.text).add_modifier(Modifier::REVERSED | Modifier::BOLD),
            false => band.fg(t.dim),
        };
        spans.push(Span::styled(text, style));
    }
    spans.push(Span::styled(format!(" {} ", dot.0), band.fg(dot.1)));
    Line::from(spans).style(band)
}
