use serde::Serialize;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    User,
    Assistant,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ToolState {
    Running,
    Done,
    Error,
}

/// The header of something the wire will not carry. `id` resolves at
/// `GET /api/attachment/{pane}/{id}` and nothing else about it is a promise: it is opaque, it is
/// minted by the node that served the turn, and it stops resolving when the pane moves to another
/// transcript.
///
/// `kind` is an open string. A client that does not recognise one offers a download rather than
/// dropping the block, so a future `video` needs no protocol change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Attachment {
    pub id: String,
    pub kind: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mime: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "b", rename_all = "lowercase")]
pub enum Block {
    Md {
        text: String,
        /// Additive: the `text` beside it is the marker an installed client already renders, so a
        /// client that has never heard of this field shows what it showed before.
        #[serde(skip_serializing_if = "Option::is_none")]
        att: Option<Attachment>,
    },
    Code {
        #[serde(skip_serializing_if = "Option::is_none")]
        lang: Option<String>,
        text: String,
    },
    Tool {
        name: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        summary: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        lines: Option<u32>,
        state: ToolState,
    },
    Diff {
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        text: String,
    },
    /// A conversation this turn launched, offered for opening rather than spoken here.
    ///
    /// Its own turns are deliberately **not** inlined: an installed phone would render them as
    /// this turn's reply, which is a lie about who said what. A client that has never heard of
    /// this `b` value drops the block and shows the tool card above it exactly as it does today,
    /// which is the whole of what it shows now.
    Sub {
        /// Opaque, minted by the node that served the turn, and resolved by handing it back.
        id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        title: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        depth: Option<u32>,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Turn {
    pub id: String,
    pub role: Role,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub at: Option<String>,
    pub blocks: Vec<Block>,
}

/// A slice of the transcript running backwards from the newest turn. `cursor` is the id of the
/// oldest turn in `turns` and is what a client echoes back as `convo.load { before }`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Page {
    pub turns: Vec<Turn>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cursor: Option<String>,
    pub more: bool,
}

impl Block {
    pub fn md(text: impl Into<String>) -> Self {
        Self::Md {
            text: text.into(),
            att: None,
        }
    }
}

impl Turn {
    pub fn new(id: impl Into<String>, role: Role, at: Option<String>) -> Self {
        Self {
            id: id.into(),
            role,
            at,
            blocks: Vec::new(),
        }
    }

    /// The tool card at `at`, when that is what sits there.
    ///
    /// **A turn can hold several.** Claude emits parallel `tool_use` blocks in one assistant
    /// record and their results come back separately, in whatever order the calls finish — so a
    /// result that took the *first* card put its state and its line count on somebody else's
    /// tool, and left its own running for ever. Every parser records where its call's card went.
    pub fn tool_block_mut(&mut self, at: usize) -> Option<&mut Block> {
        match self.blocks.get_mut(at) {
            block @ Some(Block::Tool { .. }) => block,
            _ => None,
        }
    }
}
