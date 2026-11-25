use crate::conversation::abilities::AskAbility;

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct AskOption {
    pub key: String,
    pub label: String,
    pub detail: Option<String>,
    pub chosen: bool,
    pub text: bool,
}