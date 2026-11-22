use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use utoipa::ToSchema;

use crate::simulation::{ProtocolCostParameters, SimulationError, SorobanResources};

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GasGolfingSuggestion {
    pub pattern_type: String,
    pub description: String,
    pub location: Option<String>, // WASM offset or function name
    pub severity: String,         // "low", "medium", "high"
    pub gas_saved_estimate: Option<u64>,
    pub suggested_fix: String,
    pub code_example: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct GasGolfingReport {
    pub contract_name: String,
    pub analysis_timestamp: DateTime<Utc>,
    pub total_suggestions: usize,
    pub suggestions: Vec<GasGolfingSuggestion>,
    pub summary: HashMap<String, usize>, // pattern_type -> count
    /// Protocol used to convert measured resources to stroops.
    pub protocol_version: u32,
    /// Cost of the supplied measured resource sample, when available.
    pub measured_cost_stroops: Option<u64>,
    /// Static pattern matching cannot account for inputs, host calls, or
    /// compiler output. Quantified suggestions therefore carry this bound.
    pub estimate_margin_of_error_percent: u8,
}

#[derive(Clone)]
pub struct GasGolfingAnalyzer {
    cost_parameters: ProtocolCostParameters,
}

impl Default for GasGolfingAnalyzer {
    fn default() -> Self {
        Self::new()
    }
}

impl GasGolfingAnalyzer {
    pub fn new() -> Self {
        Self::for_protocol(ProtocolCostParameters::resolve(None).expect("latest protocol is supported"))
    }

    fn for_protocol(cost_parameters: ProtocolCostParameters) -> Self {
        Self { cost_parameters }
    }

    pub fn for_protocol_version(protocol_version: u32) -> Result<Self, SimulationError> {
        Ok(Self::for_protocol(ProtocolCostParameters::for_protocol(
            protocol_version,
        )?))
    }

    pub fn analyze_wasm(&self, wasm_bytes: &[u8], contract_name: &str) -> GasGolfingReport {
        self.analyze_wasm_with_measurement(wasm_bytes, contract_name, None)
    }

    /// Analyze bytecode and optionally quantify savings from a measured
    /// Soroban resource sample. Without a sample, savings are deliberately
    /// left unset: WASM bytes alone do not reveal runtime inputs or host costs.
    pub fn analyze_wasm_with_measurement(
        &self,
        wasm_bytes: &[u8],
        contract_name: &str,
        measured_resources: Option<&SorobanResources>,
    ) -> GasGolfingReport {
        let mut suggestions = Vec::new();
        let mut summary = HashMap::new();
        let measured_cost_stroops = measured_resources.map(|resources| self.cost_parameters.cost_of(resources));

        // Analyze WASM bytecode for common gas-heavy patterns
        suggestions.extend(self.analyze_loop_patterns(wasm_bytes, measured_cost_stroops));
        suggestions.extend(self.analyze_memory_patterns(wasm_bytes, measured_cost_stroops));
        suggestions.extend(self.analyze_arithmetic_patterns(wasm_bytes, measured_cost_stroops));
        suggestions.extend(self.analyze_storage_patterns(wasm_bytes, measured_cost_stroops));
        suggestions.extend(self.analyze_branching_patterns(wasm_bytes, measured_cost_stroops));

        // Build summary
        for suggestion in &suggestions {
            *summary.entry(suggestion.pattern_type.clone()).or_insert(0) += 1;
        }

        GasGolfingReport {
            contract_name: contract_name.to_string(),
            analysis_timestamp: Utc::now(),
            total_suggestions: suggestions.len(),
            suggestions,
            summary,
            protocol_version: self.cost_parameters.protocol_version,
            measured_cost_stroops,
            estimate_margin_of_error_percent: if measured_cost_stroops.is_some() { 20 } else { 100 },
        }
    }

    fn analyze_loop_patterns(&self, wasm_bytes: &[u8], measured_cost: Option<u64>) -> Vec<GasGolfingSuggestion> {
        let mut suggestions = Vec::new();

        // Look for inefficient loop patterns
        // This is a simplified analysis - in practice, you'd use wasmparser crate
        if wasm_bytes.windows(4).any(|w| w == [0x02, 0x40, 0x03, 0x40]) {
            // Block + loop pattern that might be inefficient
            suggestions.push(GasGolfingSuggestion {
                pattern_type: "loop_optimization".to_string(),
                description: "Detected potential loop optimization opportunity".to_string(),
                location: Some("unknown".to_string()),
                severity: "medium".to_string(),
                gas_saved_estimate: measured_cost.map(|cost| cost.saturating_mul(5) / 100),
                suggested_fix: "Consider using bitwise operations or lookup tables for repetitive calculations".to_string(),
                code_example: Some("Replace: for(i = 0; i < 256; i++) { if(i & mask) count++; }\nWith: count = bit_count(mask);".to_string()),
            });
        }

        suggestions
    }

    fn analyze_memory_patterns(&self, wasm_bytes: &[u8], measured_cost: Option<u64>) -> Vec<GasGolfingSuggestion> {
        let mut suggestions = Vec::new();

        // Look for excessive memory allocations
        let alloc_count = wasm_bytes.windows(2).filter(|w| w == &[0x20, 0x00]).count();
        if alloc_count > 10 {
            suggestions.push(GasGolfingSuggestion {
                pattern_type: "memory_allocation".to_string(),
                description: format!("High memory allocation count: {}", alloc_count),
                location: None,
                severity: "high".to_string(),
                gas_saved_estimate: measured_cost.map(|cost| cost.saturating_mul(10) / 100),
                suggested_fix: "Reuse memory buffers and minimize allocations in hot paths"
                    .to_string(),
                code_example: Some(
                    "Use a pre-allocated buffer instead of creating new vectors in loops"
                        .to_string(),
                ),
            });
        }

        suggestions
    }

    fn analyze_arithmetic_patterns(&self, wasm_bytes: &[u8], measured_cost: Option<u64>) -> Vec<GasGolfingSuggestion> {
        let mut suggestions = Vec::new();

        // Look for expensive division operations
        let div_count = wasm_bytes
            .iter()
            .filter(|&&b| b == 0x6D || b == 0x6E)
            .count();
        if div_count > 5 {
            suggestions.push(GasGolfingSuggestion {
                pattern_type: "arithmetic_optimization".to_string(),
                description: format!("Multiple division operations detected: {}", div_count),
                location: None,
                severity: "medium".to_string(),
                gas_saved_estimate: measured_cost.map(|cost| cost.saturating_mul(3) / 100),
                suggested_fix:
                    "Replace divisions with multiplications by reciprocals or use bitwise shifts"
                        .to_string(),
                code_example: Some("Replace: x / 2\nWith: x >> 1".to_string()),
            });
        }

        // Look for multiplication by constants that could be shifts
        if wasm_bytes.windows(3).any(|w| w == [0x41, 0x02, 0x6C]) {
            suggestions.push(GasGolfingSuggestion {
                pattern_type: "multiplication_optimization".to_string(),
                description: "Multiplication by small constant detected".to_string(),
                location: None,
                severity: "low".to_string(),
                gas_saved_estimate: measured_cost.map(|cost| cost / 100),
                suggested_fix: "Use bitwise shifts for multiplication/division by powers of 2"
                    .to_string(),
                code_example: Some("Replace: x * 8\nWith: x << 3".to_string()),
            });
        }

        suggestions
    }

    fn analyze_storage_patterns(&self, wasm_bytes: &[u8], measured_cost: Option<u64>) -> Vec<GasGolfingSuggestion> {
        let mut suggestions = Vec::new();

        // Look for repeated storage operations that could be batched
        let storage_ops = wasm_bytes
            .iter()
            .filter(|&&b| b == 0xFC || b == 0xFD)
            .count();
        if storage_ops > 15 {
            suggestions.push(GasGolfingSuggestion {
                pattern_type: "storage_batching".to_string(),
                description: format!("High storage operation count: {}", storage_ops),
                location: None,
                severity: "high".to_string(),
                gas_saved_estimate: measured_cost.map(|cost| cost.saturating_mul(15) / 100),
                suggested_fix: "Batch storage operations and minimize redundant reads/writes"
                    .to_string(),
                code_example: Some(
                    "Use a single storage update instead of multiple separate calls".to_string(),
                ),
            });
        }

        suggestions
    }

    fn analyze_branching_patterns(&self, wasm_bytes: &[u8], measured_cost: Option<u64>) -> Vec<GasGolfingSuggestion> {
        let mut suggestions = Vec::new();

        // Look for deeply nested conditionals
        let branch_count = wasm_bytes
            .iter()
            .filter(|&&b| b == 0x04 || b == 0x05)
            .count();
        if branch_count > 20 {
            suggestions.push(GasGolfingSuggestion {
                pattern_type: "branch_optimization".to_string(),
                description: format!("Complex branching detected: {} branches", branch_count),
                location: None,
                severity: "medium".to_string(),
                gas_saved_estimate: measured_cost.map(|cost| cost.saturating_mul(5) / 100),
                suggested_fix:
                    "Simplify conditional logic and consider lookup tables for complex decisions"
                        .to_string(),
                code_example: Some(
                    "Replace nested if-else with a lookup table or early returns".to_string(),
                ),
            });
        }

        suggestions
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gas_golfing_analyzer() {
        let analyzer = GasGolfingAnalyzer::new();

        // Simple WASM-like bytecode for testing
        let wasm_bytes = vec![
            0x02, 0x40, 0x03, 0x40, // block/loop pattern
            0x20, 0x00, 0x20, 0x00, // memory ops
            0x6D, 0x6E, 0x6D, // divisions
            0x41, 0x02, 0x6C, // multiply by 2
        ];

        let report = analyzer.analyze_wasm(&wasm_bytes, "test_contract");

        assert!(!report.suggestions.is_empty());
        assert_eq!(report.contract_name, "test_contract");
        assert!(report.total_suggestions > 0);
        assert_eq!(report.protocol_version, crate::simulation::LATEST_PROTOCOL_VERSION);
        assert!(report.suggestions.iter().all(|s| s.gas_saved_estimate.is_none()));
        assert_eq!(report.estimate_margin_of_error_percent, 100);
    }

    #[test]
    fn measured_cost_uses_the_selected_protocol_parameters() {
        let analyzer = GasGolfingAnalyzer::for_protocol_version(22).unwrap();
        let resources = SorobanResources {
            cpu_instructions: 10_000_000,
            ram_bytes: 4_096,
            ledger_read_bytes: 1_024,
            ledger_write_bytes: 1_024,
            ..Default::default()
        };
        let wasm_bytes = vec![0x02, 0x40, 0x03, 0x40];

        let report = analyzer.analyze_wasm_with_measurement(
            &wasm_bytes,
            "measured_contract",
            Some(&resources),
        );

        assert_eq!(report.protocol_version, 22);
        assert_eq!(report.measured_cost_stroops, Some(1_006));
        assert_eq!(report.estimate_margin_of_error_percent, 20);
        assert_eq!(report.suggestions[0].gas_saved_estimate, Some(50));
    }
}
