//! The session and tasks sheets (docs/27): what a source lists, as rows a keyboard can work.

use super::declared::{Held, task_name};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use kampr_client::convo::Control;
use kampr_client::convo::word;
use kampr_core::conversation::{Steer, Stream};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Widget, Wrap};
use serde_json::{Map, Value, json};

const WIDTH: u16 = 76;
/// Steers that say nothing without words: a rename is a name, an aside is a question.
const WORDED: [Steer; 2] = [Steer::Rename, Steer::Aside];
/// Steers with a home of their own: stop is `alt+k`, and stopping a task is the tasks sheet's `x`.
const ELSEWHERE: [Steer; 2] = [Steer::Interrupt, Steer::StopTask];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Session,
    Tasks,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Do {
    Set { control: String, value: Value },
    Act { action: String, args: Value },
    Steer { steer: String, text: Option<String> },
    Follow(String),
    Sub { id: String, head: String },
}

pub enum Took {
    Stay,
    Close,
    Do(Do),
}

#[derive(Debug, Clone)]
enum Row {
    Control(usize),
    Action(usize),
    Steer(String),
    Task(usize),
}

#[derive(Debug, Clone)]
enum Then {
    Set(usize),
    Arg {
        action: usize,
        index: usize,
        args: Map<String, Value>,
    },
    Steer(String),
}

#[derive(Debug, Clone)]
enum Stage {
    List,
    Choose { control: usize, at: usize },
    Line { prompt: String, buf: String, then: Then },
    Confirm { sentence: String, then: Do },
    Output(String),
}

pub struct Sheet {
    pub pane: String,
    pub kind: Kind,
    at: usize,
    stage: Stage,
    acted: Option<String>,
}

impl Sheet {
    pub fn new(pane: &str, kind: Kind) -> Self {
        Self {
            pane: pane.to_string(),
            kind,
            at: 0,
            stage: Stage::List,
            acted: None,
        }
    }

    pub fn acted(&mut self, text: String) {
        self.acted = Some(text);
    }

    fn rows(&self, held: &Held) -> Vec<Row> {
        match self.kind {
            Kind::Session => {
                let mut rows: Vec<Row> = (0..held.controls.len()).map(Row::Control).collect();
                rows.extend((0..held.actions.len()).map(Row::Action));
                let elsewhere: Vec<String> = ELSEWHERE.into_iter().map(word).collect();
                rows.extend(
                    held.abilities
                        .steer
                        .iter()
                        .filter(|s| !elsewhere.contains(s))
                        .map(|s| Row::Steer(s.clone())),
                );
                rows
            }
            Kind::Tasks => (0..held.tasks.len()).map(Row::Task).collect(),
        }
    }

    pub fn key(&mut self, key: KeyEvent, held: &Held) -> Took {
        let stage = std::mem::replace(&mut self.stage, Stage::List);
        let (stage, took) = self.step(stage, key, held);
        self.stage = stage;
        took
    }

