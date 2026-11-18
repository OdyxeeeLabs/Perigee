//! Resource-budget report for representative contract operations (#886).
//!
//! This crate has no runtime code. Its tests invoke one representative operation
//! per scenario inside the Soroban test host and print, for each, the CPU
//! instructions and memory bytes the host's cost model charged:
//!
//! ```text
//! RESOURCE|<contract>|<operation>|cpu=<instructions>|mem=<bytes>
//! ```
//!
//! Both numbers come from the host's deterministic cost model, not from a clock,
//! so they are reproducible across machines and safe to compare in CI. See
//! `README.md` for the command, the toolchain and how baselines are updated.

#[cfg(test)]
mod report;
