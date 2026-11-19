// Prevent regressions: production code must not use .unwrap() or .expect().
// The cfg_attr below exempts test code (where panicking on assertion failures
// is idiomatic and desirable).
#![warn(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

pub mod agent_fleet;
pub mod backpressure;
pub mod billing_service;
pub mod config;
pub mod cache;
pub mod comparison;
pub mod error_codes;
pub mod errors;
pub use error_codes::{ErrorCode, ErrorResponse};
pub use errors::AppError;
pub mod fee;

pub mod billing_service {
    pub use crate::fee::service::*;
}

pub mod fee_analytics {
    pub use crate::fee::analytics::*;
}

pub mod fee_collector {
    pub use crate::fee::collector::*;
}

pub mod fee_store {
    pub use crate::fee::persistence::*;
}

pub use errors::{ApiJson, AppError};
pub mod fee_analytics;
pub mod fee_collector;
pub mod fee_store;
pub mod gas_golfing;
pub mod insights;
pub mod merkle_tree;
pub mod parser;
pub mod routing;
pub mod rpc_provider;
pub mod runner;
pub mod simulation;
pub mod stellar_service;
pub mod simulation_service;
pub mod wasm_branch_analysis;
pub mod safe_math;
pub mod hysteresis;
pub mod strategy_persistence;
pub mod circuit_breaker;
pub mod graphql_gateway;
pub mod job_queue;
pub mod rest_api;
pub mod webhooks;
pub mod vault_validation;
pub mod namespaced_errors;
pub mod policy_expiry;
pub mod oracle_guard;
pub mod reputation;
pub mod rotation_journal;
pub mod log_redaction;
pub mod logging;
pub mod nonce_partition;
pub mod two_phase_commit;
pub mod signed_receipt;
pub mod config_versioning;
pub mod settlement;
pub mod archival;
pub mod index_audit;
pub mod outbox;
pub mod soft_delete;
pub mod note_privacy;
pub mod rounding;
pub mod agent_identity;
pub mod agent_health;
pub mod failover;
pub mod rate_limiter;
pub mod validation;
pub mod fee_validation;
pub mod secure_ids;
pub mod audit_log;
pub mod session;
pub mod metrics;
pub mod input_sanitization;
pub mod idempotency;
pub mod pagination;
pub mod validation_pipeline;
pub mod notifications;
pub mod timestamps;
pub mod db;

#[cfg(test)]
pub mod fuzz_simulation;
#[cfg(test)]
pub mod fuzz_tests;
