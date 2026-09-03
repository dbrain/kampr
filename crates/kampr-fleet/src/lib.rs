//! Running one command across the herd, in ptys this process owns.
//!
//! A fleet run is deliberately **not** a herdr pane. Probes #331 and #332 measured why: from
//! outside a pane the node cannot tell a job that is waiting for an answer from one that is
//! working, and for a job running as root it can read nothing whatsoever. A supervisor that forks
//! the command and shares its privilege can (#334). The second reason is the operator's: panes the
//! herd never hears about cannot clutter a desk, so a fleet run leaves no trace on the machine's
//! own screen.

pub mod env;
pub mod exec;
pub mod job;
pub mod provider;
pub mod secretish;
pub mod tail;
pub mod waiting;

pub use env::{FleetPath, PathOrigin, PathSearch, fleet_path};
pub use exec::{Geometry, Killer, RunEvent, State, Supervisor, Writer};
pub use job::Job;
pub use kampr_core::question::{self as prompt, Question, Shape};
pub use provider::FleetProvider;
pub use secretish::secretish;
pub use tail::Tail;
pub use waiting::{Procfs, Waiting};
