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

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "b", rename_all = "lowercase")]
pub enum Block {
    Md {
        text: String,
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
