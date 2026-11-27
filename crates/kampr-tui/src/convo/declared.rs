//! What a conversation's source declares beyond its turns (docs/21 Phase 2): kept per pane, drawn
//! as rows above the reply box, and asked by the keys before they offer anything. A piece the
//! source does not declare takes no row and answers no key, which is the web client's rule.

use crate::theme::Theme;
use kampr_client::Event;
use kampr_client::convo::{Abilities, Control, ConvoUpdate, Live, Task, Trouble};
use kampr_core::conversation::{
    Action, ContextUse, Fact, InputAbility, Limits, SessionFacts, Steer, Suggestion, TaskEnd,
};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use std::collections::HashMap;

const FACTS: [Fact; 4] = [Fact::Account, Fact::Billing, Fact::Limits, Fact::Context];

#[derive(Debug, Default)]
pub struct Declared {
    panes: HashMap<String, Held>,
}

#[derive(Debug, Default, Clone)]
pub struct Held {
    pub abilities: Abilities,
    pub live: Option<Live>,
    pub tasks: Vec<Task>,
    pub output: HashMap<String, Output>,
    pub state: Option<String>,
    pub trouble: Option<Trouble>,
    pub session: Option<SessionFacts>,
    pub controls: Vec<Control>,
    pub actions: Vec<Action>,
    pub limits: Option<Limits>,
    pub context: Option<ContextUse>,
    pub suggested: Option<Suggested>,
    /// This client's own sends that went with an id, oldest first, until the harness hears them.
    pub queued: Vec<(String, String)>,
}

#[derive(Debug, Default, Clone)]
pub struct Output {
    pub lines: Vec<String>,
    pub ended: Option<TaskEnd>,
}

#[derive(Debug, Clone)]
pub struct Suggested {
    pub kind: String,
    pub query: String,
    pub items: Vec<Suggestion>,
}

impl Held {
    pub fn running(&self) -> bool {
        self.state.as_deref() == Some("running")
    }

    pub fn can(&self, ability: InputAbility) -> bool {
        self.abilities.input(ability)
    }

    pub fn stops(&self) -> bool {
        self.running() && self.abilities.steers(Steer::Interrupt)
    }

    /// A reply goes as one `input` carrying its own id wherever the source works a queue; a source
    /// that declares none keeps the text-then-return a terminal needs.
    pub fn queues(&self) -> bool {
        [
            InputAbility::QueueNext,
            InputAbility::SendNow,
            InputAbility::Unqueue,
        ]
        .into_iter()
        .any(|a| self.can(a))
    }

    pub fn has_session(&self) -> bool {
        self.facts_drawn()
            || !self.controls.is_empty()
            || !self.actions.is_empty()
            || !self.abilities.steer.is_empty()
    }

    fn facts_drawn(&self) -> bool {
        FACTS.into_iter().any(|f| self.abilities.fact(f))
    }

    pub fn facts(&self) -> Vec<String> {
        let mut parts = Vec::new();
        if let Some(model) = self.session.as_ref().and_then(|s| s.model.clone()) {
            parts.push(model);
        }
        if let Some(c) = self.context {
            parts.push(match c.total {
                0 => format!("context {}", tokens(c.used)),
                total => format!("context {}/{}", tokens(c.used), tokens(total)),
            });
        }
        if let Some(limits) = self.limits.as_ref().and_then(limits) {
            parts.push(limits);
        }
        if let Some(billing) = self.session.as_ref().and_then(billing) {
            parts.push(billing);
        }
        parts
    }

    /// The `/` or `@` word at the end of a draft, and the kind a source suggests for it.
    pub fn asking(&self, draft: &str) -> Option<(&'static str, String)> {
        if self.can(InputAbility::Slash)
            && let Some(rest) = draft.strip_prefix('/')
            && !rest.contains(char::is_whitespace)
        {
            return Some(("commands", rest.to_string()));
        }
        let (_, word) = draft
            .rsplit_once('@')
            .filter(|_| self.can(InputAbility::Mention))?;
        (!word.contains(char::is_whitespace)).then(|| ("files", word.to_string()))
    }

