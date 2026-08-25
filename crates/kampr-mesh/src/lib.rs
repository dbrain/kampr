//! The mesh: one hub, many peers, one herd.
//!
//! Peers dial **outbound** to a hub and the hub answers nothing back. That single decision is what
//! makes a laptop behind NAT joinable and what lets an operator point one reverse proxy at one
//! hostname and reach every host. "Hub" is therefore a role a node is configured into, not a
//! separate build: the same binary dials out, accepts dial-ins, or both.
//!
//! Once the handshake is done the link carries the ordinary v1 client protocol, backwards: the
//! hub is the *client* of the peer, sending `watch` and `input`, and the peer serves it with the
//! very same session code that serves a browser. Nothing about the relay is a second protocol,
//! and the per-connection backpressure rule therefore applies at both hops by construction.

pub mod dial;
pub mod handshake;
pub mod peers;
pub mod shadow;
pub mod transport;

pub use dial::{DialPolicy, Hub, dial, mesh_url, supervise};
pub use handshake::{Accepted, HandshakeError, HubIdentity, MESH_PROTOCOL, Presence, accept, greet};
pub use peers::{
    ATT_CHUNK_BYTES, ATT_WINDOW, AttHeader, FetchError, PeerHerd, PeerState, Peers, PeersConfig, RelayError,
    RemoteEvent, RemoteWatcher, Transfer,
};
pub use transport::{Incoming, Link, Outgoing};
