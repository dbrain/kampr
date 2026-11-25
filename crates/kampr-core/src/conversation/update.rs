use crate::conversation::abilities::Abilities;
use crate::conversation::ask::Ask;
use crate::conversation::command::SuggestKind;
use crate::conversation::control::{Action, Control};
use crate::conversation::facts::{
    ContextUse, Cost, Limits, RunState, SessionFacts, Suggestion, Task, TaskEnd, Trouble,
};
use kampr_journal::{ConvoFound, Facets, Page, Turn};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq)]
pub enum Update {
    Abilities(Abilities),
    Page(Page),
    Turns(Vec<Turn>),
    Older(Page),
    Live(Live),
    Facets(Facets),
    Asks(Vec<Ask>),
    Found {
        query: String,
        found: ConvoFound,
    },
    SubPage {
        id: String,
        page: Page,
    },
    SubTurns {
        id: String,
        turns: Vec<Turn>,
    },
    SubLive {
        id: String,
        live: Live,
    },
    Tasks(Vec<Task>),
    TaskOutput {
        id: String,
        at: u64,
        text: String,
        ended: Option<TaskEnd>,
    },
    State(RunState),
    Trouble(Option<Trouble>),
    Session(SessionFacts),
    Controls(Vec<Control>),
    Actions(Vec<Action>),
    Limits(Limits),
    Context(ContextUse),
    Cost(Cost),
    Suggested {
        kind: SuggestKind,
        query: String,
        items: Vec<Suggestion>,
    },
    Acted {
        action: String,
        ok: bool,
        text: Option<String>,
    },
    Notice(Notice),
    Missing(String),
    Failed(String),
}

/// The message being written: its text so far, and its tool calls as their arguments arrive.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Live {
    pub text: String,
    /// A token estimate of what is being thought while its text is withheld (#644).
    pub thinking: Option<u64>,
    pub tools: Vec<LiveTool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LiveTool {
    pub id: String,
    pub tool: String,
    pub args: String,
    pub state: kampr_journal::ToolState,
}

/// Something the conversation wants said that no other update carries.
#[derive(Debug, Clone, PartialEq)]
pub struct Notice {
    pub text: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_update_is_cloneable() {
        let turns = vec![Turn::new("t1", kampr_journal::Role::User, None)];
        let updates = vec![
            Update::Abilities(Abilities::default()),
            Update::Page(Page {
                turns: turns.clone(),
                cursor: None,
                more: false,
            }),
            Update::Turns(turns.clone()),
            Update::Older(Page {
                turns: Vec::new(),
                cursor: None,
                more: false,
            }),
            Update::Live(Live::default()),
            Update::Facets(Facets::default()),
            Update::Asks(Vec::new()),
            Update::Found {
                query: "q".into(),
                found: ConvoFound {
                    total: 0,
                    hits: Vec::new(),
                },
            },
            Update::SubPage {
                id: "s1".into(),
                page: Page {
                    turns: Vec::new(),
                    cursor: None,
                    more: false,
                },
            },
            Update::SubTurns {
                id: "s1".into(),
                turns,
            },
            Update::SubLive {
                id: "s1".into(),
                live: Live::default(),
            },
            Update::Tasks(Vec::new()),
            Update::TaskOutput {
                id: "t".into(),
                at: 0,
                text: "out".into(),
                ended: None,
            },
            Update::State(RunState::Running),
            Update::Trouble(None),
            Update::Session(SessionFacts {
                id: "s1".into(),
                cwd: None,
                title: None,
                model: None,
                mode: None,
                version: None,
                account: None,
                billing: Default::default(),
                started: None,
            }),
            Update::Controls(Vec::new()),
            Update::Actions(Vec::new()),
            Update::Limits(Limits::default()),
            Update::Context(ContextUse {
                used: 0,
                total: 0,
            }),
            Update::Cost(Cost {
                usd: 0.0,
                input_tokens: 0,
                output_tokens: 0,
            }),
            Update::Suggested {
                kind: SuggestKind::Files,
                query: "q".into(),
                items: Vec::new(),
            },
            Update::Acted {
                action: "a".into(),
                ok: true,
                text: None,
            },
            Update::Notice(Notice {
                text: "n".into(),
            }),
            Update::Missing("m".into()),
            Update::Failed("f".into()),
        ];
        for update in updates {
            let _ = update.clone();
        }
    }
}