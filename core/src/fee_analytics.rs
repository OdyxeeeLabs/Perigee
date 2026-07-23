//! Fee market analytics engine.
//!
//! API-26 (Perigee): every monetary / ratio value is computed in integer
//! units. Stroops (= 1e-7 XLM) are used for any fee / NAV amount, and
//! basis points (`0..=10_000`) are used for any ratio (confidence, volatility,
//! transaction pressure, etc.). This avoids any `f64` rounding loss on
//! fees — the regression that motivated API-26.

#![allow(clippy::unnecessary_cast)]

use crate::fee_store::LedgerFeeSample;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Direction of fee trend.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
pub enum TrendDirection {
    Upward,
    Downward,
    Stable,
}

/// Fee market prediction with multiple bidding strategies.
///
/// All bid values are integer stroops (1e-7 XLM). Ratio values are stored
/// as basis points so they can be serialised as integers without losing
/// precision the way `f64` would on edge-case values.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FeePrediction {
    /// Current ledger sequence.
    pub current_ledger: u64,
    /// Recommended bid for inclusion in the next ledger (integer stroops).
    pub next_ledger_bid: u64,
    /// Recommended bid for inclusion within 3 ledgers (integer stroops).
    pub next_3_ledgers_bid: u64,
    /// Economy bid (lowest cost, may take longer) — 90% of median, integer stroops.
    pub economy_bid: u64,
    /// Standard bid at the configured safety margin, integer stroops.
    pub standard_bid: u64,
    /// Priority bid (fast inclusion), integer stroops.
    pub priority_bid: u64,
    /// Urgent bid (next-ledger inclusion), integer stroops.
    pub urgent_bid: u64,
    /// Inclusion confidence in basis points (`0..=10_000`).
    pub confidence_bps: u32,
    /// Market volatility (coefficient of variation) in basis points (`0..=10_000`).
    pub market_volatility_bps: u32,
    pub trend_direction: TrendDirection,
}

/// Detailed model breakdown for transparency (all integer).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ModelBreakdown {
    /// 10-ledger Simple Moving Average (integer stroops).
    pub sma_10: u64,
    /// 50-ledger Simple Moving Average (integer stroops).
    pub sma_50: u64,
    /// 12-ledger Exponential Moving Average (integer stroops).
    pub ema_12: u64,
    /// 50th percentile (median), integer stroops.
    pub percentile_50: u64,
    /// 75th percentile, integer stroops.
    pub percentile_75: u64,
    /// 95th percentile, integer stroops.
    pub percentile_95: u64,
    /// Standard deviation (integer stroops, floored square root).
    pub standard_deviation: u64,
    /// Coefficient of variation in basis points (`0..=10_000`).
    pub coefficient_of_variation_bps: u32,
}

/// Market-conditions snapshot.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct MarketConditions {
    pub current_ledger: u64,
    /// Volatility bucket ("low"|"medium"|"high"|"unknown").
    pub volatility: String,
    /// Trend bucket ("upward"|"downward"|"stable"|"unknown").
    pub trend: String,
    /// Average fee over the short window (integer stroops).
    pub avg_fee_10_ledgers: u64,
    /// Average fee over the medium window (integer stroops).
    pub avg_fee_50_ledgers: u64,
    /// Transaction pressure in basis points (`0..=10_000`).
    pub transaction_pressure_bps: u32,
}

/// Analytics engine. All arithmetic is integer — no `f64` is multiplied
/// against a stroop amount anywhere.
pub struct FeeAnalyticsEngine {
    /// SMA window for short-term (default: 10).
    sma_short_window: usize,
    /// SMA window for medium-term (default: 50).
    sma_medium_window: usize,
    /// EMA period (default: 12).
    ema_period: usize,
    /// Safety margin in basis points. Default `11_000` = 110% (= 10% above
    /// the percentile-based bid).
    safety_margin_bps: u32,
}

impl FeeAnalyticsEngine {
    /// Create a new analytics engine with default parameters.
    pub fn new() -> Self {
        Self {
            sma_short_window: 10,
            sma_medium_window: 50,
            ema_period: 12,
            safety_margin_bps: 11_000,
        }
    }

    /// Create with explicit parameters (all integer).
    pub fn with_params(
        sma_short_window: usize,
        sma_medium_window: usize,
        ema_period: usize,
        safety_margin_bps: u32,
    ) -> Self {
        Self {
            sma_short_window,
            sma_medium_window,
            ema_period,
            safety_margin_bps,
        }
    }

