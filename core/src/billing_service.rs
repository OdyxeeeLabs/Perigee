//! Fee-market / billing service.
//!
//! API-28 (Perigee): Business logic was previously living inside the HTTP
//! controllers in `main.rs`. This service exposes that logic through plain
//! async methods so the Axum handlers only do request → service → response
//! translation and error mapping.
//!
//! API-26 (Perigee): Monetary values use integer minor units (stroops =
//! 1e-7 XLM) end-to-end. Statistical ratios (confidence, volatility,
//! transaction pressure) are stored as basis points (0..=10000) so the
//! representations stay integer/deterministic. No f64 multiplications
//! are performed on fee/NAV amounts.

use crate::errors::AppError;
use crate::fee_analytics::{FeeAnalyticsEngine, FeePrediction, MarketConditions, ModelBreakdown};
use crate::fee_store::{FeeStore, LedgerFeeSample};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

/// Default safety margin in basis points: 11000 bps = 110% (= 10% above
/// the percentile-based bid).
pub const DEFAULT_SAFETY_MARGIN_BPS: u32 = 11_000;

/// Lower bound (50% = no margin) and upper bound (500% = 5x) for any
/// caller-supplied safety margin. Generous on purpose but rejects obvious
/// misconfigurations such as 0 or 1e9.
pub const SAFETY_MARGIN_MIN_BPS: u32 = 5_000;
pub const SAFETY_MARGIN_MAX_BPS: u32 = 50_000;

/// How quickly a bid should land on chain.
#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InclusionSpeed {
    /// Aim to be included in the very next ledger.
    NextLedger,
    /// Target inclusion within 3 ledgers.
    Next3Ledgers,
    /// Lowest-cost bid; may take longer.
    Economy,
    /// Balanced choice.
    Standard,
    /// Fast inclusion.
    Priority,
}

impl InclusionSpeed {
    /// Parse the wire-format string used in query params / JSON bodies.
    pub fn parse(s: Option<&str>) -> Self {
        match s.unwrap_or("").to_ascii_lowercase().as_str() {
            "next_ledger" => Self::NextLedger,
            "next_3_ledgers" => Self::Next3Ledgers,
            "economy" => Self::Economy,
            "standard" => Self::Standard,
            _ => Self::Priority,
        }
    }
}

#[derive(Debug, Clone)]
pub struct FeeRecommendationInputs {
    pub inclusion_speed: InclusionSpeed,
    /// Safety margin in basis points (5_000 = 50%, 11_000 = 110%, etc.).
    pub safety_margin_bps: u32,
}

impl Default for FeeRecommendationInputs {
    fn default() -> Self {
        Self {
            inclusion_speed: InclusionSpeed::Priority,
            safety_margin_bps: DEFAULT_SAFETY_MARGIN_BPS,
        }
    }
}

/// API DTO for a fee recommendation.
#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FeeRecommendationResult {
    /// Recommended fee bid in stroops (integer minor units).
    pub recommended_bid: u64,
    /// Resource-fee component of the bid, also in stroops.
    pub resource_fee_estimate: u64,
    /// Total estimated cost in stroops (= bid + resource_fee at this time).
    pub total_estimated_cost: u64,
    /// Inclusion confidence in basis points (0..=10000).
    pub inclusion_confidence_bps: u32,
    /// Expected ledgers until on-chain inclusion.
    pub expected_inclusion_ledgers: u32,
    pub market_conditions: MarketConditions,
    pub model_breakdown: ModelBreakdown,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Clone, Default, Deserialize, ToSchema)]
pub struct FeeHistoryQuery {
    pub limit: Option<i64>,
    pub from_ledger: Option<i64>,
    pub to_ledger: Option<i64>,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FeeHistoryResult {
    pub samples: Vec<LedgerFeeSample>,
    pub total_count: i64,
}

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FeeAnalyticsResult {
    pub current_ledger: u64,
    pub prediction: FeePrediction,
    pub market_conditions: MarketConditions,
    pub model_breakdown: ModelBreakdown,
    pub sample_count: usize,
    pub timestamp: DateTime<Utc>,
}

/// Fee business logic. Decoupled from the HTTP/transport layer so it can be
/// reused from CLI subcommands, job runners, JSON-RPC adapters, etc.
pub struct FeeService {
    store: Arc<FeeStore>,
    engine: FeeAnalyticsEngine,
}

impl FeeService {
    pub fn new(store: Arc<FeeStore>, engine: FeeAnalyticsEngine) -> Self {
        Self { store, engine }
    }

    /// Validate a basis-point safety margin against the supported range.
    pub fn validate_safety_margin_bps(bps: u32) -> Result<u32, AppError> {
        if !(SAFETY_MARGIN_MIN_BPS..=SAFETY_MARGIN_MAX_BPS).contains(&bps) {
            return Err(AppError::BadRequest(format!(
                "safety_margin_bps must be in [{}, {}] (got {})",
                SAFETY_MARGIN_MIN_BPS, SAFETY_MARGIN_MAX_BPS, bps
            )));
        }
        Ok(bps)
    }

    /// Backwards-compatible conversion from the legacy `f64` safety margin
    /// (e.g. 1.10 → 11000 bps) used in older API requests.
    pub fn safety_margin_to_bps(safety_margin: f64) -> Result<u32, AppError> {
        if !safety_margin.is_finite() || safety_margin <= 0.0 {
            return Err(AppError::BadRequest(format!(
                "safety_margin must be a finite positive multiplier (got {})",
                safety_margin
            )));
        }
        let bps = (safety_margin * 10_000.0).round() as i64;
        match u32::try_from(bps) {
            Ok(v) => Self::validate_safety_margin_bps(v),
            Err(_) => Err(AppError::BadRequest(format!(
                "safety_margin {} bps out of range [{}, {}]",
                bps, SAFETY_MARGIN_MIN_BPS, SAFETY_MARGIN_MAX_BPS
            ))),
        }
    }

