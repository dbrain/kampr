//! The HTTP and WebSocket surface, and the protocol driver behind it.
//!
//! Two rules shape this crate. A slow client is *reset*, never buffered: the per-connection queue
//! is bounded and overflow drops that pane's patches in favour of one `grid.reset`, because a
//! phone on a bad link must not be able to grow the node's memory. And the style table is per
//! connection, because the protocol only promises id stability for the life of a socket.

pub mod assetlinks;
pub mod assets;
pub mod attach;
pub mod caps;
pub mod config;
pub mod convo;
pub mod herd;
pub mod holds;
pub mod http;
pub mod manage;
pub mod mesh;
pub mod outbox;
pub mod pending;
pub mod push;
pub mod relay;
pub mod session;
pub mod sessions;
pub mod state;
pub mod toast;
pub mod update;
pub mod wire;

pub use config::{Config, ConfigError};
pub use state::{BUILD, Node};
