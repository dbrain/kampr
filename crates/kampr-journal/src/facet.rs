use serde::Serialize;

/// What a harness wrote down beside the conversation that is about the *session* rather than about
/// a turn, normalised across the three.
///
/// **Nothing here is Claude-shaped, and that is the whole design.** Kampr serves `claude`, `codex`
/// and `agy`, and the wire is additive for ever: a `title` field named after the record Claude
/// happens to write is a promise the other two have to keep. So every facet is optional, absent by
/// default, and filled only where a harness has been *measured* to carry an equivalent — a harness
/// with nothing to say says nothing, and a client draws nothing for what it does not get.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Facets {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<Title>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub timings: Vec<Timing>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub queued: Vec<Queued>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<Mode>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub compactions: Vec<Compaction>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleSource {
    /// A person typed it.
    Manual,
    /// The harness made it up, however good it is.
    Generated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Title {
    pub text: String,
    pub source: TitleSource,
}

/// The three levels a session title can come from, strongest first.
///
/// **Automatic only where nothing manual exists**, at every level: what a person typed beside the
/// conversation, then what the harness generated for it, then whatever the harness is calling the
/// session for want of anything better. A harness fills the levels it has and leaves the rest.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Titles {
    pub manual: Option<String>,
    pub generated: Option<String>,
    pub named: Option<String>,
}

impl Titles {
    pub fn resolve(&self) -> Option<Title> {
        fn pick(slot: &Option<String>) -> Option<&str> {
            slot.as_deref().map(str::trim).filter(|t| !t.is_empty())
        }
        if let Some(text) = pick(&self.manual) {
            return Some(Title {
                text: text.to_string(),
                source: TitleSource::Manual,
            });
        }
        let generated = pick(&self.generated).or_else(|| pick(&self.named))?;
        Some(Title {
            text: generated.to_string(),
            source: TitleSource::Generated,
        })
    }
}

/// How long one turn took, named by the turn it closes rather than by an ordinal — so a client can
/// hang it off a turn it already holds, and nothing has to infer a duration from two timestamps.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Timing {
    pub turn: String,
    pub duration_ms: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub messages: Option<u32>,
}

/// A prompt the operator has sent that the harness has not started on yet.
///
/// Only the outstanding ones: a harness records the enqueue and the removal both, and a prompt the
/// turn has already absorbed is not something anybody is still waiting on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Queued {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// Open strings on purpose: `plan` and `bypassPermissions` are Claude's vocabulary, another
/// harness will have its own, and a client renders the word it is given rather than matching an
/// enum it would have to be reinstalled to extend.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Mode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
pub struct Compaction {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub trigger: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pre_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub post_tokens: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub dropped_tokens: Option<u64>,
}