    /// Build a fee recommendation from the most-recent fee samples.
    pub async fn recommend(
        &self,
        inputs: FeeRecommendationInputs,
    ) -> Result<FeeRecommendationResult, AppError> {
        let engine = self.engine.with_safety_margin_bps(inputs.safety_margin_bps);
        let samples = self
            .store
            .get_recent_samples(100)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to fetch fee data: {}", e)))?;
        let current_ledger = samples.first().map(|s| s.ledger_sequence as u64).unwrap_or(0);

        let prediction = engine.predict(&samples, current_ledger);
        let market_conditions = engine.get_market_conditions(&samples, current_ledger);
        let model_breakdown = engine.get_model_breakdown(&samples);

        let (recommended_bid, expected_inclusion_ledgers) = match inputs.inclusion_speed {
            InclusionSpeed::NextLedger => (prediction.next_ledger_bid, 1),
            InclusionSpeed::Next3Ledgers => (prediction.next_3_ledgers_bid, 3),
            InclusionSpeed::Economy => (prediction.economy_bid, 10),
            InclusionSpeed::Standard => (prediction.standard_bid, 3),
            InclusionSpeed::Priority => (prediction.priority_bid, 1),
        };

        Ok(FeeRecommendationResult {
            recommended_bid,
            resource_fee_estimate: 0,
            total_estimated_cost: recommended_bid,
            inclusion_confidence_bps: prediction.confidence_bps,
            expected_inclusion_ledgers,
            market_conditions,
            model_breakdown,
            timestamp: Utc::now(),
        })
    }

    /// Recent fee samples plus the historical total in the table.
    pub async fn history(&self, query: FeeHistoryQuery) -> Result<FeeHistoryResult, AppError> {
        let limit = query.limit.unwrap_or(50).clamp(1, 1_000);
        let samples = if let (Some(from), Some(to)) = (query.from_ledger, query.to_ledger) {
            self.store
                .get_samples_in_range(from, to)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to fetch fee history: {}", e)))?
        } else {
            self.store
                .get_recent_samples(limit)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to fetch fee history: {}", e)))?
        };

        let total_count = self
            .store
            .get_sample_count()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to get sample count: {}", e)))?;
        Ok(FeeHistoryResult { samples, total_count })
    }

    /// Composite fee-market analytics (prediction + market + model breakdown).
    pub async fn analytics(&self) -> Result<FeeAnalyticsResult, AppError> {
        let samples = self
            .store
            .get_recent_samples(200)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to fetch fee data: {}", e)))?;
        let current_ledger = samples.first().map(|s| s.ledger_sequence as u64).unwrap_or(0);
        let prediction = self.engine.predict(&samples, current_ledger);
        let market_conditions = self.engine.get_market_conditions(&samples, current_ledger);
        let model_breakdown = self.engine.get_model_breakdown(&samples);

        Ok(FeeAnalyticsResult {
            current_ledger,
            prediction,
            market_conditions,
            model_breakdown,
            sample_count: samples.len(),
            timestamp: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_safety_margin_bps_in_range() {
        assert!(FeeService::validate_safety_margin_bps(11_000).is_ok());
        assert!(FeeService::validate_safety_margin_bps(5_000).is_ok());
        assert!(FeeService::validate_safety_margin_bps(50_000).is_ok());
    }

    #[test]
    fn test_validate_safety_margin_bps_out_of_range() {
        assert!(FeeService::validate_safety_margin_bps(4_999).is_err());
        assert!(FeeService::validate_safety_margin_bps(50_001).is_err());
    }

    #[test]
    fn test_safety_margin_to_bps() {
        assert_eq!(FeeService::safety_margin_to_bps(1.10).unwrap(), 11_000);
        assert_eq!(FeeService::safety_margin_to_bps(1.0).unwrap(), 10_000);
        assert_eq!(FeeService::safety_margin_to_bps(2.0).unwrap(), 20_000);
    }

    #[test]
    fn test_safety_margin_to_bps_rejects_invalid() {
        assert!(FeeService::safety_margin_to_bps(0.0).is_err());
        assert!(FeeService::safety_margin_to_bps(-1.0).is_err());
        assert!(FeeService::safety_margin_to_bps(f64::NAN).is_err());
        assert!(FeeService::safety_margin_to_bps(10.0).is_err()); // 100_000 bps > max
    }

    #[test]
    fn test_inclusion_speed_parse() {
        assert_eq!(InclusionSpeed::parse(Some("economy")), InclusionSpeed::Economy);
        assert_eq!(InclusionSpeed::parse(Some("priority")), InclusionSpeed::Priority);
        assert_eq!(InclusionSpeed::parse(Some("next_ledger")), InclusionSpeed::NextLedger);
        assert_eq!(InclusionSpeed::parse(Some("next_3_ledgers")), InclusionSpeed::Next3Ledgers);
        assert_eq!(InclusionSpeed::parse(Some("standard")), InclusionSpeed::Standard);
        // Unknown → Priority (matches existing handler behaviour).
        assert_eq!(InclusionSpeed::parse(Some("garbage")), InclusionSpeed::Priority);
        assert_eq!(InclusionSpeed::parse(None), InclusionSpeed::Priority);
    }
}
