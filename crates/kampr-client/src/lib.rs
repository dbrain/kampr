//! The client half of the wire protocol, with no terminal attached.
//!
//! It dials a node, holds the greeting, the herd and one decoded grid per pane, and keeps the
//! socket up across an outage. Everything a Kampr client has to get right that is not *drawing* —
//! the reset/patch link rule, the open error vocabulary, the unasked third greeting frame, the
//! mid-connection role change — lives here so the TUI is not the only thing that can ever use it.
//!
//! **The decoding is [`kampr_mesh::shadow`]'s.** A hub already decodes exactly these frames into
//! exactly these cells, and a second implementation of `decode_row` is a second set of Unicode
//! width bugs.
//!
//! Two rules shape the state it keeps. Grids **survive a reconnect**, marked stale and swapped on
//! the `grid.reset` that follows — a full grid is about 3 KB and herdr coalesces bursts to end
//! state, so there is never a backlog to drain and never a spinner. And nothing is ever
//! optimistically mutated: a `manage` op waits for the `herd.patch`, because the node is
//! authoritative.

pub mod client;
pub mod dial;
pub mod frames;
pub mod herd;
pub mod pair;
pub mod pane;
pub mod profile;
pub mod resolve;

pub use client::{Client, ManageError, Policy, State};
pub use dial::{DialError, ws_url};
pub use frames::{
    Caps, ConvoPage, Event, Failure, Hello, Managed, NodeCaps, Pending, PendingOption, Role, Security,
    SessionCaps,
};
pub use herd::{Gone, Herd, NodeGroup};
pub use pair::{PairError, Paired, pair};
pub use pane::PaneState;
pub use profile::{ClientConfig, Profile};
pub use resolve::{ResolveError, Session, Via, hostname, resolve};
