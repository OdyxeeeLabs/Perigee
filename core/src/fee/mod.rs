pub mod analytics;
pub mod calculation;
pub mod collector;
pub mod persistence;
pub mod service;
pub mod validation;

pub use analytics::{
    AssetHwmTracker, FeeAnalyticsEngine, FeePrediction, LossRecoveryHwm, MarketConditions,
    ModelBreakdown, TrendDirection,
};
pub use calculation::{
    calculate_bid, select_bid, FeeAnalyticsResult, FeeHistoryQuery, FeeHistoryResult,
    FeeRecommendationInputs, FeeRecommendationResult, InclusionSpeed,
    DEFAULT_SAFETY_MARGIN_BPS, SAFETY_MARGIN_MAX_BPS, SAFETY_MARGIN_MIN_BPS,
};
pub use collector::{deduct_network_fees, FeeCollector, FeeCollectorConfig, FeeCollectorError};
pub use persistence::{
    calculate_sweep_amount, needs_sweep, FeeStore, FeeStoreError, LedgerFeeSample,
    TransactionFeeRecord, UnclaimedFeeCap,
};
pub use service::{ChargeReceipt, ChargeRequest, FeeService, IdempotencyStore, IDEMPOTENCY_TTL};
pub use validation::{
    safety_margin_to_bps, validate_charge_request, validate_ledger_fee_sample,
    validate_safety_margin_bps, FeeValidationError, FeeValidator,
};
