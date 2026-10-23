//! Running one command across the herd, as native sessions with a cohort.
//!
//! A fleet run is deliberately **not** a herdr pane. Probes #331 and #332 measured why: from
//! outside a pane the node cannot tell a job that is waiting for an answer from one that is
//! working, and for a job running as root it can read nothing whatsoever. The process that forked
//! the command and shares its privilege can (#334), and for a native session that is its keeper
//! (docs/17). The second reason is the operator's: panes the herd never hears about cannot clutter
//! a desk, so a fleet run leaves no trace on the machine's own screen.

pub mod env;
pub mod job;
mod ladder;
pub mod provider;
pub mod secretish;
mod tail;
pub mod waiting;

pub use env::{FleetPath, PathOrigin, PathSearch, fleet_path};
pub use job::Job;
pub use kampr_core::question::{self as prompt, Question, Shape};
pub use ladder::State;
pub use provider::{FleetProvider, Geometry};
pub use secretish::secretish;
pub use waiting::Waiting;