    /// What the draft becomes when the first suggestion is taken, while one matches it.
    pub fn complete(&self, draft: &str) -> Option<String> {
        let (kind, query) = self.asking(draft)?;
        let s = self
            .suggested
            .as_ref()
            .filter(|s| s.kind == kind && s.query == query)?;
        let first = s.items.first()?;
        Some(match kind {
            "commands" => format!("/{} ", first.label.trim_start_matches('/')),
            _ => format!("{}@{} ", &draft[..draft.rfind('@').unwrap_or_default()], first.id),
        })
    }
}

impl Declared {
    pub fn absorb(&mut self, event: &Event) {
        match event {
            Event::ConvoUpdate { pane, update } => self.apply(pane, update.clone()),
            Event::ConvoHeard { pane, text } => {
                if let Some(held) = self.panes.get_mut(pane)
                    && let Some(at) = held.queued.iter().position(|(_, t)| t.trim() == text.trim())
                {
                    held.queued.remove(at);
                }
            }
            Event::Disconnected { .. } => {
                for held in self.panes.values_mut() {
                    held.live = None;
                    held.state = None;
                    held.trouble = None;
                    held.output.clear();
                    held.suggested = None;
                    held.queued.clear();
                }
            }
            _ => {}
        }
    }

    fn apply(&mut self, pane: &str, update: ConvoUpdate) {
        let held = self.panes.entry(pane.to_string()).or_default();
        match update {
            ConvoUpdate::Abilities(a) => held.abilities = a,
            ConvoUpdate::Live(live) => {
                held.live = (!live.text.is_empty() || !live.tools.is_empty() || live.thinking.is_some())
                    .then_some(live)
            }
            ConvoUpdate::Tasks(tasks) => held.tasks = tasks,
            ConvoUpdate::Task { id, text, ended, .. } => {
                let out = held.output.entry(id).or_default();
                out.lines.extend(text.lines().map(str::to_string));
                if ended.is_some() {
                    out.ended = ended;
                }
            }
            ConvoUpdate::State(state) => held.state = Some(state),
            ConvoUpdate::Trouble(trouble) => held.trouble = trouble,
            ConvoUpdate::Session(facts) => held.session = Some(facts),
            ConvoUpdate::Controls(c) => held.controls = c,
            ConvoUpdate::Actions(a) => held.actions = a,
            ConvoUpdate::Limits(l) => held.limits = Some(l),
            ConvoUpdate::Context(c) => held.context = Some(c),
            ConvoUpdate::Suggested { kind, query, items } => {
                held.suggested = Some(Suggested { kind, query, items })
            }
            ConvoUpdate::Acted { .. } => {}
        }
    }

    pub fn get(&self, pane: &str) -> Option<&Held> {
        self.panes.get(pane)
    }

    pub fn sent(&mut self, pane: &str, id: String, text: String) {
        self.panes
            .entry(pane.to_string())
            .or_default()
            .queued
            .push((id, text));
    }

    pub fn take_back(&mut self, pane: &str) -> Option<(String, String)> {
        self.panes.get_mut(pane)?.queued.pop()
    }

