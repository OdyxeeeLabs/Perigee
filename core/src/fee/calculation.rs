use crate::fee::analytics::{FeePrediction, MarketConditions, ModelBreakdown};
use crate::fee::persistence::LedgerFeeSample;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

pub const DEFAULT_SAFETY_MARGIN_BPS: u32 = 11_000;
pub const SAFETY_MARGIN_MIN_BPS: u32 = 5_000;
pub const SAFETY_MARGIN_MAX_BPS: u32 = 50_000;

#[derive(Debug, Clone, Copy, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum InclusionSpeed {
    NextLedger,
    Next3Ledgers,
    Economy,
    Standard,
    Priority,
}

impl InclusionSpeed {
    pub fn parse(value: Option<&str>) -> Self {
        match value.unwrap_or_default().trim().to_ascii_lowercase().as_str() {
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

#[derive(Debug, Clone, Serialize, ToSchema)]
pub struct FeeRecommendationResult {
    pub recommended_bid: u64,
    pub resource_fee_estimate: u64,
    pub total_estimated_cost: u64,
    pub inclusion_confidence_bps: u32,
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

pub fn ceil_mul_bps(amount: u64, bps: u64) -> u64 {
    crate::rounding::apply_bps_ceil(amount, bps)
}

pub fn calculate_bid(base_fee: i64, safety_margin_bps: u32) -> u64 {
    if base_fee <= 0 {
        return 0;
    }
    ceil_mul_bps(base_fee as u64, safety_margin_bps as u64)
}

pub fn select_bid(prediction: &FeePrediction, speed: InclusionSpeed) -> (u64, u32) {
    match speed {
        InclusionSpeed::NextLedger => (prediction.next_ledger_bid, 1),
        InclusionSpeed::Next3Ledgers => (prediction.next_3_ledgers_bid, 3),
        InclusionSpeed::Economy => (prediction.economy_bid, 10),
        InclusionSpeed::Standard => (prediction.standard_bid, 3),
        InclusionSpeed::Priority => (prediction.priority_bid, 1),
    }
}
