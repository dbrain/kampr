use crate::conversation::abilities::{Fact, Steer};
use crate::conversation::facts::{FileRef, Image};
use serde_json::Value;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum When {
    Idle,
    Next,
    Now,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Reply {
    Allow {
        input: Option<Value>,
        remember: bool,
    },
    Deny {
        message: Option<String>,
    },
    Choose {
        answers: Vec<String>,
    },
    Plan {
        approve: bool,
        mode: Option<String>,
        feedback: Option<String>,
    },
    Form {
        values: Value,
    },
    Dismiss,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SuggestKind {
    Files,
    Commands,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Input {
        text: String,
        msg_id: String,
        images: Vec<Image>,
        files: Vec<FileRef>,
        when: When,
    },
    Unqueue {
        msg_id: String,
    },
    Answer {
        ask: String,
        reply: Reply,
    },
    Steer(Steer, Option<String>),
    Set {
        control: String,
        value: Value,
    },
    Act {
        action: String,
        args: Value,
    },
    Older {
        before: Option<String>,
    },
    Find {
        query: String,
        cap: usize,
    },
    Sub {
        id: String,
        before: Option<String>,
    },
    Task {
        id: String,
        from: u64,
    },
    Unfollow {
        id: String,
    },
    Suggest {
        kind: SuggestKind,
        query: String,
    },
    Refresh(Fact),
}