    fn step(&mut self, stage: Stage, key: KeyEvent, held: &Held) -> (Stage, Took) {
        match stage {
            Stage::List => self.list(key, held),
            Stage::Choose { control, at } => {
                let options = held.controls.get(control).map_or(0, |c| c.options.len());
                match key.code {
                    KeyCode::Esc => (Stage::List, Took::Stay),
                    KeyCode::Up => (
                        Stage::Choose {
                            control,
                            at: at.saturating_sub(1),
                        },
                        Took::Stay,
                    ),
                    KeyCode::Down => (
                        Stage::Choose {
                            control,
                            at: (at + 1).min(options.saturating_sub(1)),
                        },
                        Took::Stay,
                    ),
                    KeyCode::Enter => match held
                        .controls
                        .get(control)
                        .and_then(|c| c.options.get(at).map(|o| (c, o)))
                    {
                        Some((c, o)) => confirmed(c.confirm.clone(), set(c, json!(o.value))),
                        None => (Stage::List, Took::Stay),
                    },
                    _ => (Stage::Choose { control, at }, Took::Stay),
                }
            }
            Stage::Line {
                prompt,
                mut buf,
                then,
            } => match key.code {
                KeyCode::Esc => (Stage::List, Took::Stay),
                KeyCode::Backspace => {
                    buf.pop();
                    (Stage::Line { prompt, buf, then }, Took::Stay)
                }
                KeyCode::Char(c) if !key.modifiers.contains(KeyModifiers::CONTROL) => {
                    buf.push(c);
                    (Stage::Line { prompt, buf, then }, Took::Stay)
                }
                KeyCode::Enter => line_done(prompt, buf, then, held),
                _ => (Stage::Line { prompt, buf, then }, Took::Stay),
            },
            Stage::Confirm { sentence, then } => match key.code {
                KeyCode::Enter | KeyCode::Char('y') => (Stage::List, Took::Do(then)),
                KeyCode::Esc | KeyCode::Char('n') => (Stage::List, Took::Stay),
                _ => (Stage::Confirm { sentence, then }, Took::Stay),
            },
            Stage::Output(id) => match key.code {
                KeyCode::Esc | KeyCode::Char('q') => (Stage::List, Took::Stay),
                _ => (Stage::Output(id), Took::Stay),
            },
        }
    }