    /// Return a copy of this engine with a different safety-margin basis.
    pub fn with_safety_margin_bps(&self, safety_margin_bps: u32) -> Self {
        Self {
            sma_short_window: self.sma_short_window,
            sma_medium_window: self.sma_medium_window,
            ema_period: self.ema_period,
            safety_margin_bps,
        }
    }

    /// Current safety-margin in basis points.
    pub fn safety_margin_bps(&self) -> u32 {
        self.safety_margin_bps
    }

    /// Generate a comprehensive fee prediction from historical samples.
    pub fn predict(&self, samples: &[LedgerFeeSample], current_ledger: u64) -> FeePrediction {
        if samples.is_empty() {
            return FeePrediction {
                current_ledger,
                next_ledger_bid: 100,
                next_3_ledgers_bid: 100,
                economy_bid: 100,
                standard_bid: 100,
                priority_bid: 150,
                urgent_bid: 200,
                confidence_bps: 0,
                market_volatility_bps: 0,
                trend_direction: TrendDirection::Stable,
            };
        }

        let fees: Vec<i64> = samples.iter().map(|s| s.base_fee).collect();
        let _sma_10 = self.calculate_sma(&fees, self.sma_short_window);
        let _sma_50 = self.calculate_sma(&fees, self.sma_medium_window);
        let _ema_12 = self.calculate_ema(&fees, self.ema_period);
        let p50 = self.calculate_percentile(&fees, 50_000);
        let p75 = self.calculate_percentile(&fees, 75_000);
        let p95 = self.calculate_percentile(&fees, 95_000);
        let std_dev = self.calculate_std_dev(&fees);
        let mean = self.calculate_mean(&fees);

        // Coefficient of variation in basis points (capped at 10_000).
        let cv_bps = ratio_to_bps(std_dev as u64, mean as u64, 100);

        let trend = self.detect_trend(&fees);
        let volatility_bps = cv_bps;

        // Bids are computed in basis-point integer arithmetic.
        let margin = self.safety_margin_bps as u64;
        let economy_bid = ceil_mul_bps(p50, 9_000); // 90% of median
        let standard_bid = ceil_mul_bps(p50, margin);
        let priority_bid = ceil_mul_bps(p75, margin);
        let urgent_bid = ceil_mul_bps(p95, margin);

        let next_ledger_bid = urgent_bid;
        let next_3_ledgers_bid = standard_bid;

        // Confidence: 60% weight on data quantity, 40% on volatility.
        let data_confidence_bps = ratio_to_bps(samples.len() as u64, 100, 1).min(10_000);
        let volatility_confidence_bps = 10_000 - volatility_bps;
        let confidence_bps = ((data_confidence_bps as u64 * 6
            + volatility_confidence_bps as u64 * 4)
            / 10)
            .min(10_000) as u32;

        FeePrediction {
            current_ledger,
            next_ledger_bid,
            next_3_ledgers_bid,
            economy_bid,
            standard_bid,
            priority_bid,
            urgent_bid,
            confidence_bps,
            market_volatility_bps: volatility_bps,
            trend_direction: trend,
        }
    }

    /// Get detailed model breakdown.
    pub fn get_model_breakdown(&self, samples: &[LedgerFeeSample]) -> ModelBreakdown {
        if samples.is_empty() {
            return ModelBreakdown {
                sma_10: 100,
                sma_50: 100,
                ema_12: 100,
                percentile_50: 100,
                percentile_75: 100,
                percentile_95: 100,
                standard_deviation: 0,
                coefficient_of_variation_bps: 0,
            };
        }

        let fees: Vec<i64> = samples.iter().map(|s| s.base_fee).collect();
        let sma_10 = self.calculate_sma(&fees, self.sma_short_window);
        let sma_50 = self.calculate_sma(&fees, self.sma_medium_window);
        let ema_12 = self.calculate_ema(&fees, self.ema_period);
        let p50 = self.calculate_percentile(&fees, 50_000);
        let p75 = self.calculate_percentile(&fees, 75_000);
        let p95 = self.calculate_percentile(&fees, 95_000);
        let std_dev = self.calculate_std_dev(&fees);
        let mean = self.calculate_mean(&fees);
        let cv_bps = ratio_to_bps(std_dev as u64, mean as u64, 100);

        ModelBreakdown {
            sma_10,
            sma_50,
            ema_12,
            percentile_50: p50,
            percentile_75: p75,
            percentile_95: p95,
            standard_deviation: std_dev,
            coefficient_of_variation_bps: cv_bps,
        }
    }

