use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::marker::SessionMarker;

/// What a harness wrote down beside the conversation that is about the *session* rather than about
/// a turn, normalised across the three.
///
/// **Nothing here is Claude-shaped, and that is the whole design.** Kampr serves `claude`, `codex`
/// and `agy`, and the wire is additive for ever: a `title` field named after the record Claude
/// happens to write is a promise the other two have to keep. So every facet is optional, absent by
/// default, and filled only where a harness has been *measured* to carry an equivalent — a harness
/// with nothing to say says nothing, and a client draws nothing for what it does not get.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TitleSource {
    /// A person typed it.
    Manual,
    /// The harness made it up, however good it is.
    Generated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Timing {
    pub turn: String,
    pub duration_ms: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<u32>,
}

/// A prompt the operator has sent that the harness has not started on yet.
///
/// Only the outstanding ones: a harness records the enqueue and the removal both, and a prompt the
/// turn has already absorbed is not something anybody is still waiting on.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Queued {
    pub text: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
}

/// Open strings on purpose: `plan` and `bypassPermissions` are Claude's vocabulary, another
/// harness will have its own, and a client renders the word it is given rather than matching an
/// enum it would have to be reinstalled to extend.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Mode {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub permission: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
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

/// A harness's facet collector, kept between reads so a poll costs the records the transcript has
/// grown by rather than the whole file.
pub trait FacetFold: Send {
    fn facets(&mut self, transcript: &Path, marker: Option<&SessionMarker>) -> Facets;

    /// The title levels alone, unresolved.
    ///
    /// **For the caller whose weakest level is a different string.** A pane entry in the herd is
    /// titled off the same transcript the conversation is, but it refuses a name the harness
    /// derived for itself (#311) where the conversation keeps it — and a resolved [`Title`] gives
    /// it no way to substitute one without reading the file again. It is also the cheaper half:
    /// [`Self::facets`] clones a session's every timing and compaction to answer, and a title
    /// needs none of them.
    ///
    /// The default reads the whole collection and puts the winner back on the level it came from,
    /// which is what a fold with no cheaper path can honestly say.
    fn titles(&mut self, transcript: &Path, marker: Option<&SessionMarker>) -> Titles {
        match self.facets(transcript, marker).title {
            Some(Title {
                text,
                source: TitleSource::Manual,
            }) => Titles {
                manual: Some(text),
                ..Titles::default()
            },
            Some(Title { text, .. }) => Titles {
                generated: Some(text),
                ..Titles::default()
            },
            None => Titles::default(),
        }
    }
}

/// A [`FacetFold`] and what was last published off it.
///
/// **The comparison is the point.** A conversation is followed every 400 ms and a queued prompt
/// moves once in a while, so a `convo.facets` per tick per pane would be a frame for nothing —
/// the client is sent one only when the fold has actually moved.
pub struct FacetFeed {
    fold: Box<dyn FacetFold>,
    last: Facets,
}

impl FacetFeed {
    pub fn new(fold: Box<dyn FacetFold>) -> Self {
        Self {
            fold,
            last: Facets::default(),
        }
    }

    /// What this feed last published, for a reader who was not there when it did.
    ///
    /// [`moved`](Self::moved) answers the *difference*, which is all a client following the pane
    /// needs and is nothing at all to a client that has just arrived — a fold kept warm across a
    /// re-watch (#409) has usually not moved since, so the delta is empty and the facets are still
    /// on the screen of whoever asked first.
    pub fn last(&self) -> Facets {
        self.last.clone()
    }

    /// The facets as they are now, or `None` when nothing has moved since the last call. The first
    /// call answers `None` for a harness with nothing to say, which is the same message as the
    /// `{}` it would otherwise have sent.
    pub fn moved(&mut self, transcript: &Path, marker: Option<&SessionMarker>) -> Option<Facets> {
        let next = self.fold.facets(transcript, marker);
        if next == self.last {
            return None;
        }
        self.last = next.clone();
        Some(next)
    }
}
