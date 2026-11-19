//! Core fee market integration: calculation, collection, persistence, analytics.
//!
//! On-chain fee transparency and sponsorship live in `contracts/fee_engine`
//! (#669, #670). This module provides off-chain aggregation and validation.

pub mod analytics;
pub mod calculation;
pub mod collector;
pub mod persistence;
pub mod service;
pub mod validation;

pub use analytics::{filter_by_vault, summarize, FeeAnalyticsSummary};
pub use collector::from_event_parts;
pub use persistence::FeeHistoryStore;
pub use service::{
    validate_sponsored_request, FeeKind, FeeRecordView, SponsoredFeeRequest,
};
