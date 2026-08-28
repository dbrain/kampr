//! What the last frame actually drew, and the rectangles a click resolves against.
//!
//! Splitting the draw path out of [`super::App`] is only a file boundary: these are inherent
//! methods on the same type, and a child module can still see its parent's fields. The seam is
//! state and input on one side, paint on the other.

use super::{App, Screen, View};
use crate::keymap::Mode;
use crate::render::fit::{self, Pan, Placement};
use crate::render::grid::{self, Grid, Highlight};
use crate::sidebar::{self, Sidebar};
use crate::theme::Theme;
use kampr_client::PendingOption;
use kampr_core::wire::PaneEntry;
use kampr_term::Cell;
use ratatui::Frame;
use ratatui::layout::{Constraint, Layout as Split, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget};

/// One pane as the last frame drew it: the box it was given, and where its live grid landed
/// inside it. **The rect is not the grid** — a pan and a scroll move the columns and rows under
/// it, so a click worked out of the rect alone names the wrong cell (#68, #84, #205).
#[derive(Debug, Clone)]
pub struct Placed {
    pub pane: String,
    pub rect: Rect,
    /// How many rows of ring sit above the live grid, which is where the surface's row numbering
    /// starts from.
    pub ring: u16,
    /// `None` for a pane drawing something that is not a grid: a conversation, a reason, or the
    /// wait for the first frame.
    pub placement: Option<Placement>,
}

/// A pending prompt's option chips, hit-tested by [`crate::mouse::Mouse::answer`].
#[derive(Debug, Clone)]
pub struct Chips {
    pub pane: String,
    pub options: Vec<PendingOption>,
    pub rects: Vec<Rect>,
}

/// The rectangles the last frame actually drew, so a click resolves against what is on screen
/// rather than against a layout recomputed from stale state. W5's hit testing reads this.
#[derive(Debug, Clone, Default)]
pub struct Layout {
    pub sidebar: Rect,
    /// Every sidebar line in the order it was drawn, and the pane it opens when it opens one.
    pub rows: Vec<(Option<String>, Rect)>,
    pub tabs: Vec<(String, Rect)>,
    pub panes: Vec<Placed>,
    /// The herd view's pane rows — the triage screen a desk cannot draw.
    pub herd: Vec<(String, Rect)>,
    pub chips: Vec<Chips>,
    /// An attachment's marker: pane, id, and the row it was drawn on.
    pub attachments: Vec<(String, String, Rect)>,
    pub status: Rect,
    pub footer: Rect,
}

impl Layout {
    pub fn pane(&self, id: &str) -> Option<&Placed> {
        self.panes.iter().find(|placed| placed.pane == id)
    }
}

fn rows_of(area: Rect, from: usize, count: usize) -> Vec<Rect> {
    (0..count.min(area.height as usize))
        .map(|i| Rect {
            x: area.x,
            y: area.y + (from + i) as u16,
            width: area.width,
            height: 1,
        })
        .collect()
}

