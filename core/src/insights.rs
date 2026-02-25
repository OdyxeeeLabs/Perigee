use crate::simulation::SorobanResources;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// Severity levels for optimization insights.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

/// A diagnostic insight for a Soroban contract.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Insight {
    pub severity: Severity,
    pub message: String,
    pub suggested_fix: String,
}

/// Trait for extensible optimization rules.
pub trait OptimizationRule: Send + Sync {
    fn check(&self, resources: &SorobanResources) -> Option<Insight>;
}

/// Detects high ledger write volume relative to transaction size.
pub struct StorageEfficiencyRule;

impl OptimizationRule for StorageEfficiencyRule {
    fn check(&self, resources: &SorobanResources) -> Option<Insight> {
        if resources.transaction_size_bytes > 0
            && resources.ledger_write_bytes > 10240
            && (resources.ledger_write_bytes as f64 / resources.transaction_size_bytes as f64) > 0.7
        {
            return Some(Insight {
                severity: Severity::Warning,
                message: "High ledger write volume detected relative to transaction size.".to_string(),
                suggested_fix: "Consider using Temporary storage for scratch data or compressing large state objects.".to_string(),
            });
        }
        None
    }
}

/// Identifies heavy computation logic with minimal ledger activity.
pub struct InstructionDensityRule;

impl OptimizationRule for InstructionDensityRule {
    fn check(&self, resources: &SorobanResources) -> Option<Insight> {
        if resources.cpu_instructions > 5_000_000 && resources.ledger_read_bytes < 1024 {
            return Some(Insight {
                severity: Severity::Info,
                message: "Heavy computation detected with minimal ledger I/O.".to_string(),
                suggested_fix: "Ensure loops are bounded and avoid expensive cryptographic operations if possible.".to_string(),
            });
        }
        None
    }
}

/// Flags transactions with more than 10 ledger keys in the footprint.
pub struct FootprintBloatRule;

impl OptimizationRule for FootprintBloatRule {
    fn check(&self, resources: &SorobanResources) -> Option<Insight> {
        if resources.footprint_size > 10 {
            return Some(Insight {
                severity: Severity::Critical,
                message: format!(
                    "Footprint contains {} keys, exceeding recommended limit (10).",
                    resources.footprint_size
                ),
                suggested_fix: "Consolidate related data into fewer LedgerEntries or use Instance storage for shared configuration.".to_string(),
            });
        }
        None
    }
}

/// The engine responsible for applying optimization rules and scoring.
pub struct InsightsEngine {
    rules: Vec<Box<dyn OptimizationRule>>,
}

impl InsightsEngine {
    /// Creates a new `InsightsEngine` with default heuristic rules.
    pub fn new() -> Self {
        Self {
            rules: vec![
                Box::new(StorageEfficiencyRule),
                Box::new(InstructionDensityRule),
                Box::new(FootprintBloatRule),
            ],
        }
    }

    /// Evaluates all rules against the provided resources.
    pub fn get_insights(&self, resources: &SorobanResources) -> Vec<Insight> {
        self.rules
            .iter()
            .filter_map(|rule| rule.check(resources))
            .collect()
    }

    /// Calculates a weighted efficiency score (0-100).
    pub fn calculate_efficiency_score(&self, resources: &SorobanResources) -> u8 {
        let mut score: i32 = 100;

        // Penalty for high CPU instructions (base allowance: 1M)
        if resources.cpu_instructions > 1_000_000 {
            let cpu_over = resources.cpu_instructions.saturating_sub(1_000_000);
            // 5 points penalty for every 500k instructions over 1M.
            let penalty = ((cpu_over / 500_000) * 5).min(40) as i32;
            score -= penalty;
        }

        // Penalty for high footprint size (base allowance: 5 keys)
        if resources.footprint_size > 5 {
            let fp_over = resources.footprint_size.saturating_sub(5);
            // 10 points penalty for every key over 5.
            let penalty = (fp_over * 10).min(40) as i32;
            score -= penalty;
        }

        // Penalty for high RAM usage (base allowance: 100KB)
        if resources.ram_bytes > 102_400 {
            let ram_over = resources.ram_bytes.saturating_sub(102_400);
            // 5 points penalty for every 64KB over 100KB.
            let penalty = ((ram_over / 65_536) * 5).min(20) as i32;
            score -= penalty;
        }

        score.max(0).min(100) as u8
    }
}

impl Default for InsightsEngine {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_footprint_bloat_rule() {
        let rule = FootprintBloatRule;
        let mut resources = SorobanResources::default();
        resources.footprint_size = 15;

        let insight = rule.check(&resources).expect("Rule should trigger");
        assert_eq!(insight.severity, Severity::Critical);
        assert!(insight.message.contains("15 keys"));
    }

    #[test]
    fn test_efficiency_score_calculation() {
        let engine = InsightsEngine::new();
        let resources = SorobanResources::default();
        assert_eq!(engine.calculate_efficiency_score(&resources), 100);

        let moderate_heavy = SorobanResources {
            cpu_instructions: 2_000_000, // 1M over -> 2 * 5 penalty = 10
            footprint_size: 7,          // 2 over -> 2 * 10 penalty = 20
            ram_bytes: 50_000,          // Under limit
            ..Default::default()
        };
        // Expected score: 100 - 10 - 20 = 70
        assert_eq!(engine.calculate_efficiency_score(&moderate_heavy), 70);

        let very_heavy = SorobanResources {
            cpu_instructions: 10_000_000, // Max CPU penalty (40)
            footprint_size: 20,          // Max Footprint penalty (40)
            ram_bytes: 1_000_000,        // Max RAM penalty (20)
            ..Default::default()
        };
        // Expected score: 100 - 40 - 40 - 20 = 0
        assert_eq!(engine.calculate_efficiency_score(&very_heavy), 0);
    }
}
