use std::path::PathBuf;
use std::time::Duration;

use kampr_core::conversation::{Command, Conversation, Emit};
use kampr_keep::SessionDir;
use tokio::sync::mpsc;

use crate::config::Config;
use crate::kept::Kept;
use crate::stored::Store;

pub(crate) mod activity;
mod asks;
mod child;
mod commands;
mod controls;
mod facts;
mod guards;
mod lines;
mod machine;
mod reader;
mod responses;
mod state;
mod tasks;
#[cfg(test)]
mod tests;
mod transcript;
mod turn;

use child::Keeper;
use machine::{Machine, Settings};

const BUFFER: usize = 64;
/// How often a task the pane watches is read again.
const POLL: Duration = Duration::from_secs(1);

/// A headless Claude pane: the stream-json child under a keeper's pipes, mapped to the seam.
pub struct ClaudeConversation {
    settings: Settings,
    session_path: PathBuf,
    session_id: String,
}

impl ClaudeConversation {
    pub(crate) fn new(config: &Config, session_path: PathBuf, session_id: String, store: Store) -> Self {
        Self {
            settings: Settings {
                policy: config.billing,
                overage: config.overage,
                unattended_cap: config.unattended_turns,
                nonce: nonce(),
                store,
                kept: Kept::at(session_path.clone()),
            },
            session_path,
            session_id,
        }
    }
}

fn nonce() -> String {
    let mut bytes = [0u8; 4];
    getrandom::fill(&mut bytes).expect("entropy for a request nonce");
    format!("k{}", u32::from_le_bytes(bytes))
}

#[async_trait::async_trait]
impl Conversation for ClaudeConversation {
    async fn run(self: Box<Self>, mut commands: mpsc::Receiver<Command>, emit: Emit) {
        let Self {
            settings,
            session_path,
            session_id,
        } = *self;
        let child = Box::new(Keeper(SessionDir::at(&session_path)));
        let mut machine = Machine::new(settings, session_id, child, emit);
        machine.opened();

        let (tx, mut items) = mpsc::channel(BUFFER);
        reader::spawn(session_path, tx);
        let mut poll = tokio::time::interval(POLL);
        poll.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
        // The pane going away ends the conversation and not the session: the child belongs to
        // its keeper, and the next pane to open it reads the log from the start.
        while !machine.done() {
            tokio::select! {
                item = items.recv() => match item {
                    Some(item) => machine.item(item),
                    None => break,
                },
                command = commands.recv() => match command {
                    Some(command) => machine.command(command),
                    None => break,
                },
                _ = poll.tick() => machine.tick(),
            }
        }
    }
}