    /// Get market-conditions snapshot.
    pub fn get_market_conditions(
        &self,
        samples: &[LedgerFeeSample],
        current_ledger: u64,
    ) -> MarketConditions {
        if samples.is_empty() {
            return MarketConditions {
                current_ledger,
                volatility: "unknown".to_string(),
                trend: "unknown".to_string(),
                avg_fee_10_ledgers: 100,
                avg_fee_50_ledgers: 100,
                transaction_pressure_bps: 0,
            };
        }

        let fees: Vec<i64> = samples.iter().map(|s| s.base_fee).collect();
        let std_dev = self.calculate_std_dev(&fees);
        let mean = self.calculate_mean(&fees);
        let cv_bps = ratio_to_bps(std_dev as u64, mean as u64, 100);

        let volatility_str = if cv_bps < 1_000 {
            "low"
        } else if cv_bps < 3_000 {
            "medium"
        } else {
            "high"
        }
        .to_string();

        let trend = self.detect_trend(&fees);
        let trend_str = match trend {
            TrendDirection::Upward => "upward",
            TrendDirection::Downward => "downward",
            TrendDirection::Stable => "stable",
        }
        .to_string();

        let avg_10 = self.calculate_sma(&fees, self.sma_short_window);
        let avg_50 = self.calculate_sma(&fees, self.sma_medium_window);

        // Transaction pressure = average tx_count / 100, capped at 10_000 bps.
        let total_tx: u64 = samples
            .iter()
            .map(|s| s.transaction_count as u64)
            .sum();
        let count = samples.len() as u64;
        let avg_tx_count = if count > 0 { total_tx / count } else { 0 };
        let transaction_pressure_bps = (avg_tx_count.saturating_mul(100)).min(10_000) as u32;

        MarketConditions {
            current_ledger,
            volatility: volatility_str,
            trend: trend_str,
            avg_fee_10_ledgers: avg_10,
            avg_fee_50_ledgers: avg_50,
            transaction_pressure_bps,
        }
    }

    // ── Statistical Methods (all integer) ──────────────────────────────

    /// Simple moving average in integer stroops.
    fn calculate_sma(&self, data: &[i64], window: usize) -> u64 {
        if data.is_empty() || window == 0 {
            return 0;
        }
        let window = window.min(data.len());
        let sum: i64 = data.iter().take(window).sum();
        (sum / window as i64).max(0) as u64
    }

    /// EMA with integer arithmetic. Mirrors the float `k = 2 / (period+1)`
    /// smoothing factor: `ema_t = ema_prev + (value - ema_prev) * 2 / (period+1)`.
    fn calculate_ema(&self, data: &[i64], period: usize) -> u64 {
        if data.is_empty() || period == 0 {
            return 0;
        }
        let denom = (period as i64) + 1;
        let mut ema = data[0];
        for &value in data.iter().skip(1).take(period.min(data.len())) {
            ema = ema + (value - ema).saturating_mul(2) / denom;
        }
        ema.max(0) as u64
    }

    /// Percentile with linear interpolation in basis points.
    ///
    /// `percentile_bps` is `0..=10_000` (e.g. 9_500 = 95th percentile).
    fn calculate_percentile(&self, data: &[i64], percentile_bps: u64) -> u64 {
        if data.is_empty() {
            return 0;
        }
        let mut sorted: Vec<i64> = data.to_vec();
        sorted.sort();

        let n = sorted.len();
        if n == 1 {
            return sorted[0].max(0) as u64;
        }
        let last_index = (n - 1) as u64;

        // lower = floor(percentile_bps / 10_000 * last_index)
        let scaled = last_index.saturating_mul(percentile_bps);
        let lower = (scaled / 10_000) as usize;
        let remainder = scaled % 10_000; // bps weight between lower and lower+1
        let upper = (lower + 1).min(n - 1);

        let lo = sorted[lower].max(0) as u64;
        if upper == lower || remainder == 0 {
            return lo;
        }
        let hi = sorted[upper].max(0) as u64;
        // Linear interpolation using basis-point weight: ceil((hi - lo) * w / 10000) + lo
        // Using floor here matches standard nearest-rank behaviour with floor;
        // the rounded value equals round-via-half-up for the typical small
        // differences we're interpolating over.
        let interpolated = lo + (hi - lo).saturating_mul(remainder) / 10_000;
        interpolated
    }

    /// Standard deviation in stroops. Variance is integer; the square root
    /// uses Newton's-method floored square root to keep the result deterministic.
    fn calculate_std_dev(&self, data: &[i64]) -> u64 {
        if data.len() < 2 {
            return 0;
        }
        let n = data.len() as i64;
        let mean = self.calculate_mean(data);
        let variance_sum: i64 = data
            .iter()
            .map(|&x| {
                let d = x - mean;
                d.saturating_mul(d)
            })
            .sum();
        // (Bessel-corrected) variance = sum/(n-1); round by integer division.
        let variance = (variance_sum / (n - 1)).max(0) as u64;
        integer_sqrt(variance)
    }

