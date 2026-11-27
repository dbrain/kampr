//! What a conversation's source declares beyond its turns (docs/21 Phase 2), and the verbs that act
//! on it.
//!
//! **Every enum the node sends is an open word here.** kampr-core's own types are strict, and one
//! word this build has never heard of would drop the whole frame; a word it does not know offers
//! nothing instead. A word is compared with kampr-core's own serialisation of it, so this client
//! and the node cannot spell one differently.

use crate::Client;
use kampr_core::conversation::{
    Action, ContextUse, ControlOption, Fact, InputAbility, Limits, Retry, SessionFacts, Steer, Stream,
    Suggestion, TaskEnd,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize)]
#[serde(default)]
pub struct Abilities {
    pub input: Vec<String>,
    pub steer: Vec<String>,
    pub asks: Vec<String>,
    pub streams: Vec<String>,
    pub facts: Vec<String>,
    pub lifecycle: Vec<String>,
}

pub fn word(of: impl Serialize) -> String {
    serde_json::to_value(of)
        .ok()
        .and_then(|v| v.as_str().map(str::to_string))
        .unwrap_or_default()
}

impl Abilities {
    pub fn input(&self, ability: InputAbility) -> bool {
        self.input.contains(&word(ability))
    }

    pub fn steers(&self, steer: Steer) -> bool {
        self.steer.contains(&word(steer))
    }

    pub fn fact(&self, fact: Fact) -> bool {
        self.facts.contains(&word(fact))
    }

    pub fn stream(&self, stream: Stream) -> bool {
        self.streams.contains(&word(stream))
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Live {
    pub text: String,
    pub thinking: Option<u64>,
    pub tools: Vec<LiveTool>,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct LiveTool {
    pub id: String,
    pub tool: String,
    pub args: String,
    pub state: String,
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Task {
    pub id: String,
    pub kind: String,
    pub call: Option<String>,
    pub parent: Option<String>,
    pub title: Option<String>,
    pub since: Option<String>,
    pub backgrounded: bool,
    pub state: String,
}

impl Task {
    pub fn running(&self) -> bool {
        self.state != "ended"
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Trouble {
    pub kind: String,
    pub text: String,
    pub status: String,
    pub retry: Option<Retry>,
    /// `"login"`, `"billing"` or `{"wait": {"until": …}}`.
    pub fix: Option<Value>,
}

impl Trouble {
    pub fn fix(&self) -> Option<&str> {
        match self.fix.as_ref()? {
            Value::String(word) => Some(word),
            Value::Object(fix) => fix.keys().next().map(String::as_str),
            _ => None,
        }
    }

    pub fn until(&self) -> Option<&str> {
        self.fix.as_ref()?["wait"]["until"].as_str()
    }
}

#[derive(Debug, Clone, Default, PartialEq, Deserialize)]
#[serde(default)]
pub struct Control {
    pub id: String,
    pub label: String,
    pub kind: String,
    pub value: Option<Value>,
    pub options: Vec<ControlOption>,
    pub group: Option<String>,
    pub confirm: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ConvoUpdate {
    Abilities(Abilities),
    Live(Live),
    Tasks(Vec<Task>),
    Task {
        id: String,
        at: u64,
        text: String,
        ended: Option<TaskEnd>,
    },
    State(String),
    Trouble(Option<Trouble>),
    Session(SessionFacts),
    Controls(Vec<Control>),
    Actions(Vec<Action>),
    Limits(Limits),
    Context(ContextUse),
    Suggested {
        kind: String,
        query: String,
        items: Vec<Suggestion>,
    },
    Acted {
        action: String,
        ok: bool,
        text: Option<String>,
    },
}

fn field<T: serde::de::DeserializeOwned>(message: &Value, name: &str) -> Option<T> {
    serde_json::from_value(message.get(name)?.clone()).ok()
}

fn text(message: &Value, name: &str) -> Option<String> {
    message[name].as_str().map(str::to_string)
}

/// `None` for a `t` this is not, or a frame whose payload does not decode.
pub fn decode(t: &str, message: &Value) -> Option<(String, ConvoUpdate)> {
    let pane = text(message, "pane")?;
    let update = match t {
        "convo.abilities" => ConvoUpdate::Abilities(field(message, "abilities")?),
        "convo.live" => ConvoUpdate::Live(field(message, "live")?),
        "convo.tasks" => ConvoUpdate::Tasks(field(message, "tasks")?),
        "convo.task" => ConvoUpdate::Task {
            id: text(message, "id")?,
            at: message["at"].as_u64().unwrap_or_default(),
            text: text(message, "text").unwrap_or_default(),
            ended: field(message, "ended"),
        },
        "convo.state" => ConvoUpdate::State(text(message, "state")?),
        "convo.trouble" => ConvoUpdate::Trouble(field(message, "trouble")),
        "convo.session" => ConvoUpdate::Session(field(message, "facts")?),
        "convo.controls" => ConvoUpdate::Controls(field(message, "controls")?),
        "convo.actions" => ConvoUpdate::Actions(field(message, "actions")?),
        "convo.limits" => ConvoUpdate::Limits(field(message, "limits")?),
        "convo.context" => ConvoUpdate::Context(field(message, "context")?),
        "convo.suggested" => ConvoUpdate::Suggested {
            kind: text(message, "kind")?,
            query: text(message, "query").unwrap_or_default(),
            items: field(message, "items").unwrap_or_default(),
        },
        "convo.acted" => ConvoUpdate::Acted {
            action: text(message, "action")?,
            ok: message["ok"].as_bool().unwrap_or_default(),
            text: text(message, "text"),
        },
        _ => return None,
    };
    Some((pane, update))
}

impl Client {
    pub fn convo_steer(&self, pane: &str, steer: &str, text: Option<&str>) -> bool {
        let mut message = json!({ "t": "convo.steer", "pane": pane, "steer": steer });
        if let Some(text) = text {
            message["text"] = json!(text);
        }
        self.send(message)
    }

    pub fn convo_set(&self, pane: &str, control: &str, value: Value) -> bool {
        self.send(json!({ "t": "convo.set", "pane": pane, "control": control, "value": value }))
    }

    pub fn convo_act(&self, pane: &str, action: &str, args: Value) -> bool {
        self.send(json!({ "t": "convo.act", "pane": pane, "action": action, "args": args }))
    }

    pub fn convo_follow(&self, pane: &str, id: &str, from: u64) -> bool {
        self.send(json!({ "t": "convo.follow", "pane": pane, "id": id, "from": from }))
    }

    pub fn convo_suggest(&self, pane: &str, kind: &str, query: &str) -> bool {
        self.send(json!({ "t": "convo.suggest", "pane": pane, "kind": kind, "query": query }))
    }

    /// One whole message to a conversation, under the id an `input.unqueue` can name. `when` is
    /// `next` or `now`; absent goes when the conversation is idle.
    pub fn say(&self, pane: &str, text: &str, id: &str, when: Option<&str>) -> bool {
        let mut message = json!({ "t": "input", "pane": pane, "text": text, "id": id });
        if let Some(when) = when {
            message["when"] = json!(when);
        }
        self.send(message)
    }

    pub fn unqueue(&self, pane: &str, id: &str) -> bool {
        self.send(json!({ "t": "input.unqueue", "pane": pane, "id": id }))
    }
}
