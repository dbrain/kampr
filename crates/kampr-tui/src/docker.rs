mod stats;
mod view;

use crossterm::event::{KeyCode, KeyEvent};
use kampr_core::wire::ContainerEntry;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

const REFRESH: Duration = Duration::from_secs(3);
const PAGE: usize = 10;
const PAN: usize = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Shell,
    Restart,
    Power,
    Kill,
    Remove,
    Adopt,
}

pub fn alive(entry: &ContainerEntry) -> bool {
    matches!(entry.state.as_str(), "running" | "restarting" | "paused")
}

impl Op {
    fn name(self, entry: &ContainerEntry) -> &'static str {
        match self {
            Self::Shell => "docker.shell",
            Self::Restart => "docker.restart",
            Self::Power if alive(entry) => "docker.stop",
            Self::Power => "docker.start",
            Self::Kill => "docker.kill",
            Self::Remove => "docker.remove",
            Self::Adopt if entry.child.is_some() => "docker.release",
            Self::Adopt => "docker.adopt",
        }
    }

    pub fn applies(self, entry: &ContainerEntry) -> bool {
        match self {
            Self::Kill => alive(entry),
            Self::Remove => !alive(entry),
            _ => true,
        }
    }

    pub fn body(self, node: &str, entry: &ContainerEntry) -> Value {
        json!({ "op": self.name(entry), "node": node, "id": entry.id })
    }
}

pub fn ask(node: &str, id: &str) -> Value {
    json!({ "op": "docker.detail", "node": node, "id": id })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    Info,
    Logs,
    Stats,
    Env,
    Mounts,
}

impl Tab {
    const ALL: [Tab; 5] = [Tab::Info, Tab::Logs, Tab::Stats, Tab::Env, Tab::Mounts];

    fn word(self) -> &'static str {
        match self {
            Self::Info => "Info",
            Self::Logs => "Logs",
            Self::Stats => "Stats",
            Self::Env => "Env",
            Self::Mounts => "Mounts",
        }
    }

    fn step(self, by: i32) -> Self {
        let at = Self::ALL.iter().position(|t| *t == self).unwrap_or(0) as i32;
        Self::ALL[(at + by).rem_euclid(Self::ALL.len() as i32) as usize]
    }
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
    opened: Instant,
    tab: Tab,
    top: usize,
    left: usize,
    follow: bool,
    /// The furthest `top` the last frame could scroll to, so a key knows where the tail was.
    bottom: usize,
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
            opened: Instant::now(),
            tab: Tab::Info,
            top: 0,
            left: 0,
            follow: true,
            bottom: 0,
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

    fn show(&mut self, tab: Tab) {
        if tab != self.tab {
            self.tab = tab;
            self.top = 0;
            self.left = 0;
            self.follow = true;
        }
    }

    fn up(&mut self, by: usize) {
        if self.tab == Tab::Logs && self.follow {
            self.top = self.bottom;
        }
        self.follow = false;
        self.top = self.top.saturating_sub(by);
    }

    fn down(&mut self, by: usize) {
        if !(self.tab == Tab::Logs && self.follow) {
            self.top = self.top.saturating_add(by);
        }
    }

    pub fn key(&mut self, key: KeyEvent) -> Took {
        let logs = self.tab == Tab::Logs;
        match key.code {
            KeyCode::Esc | KeyCode::Char('q') => return Took::Close,
            KeyCode::Char('s') => return Took::Act(Op::Shell),
            KeyCode::Char('r') => return Took::Act(Op::Restart),
            KeyCode::Char('x') => return Took::Act(Op::Power),
            KeyCode::Char('K') => return Took::Act(Op::Kill),
            KeyCode::Char('D') => return Took::Act(Op::Remove),
            KeyCode::Char('a') => return Took::Act(Op::Adopt),
            KeyCode::Char(c @ '1'..='5') => self.show(Tab::ALL[c as usize - '1' as usize]),
            KeyCode::Char('l') | KeyCode::Tab => self.show(self.tab.step(1)),
            KeyCode::Char('h') | KeyCode::BackTab => self.show(self.tab.step(-1)),
            KeyCode::Right if logs => self.left = self.left.saturating_add(PAN),
            KeyCode::Left if logs => self.left = self.left.saturating_sub(PAN),
            KeyCode::Right => self.show(self.tab.step(1)),
            KeyCode::Left => self.show(self.tab.step(-1)),
            KeyCode::Down | KeyCode::Char('j') => self.down(1),
            KeyCode::Up | KeyCode::Char('k') => self.up(1),
            KeyCode::PageDown => self.down(PAGE),
            KeyCode::PageUp => self.up(PAGE),
            KeyCode::Home | KeyCode::Char('g') => {
                self.follow = false;
                self.top = 0;
                self.left = 0;
            }
            KeyCode::End | KeyCode::Char('G') => {
                self.follow = logs;
                self.top = self.bottom;
            }
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
}

pub fn bytes(n: u64) -> String {
    let mib = n as f64 / 1_048_576.0;
    match mib {
        m if m >= 1024.0 => format!("{:.1} GB", m / 1024.0),
        m if m >= 1.0 => format!("{} MB", m as u64),
        _ => format!("{} KB", n / 1024),
    }
}