impl App {
    pub fn draw(&mut self, frame: &mut Frame) {
        self.settle_manage();
        let area = frame.area();
        let t = self.options.theme;
        let mut layout = Layout::default();
        let body = if self.sidebar_open && area.width > sidebar::WIDTH * 2 {
            let [left, right] =
                Split::horizontal([Constraint::Length(sidebar::WIDTH), Constraint::Min(10)]).areas(area);
            layout.sidebar = left;
            let rows = self.rows();
            let selected = matches!(self.router.mode(), Mode::Navigate).then_some(self.pick);
            let top = self.sidebar_view(rows.len(), left.height as usize, selected);
            Sidebar {
                rows: &rows,
                theme: &t,
                selected,
                focused: self.focused(),
                top,
            }
            .render(left, frame.buffer_mut());
            layout.rows = rows
                .iter()
                .skip(top)
                .take(left.height as usize)
                .zip(rows_of(left, 0, left.height as usize))
                .map(|(row, rect)| (row.pane().map(str::to_string), rect))
                .collect();
            right
        } else {
            area
        };
        let [strip, panes, status, footer] = Split::vertical([
            Constraint::Length(1),
            Constraint::Min(3),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(body);
        layout.status = status;
        layout.footer = footer;
        self.draw_strip(frame, strip, &mut layout);
        match self.screen {
            Screen::Panes => self.draw_panes(frame, panes, &mut layout),
            Screen::Herd => self.draw_herd(frame, panes, &mut layout),
        }
        self.draw_status(frame, status);
        self.draw_footer(frame, footer);
        if self.keybinds {
            self.draw_keybinds(frame, area);
        }
        self.manage.render(frame.buffer_mut(), panes, &t);
        self.layout = layout;
    }

    fn draw_strip(&self, frame: &mut Frame, area: Rect, layout: &mut Layout) {
        let t = self.options.theme;
        let here = self
            .focus
            .as_ref()
            .and_then(|f| self.client.state().herd.pane(f).and_then(|p| p.tab_id.clone()));
        let mut spans = Vec::new();
        let mut x = area.x;
        for (id, name) in self.tabs() {
            let on = Some(id.as_str()) == here.as_deref();
            let text = match on {
                true => format!(" ❰ {name} ❱ "),
                false => format!("  {name}  "),
            };
            let width = text.chars().count() as u16;
            layout.tabs.push((
                id,
                Rect {
                    x,
                    y: area.y,
                    width,
                    height: 1,
                },
            ));
            x = x.saturating_add(width);
            spans.push(Span::styled(
                text,
                match on {
                    true => Style::default()
                        .fg(t.on_accent)
                        .bg(t.accent)
                        .add_modifier(Modifier::BOLD),
                    false => Style::default().fg(t.dim).bg(t.bar),
                },
            ));
            spans.push(Span::styled("│", Style::default().fg(t.line).bg(t.bar)));
        }
        // A write affordance a read-only device cannot use is absent, not disabled.
        if self.writes() && self.client.state().caps().manage {
            spans.push(Span::styled(" + ", Style::default().fg(t.mute).bg(t.bar)));
        }
        Paragraph::new(Line::from(spans))
            .style(Style::default().bg(t.bar))
            .render(area, frame.buffer_mut());
    }

    fn draw_panes(&mut self, frame: &mut Frame, area: Rect, layout: &mut Layout) {
        let mosaic = self.mosaic();
        if mosaic.is_empty() {
            let t = self.options.theme;
            let state = self.client.state();
            let why = match state.herd.known {
                true => "no panes in this herd",
                false => "waiting for the herd",
            };
            Paragraph::new(why)
                .style(Style::default().fg(t.mute).bg(t.bg))
                .render(area, frame.buffer_mut());
            return;
        }
        let shown = mosaic.len().min(4);
        let focused_at = mosaic
            .iter()
            .position(|p| Some(p.as_str()) == self.focus.as_deref())
            .unwrap_or(0);
        // Resize mode's `h`/`l` move this, and it is kampr's own split — the pane is untouched.
        let widths: Vec<Constraint> = match shown {
            2 => (0..2)
                .map(|i| match i == focused_at {
                    true => Constraint::Percentage(self.split),
                    false => Constraint::Percentage(100 - self.split),
                })
                .collect(),
            n => (0..n).map(|_| Constraint::Ratio(1, n as u32)).collect(),
        };
        let boxes = Split::horizontal(widths).split(area);
        let mut caret = None;
        for (pane, rect) in mosaic.iter().zip(boxes.iter()) {
            if let Some(at) = self.draw_pane(frame, *rect, pane, layout) {
                caret = Some(at);
            }
        }
        if let Some(at) = caret
            && !self.router.modal()
        {
            frame.set_cursor_position(at);
        }
    }

    fn draw_pane(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        pane: &str,
        layout: &mut Layout,
    ) -> Option<ratatui::layout::Position> {
        let t = self.options.theme;
        let focused = self.focus.as_deref() == Some(pane);
        let mut placed = Placed {
            pane: pane.to_string(),
            rect: area,
            ring: 0,
            placement: None,
        };
        let (title, detail, stale, painted, geometry, entry_cols) = {
            let state = self.client.state();
            let entry = state.herd.pane(pane).cloned();
            let held = state.pane(pane);
            (
                entry.as_ref().map(label).unwrap_or_else(|| pane.to_string()),
                entry.as_ref().and_then(|e| e.detail.clone()),
                held.is_some_and(|p| p.stale()),
                held.is_some_and(|p| p.painted()),
                held.map(|p| p.geometry()).unwrap_or((0, 0)),
                entry.as_ref().and_then(|e| e.cols),
            )
        };
        let border = match focused {
            true => Style::default().fg(t.accent),
            false => Style::default().fg(t.line),
        };
        let head = match stale {
            true => format!(" {title} · stale "),
            false => format!(" {title} "),
        };
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(border)
            .title(Span::styled(
                head,
                Style::default()
                    .fg(if focused { t.accent_hi } else { t.dim })
                    .add_modifier(Modifier::BOLD),
            ))
            .style(Style::default().bg(t.bg));
        let inner = block.inner(area);
        block.render(area, frame.buffer_mut());
        if inner.width == 0 || inner.height == 0 {
            layout.panes.push(placed);
            return None;
        }
        // #233: a pane whose stream is dead must say why. No `grid.reset` is sent for one, so a
        // blank grid with a flashing cursor is exactly the lie this field exists to prevent.
        if let Some(detail) = detail
            && !painted
        {
            Paragraph::new(vec![
                Line::from(Span::styled(
                    "no picture",
                    Style::default().fg(t.blocked).add_modifier(Modifier::BOLD),
                )),
                Line::from(Span::styled(detail, Style::default().fg(t.dim))),
            ])
            .render(inner, frame.buffer_mut());
            layout.panes.push(placed);
            return None;
        }
        if self.view(pane) == View::Conversation && self.convo.has(pane) {
            let role = self.client.state().role;
            let marks = self
                .convo
                .render(frame.buffer_mut(), inner, pane, &t, &mut self.images, role);
            if let Some(pending) = self.convo.pending(pane) {
                layout.chips.push(Chips {
                    pane: pane.to_string(),
                    options: pending.options.clone(),
                    rects: marks.options,
                });
            }
            for (id, rect) in marks.attachments {
                layout.attachments.push((pane.to_string(), id, rect));
            }
            layout.panes.push(placed);
            return None;
        }
        if !painted {
            Paragraph::new(Span::styled(
                "waiting for the first frame",
                Style::default().fg(t.mute),
            ))
            .render(inner, frame.buffer_mut());
            layout.panes.push(placed);
            return None;
        }
        // Never derive geometry: `cols` is absent until measured, and the layout rect is a width
        // no row was ever wrapped at.
        let need = fit::Need {
            cols: entry_cols.unwrap_or(geometry.0),
            rows: geometry.1,
        };
        let ring = self.rings.get(pane).map(Vec::as_slice).unwrap_or_default();
        let placement = fit::place(
            inner,
            need,
            ring.len().min(u16::MAX as usize) as u16,
            self.scrolls.get(pane).copied().unwrap_or_default(),
            self.pans.get(pane).copied().unwrap_or_default(),
        );
        self.pans.insert(pane.to_string(), placement.pan);
        self.scrolls.insert(pane.to_string(), placement.scroll);
        placed.ring = ring.len().min(u16::MAX as usize) as u16;
        placed.placement = Some(placement);
        let selected = self
            .mouse
            .selection()
            .filter(|s| s.pane == pane)
            .map(|s| Highlight {
                from: s.from,
                to: s.to,
                block: s.block,
            });
        let state = self.client.state();
        let held = state.pane(pane)?;
        if placement.history.height > 0 {
            Grid {
                rows: &ring[placement.skip_history as usize..],
                pan: Pan {
                    col: placement.pan.col,
                    row: 0,
                },
                theme: &t,
                base: placement.skip_history,
                selected,
            }
            .render(placement.history, frame.buffer_mut());
        }
        if placement.grid.height > 0 {
            Grid {
                rows: held.rows(),
                pan: Pan {
                    col: placement.pan.col,
                    row: placement.skip_grid,
                },
                theme: &t,
                base: placed.ring,
                selected,
            }
            .render(placement.grid, frame.buffer_mut());
        }
        // The caret belongs to the live viewport; scrolled into the ring there is nothing for it
        // to sit on, and a caret drawn over history is a lie about where typing lands.
        let caret = (focused && placement.scroll == 0)
            .then(|| grid::caret(held.cursor(), placement.grid, placement.pan))
            .flatten();
        // Assembling the surface costs a fat pointer per line of scrollback, so it is built for
        // the one frame after a click or a drag rather than every frame.
        let pointed = match self.mouse.wants(pane) {
            true => {
                let surface: Vec<&[Cell]> = ring
                    .iter()
                    .map(Vec::as_slice)
                    .chain(held.rows().iter().map(Vec::as_slice))
                    .collect();
                (
                    self.mouse.copy(&surface, need.cols),
                    self.mouse.link(&surface, held.links(), need.cols),
                )
            }
            false => (None, None),
        };
        drop(state);
        self.pointed(pointed.0, pointed.1);
        layout.panes.push(placed);
        caret
    }

    fn draw_herd(&mut self, frame: &mut Frame, area: Rect, layout: &mut Layout) {
        let t = self.options.theme;
        let rows: Vec<(Option<String>, Line)> = {
            let state = self.client.state();
            let mut rows = Vec::new();
            for pane in sidebar::triage(&state.herd) {
                if pane.agent_status != kampr_core::provider::AgentStatus::Blocked {
                    break;
                }
                rows.push((Some(pane.id.clone()), herd_line(pane, &state.herd, &t, true)));
            }
            for group in state.herd.groups() {
                rows.push((
                    None,
                    Line::from(Span::styled(
                        format!(" {} ", group.node.name),
                        Style::default().fg(t.accent).add_modifier(Modifier::BOLD),
                    )),
                ));
                for pane in group.panes {
                    rows.push((Some(pane.id.clone()), herd_line(pane, &state.herd, &t, false)));
                }
            }
            rows
        };
        layout.herd = rows
            .iter()
            .zip(rows_of(area, 0, rows.len()))
            .filter_map(|((pane, _), rect)| pane.clone().map(|id| (id, rect)))
            .collect();
        Paragraph::new(rows.into_iter().map(|(_, line)| line).collect::<Vec<_>>())
            .style(Style::default().bg(t.bg).fg(t.text))
            .render(area, frame.buffer_mut());
    }

    fn draw_status(&self, frame: &mut Frame, area: Rect) {
        let t = self.options.theme;
        let mut parts: Vec<String> = Vec::new();
        if let Some(pane) = &self.focus {
            parts.push(pane.clone());
            let state = self.client.state();
            if let Some(entry) = state.herd.pane(pane) {
                if let Some(agent) = &entry.agent {
                    parts.push(agent.clone());
                }
                // The status line is the one surface wide enough for a whole path, and a pane's
                // directory is most of what says which pane it is. `~` because the node's own
                // `$HOME` is not on the wire and never will be — see `naming::home_relative`.
                if let Some(cwd) = &entry.cwd {
                    parts.push(kampr_core::naming::home_relative(cwd).into_owned());
                }
                // Never derive geometry. The herd entry omits `cols` until something has
                // measured it; `grid.reset` always carries the measurement, so the shadow is
                // the second place a width can honestly come from — and the rect is neither.
                let measured = entry
                    .cols
                    .or_else(|| state.pane(pane).map(|p| p.geometry().0).filter(|c| *c > 0));
                match measured {
                    Some(cols) => parts.push(format!("{cols}×{}", entry.rows)),
                    None => parts.push(format!("?×{}", entry.rows)),
                }
                if let Some(watchers) = entry.watchers.filter(|w| *w > 1) {
                    parts.push(format!("{watchers} watching"));
                }
            }
            if let Some(held) = state.pane(pane) {
                let (cols, _) = held.geometry();
                let pan = self.pans.get(pane).copied().unwrap_or_default();
                let shown = self
                    .layout
                    .pane(pane)
                    .map(|placed| placed.rect.width.saturating_sub(2))
                    .unwrap_or(cols);
                if shown < cols {
                    parts.push(format!(
                        "⇱ {}–{}/{cols}",
                        pan.col + 1,
                        (pan.col + shown).min(cols)
                    ));
                }
                match self.scrolls.get(pane).copied().unwrap_or_default() {
                    0 => {}
                    up => parts.push(format!("\u{2191} {up} back")),
                }
            }
            if let Some(footer) = self.mouse.footer(pane) {
                parts.push(footer);
            }
        }
        if let Some(url) = &self.offered {
            parts.push(format!("link {url} · ^b o opens it"));
        }
        if let Some(rung) = &self.rung {
            parts.push(rung.report());
        }
        if !self.client.state().connected {
            parts.push("disconnected".into());
        }
        if !self.writes() {
            parts.push("readonly".into());
        }
        if !self.note.is_empty() && self.noted.elapsed() < super::NOTE_LIFETIME {
            parts.push(self.note.clone());
        }
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", parts.join(" · ")),
            Style::default().fg(t.dim).bg(t.bar),
        )))
        .render(area, frame.buffer_mut());
    }

    fn draw_footer(&self, frame: &mut Frame, area: Rect) {
        let t = self.options.theme;
        let line = match self
            .manage
            .footer()
            .or_else(|| self.router.footer().map(str::to_string))
        {
            Some(text) => Span::styled(
                format!(" {text}"),
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD),
            ),
            None => {
                let mut hints = vec!["^b ? help", "^b w sidebar", "^b ⇧h herd", "^b q detach"];
                if self.writes() && self.client.state().caps().manage {
                    hints.insert(3, "^b c new");
                    hints.insert(4, "^b , rename");
                }
                Span::styled(
                    format!(" {}", hints.join("  ")),
                    Style::default().fg(t.mute).bg(t.bar),
                )
            }
        };
        Paragraph::new(Line::from(line))
            .style(Style::default().bg(t.bar))
            .render(area, frame.buffer_mut());
    }

    /// **Sized to the terminal and scrollable.** It used to be `rows + 2` clamped to the screen
    /// with no way to move it, so on a short terminal the tail was cut and unreachable — which is
    /// the half a first-time reader most needs.
    fn draw_keybinds(&mut self, frame: &mut Frame, area: Rect) {
        let t = self.options.theme;
        let manage = self.writes() && self.client.state().caps().manage;
        let mut lines: Vec<Line> = Vec::new();
        for section in crate::help(manage) {
            if !lines.is_empty() {
                lines.push(Line::default());
            }
            lines.push(Line::from(Span::styled(
                format!(" {} ", section.title),
                Style::default()
                    .fg(t.on_accent)
                    .bg(t.accent)
                    .add_modifier(Modifier::BOLD),
            )));
            for (bind, what) in section.rows {
                lines.push(Line::from(vec![
                    Span::styled(format!(" {bind:<28}"), Style::default().fg(t.accent)),
                    Span::styled((*what).to_string(), Style::default().fg(t.text)),
                ]));
            }
        }
        // Wide enough for the longest row this table holds — a 28-column bind beside a
        // 57-column description — and capped at the terminal, which then clips rather than wraps.
        let width = 90.min(area.width);
        let height = area.height.min(lines.len() as u16 + 4);
        let body = height.saturating_sub(4) as usize;
        let top = self.help_top.min(lines.len().saturating_sub(body));
        self.help_top = top;
        let more = top + body < lines.len();
        let foot = match (top > 0, more) {
            (_, true) => " up/down · pgup/pgdn more · esc closes ",
            (true, false) => " up/down back · esc closes ",
            (false, false) => " esc closes ",
        };
        let shown: Vec<Line> = std::iter::once(Line::from(Span::styled(
            format!(" {}", crate::HELP_HEAD),
            Style::default().fg(t.mute),
        )))
        .chain(lines.into_iter().skip(top).take(body))
        .collect();
        let popup = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + (area.height.saturating_sub(height)) / 2,
            width,
            height,
        };
        Clear.render(popup, frame.buffer_mut());
        Paragraph::new(shown)
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(Span::styled(
                        " kampr ",
                        Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                    ))
                    .title_bottom(Span::styled(foot, Style::default().fg(t.mute))),
            )
            .style(Style::default().bg(t.surface))
            .render(popup, frame.buffer_mut());
    }
}

fn label(entry: &PaneEntry) -> String {
    sidebar::name(entry)
}

fn herd_line<'a>(pane: &PaneEntry, herd: &kampr_client::Herd, t: &Theme, flag: bool) -> Line<'a> {
    let node = herd
        .node(&pane.node_id)
        .map(|n| n.name.clone())
        .unwrap_or_default();
    let mark = if flag { "⚑ " } else { "  " };
    Line::from(vec![
        Span::styled(
            format!("{mark}{:<14}", node),
            Style::default().fg(t.status(pane.agent_status)),
        ),
        Span::styled(
            format!("{:<20}", pane.workspace.clone().unwrap_or_default()),
            Style::default().fg(t.text),
        ),
        Span::styled(
            format!("{:<10}", pane.agent.clone().unwrap_or_else(|| "shell".into())),
            Style::default().fg(t.dim),
        ),
        Span::styled(pane.id.clone(), Style::default().fg(t.mute)),
    ])
}