    fn list(&mut self, key: KeyEvent, held: &Held) -> (Stage, Took) {
        let rows = self.rows(held);
        self.at = self.at.min(rows.len().saturating_sub(1));
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => (Stage::List, Took::Close),
            KeyCode::Up | KeyCode::Char('k') => {
                self.at = self.at.saturating_sub(1);
                (Stage::List, Took::Stay)
            }
            KeyCode::Down | KeyCode::Char('j') => {
                self.at = (self.at + 1).min(rows.len().saturating_sub(1));
                (Stage::List, Took::Stay)
            }
            KeyCode::Char('x') => match rows.get(self.at) {
                Some(Row::Task(i)) if held.abilities.steers(Steer::StopTask) && held.tasks[*i].running() => (
                    Stage::List,
                    Took::Do(Do::Steer {
                        steer: word(Steer::StopTask),
                        text: Some(held.tasks[*i].id.clone()),
                    }),
                ),
                _ => (Stage::List, Took::Stay),
            },
            KeyCode::Enter => match rows.get(self.at) {
                Some(row) => self.open(row.clone(), held),
                None => (Stage::List, Took::Stay),
            },
            _ => (Stage::List, Took::Stay),
        }
    }

    fn open(&mut self, row: Row, held: &Held) -> (Stage, Took) {
        match row {
            Row::Control(i) => {
                let c = &held.controls[i];
                match c.kind.as_str() {
                    "choice" => {
                        let at = c
                            .options
                            .iter()
                            .position(|o| Some(json!(o.value)) == c.value)
                            .unwrap_or(0);
                        (Stage::Choose { control: i, at }, Took::Stay)
                    }
                    "toggle" => {
                        let on = c.value.as_ref().and_then(Value::as_bool).unwrap_or(false);
                        confirmed(c.confirm.clone(), set(c, json!(!on)))
                    }
                    _ => (
                        Stage::Line {
                            prompt: c.label.clone(),
                            buf: shown(c.value.as_ref()),
                            then: Then::Set(i),
                        },
                        Took::Stay,
                    ),
                }
            }
            Row::Action(i) => next_arg(i, 0, Map::new(), held),
            Row::Steer(steer) => match WORDED.into_iter().map(word).any(|w| w == steer) {
                true => (
                    Stage::Line {
                        prompt: steer.clone(),
                        buf: String::new(),
                        then: Then::Steer(steer),
                    },
                    Took::Stay,
                ),
                false => (Stage::List, Took::Do(Do::Steer { steer, text: None })),
            },
            Row::Task(i) => {
                let task = &held.tasks[i];
                match task.kind.as_str() {
                    "agent" => (
                        Stage::List,
                        Took::Do(Do::Sub {
                            id: task.id.clone(),
                            head: task_name(task),
                        }),
                    ),
                    _ => (
                        Stage::Output(task.id.clone()),
                        Took::Do(Do::Follow(task.id.clone())),
                    ),
                }
            }
        }
    }

    pub fn render(&self, held: &Held, area: Rect, buf: &mut Buffer, t: &Theme) {
        let width = area.width.min(WIDTH);
        let mut lines = Vec::new();
        let dim = Style::default().fg(t.dim);
        if self.kind == Kind::Session {
            let facts = held.facts();
            if !facts.is_empty() {
                lines.push(Line::from(Span::styled(facts.join(" · "), dim)));
                lines.push(Line::default());
            }
        }
        match &self.stage {
            Stage::Output(id) => {
                let out = held.output.get(id);
                let shown: Vec<&String> = out
                    .map(|o| o.lines.iter().rev().take(20).collect())
                    .unwrap_or_default();
                if shown.is_empty() {
                    lines.push(Line::from(Span::styled("waiting for output", dim)));
                }
                lines.extend(shown.into_iter().rev().map(|l| Line::from(l.clone())));
                if let Some(end) = out.and_then(|o| o.ended.as_ref()) {
                    let word = if end.ok { "ended" } else { "failed" };
                    lines.push(Line::from(Span::styled(
                        format!("{word}: {}", end.summary.clone().unwrap_or_default()),
                        Style::default().fg(if end.ok { t.done } else { t.blocked }),
                    )));
                }
                if !held.abilities.stream(Stream::TaskOutput) {
                    lines.push(Line::from(Span::styled(
                        "this source does not stream task output",
                        dim,
                    )));
                }
            }
            stage => {
                for (i, row) in self.rows(held).iter().enumerate() {
                    let picked = i == self.at;
                    let mark = if picked { "▸ " } else { "  " };
                    let style = match picked {
                        true => Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                        false => Style::default().fg(t.text),
                    };
                    let (label, note) = row_text(row, held);
                    lines.push(Line::from(vec![
                        Span::styled(format!("{mark}{label}"), style),
                        Span::styled(format!("  {note}"), dim),
                    ]));
                    if picked
                        && let (Stage::Choose { control, at }, Row::Control(i)) = (stage, row)
                        && control == i
                    {
                        for (n, o) in held.controls[*i].options.iter().enumerate() {
                            let mark = if n == *at { "    ● " } else { "    ○ " };
                            let detail = o.detail.clone().map(|d| format!("  {d}")).unwrap_or_default();
                            lines.push(Line::from(vec![
                                Span::styled(format!("{mark}{}", o.label), Style::default().fg(t.text)),
                                Span::styled(detail, dim),
                            ]));
                        }
                    }
                }
                if self.rows(held).is_empty() {
                    lines.push(Line::from(Span::styled("nothing listed", dim)));
                }
                match stage {
                    Stage::Line { prompt, buf, .. } => {
                        lines.push(Line::default());
                        lines.push(Line::from(vec![
                            Span::styled(format!("{prompt}: "), Style::default().fg(t.accent)),
                            Span::styled(format!("{buf}▏"), Style::default().fg(t.text)),
                        ]));
                    }
                    Stage::Confirm { sentence, .. } => {
                        lines.push(Line::default());
                        lines.push(Line::from(Span::styled(
                            sentence.clone(),
                            Style::default().fg(t.working),
                        )));
                        lines.push(Line::from(Span::styled("y yes · n no", dim)));
                    }
                    _ => {}
                }
            }
        }
        if let Some(acted) = &self.acted {
            lines.push(Line::default());
            lines.push(Line::from(Span::styled(
                acted.clone(),
                Style::default().fg(t.done),
            )));
        }
        let height = (lines.len() as u16 + 2).min(area.height);
        let rect = Rect {
            x: area.x + (area.width - width) / 2,
            y: area.y + area.height.saturating_sub(height) / 3,
            width,
            height,
        };
        let (title, keys) = match (self.kind, &self.stage) {
            (_, Stage::Output(_)) => (" output ", " esc back "),
            (Kind::Session, _) => (" session ", " ↑↓ move · enter change or run · esc close "),
            (Kind::Tasks, _) => (" tasks ", " ↑↓ move · enter open · x stop · esc close "),
        };
        Clear.render(rect, buf);
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(Style::default().fg(t.accent))
                    .title(Span::styled(
                        title,
                        Style::default().fg(t.accent_hi).add_modifier(Modifier::BOLD),
                    ))
                    .title_bottom(Span::styled(keys, Style::default().fg(t.mute))),
            )
            .style(Style::default().bg(t.surface))
            .render(rect, buf);
    }
}

