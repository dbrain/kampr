mod ask;
mod elicit;
mod tool;

use crate::pending::Pending;
use kampr_core::wire::PendingOption;
use serde_json::{Value, json};

const NO_WORDS: &str = "this answer takes no words";

#[derive(Debug)]
pub struct Card {
    kind: Kind,
    by: Option<String>,
}

#[derive(Debug)]
enum Kind {
    Tool(tool::Tool),
    Plan(tool::Plan),
    Ask(ask::Ask),
    Elicit(elicit::Elicit),
}

pub enum Pressed {
    Moved,
    Decided(String),
}

impl Card {
    pub fn of(event: &Value) -> Option<Self> {
        let tool = event["tool_name"].as_str()?;
        let input = &event["tool_input"];
        let kind = match tool {
            "AskUserQuestion" => ask::Ask::of(input).map(Kind::Ask),
            "ExitPlanMode" => Some(Kind::Plan(tool::Plan::of(input))),
            _ => None,
        }
        .unwrap_or_else(|| Kind::Tool(tool::Tool::of(tool, event)));
        // A subagent's dialog is not drawn on the main screen (#636), so the card is all that
        // says whose request it is.
        let by = event["agent_type"]
            .as_str()
            .filter(|a| !a.trim().is_empty())
            .map(|a| format!("{a} subagent"));
        Some(Self { kind, by })
    }

    pub fn elicitation(event: &Value) -> Self {
        Self {
            kind: Kind::Elicit(elicit::Elicit::of(event)),
            by: None,
        }
    }

    pub fn elicits(&self) -> bool {
        matches!(self.kind, Kind::Elicit(_))
    }

    pub fn subagent(&self) -> bool {
        self.by.is_some()
    }

    pub fn step(&self) -> Option<usize> {
        match &self.kind {
            Kind::Ask(ask) => ask.step(),
            _ => None,
        }
    }

    pub fn view(&self) -> Pending {
        let mut view = match &self.kind {
            Kind::Tool(tool) => tool.view(),
            Kind::Plan(plan) => plan.view(),
            Kind::Ask(ask) => ask.view(),
            Kind::Elicit(elicit) => elicit.view(),
        };
        if let Some(by) = &self.by {
            view.header = Some(match view.header {
                Some(header) => format!("{header} · {by}"),
                None => by.clone(),
            });
        }
        view
    }

    pub fn press(&mut self, key: &str, text: Option<&str>) -> Result<Pressed, String> {
        let text = text.map(str::trim).filter(|t| !t.is_empty());
        match &mut self.kind {
            Kind::Tool(tool) => tool.press(key, text),
            Kind::Plan(plan) => plan.press(key, text),
            Kind::Ask(ask) => ask.press(key, text),
            Kind::Elicit(_) => Err(elicit::AT_THE_TERMINAL.into()),
        }
    }

    pub fn submit(&mut self) -> Result<Pressed, String> {
        match &mut self.kind {
            Kind::Ask(ask) => ask.submit(),
            Kind::Elicit(_) => Err(elicit::AT_THE_TERMINAL.into()),
            _ => Err("this question takes one answer, and a press gives it".into()),
        }
    }
}

fn card(
    question: String,
    header: Option<String>,
    options: Vec<PendingOption>,
    context: Option<String>,
) -> Pending {
    Pending {
        question,
        options,
        header,
        multi: false,
        cursor: None,
        context,
        id: None,
    }
}

fn decision(decision: Value) -> String {
    json!({ "hookSpecificOutput": { "hookEventName": "PermissionRequest", "decision": decision } })
        .to_string()
}

fn option(key: &str, label: &str, detail: Option<&str>, chosen: bool) -> PendingOption {
    PendingOption {
        key: key.to_string(),
        label: label.to_string(),
        detail: detail.map(str::to_string),
        chosen,
        text: false,
    }
}

fn worded(key: &str, label: &str) -> PendingOption {
    PendingOption {
        text: true,
        ..option(key, label, None, false)
    }
}

fn not_an_answer(key: &str) -> String {
    format!("{key} is not one of this question's answers")
}

fn capped(text: &str, most: usize) -> String {
    match text.char_indices().nth(most) {
        Some((cut, _)) => format!("{}…", &text[..cut]),
        None => text.to_string(),
    }
}
