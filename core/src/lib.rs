pub mod cache;
pub mod comparison;
pub mod errors;
pub mod gas_golfing;
pub mod insights;
pub mod parser;
pub mod routing;
pub mod rpc_provider;
pub mod runner;
pub mod simulation;
pub mod wasm_branch_analysis;

#[cfg(test)]
pub mod fuzz_tests;
#[cfg(test)]
pub mod fuzz_simulation;
