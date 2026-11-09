//! A node hosting another node over a command's stdin and stdout (docs/19 "Adoption"): the host
//! spawns the command and is the hub of the link, the command runs `kampr serve --peer-stdio` and
//! is its peer. Nothing here knows what the command is.

mod env;
mod guest;
mod host;

pub use env::PeerStdio;
pub use guest::{prepare, serve_parent};
pub use host::{Child, ChildSpec, ChildState, child_node_id, spawn, supervise};