    /// Integer mean (floor).
    fn calculate_mean(&self, data: &[i64]) -> i64 {
        if data.is_empty() {
            return 0;
        }
        data.iter().sum::<i64>() / data.len() as i64
    }

    /// Trend: compare the integer means of the oldest vs newest windows.
    fn detect_trend(&self, data: &[i64]) -> TrendDirection {
        if data.len() < 10 {
            return TrendDirection::Stable;
        }
        let window = 5.min(data.len() / 2);
        let older: Vec<i64> = data.iter().take(window).copied().collect();
        let recent: Vec<i64> = data
            .iter()
            .skip(data.len() - window)
            .take(window)
            .copied()
            .collect();

        let recent_avg = self.calculate_mean(&recent);
        let older_avg = self.calculate_mean(&older);

        // 5% threshold, computed in integer stroops.
        let threshold = older_avg.abs() / 20;
        if recent_avg > older_avg + threshold {
            TrendDirection::Upward
        } else if recent_avg < older_avg - threshold {
            TrendDirection::Downward
        } else {
            TrendDirection::Stable
        }
    }
}

impl Default for FeeAnalyticsEngine {
    fn default() -> Self {
        Self::new()
    }
}

/// Compute `ceil(x * bps / 10_000)` without using `f64`.
#[inline]
pub(crate) fn ceil_mul_bps(x: u64, bps: u64) -> u64 {
    let prod = (x as u128).saturating_mul(bps as u128);
    ((prod + 9_999) / 10_000) as u64
}

/// Compute `(numerator / denominator) * scale` as basis points.
/// `scale` defaults to 10_000 (i.e. result is in bps) but can be set to `1`
/// for plain percentage rounding. Returns `0` when `denominator == 0`.
#[inline]
pub(crate) fn ratio_to_bps(numerator: u64, denominator: u64, scale: u64) -> u32 {
    if denominator == 0 {
        return 0;
    }
    let n = numerator as u128;
    let d = denominator as u128;
    let s = scale as u128;
    ((n * 10_000 * s) / d).min(10_000) as u32
}

