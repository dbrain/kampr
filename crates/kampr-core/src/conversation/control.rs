use serde_json::Value;

/// A setting the source lists: the pane draws it from the description and never learns its name.
#[derive(Debug, Clone, PartialEq)]
pub struct Control {
    pub id: String,
    pub label: String,
    pub kind: ControlKind,
    pub value: Option<Value>,
    pub options: Vec<ControlOption>,
    pub group: Option<String>,
    /// The sentence shown before a change that costs money.
    pub confirm: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ControlKind {
    Choice,
    Toggle,
    Number,
    Text,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ControlOption {
    pub value: String,
    pub label: String,
    pub detail: Option<String>,
}

/// A verb the source lists, described: adding something is an action with described arguments.
#[derive(Debug, Clone, PartialEq)]
pub struct Action {
    pub id: String,
    pub label: String,
    pub detail: Option<String>,
    pub args: Vec<ArgSpec>,
    pub group: Option<String>,
    pub confirm: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ArgSpec {
    pub name: String,
    pub label: String,
    pub required: bool,
    /// Non-empty makes the argument a choice from this list.
    pub options: Vec<String>,
}