//! What a node does with a muffit pane (docs/18): no terminal pump, a conversation follower, and
//! questions and answers over muffit's REST rather than keystrokes.

use crate::wire::Wire;
use kampr_core::wire::{ErrorCode, PendingOption, PendingSource, ServerMsg};
use kampr_muffit::follow::{Command, Follow, Update};
use kampr_muffit::wire::PermissionPrompt;
use kampr_muffit::{Answer, MuffitProvider};
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const COMMANDS: usize = 64;
pub const MATCHES: usize = 50;

pub type Commands = mpsc::Sender<Command>;

const ANSWERS: [(&str, &str, Answer); 3] = [
    ("1", "Allow", Answer::Allow),
    ("2", "Always allow", Answer::Always),
    ("3", "Deny", Answer::Deny),
];

pub fn answer_for(key: &str) -> Option<Answer> {
    ANSWERS.iter().find(|(k, _, _)| *k == key).map(|(_, _, a)| *a)
}

pub fn pending(pane: &str, prompt: Option<&PermissionPrompt>) -> ServerMsg {
    ServerMsg::Pending {
        pane: pane.to_string(),
        question: prompt.map(|p| p.action.clone()),
        header: prompt.map(|p| p.tool.clone()),
        multi: false,
        options: match prompt {
            Some(_) => ANSWERS
                .iter()
                .map(|(key, label, _)| PendingOption {
                    key: (*key).into(),
                    label: (*label).into(),
                    detail: None,
                    chosen: false,
                })
                .collect(),
            None => Vec::new(),
        },
        source: PendingSource::Transcript,
    }
}

/// The prompt a muffit session is waiting on, off the session stream the node already holds.
pub fn prompt(provider: &MuffitProvider, session: &str) -> Option<PermissionPrompt> {
    provider.info(session)?.permission
}

/// Follows the session's conversation for one watcher, until the watch ends.
pub fn follow(
    provider: Arc<MuffitProvider>,
    session: String,
    wire: Arc<Wire>,
    pane: String,
) -> (JoinHandle<()>, Commands) {
    let (commands, received) = mpsc::channel(COMMANDS);
    let alive = {
        let provider = provider.clone();
        let session = session.clone();
        Box::new(move || provider.is_session_live(&session))
    };
    let follow = Follow {
        client: provider.client().clone(),
        session,
        commands: received,
        alive,
        revision: provider.revision(),
    };
    let task = tokio::spawn(follow.run(move |update| emit(&wire, &pane, update)));
    (task, commands)
}

fn emit(wire: &Wire, pane: &str, update: Update) -> bool {
    let msg = match update {
        Update::Page(page) => ServerMsg::convo(pane, page, true),
        Update::Older(page) => ServerMsg::convo(pane, page, false),
        Update::Turns(turns) => ServerMsg::ConvoTurn {
            pane: pane.into(),
            sub: None,
            turns,
        },
        Update::Facets(facets) => ServerMsg::ConvoFacets {
            pane: pane.into(),
            facets,
        },
        Update::Pending(prompt) => pending(pane, prompt.as_ref()),
        Update::Found { query, found } => crate::convo::found(pane, &query, found),
        Update::SubPage { id, page } => ServerMsg::convo_sub(pane, &id, page),
        Update::SubTurns { id, turns } => ServerMsg::ConvoTurn {
            pane: pane.into(),
            sub: Some(id),
            turns,
        },
        Update::Missing(message) => return wire.error(ErrorCode::NotFound, &message, Some(pane)),
        Update::Failed(message) => return wire.error(ErrorCode::SessionUnavailable, &message, Some(pane)),
    };
    wire.send(&msg)
}
