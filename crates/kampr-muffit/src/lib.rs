//! A muffit daemon's sessions as panes of a Kampr herd (docs/18).

mod abilities;
pub mod answers;
mod catalog;
mod client;
mod directory;
pub mod follow;
pub mod input;
mod live;
pub mod mapper;
mod provider;
mod reach;
pub mod source;
pub mod wire;

pub use client::{Answer, Answered, Client};
pub use directory::Directory;
pub use provider::{AGENT, MuffitProvider, PREFIX, agent_status, pane_id, pane_info, session_of, this_host};
pub use reach::{Reach, Refusal, Refusals};
pub use source::{MuffitConversation, MuffitSource, asks_of};
