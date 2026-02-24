use crate::simulation::SorobanResources;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct Insight {
    pub severity: Severity,
    pub message: String,
    pub suggested_fix: String,
}

pub trait OptimizationRule: Send + Sync {
    fn check(&self, resources: &SorobanResources) -> Option<Insight>;
}

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

pub struct InsightsEngine {
    rules: Vec<Box<dyn OptimizationRule>>,
}

impl InsightsEngine {
    pub fn new() -> Self {
        Self {
            rules: vec![
                Box::new(StorageEfficiencyRule),
                Box::new(InstructionDensityRule),
                Box::new(FootprintBloatRule),
            ],
        }
    }

    pub fn get_insights(&self, resources: &SorobanResources) -> Vec<Insight> {
        self.rules
            .iter()
            .filter_map(|rule| rule.check(resources))
            .collect()
    }

    pub fn calculate_efficiency_score(&self, resources: &SorobanResources) -> u8 {
        let mut score: i32 = 100;

        if resources.cpu_instructions > 1_000_000 {
            score -= ((resources.cpu_instructions - 1_000_000) / 500_000) as i32;
        }

        if resources.footprint_size > 5 {
            score -= ((resources.footprint_size - 5) * 5) as i32;
        }

        if resources.ram_bytes > 102_400 {
            score -= ((resources.ram_bytes - 102_400) / 51_200) as i32;
        }

        score.clamp(0, 100) as u8
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
    fn test_efficiency_score() {
        let engine = InsightsEngine::new();
        let resources = SorobanResources::default();
        assert_eq!(engine.calculate_efficiency_score(&resources), 100);
    }
}
