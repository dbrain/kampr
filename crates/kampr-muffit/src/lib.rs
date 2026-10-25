//! A muffit daemon's sessions as panes of a Kampr herd (docs/18).

mod client;
mod directory;
pub mod follow;
pub mod input;
mod live;
pub mod mapper;
mod provider;
pub mod wire;

pub use client::{Answer, Answered, Client};
pub use directory::Directory;
pub use provider::{AGENT, MuffitProvider, PREFIX, agent_status, pane_id, pane_info, session_of};
