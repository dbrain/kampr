use crate::wire::Wire;
use kampr_core::conversation::ask::Ask;
use kampr_core::conversation::{Command, Source, Update};
use kampr_core::wire::{ErrorCode, PendingOption, PendingSource, ServerMsg};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;

const COMMANDS: usize = 64;
pub const MATCHES: usize = 50;

/// What this node serves a pane's conversation from, and where it lives in the source's own
/// namespace.
pub struct ConversationRef {
    pub source: Arc<dyn Source>,
    pub pane: String,
    pub terminal: bool,
}

/// A conversation open for one watcher: the channel its commands go in and the task it runs in.
pub struct ConversationLink {
    commands: mpsc::Sender<Command>,
    task: JoinHandle<()>,
}

impl ConversationLink {
    pub fn stop(&self) {
        self.task.abort();
    }
}

/// One per socket, keyed by the pane the socket is watching: the conversation it has open.
#[derive(Default)]
pub struct Conversations(HashMap<String, ConversationLink>);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandError {
    /// A conversation is open for the pane and its channel is full or closed.
    Busy,
    /// No conversation is open for the pane.
    Closed,
}

impl Conversations {
    /// Opens the source's conversation for the pane and runs it until the watch ends.
    pub async fn open(
        &mut self,
        convo: &ConversationRef,
        pane: &str,
        wire: &Arc<Wire>,
    ) {
        let conversation = match convo.source.open(&convo.pane).await {
            Ok(conversation) => conversation,
            Err(refusal) => {
                wire.error(
                    ErrorCode::SessionUnavailable,
                    &refusal.message,
                    Some(pane),
                );
                return;
            }
        };
        let (commands, received) = mpsc::channel(COMMANDS);
        let pane_key = pane.to_string();
        let wire = wire.clone();
        let pane = pane.to_string();
        let task = tokio::spawn(conversation.run(
            received,
            Box::new(move |update| emit(&wire, &pane, update)),
        ));
        self.0.insert(
            pane_key,
            ConversationLink {
                commands,
                task,
            },
        );
    }

    /// A verb the conversation takes, sent without waiting: the pane gets the refusal.
    pub fn command(&self, pane: &str, command: Command) -> Result<(), CommandError> {
        match self.0.get(pane) {
            Some(link) => link
                .commands
                .try_send(command)
                .map_err(|_| CommandError::Busy),
            None => Err(CommandError::Closed),
        }
    }

    /// A message the composer sends, queued behind the conversation's own work.
    pub async fn send(&self, pane: &str, command: Command) -> Result<(), CommandError> {
        match self.0.get(pane) {
            Some(link) => link
                .commands
                .send(command)
                .await
                .map_err(|_| CommandError::Closed),
            None => Err(CommandError::Closed),
        }
    }

    pub fn take(&mut self, pane: &str) -> Option<ConversationLink> {
        self.0.remove(pane)
    }

    pub fn drain(&mut self) -> impl Iterator<Item = ConversationLink> + '_ {
        self.0.drain().map(|link| link.1)
    }
}

fn emit(wire: &Wire, pane: &str, update: Update) -> bool {
    match update {
        Update::Page(page) => wire.send(&ServerMsg::convo(pane, page, true)),
        Update::Older(page) => wire.send(&ServerMsg::convo(pane, page, false)),
        Update::Turns(turns) => wire.send(&ServerMsg::ConvoTurn {
            pane: pane.into(),
            sub: None,
            turns,
        }),
        Update::Facets(facets) => wire.send(&ServerMsg::ConvoFacets {
            pane: pane.into(),
            facets,
        }),
        Update::Asks(asks) => wire.send(&pending(pane, &asks)),
        Update::Found { query, found } => wire.send(&crate::convo::found(pane, &query, found)),
        Update::SubPage { id, page } => wire.send(&ServerMsg::convo_sub(pane, &id, page)),
        Update::SubTurns { id, turns } => wire.send(&ServerMsg::ConvoTurn {
            pane: pane.into(),
            sub: Some(id),
            turns,
        }),
        Update::Missing(message) => wire.error(ErrorCode::NotFound, &message, Some(pane)),
        Update::Failed(message) => {
            wire.error(ErrorCode::SessionUnavailable, &message, Some(pane))
        }
        // The seam's wider vocabulary has no wire frame until the wire learns it.
        _ => true,
    }
}

fn pending(pane: &str, asks: &[Ask]) -> ServerMsg {
    let ask = asks.first();
    ServerMsg::Pending {
        pane: pane.into(),
        question: ask.map(|ask| ask.question.clone()),
        header: ask.and_then(|ask| ask.header.clone()),
        multi: ask.is_some_and(|ask| ask.multi),
        context: ask.and_then(|ask| ask.context.clone()),
        id: ask.and_then(|ask| ask.id.clone()),
        options: ask
            .map(|ask| {
                ask.options
                    .iter()
                    .map(|option| PendingOption {
                        key: option.key.clone(),
                        label: option.label.clone(),
                        detail: option.detail.clone(),
                        chosen: option.chosen,
                        text: option.text,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        source: PendingSource::Transcript,
    }
}

/// A pane with no herdr behind it: a native run or a muffit session, each kept by its own keeper.
pub fn session_pane(local: &str) -> bool {
    local.starts_with(kampr_native::PREFIX) || local.starts_with(kampr_muffit::PREFIX)
}