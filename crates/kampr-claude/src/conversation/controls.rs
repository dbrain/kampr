use kampr_core::conversation::Update;
use kampr_core::conversation::abilities::{
    Abilities, AskAbility, Fact, InputAbility, Lifecycle, Steer, Stream,
};
use kampr_core::conversation::control::{Action, Control, ControlKind, ControlOption};
use serde_json::Value;

use super::machine::Machine;
use super::state::State;

/// The permission modes the mode control lists. Bypass is its own control, because it is the one
/// change of mode that needs a sentence before it.
const MODES: [&str; 4] = ["default", "acceptEdits", "plan", "dontAsk"];
pub(super) const BYPASS: &str = "bypassPermissions";

#[derive(Debug, Clone, Default)]
pub(super) struct Model {
    value: String,
    resolved: Option<String>,
    label: String,
    description: Option<String>,
}

/// What `initialize` says the session offers.
#[derive(Debug, Clone, Default)]
pub(super) struct Catalog {
    pub commands: Vec<(String, Option<String>)>,
    models: Vec<Model>,
}

impl Catalog {
    pub fn from(body: &Value) -> Self {
        let text = |v: &Value, key: &str| v.get(key).and_then(Value::as_str).map(str::to_string);
        Self {
            commands: body
                .get("commands")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|c| {
                    Some((
                        text(c, "name")?.trim_start_matches('/').to_string(),
                        text(c, "description"),
                    ))
                })
                .collect(),
            models: body
                .get("models")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|m| m.get("disabled").and_then(Value::as_bool) != Some(true))
                .filter_map(|m| {
                    let value = text(m, "value")?;
                    Some(Model {
                        label: text(m, "displayName").unwrap_or_else(|| value.clone()),
                        resolved: text(m, "resolvedModel"),
                        description: text(m, "description"),
                        value,
                    })
                })
                .collect(),
        }
    }

    pub fn has_model(&self, value: &str) -> bool {
        self.models.iter().any(|m| m.value == value)
    }
}

pub(super) fn is_mode(value: &str) -> bool {
    MODES.contains(&value)
}

pub(super) fn abilities() -> Abilities {
    let mut abilities = Abilities::default();
    abilities.input.extend([
        InputAbility::Images,
        InputAbility::Files,
        InputAbility::Slash,
        InputAbility::Mention,
        InputAbility::QueueNext,
        InputAbility::SendNow,
        InputAbility::Unqueue,
    ]);
    abilities.steer.insert(Steer::Interrupt);
    abilities.asks.extend([
        AskAbility::Permission,
        AskAbility::Always,
        AskAbility::EditInput,
        AskAbility::Question,
        AskAbility::Plan,
        AskAbility::Form,
    ]);
    abilities
        .streams
        .extend([Stream::Text, Stream::TaskOutput, Stream::SubLive]);
    abilities.facts.extend([
        Fact::Account,
        Fact::Billing,
        Fact::Limits,
        Fact::Context,
        Fact::Cost,
        Fact::Trouble,
    ]);
    abilities
        .lifecycle
        .extend([Lifecycle::Create, Lifecycle::Resume, Lifecycle::End]);
    abilities
}

/// Actions from `initialize.commands`: a verb the source itself listed. Kampr adds none of its
/// own.
pub(super) fn actions(st: &State) -> Vec<Action> {
    st.catalog
        .commands
        .iter()
        .map(|(name, description)| Action {
            id: name.clone(),
            label: format!("/{name}"),
            detail: description.clone(),
            args: Vec::new(),
            group: None,
            confirm: None,
        })
        .collect()
}

fn option(value: &str, label: &str, detail: Option<String>) -> ControlOption {
    ControlOption {
        value: value.into(),
        label: label.into(),
        detail,
    }
}

/// Controls from `initialize` and the session's own reports. Guard 8: what costs money carries
/// its sentence. Fast mode is paid from usage credits whatever the plan (#671); no model is
/// marked as outside the plan anywhere the binary says, so none is claimed to be.
pub(super) fn controls(st: &State) -> Vec<Control> {
    let mut controls = vec![
        Control {
            id: "mode".into(),
            label: "Permission mode".into(),
            kind: ControlKind::Choice,
            value: st.mode.clone().map(Value::String),
            options: MODES.iter().map(|m| option(m, m, None)).collect(),
            group: Some("session".into()),
            confirm: None,
        },
        Control {
            id: "bypass".into(),
            label: "Bypass permissions".into(),
            kind: ControlKind::Toggle,
            value: Some(Value::Bool(st.mode.as_deref() == Some(BYPASS))),
            options: Vec::new(),
            group: Some("session".into()),
            confirm: Some("bypass permissions lets the agent run every tool without asking you".into()),
        },
    ];
    if let Some(fast) = st.fast {
        controls.push(Control {
            id: "fast".into(),
            label: "Fast mode".into(),
            kind: ControlKind::Toggle,
            value: Some(Value::Bool(fast)),
            options: Vec::new(),
            group: Some("billing".into()),
            confirm: Some("fast mode is paid from usage credits, not from your plan".into()),
        });
    }
    if !st.catalog.models.is_empty() {
        let current = st.model.as_deref().map(|m| {
            st.catalog
                .models
                .iter()
                .find(|c| c.value == m || c.resolved.as_deref() == Some(m))
                .map_or(m, |c| c.value.as_str())
        });
        controls.push(Control {
            id: "model".into(),
            label: "Model".into(),
            kind: ControlKind::Choice,
            value: current.map(|m| Value::String(m.to_string())),
            options: st
                .catalog
                .models
                .iter()
                .map(|m| option(&m.value, &m.label, m.description.clone()))
                .collect(),
            group: Some("session".into()),
            confirm: None,
        });
    }
    controls
}

impl Machine {
    /// Say the actions and controls again when what they are made of has changed.
    pub(super) fn publish_controls(&mut self) {
        let actions = actions(&self.st);
        if actions != self.st.actions {
            self.st.actions.clone_from(&actions);
            self.say(Update::Actions(actions));
        }
        let controls = controls(&self.st);
        if controls != self.st.controls {
            self.st.controls.clone_from(&controls);
            self.say(Update::Controls(controls));
        }
    }
}
