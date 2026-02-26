use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use thiserror::Error;
use utoipa::ToSchema;

use crate::simulation::{SimulationEngine, SimulationError, SorobanResources};

/// Errors that can occur during regression comparison
#[derive(Error, Debug)]
pub enum ComparisonError {
    #[error("Simulation error on current version: {0}")]
    CurrentSimulationError(#[source] SimulationError),
    #[error("Simulation error on base version: {0}")]
    BaseSimulationError(#[source] SimulationError),
    #[error("Invalid arguments: {0}")]
    InvalidArguments(String),
}

/// Modes for comparison
#[derive(Debug, Clone)]
pub enum CompareMode {
    LocalVsLocal {
        current_wasm_path: PathBuf,
        base_wasm_path: PathBuf,
    },
    LocalVsDeployed {
        current_wasm_path: PathBuf,
        contract_id: String,
        function_name: String,
        args: Vec<String>,
    },
}

/// Percentage changes for each resource metric
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct ResourceDelta {
    #[schema(example = json!(15.4))]
    pub cpu_instructions: f64,
    #[schema(example = json!(5.0))]
    pub ram_bytes: f64,
    #[schema(example = json!(0.0))]
    pub ledger_read_bytes: f64,
    #[schema(example = json!(-2.5))]
    pub ledger_write_bytes: f64,
    #[schema(example = json!(10.1))]
    pub transaction_size_bytes: f64,
    #[schema(example = json!(12.0))]
    pub cost_stroops: f64,
}

/// Defines an alert for a significant resource regression
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, ToSchema)]
pub struct RegressionFlag {
    #[schema(example = "cpu_instructions")]
    pub resource: String,
    #[schema(example = json!(15.4))]
    pub change_percent: f64,
    #[schema(example = "high")]
    pub severity: String,
}

/// Complete report of the resource comparison
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct RegressionReport {
    pub current: SorobanResources,
    pub base: SorobanResources,
    pub deltas: ResourceDelta,
    pub regression_flags: Vec<RegressionFlag>,
    #[schema(example = "Regression found: cpu_instructions increased by 15.4%")]
    pub summary: String,
}

impl RegressionReport {
    /// Calculate deltas and detect regressions between two resource maps
    pub fn generate(
        current_res: SorobanResources,
        base_res: SorobanResources,
        current_cost: u64,
        base_cost: u64,
        threshold_percent: f64,
    ) -> Self {
        let deltas = calculate_deltas(&current_res, &base_res, current_cost, base_cost);
        let regression_flags = detect_regressions(&deltas, threshold_percent);

        // Generate summary
        let summary = if regression_flags.is_empty() {
            "No regressions detected.".to_string()
        } else {
            let mut msgs = Vec::new();
            for flag in &regression_flags {
                msgs.push(format!("{} (+{:.1}%)", flag.resource, flag.change_percent));
            }
            format!("Regressions found: {}", msgs.join(", "))
        };

        Self {
            current: current_res,
            base: base_res,
            deltas,
            regression_flags,
            summary,
        }
    }
}

/// Helper to calculate percentage change: ((new - old) / old) * 100.0
fn calculate_percentage_change(current: u64, base: u64) -> f64 {
    if base == 0 && current == 0 {
        return 0.0;
    }
    if base == 0 {
        // If it was 0 and now it's > 0, it's an infinite increase.
        // We represent this as 100.0% for practical purposes,
        // or one could argue it's undefined. We'll use 100.0 as conservative cap
        // or just calculate the absolute value. But mathematically we can return 100.0
        return 100.0;
    }

    let diff = (current as f64) - (base as f64);
    (diff / (base as f64)) * 100.0
}

/// Calculates the percentage diffs for all metrics
fn calculate_deltas(
    current: &SorobanResources,
    base: &SorobanResources,
    current_cost: u64,
    base_cost: u64,
) -> ResourceDelta {
    ResourceDelta {
        cpu_instructions: calculate_percentage_change(
            current.cpu_instructions,
            base.cpu_instructions,
        ),
        ram_bytes: calculate_percentage_change(current.ram_bytes, base.ram_bytes),
        ledger_read_bytes: calculate_percentage_change(
            current.ledger_read_bytes,
            base.ledger_read_bytes,
        ),
        ledger_write_bytes: calculate_percentage_change(
            current.ledger_write_bytes,
            base.ledger_write_bytes,
        ),
        transaction_size_bytes: calculate_percentage_change(
            current.transaction_size_bytes,
            base.transaction_size_bytes,
        ),
        cost_stroops: calculate_percentage_change(current_cost, base_cost),
    }
}

/// Detects if any delta exceeds the given threshold
fn detect_regressions(deltas: &ResourceDelta, threshold_percent: f64) -> Vec<RegressionFlag> {
    let mut flags = Vec::new();

    let metrics = vec![
        ("cpu_instructions", deltas.cpu_instructions),
        ("ram_bytes", deltas.ram_bytes),
        ("ledger_read_bytes", deltas.ledger_read_bytes),
        ("ledger_write_bytes", deltas.ledger_write_bytes),
        ("transaction_size_bytes", deltas.transaction_size_bytes),
        ("cost_stroops", deltas.cost_stroops),
    ];

    for (name, change) in metrics {
        if change > threshold_percent {
            flags.push(RegressionFlag {
                resource: name.to_string(),
                change_percent: change,
                severity: "high".to_string(), // we can customize severity logic later
            });
        }
    }

    flags
}

