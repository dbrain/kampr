//! A machine's Docker containers beside its panes (docs/19).

pub mod api;
pub mod archive;
mod client;
pub mod crashes;
pub mod detail;
mod directory;
pub mod elf;
pub mod entry;
pub mod image;
pub mod journal;
pub mod logs;
pub mod shell;
pub mod socket;
pub mod stats;

pub use client::{ApiError, Client, Events, PathStat, is_not_found};
pub use detail::detail;
pub use directory::{Directory, now};
pub use journal::Journal;