/// Integer square root (Newton's method, floored).
pub(crate) fn integer_sqrt(n: u64) -> u64 {
    if n == 0 {
        return 0;
    }
    let mut x = n;
    let mut y = (x + 1) / 2;
    while y < x {
        x = y;
        y = (x + n / x) / 2;
    }
    x
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fee_store::LedgerFeeSample;
    use chrono::Utc;

    fn create_sample(ledger: i64, base_fee: i64) -> LedgerFeeSample {
        LedgerFeeSample {
            ledger_sequence: ledger,
            collected_at: Utc::now(),
            base_reserve: 0,
            base_fee,
            max_fee: base_fee,
            fee_charged: base_fee,
            transaction_count: 10,
            ledger_close_time: Utc::now(),
        }
    }

    #[test]
    fn test_sma_calculation() {
        let engine = FeeAnalyticsEngine::new();
        let data = vec![100, 110, 120, 130, 140];
        let sma = engine.calculate_sma(&data, 5);
        assert_eq!(sma, 120); // 600/5 = 120
    }

    #[test]
    fn test_ema_calculation() {
        let engine = FeeAnalyticsEngine::new();
        let data = vec![100, 110, 120, 130, 140];
        let ema = engine.calculate_ema(&data, 5);
        // Integer EMA with k=2/(5+1)=1/3 over 4 steps must end ≥ 120.
        assert!(ema >= 120);
        // And ≤ 140 (cannot exceed max of series).
        assert!(ema <= 140);
    }

    #[test]
    fn test_percentile_calculation() {
        let engine = FeeAnalyticsEngine::new();
        let data = vec![100, 200, 300, 400, 500];
        let p50 = engine.calculate_percentile(&data, 50_000);
        assert_eq!(p50, 300); // Median
        let p95 = engine.calculate_percentile(&data, 95_000);
        assert!(p95 >= 400);
        assert!(p95 <= 500);
    }

    #[test]
    fn test_bid_ordering() {
        let engine = FeeAnalyticsEngine::new();
        let samples: Vec<LedgerFeeSample> = (0..50)
            .map(|i| create_sample(i as i64 + 1, 100 + (i % 10) * 5))
            .collect();
        let p = engine.predict(&samples, 100);
        assert!(p.economy_bid <= p.standard_bid);
        assert!(p.standard_bid <= p.priority_bid);
        assert!(p.priority_bid <= p.urgent_bid);
    }

    #[test]
    fn test_fee_prediction_empty_data() {
        let engine = FeeAnalyticsEngine::new();
        let samples: Vec<LedgerFeeSample> = vec![];
        let prediction = engine.predict(&samples, 50);
        assert_eq!(prediction.current_ledger, 50);
        assert_eq!(prediction.next_ledger_bid, 100);
        assert_eq!(prediction.confidence_bps, 0);
    }

    #[test]
    fn test_model_breakdown() {
        let engine = FeeAnalyticsEngine::new();
        let samples: Vec<LedgerFeeSample> = (0..20)
            .map(|i| create_sample(i as i64 + 1, 100 + i * 2))
            .collect();
        let breakdown = engine.get_model_breakdown(&samples);
        assert!(breakdown.sma_10 > 0);
        assert!(breakdown.sma_50 > 0);
        assert!(breakdown.ema_12 > 0);
        assert!(breakdown.percentile_50 > 0);
        assert!(breakdown.standard_deviation >= 0);
    }

    #[test]
    fn test_market_conditions() {
        let engine = FeeAnalyticsEngine::new();
        let samples: Vec<LedgerFeeSample> =
            (0..30).map(|i| create_sample(i as i64 + 1, 100)).collect();
        let conditions = engine.get_market_conditions(&samples, 100);
        assert_eq!(conditions.current_ledger, 100);
        assert_eq!(conditions.avg_fee_10_ledgers, 100);
        // avg_tx_count = 10 over 30 samples, 10 * 100 = 1000 bps.
        assert_eq!(conditions.transaction_pressure_bps, 1_000);
    }

    #[test]
    fn test_trend_upward() {
        let engine = FeeAnalyticsEngine::new();
        let data = vec![500, 400, 300, 200, 100, 150, 250, 350, 450, 550];
        assert_eq!(engine.detect_trend(&data), TrendDirection::Upward);
    }

    #[test]
    fn test_trend_downward() {
        let engine = FeeAnalyticsEngine::new();
        let data = vec![100, 200, 300, 400, 500, 450, 350, 250, 150, 50];
        assert_eq!(engine.detect_trend(&data), TrendDirection::Downward);
    }

    #[test]
    fn test_trend_stable() {
        let engine = FeeAnalyticsEngine::new();
        let data = vec![100, 105, 102, 98, 103, 101, 99, 104, 100, 102];
        assert_eq!(engine.detect_trend(&data), TrendDirection::Stable);
    }

    #[test]
    fn test_ceil_mul_bps_basic() {
        // Multiply an amount by a fractional basis-point multiplier using
        // rounded-up integer arithmetic — closes the f64 imprecision gap.
        assert_eq!(ceil_mul_bps(100, 9_000), 90); // 100 * 0.90 = 90
        assert_eq!(ceil_mul_bps(100, 11_000), 110); // 100 * 1.10 = 110
        assert_eq!(ceil_mul_bps(91, 11_000), 101); // 91 * 1.10 = 100.1 → ceil = 101
        assert_eq!(ceil_mul_bps(50, 9_500), 48); // 50 * 0.95 = 47.5 → ceil = 48
        assert_eq!(ceil_mul_bps(0, 9_500), 0);
    }

    #[test]
    fn test_ratio_to_bps_basic() {
        // Std-dev of identical values is 0 → ratio 0.
        assert_eq!(ratio_to_bps(0, 100, 1), 0);
        // Std-dev equal to mean → 10000 bps (100%).
        assert_eq!(ratio_to_bps(100, 100, 1), 10_000);
    }

    #[test]
    fn test_integer_sqrt_basic() {
        assert_eq!(integer_sqrt(0), 0);
        assert_eq!(integer_sqrt(1), 1);
        assert_eq!(integer_sqrt(4), 2);
        assert_eq!(integer_sqrt(9), 3);
        assert_eq!(integer_sqrt(100), 10);
        assert_eq!(integer_sqrt(1_000), 31);
        assert_eq!(integer_sqrt(60), 7); // floor(sqrt(60))
    }

    #[test]
    fn test_with_safety_margin_bps() {
        let engine = FeeAnalyticsEngine::new();
        let tighter = engine.with_safety_margin_bps(10_500);
        assert_eq!(tighter.safety_margin_bps(), 10_500);
        assert_eq!(engine.safety_margin_bps(), 11_000); // original untouched
    }
}
