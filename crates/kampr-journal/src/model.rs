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

    pub fn tool_block_mut(&mut self) -> Option<&mut Block> {
        self.blocks.iter_mut().find(|b| matches!(b, Block::Tool { .. }))
    }
}
