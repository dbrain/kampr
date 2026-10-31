//! The HTTP and WebSocket surface, and the protocol driver behind it.
//!
//! Two rules shape this crate. A slow client is *reset*, never buffered: the per-connection queue
//! is bounded and overflow drops that pane's patches in favour of one `grid.reset`, because a
//! phone on a bad link must not be able to grow the node's memory. And the style table is per
//! connection, because the protocol only promises id stability for the life of a socket.

pub mod assetlinks;
pub mod assets;
pub mod attach;
pub mod book;
pub mod caps;
pub mod config;
pub mod convo;
pub mod desk;
pub mod fs;
pub mod git;
pub mod herd;
pub mod holds;
pub mod http;
pub mod lost;
pub mod manage;
pub mod mesh;
pub mod muffit;
pub mod outbox;
pub mod paste;
pub mod pasted;
pub mod pending;
pub mod push;
pub mod reads;
pub mod relay;
pub mod restore;
pub mod served;
pub mod session;
pub mod sessions;
pub mod state;
pub mod structure;
pub mod toast;
pub mod update;
pub mod upload;
pub mod upload_peer;
pub mod upload_route;
pub mod warm;
pub mod wire;
pub mod zipped;

pub use config::{Config, ConfigError};
pub use state::{BUILD, Node};
