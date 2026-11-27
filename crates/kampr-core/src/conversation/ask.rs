use crate::conversation::abilities::AskAbility;
use serde::{Deserialize, Serialize};

/// A standing question, the way a source phrases it. A list, because a subagent's request and
/// the main agent's stand together.
#[derive(Debug, Clone, PartialEq)]
pub struct Ask {
    pub id: Option<String>,
    pub kind: AskAbility,
    /// Which agent asks, where one can be told apart from another.
    pub by: Option<String>,
    pub header: Option<String>,
    pub question: String,
    pub multi: bool,
    pub context: Option<String>,
    pub options: Vec<AskOption>,
    /// What a form asks to be filled, in the order the source draws it.
    pub fields: Vec<AskField>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct AskOption {
    pub key: String,
    pub label: String,
    pub detail: Option<String>,
    pub chosen: bool,
    pub text: bool,
}

/// One input of an MCP server's form, read from its requested schema.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AskField {
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// The schema's own word: `string`, `number`, `integer` or `boolean`.
    #[serde(rename = "type")]
    pub kind: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub required: bool,
    /// The values a string field is limited to, where the schema names them.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<String>,
}
