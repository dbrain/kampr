use kampr_core::registry::PaneUpdate;
use kampr_core::scrollback::ScrollbackDoc;
use serde::{Deserialize, Deserializer};
use serde_json::Value;

/// This device's role, as `hello` gave it or as a mid-connection `role` frame changed it.
///
/// Anything that is not the exact string `full` reads as read-only. A node one release ahead may
/// name a role this build has never heard of, and the only safe reading of an unknown permission
/// is the one that draws no write affordances — the same rule the store applies to an unparseable
/// role row (`08-threat-model.md` §7.6).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Role {
    Full,
    #[default]
    Readonly,
}

impl Role {
    pub fn writes(self) -> bool {
        matches!(self, Self::Full)
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Full => "full",
            Self::Readonly => "readonly",
        }
    }
}

impl<'de> Deserialize<'de> for Role {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let name = String::deserialize(d)?;
        Ok(match name.as_str() {
            "full" => Self::Full,
            _ => Self::Readonly,
        })
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Caps {
    pub push: bool,
    pub scrollback: bool,
    pub conversation: bool,
    pub manage: bool,
    pub mesh: bool,
    pub attachments: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Security {
    pub tier: u8,
    pub origin: String,
    pub encrypted: bool,
    pub unencrypted_banner: bool,
    pub passkeys: bool,
    pub push: bool,
    pub installable: bool,
    pub unlocks: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
    pub expires_at: Option<i64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Hello {
    pub protocol: u32,
    pub node_id: String,
    #[serde(default)]
    pub node_name: String,
    #[serde(default)]
    pub build: String,
    #[serde(default)]
    pub role: Role,
    #[serde(default)]
    pub caps: Caps,
    #[serde(default)]
    pub device: DeviceInfo,
    #[serde(default)]
    pub security: Security,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Managed {
    pub op: String,
    pub ok: bool,
    pub id: Option<String>,
    /// Open, exactly as `error.code` is: a refusal a hub forwarded from a peer may name a code
    /// this build has never seen, and the `message` beside it is what a person reads either way.
    pub code: Option<String>,
    pub message: Option<String>,
    pub layout: Option<Value>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PendingOption {
    pub key: String,
    pub label: String,
}

/// What a node can be asked to make, as opposed to what it can do — the answer to a `caps`
/// request. `served` is the difference between a session that is running and one this node is
/// serving as a node of its own, and a client must not offer to open a pane on the latter.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct NodeCaps {
    pub node: String,
    pub agent_kinds: Vec<String>,
    pub sessions: Vec<SessionCaps>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct SessionCaps {
    pub name: String,
    pub running: bool,
    pub served: bool,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Pending {
    pub pane: String,
    pub question: Option<String>,
    pub options: Vec<PendingOption>,
    pub source: String,
}

impl Pending {
    /// A prompt is cleared by the same message with a null question; there is no resolved event.
    pub fn outstanding(&self) -> bool {
        self.question.is_some()
    }
}

/// One page of a transcript, envelope decoded and turns left as they arrived.
///
/// The merge rule lives here in the envelope — `fresh` replaces, absent merges by id — and the
/// blocks inside a turn are the conversation renderer's business.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct ConvoPage {
    pub pane: String,
    pub cursor: Option<String>,
    pub more: bool,
    pub fresh: bool,
    pub turns: Vec<Value>,
}

/// Why a pane has no picture, or why an op was refused.
///
/// `code` is a **String** and not an enum on purpose: the vocabulary is open, a hub forwards a
/// peer's codes verbatim, and a client that failed on one it did not know would hide the
/// diagnosis it was sent.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct Failure {
    pub code: String,
    pub message: String,
    pub pane: Option<String>,
}

#[derive(Debug, Clone)]
pub enum Event {
    /// The socket is up and the node has greeted it.
    Connected(Box<Hello>),
    /// The socket is gone. Cached grids are kept and marked stale; there is no spinner, and the
    /// swap happens on the `grid.reset` that follows the next greeting.
    Disconnected {
        reason: String,
    },
    Herd,
    /// A mid-connection permission change. Not a second greeting: the herd and the preferences
    /// are untouched, and only the write affordances move.
    Role(Role),
    /// `greeting` marks the unasked third frame of the greeting, which is **not** the answer to
    /// this client's own write.
    Prefs {
        greeting: bool,
    },
    Grid {
        pane: String,
        update: PaneUpdate,
    },
    Scrollback {
        pane: String,
        doc: ScrollbackDoc,
    },
    Convo(ConvoPage),
    ConvoTurn {
        pane: String,
        turns: Vec<Value>,
    },
    Pending(Pending),
    /// The answer to a `caps` request: the agent kinds this node knows and the sessions it has.
    Caps(NodeCaps),
    /// A `managed` ack nothing was waiting on — one sent without an `rid`, or one whose caller
    /// stopped waiting.
    Managed(Managed),
    Error(Failure),
    Pong {
        n: u64,
    },
}
