use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// What a source can do, as the source says it: a control the client does not draw is an ability
/// the source does not declare.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Abilities {
    pub input: BTreeSet<InputAbility>,
    pub steer: BTreeSet<Steer>,
    pub asks: BTreeSet<AskAbility>,
    pub streams: BTreeSet<Stream>,
    pub facts: BTreeSet<Fact>,
    pub lifecycle: BTreeSet<Lifecycle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum InputAbility {
    Images,
    Files,
    Slash,
    Mention,
    QueueNext,
    SendNow,
    Unqueue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Steer {
    Interrupt,
    Continue,
    Pause,
    Resume,
    Stop,
    Background,
    StopTask,
    Compact,
    Clear,
    Rewind,
    Fork,
    Rename,
    Aside,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskAbility {
    Permission,
    Always,
    EditInput,
    Question,
    FreeText,
    Plan,
    Form,
    Url,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Stream {
    Text,
    ToolArgs,
    Thinking,
    TaskOutput,
    SubLive,
    Hooks,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Fact {
    Account,
    Billing,
    Limits,
    Context,
    Cost,
    Trouble,
    Mcp,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Lifecycle {
    Create,
    Resume,
    End,
    ToTerminal,
    FromTerminal,
}