    /// The rows above the reply box, top to bottom: trouble, the turn being written, what is
    /// running, the session's facts, and the suggestions for the word being typed.
    pub fn lines(&self, pane: &str, draft: &str, width: u16, t: &Theme) -> Vec<Line<'static>> {
        let Some(held) = self.panes.get(pane) else {
            return Vec::new();
        };
        let room = width as usize;
        let mut lines = Vec::new();
        if let Some(trouble) = &held.trouble {
            lines.push(row(" ⚠ ", &trouble_text(trouble), t.blocked, t, room));
        }
        if let Some(live) = &held.live {
            lines.push(row(" ✎ ", &live_text(live), t.working, t, room));
        }
        let running: Vec<&Task> = held.tasks.iter().filter(|t| t.running()).collect();
        if !running.is_empty() {
            let names: Vec<String> = running.iter().map(|t| task_name(t)).collect();
            let text = format!("{} running · {} · alt+t", running.len(), names.join(" · "));
            lines.push(row(" ◐ ", &text, t.working, t, room));
        }
        if held.facts_drawn() {
            let mut parts = held.facts();
            if held.has_session() {
                parts.push("alt+s".into());
            }
            lines.push(row(" ≡ ", &parts.join(" · "), t.dim, t, room));
        }
        if let Some((kind, query)) = held.asking(draft)
            && let Some(s) = held
                .suggested
                .as_ref()
                .filter(|s| s.kind == kind && s.query == query)
            && !s.items.is_empty()
        {
            let labels: Vec<&str> = s.items.iter().take(6).map(|i| i.label.as_str()).collect();
            lines.push(row(" tab ", &labels.join("  "), t.accent, t, room));
        }
        if held.stops() {
            let mut keys = vec!["alt+k stop"];
            if held.can(InputAbility::SendNow) {
                keys.push("alt+n send now");
            }
            if held.can(InputAbility::QueueNext) {
                keys.push("enter queues");
            }
            if held.can(InputAbility::Unqueue) && !held.queued.is_empty() {
                keys.push("alt+u take back");
            }
            lines.push(row(" ▶ ", &keys.join(" · "), t.mute, t, room));
        }
        lines
    }
}

pub fn task_name(task: &Task) -> String {
    let name = task
        .title
        .clone()
        .or_else(|| task.call.clone())
        .unwrap_or_else(|| task.id.clone());
    match task.backgrounded {
        true => format!("{name} · backgrounded"),
        false => name,
    }
}

pub fn trouble_text(trouble: &Trouble) -> String {
    let mut parts = vec![trouble.text.clone()];
    if let Some(r) = trouble.retry {
        parts.push(format!(
            "retry {} of {} in {}s",
            r.attempt,
            r.of,
            r.in_ms.div_ceil(1000)
        ));
    }
    if let Some(until) = trouble.until() {
        parts.push(format!("held until {until}"));
    }
    match trouble.fix() {
        Some("login") => parts.push("log in at the terminal · prefix shift+v".into()),
        Some("billing") => parts.push("alt+s for options".into()),
        _ => {}
    }
    parts.join(" · ")
}

fn live_text(live: &Live) -> String {
    let mut parts = Vec::new();
    if let Some(n) = live.thinking {
        parts.push(format!("thinking ~{}", tokens(n)));
    }
    if let Some(last) = live.text.lines().rev().find(|l| !l.trim().is_empty()) {
        parts.push(last.trim().to_string());
    }
    for tool in &live.tools {
        parts.push(format!("{} {}", tool.tool, tool.args.replace('\n', " ")));
    }
    parts.join(" · ")
}

fn limits(l: &Limits) -> Option<String> {
    let mut parts: Vec<String> = l.plan.iter().cloned().collect();
    match &l.usage {
        Some(serde_json::Value::String(s)) => parts.push(s.clone()),
        Some(serde_json::Value::Number(n)) => parts.push(format!("{n}%")),
        _ => {}
    }
    if let Some(at) = &l.resets_at {
        parts.push(format!("resets {at}"));
    }
    (!parts.is_empty()).then(|| parts.join(" · "))
}

fn billing(s: &SessionFacts) -> Option<String> {
    let who = match (&s.billing.provider, &s.billing.key) {
        (Some(p), _) => format!("billed by {p}"),
        (None, Some(k)) => format!("key {k}"),
        (None, None) => return s.billing.rate_limited.then(|| "rate-limited".into()),
    };
    Some(match s.billing.rate_limited {
        true => format!("{who}, rate-limited"),
        false => who,
    })
}

fn tokens(n: u64) -> String {
    match n {
        n if n >= 1000 => format!("{}k", n / 1000),
        n => n.to_string(),
    }
}

fn row(mark: &str, text: &str, ink: ratatui::style::Color, t: &Theme, room: usize) -> Line<'static> {
    let shown = super::clip(text, room.saturating_sub(mark.chars().count()));
    Line::from(vec![
        Span::styled(
            mark.to_string(),
            Style::default().fg(ink).add_modifier(Modifier::BOLD),
        ),
        Span::styled(shown, Style::default().fg(t.text)),
    ])
}
