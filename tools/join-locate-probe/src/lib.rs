//! Focused production-source tests and timings without the renderer dependency tree.
#[path = "../../../crates/sunrise-client/src/join_seq.rs"]
pub mod join_seq;

// The residency tests read the diagnostics those sources log. `sunrise-client` serves
// `crate::test_log` to the same files (crates/sunrise-client/src/client_lib.rs):
// the probe imports the production sources, so it carries the same shim.
#[cfg(test)]
mod test_log;
