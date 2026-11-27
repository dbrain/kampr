//! The Codex app-server as a Kampr conversation source: newline-delimited JSON-RPC over a
//! `codex app-server --stdio` child, mapped to the seam in `kampr-core`.
//!
//! The transport quirk the whole design hangs on (probe row 3): the server flushes a response
//! only when it next reads a line from stdin, so every call is pipelined with a nudge.

pub mod conversation;
pub mod source;
pub mod wire;

pub use source::CodexSource;
pub use wire::{Notification, Pump, Request, ServerMsg, Wire, WireError};