fn row_text(row: &Row, held: &Held) -> (String, String) {
    match row {
        Row::Control(i) => {
            let c = &held.controls[*i];
            (c.label.clone(), current(c))
        }
        Row::Action(i) => {
            let a = &held.actions[*i];
            (a.label.clone(), a.detail.clone().unwrap_or_default())
        }
        Row::Steer(s) => (s.clone(), String::new()),
        Row::Task(i) => {
            let task = &held.tasks[*i];
            let mark = if task.running() { "◐" } else { "■" };
            let indent = if task.parent.is_some() { "  " } else { "" };
            (format!("{indent}{mark} {}", task_name(task)), task.kind.clone())
        }
    }
}

fn current(c: &Control) -> String {
    match c.kind.as_str() {
        "toggle" => match c.value.as_ref().and_then(Value::as_bool) {
            Some(true) => "on".into(),
            _ => "off".into(),
        },
        _ => c
            .options
            .iter()
            .find(|o| Some(json!(o.value)) == c.value)
            .map(|o| o.label.clone())
            .unwrap_or_else(|| shown(c.value.as_ref())),
    }
}

fn shown(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

fn set(c: &Control, value: Value) -> Do {
    Do::Set {
        control: c.id.clone(),
        value,
    }
}

fn confirmed(sentence: Option<String>, then: Do) -> (Stage, Took) {
    match sentence {
        Some(sentence) => (Stage::Confirm { sentence, then }, Took::Stay),
        None => (Stage::List, Took::Do(then)),
    }
}

fn next_arg(action: usize, index: usize, args: Map<String, Value>, held: &Held) -> (Stage, Took) {
    let a = &held.actions[action];
    match a.args.get(index) {
        Some(spec) => {
            let prompt = match spec.options.is_empty() {
                true => spec.label.clone(),
                false => format!("{} ({})", spec.label, spec.options.join(", ")),
            };
            (
                Stage::Line {
                    prompt,
                    buf: String::new(),
                    then: Then::Arg { action, index, args },
                },
                Took::Stay,
            )
        }
        None => confirmed(
            a.confirm.clone(),
            Do::Act {
                action: a.id.clone(),
                args: Value::Object(args),
            },
        ),
    }
}

fn line_done(prompt: String, buf: String, then: Then, held: &Held) -> (Stage, Took) {
    let text = buf.trim().to_string();
    match then {
        Then::Set(i) => {
            let c = &held.controls[i];
            let value = match (c.kind.as_str(), text.parse::<i64>()) {
                ("number", Ok(n)) => json!(n),
                _ => json!(text),
            };
            confirmed(c.confirm.clone(), set(c, value))
        }
        Then::Arg {
            action,
            index,
            mut args,
        } => {
            let spec = &held.actions[action].args[index];
            if text.is_empty() && spec.required {
                return (
                    Stage::Line {
                        prompt,
                        buf,
                        then: Then::Arg { action, index, args },
                    },
                    Took::Stay,
                );
            }
            if !text.is_empty() {
                args.insert(spec.name.clone(), json!(text));
            }
            next_arg(action, index + 1, args, held)
        }
        Then::Steer(steer) if text.is_empty() => (
            Stage::Line {
                prompt,
                buf,
                then: Then::Steer(steer),
            },
            Took::Stay,
        ),
        Then::Steer(steer) => (
            Stage::List,
            Took::Do(Do::Steer {
                steer,
                text: Some(text),
            }),
        ),
    }
}