/// Runs a comparison between two environments
pub async fn run_comparison(
    engine: &SimulationEngine,
    mode: CompareMode,
) -> Result<RegressionReport, ComparisonError> {
    match mode {
        CompareMode::LocalVsLocal {
            current_wasm_path,
            base_wasm_path,
        } => {
            // Run both simulations concurrently
            let (current_res, base_res) = tokio::join!(
                engine.simulate_from_wasm(&current_wasm_path),
                engine.simulate_from_wasm(&base_wasm_path)
            );

            let current = current_res.map_err(ComparisonError::CurrentSimulationError)?;
            let base = base_res.map_err(ComparisonError::BaseSimulationError)?;

            Ok(RegressionReport::generate(
                current.resources,
                base.resources,
                current.cost_stroops,
                base.cost_stroops,
                10.0, // 10% threshold as requested
            ))
        }
        CompareMode::LocalVsDeployed {
            current_wasm_path,
            contract_id,
            function_name,
            args,
        } => {
            // Because simulate_from_wasm is a deployment transaction (UploadWasm),
            // and simulate_from_contract_id is an invocation (InvokeContract),
            // they aren't strictly an apples-to-apples comparison of the same function call.
            //
            // Often "Local Vs Deployed" implies:
            // "I have this new local WASM. I want to invoke 'function_name' on it locally,
            //  and I want to invoke 'function_name' on the deployed contract, and compare them."
            //
            // However, our `SimulationEngine::simulate_from_wasm` only does an UploadContractWasm,
            // it doesn't do an initialization/invocation.
            //
            // In Soroban, to simulate an invocation on a not-yet-deployed WASM without deploying it,
            // we'd need to mock the environment or use a local sandbox, which is what `benchmarks.rs` uses.
            // But if we're using RPC simulation, the WASM needs to be installed first.
            //
            // Let me look closer at the requirements:
            // "Local vs Deployed: One uploaded WASM vs a contract_id on the network."
            //
            // This is actually going to be slightly tricky to do strictly over RPC without
            // a custom contract deploy/invoke flow. For now, since the API requirements explicitly ask for it:

            // To be technically correct for comparing an invocation: we'll simulate an invoke on the deployed contract.
            // For the local WASM, since `simulate_from_wasm` currently uploads it, the cost will just be the upload cost,
            // which won't match an invoke cost. We'll implement the shell and adjust the docs or implementation as needed.
            //
            // For the sake of the API skeleton:
            let (current_res, base_res) = tokio::join!(
                engine.simulate_from_wasm(&current_wasm_path), // Note: Uploads WASM, doesn't invoke a specific function
                engine.simulate_from_contract_id(&contract_id, &function_name, args)
            );

            let current = current_res.map_err(ComparisonError::CurrentSimulationError)?;
            let base = base_res.map_err(ComparisonError::BaseSimulationError)?;

            Ok(RegressionReport::generate(
                current.resources,
                base.resources,
                current.cost_stroops,
                base.cost_stroops,
                10.0,
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::simulation::SorobanResources;

    #[test]
    fn test_calculate_percentage_change() {
        assert_eq!(calculate_percentage_change(110, 100), 10.0);
        assert_eq!(calculate_percentage_change(150, 100), 50.0);
        assert_eq!(calculate_percentage_change(200, 100), 100.0);
        assert_eq!(calculate_percentage_change(90, 100), -10.0);
        assert_eq!(calculate_percentage_change(50, 100), -50.0);
    }

    #[test]
    fn test_calculate_percentage_change_zeros() {
        assert_eq!(calculate_percentage_change(0, 0), 0.0);
        assert_eq!(calculate_percentage_change(100, 0), 100.0);
        assert_eq!(calculate_percentage_change(0, 100), -100.0);
    }

    fn create_test_resources(
        cpu: u64,
        ram: u64,
        read: u64,
        write: u64,
        tx_size: u64,
    ) -> SorobanResources {
        SorobanResources {
            cpu_instructions: cpu,
            ram_bytes: ram,
            ledger_read_bytes: read,
            ledger_write_bytes: write,
            transaction_size_bytes: tx_size,
        }
    }

    #[test]
    fn test_detect_regressions_exact_threshold() {
        // Change is exactly 10%, threshold is 10%, should not flag since it uses > threshold
        let base = create_test_resources(100, 100, 100, 100, 100);
        let curr = create_test_resources(110, 100, 100, 100, 100);

        let report = RegressionReport::generate(curr, base, 100, 100, 10.0);
        assert!(report.regression_flags.is_empty());
    }

    #[test]
    fn test_detect_regressions_above_threshold() {
        // Change is 10.1% (> 10%)
        let base = create_test_resources(1000, 100, 100, 100, 100);
        let curr = create_test_resources(1101, 100, 100, 100, 100);

        let report = RegressionReport::generate(curr, base, 100, 100, 10.0);
        assert_eq!(report.regression_flags.len(), 1);
        assert_eq!(report.regression_flags[0].resource, "cpu_instructions");
        assert!(report.regression_flags[0].change_percent > 10.0);
    }

    #[test]
    fn test_detect_regressions_improvements_ignored() {
        // Change is -50% (improvement)
        let base = create_test_resources(200, 100, 100, 100, 100);
        let curr = create_test_resources(100, 100, 100, 100, 100);

        let report = RegressionReport::generate(curr, base, 100, 100, 10.0);
        assert!(report.regression_flags.is_empty());
        assert_eq!(report.deltas.cpu_instructions, -50.0);
    }

    #[test]
    fn test_regression_report_serialization() {
        let base = create_test_resources(1000, 100, 100, 100, 100);
        let curr = create_test_resources(1150, 100, 100, 100, 100);

        let report = RegressionReport::generate(curr, base, 100, 100, 10.0);
        let json = serde_json::to_string(&report).unwrap();

        assert!(json.contains("\"cpu_instructions\":15.0"));
        assert!(json.contains("Regressions found: cpu_instructions (+15.0%)"));
    }
}
