//! Repository-wide rules that no single crate can see, checked mechanically so they cannot drift
//! from the prose in `AGENTS.md` that states them.

mod claim_sites;
mod crate_graph;
mod herdr_tests;
mod probe_log;
mod repo;
