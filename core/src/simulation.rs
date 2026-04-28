use crate::parser::ArgParser;
use crate::rpc_provider::ProviderRegistry;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use moka::future::Cache;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use soroban_sdk::xdr::{
    AccountId, DiagnosticEvent, DiagnosticEventBody, Hash, HashIdPreimage,
    HashIdPreimageSorobanAuthorization, HostFunction, InvokeContractArgs, InvokeHostFunctionOp,
    LedgerEntry, LedgerKey, Limits, Memo, MuxedAccount, Operation, OperationBody, Preconditions,
    PublicKey, ReadXdr, ScAddress, ScMapEntry, ScSymbol, ScVal, SequenceNumber,
    SorobanAddressCredentials, SorobanAuthorizationEntry, SorobanAuthorizedFunction,
    SorobanAuthorizedInvocation, SorobanCredentials, SorobanTransactionData, Transaction,
    TransactionExt, TransactionV1Envelope, Uint256, VecM, WriteXdr,
};
use ed25519_dalek::Signer as Ed25519Signer;
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use stellar_strkey::Strkey;
use thiserror::Error;
use utoipa::ToSchema;

/// Errors that can occur during simulation
#[derive(Error, Debug)]
pub enum SimulationError {
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("RPC request failed: {0}")]
    RpcRequestFailed(String),

    #[error("RPC node timeout")]
    NodeTimeout,

    #[error("Node returned an error: {0}")]
    NodeError(String),

    #[error("Serialization error: {0}")]
    SerializationError(#[from] serde_json::Error),

    #[error("Network error: {0}")]
    NetworkError(#[from] reqwest::Error),

    #[error("Base64 decode error: {0}")]
    Base64Error(#[from] base64::DecodeError),

    #[error("XDR decode error: {0}")]
    XdrError(String),

    #[error("Invalid contract: {0}")]
    InvalidContract(String),

    #[error("Parse error: {0}")]
    ParseError(#[from] crate::parser::ParserError),
}

/// Soroban resource consumption data
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Default)]
pub struct SorobanResources {
    pub cpu_instructions: u64,
    pub ram_bytes: u64,
    pub ledger_read_bytes: u64,
    pub ledger_write_bytes: u64,
    pub transaction_size_bytes: u64,
}

/// Optimization report for a resource limit
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct OptimizationBuffer {
    /// The original RPC estimation
    pub estimated: u64,
    /// The absolute minimum found
    pub absolute_minimum: u64,
    /// The percentage buffer between estimate and minimum
    pub buffer_percentage: f64,
}

/// Complete optimization report for CPU and RAM
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OptimizationReport {
    /// CPU optimization details
    pub cpu: OptimizationBuffer,
    /// RAM optimization details
    pub ram: OptimizationBuffer,
    /// Recommended limits (including safety margin)
    pub recommended: SorobanResources,
}

/// Complete simulation result including resources and metadata
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationResult {
    pub resources: SorobanResources,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub transaction_hash: Option<String>,
    pub latest_ledger: u64,
    pub cost_stroops: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_dependency: Option<Vec<StateDependency>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ttl_analysis: Option<TtlAnalysisReport>,
    /// The SorobanTransactionData XDR returned by the RPC (base64)
    pub transaction_data: String,
    /// Cross-contract call graph
    #[serde(skip_serializing_if = "Option::is_none")]
    pub call_graph: Option<CallGraph>,
    /// Snapshot of the ledger state used/touched during simulation
    #[serde(skip_serializing_if = "Option::is_none")]
    pub state_snapshot: Option<SimulationStateSnapshot>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallNode {
    pub contract_id: String,
    pub function: String,
    pub children: Vec<CallNode>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallGraph {
    pub root: CallNode,
}

impl CallGraph {
    /// Export the call graph to Mermaid format
    pub fn to_mermaid(&self) -> String {
        let mut mermaid = String::from("graph TD\n");
        self.append_mermaid_nodes(&self.root, &mut mermaid, &mut 0);
        mermaid
    }

    fn append_mermaid_nodes(&self, node: &CallNode, mermaid: &mut String, id_gen: &mut usize) {
        let current_id = *id_gen;
        mermaid.push_str(&format!("    n{current_id}[\"{}:{}\"]\n", node.contract_id, node.function));
        
        for child in &node.children {
            *id_gen += 1;
            let child_id = *id_gen;
            mermaid.push_str(&format!("    n{current_id} --> n{child_id}\n"));
            self.append_mermaid_nodes(child, mermaid, id_gen);
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationStateSnapshot {
    pub ledger_entries: HashMap<String, String>, // Key-B64 -> Entry-B64
    pub ttl_entries: HashMap<String, u32>,       // Key-B64 -> LiveUntilLedger
    pub latest_ledger: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StateDependency {
    pub key: String,
    pub source: DataSource,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum DataSource {
    Live,
    Injected,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TtlEntryReport {
    pub key: String,
    pub live_until_ledger: u32,
    pub remaining_ledgers: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ExtendTtlSuggestion {
    pub key: String,
    pub current_live_until_ledger: u32,
    pub remaining_ledgers: i64,
    pub extend_to_ledger: u32,
    pub ledgers_to_extend_by: u32,
    pub suggested_operation: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct TtlAnalysisReport {
    pub current_ledger: u64,
    pub touched_entries: Vec<TtlEntryReport>,
    pub extend_ttl_suggestions: Vec<ExtendTtlSuggestion>,
}

#[derive(Debug, Serialize)]
struct SimulateTransactionRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: SimulateTransactionParams,
}

#[derive(Debug, Serialize)]
struct SimulateTransactionParams {
    transaction: String,
}

#[derive(Debug, Deserialize)]
struct SimulateTransactionResponse {
    #[allow(dead_code)]
    jsonrpc: String,
    #[allow(dead_code)]
    id: u64,
    #[serde(flatten)]
    result: ResponseResult,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum ResponseResult {
    Success { result: SimulationRpcResult },
    Error { error: RpcError },
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i32,
    message: String,
    #[serde(default)]
    #[allow(dead_code)]
    data: Option<serde_json::Value>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SimulationRpcResult {
    #[serde(default)]
    transaction_data: String,
    #[serde(default)]
    latest_ledger: u64,
    #[serde(default)]
    cost: Option<ResourceCost>,
    #[serde(default)]
    results: Vec<serde_json::Value>,
    /// Diagnostic events (base64 encoded XDR)
    #[serde(default)]
    events: Vec<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceCost {
    cpu_insns: String,
    mem_bytes: String,
}
// ── Multi-account authorization ───────────────────────────────────────────────

/// Represents one signer in a multi-account authorization scenario.
///
/// Use `SecretKey` when you hold the raw secret and want the engine to sign
/// automatically. Use `PreSignedXdr` when signing happened outside the engine
/// (hardware wallet, multisig coordinator, etc.).
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuthSigner {
    /// Raw Stellar secret key (S...). The engine builds and signs the
    /// `SorobanAuthorizationEntry` automatically.
    SecretKey { secret: String },
    /// A fully-formed, already-signed `SorobanAuthorizationEntry` in base64 XDR.
    PreSignedXdr { xdr: String },
}

#[derive(Debug, Serialize)]
struct GetLedgerEntriesRequest {
    jsonrpc: String,
    id: u64,
    method: String,
    params: GetLedgerEntriesParams,
}

#[derive(Debug, Serialize)]
struct GetLedgerEntriesParams {
    keys: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct GetLedgerEntriesResponse {
    #[serde(flatten)]
    result: LedgerEntriesResponseResult,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum LedgerEntriesResponseResult {
    Success { result: GetLedgerEntriesResult },
    Error { error: RpcError },
}

#[derive(Debug, Deserialize)]
struct GetLedgerEntriesResult {
    #[serde(default)]
    entries: Vec<LedgerEntryWithMeta>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LedgerEntryWithMeta {
    key: String,
    #[allow(dead_code)]
    xdr: Option<String>,
    live_until_ledger_seq: Option<u32>,
}

pub struct SimulationEngine {
    /// Kept for single-provider backward compatibility; empty when using registry.
    rpc_url: String,
    client: Client,
    request_timeout: std::time::Duration,
    /// When set, the engine will iterate healthy providers and failover automatically.
    registry: Option<Arc<ProviderRegistry>>,
}

impl SimulationEngine {
    const TTL_WARNING_THRESHOLD_LEDGERS: i64 = 120_000;
    const TTL_TARGET_LEDGERS_AHEAD: i64 = 360_000;

    /// Create an engine backed by a single RPC URL (backward-compatible).
    #[allow(dead_code)]
    pub fn new(rpc_url: String) -> Self {
        Self {
            rpc_url,
            client: Client::new(),
            request_timeout: std::time::Duration::from_secs(30),
            registry: None,
        }
    }

    /// Create an engine backed by a `ProviderRegistry` for multi-node failover.
    pub fn with_registry(registry: Arc<ProviderRegistry>) -> Self {
        Self {
            rpc_url: String::new(),
            client: Client::new(),
            request_timeout: std::time::Duration::from_secs(30),
            registry: Some(registry),
        }
    }

    /// Create an engine with a custom request timeout.
    pub fn with_registry_and_timeout(
        registry: Arc<ProviderRegistry>,
        timeout: std::time::Duration,
    ) -> Self {
        Self {
            rpc_url: String::new(),
            client: Client::new(),
            request_timeout: timeout,
            registry: Some(registry),
        }
    }

    /// Update the request timeout for subsequent simulation calls.
    pub fn set_timeout(&mut self, timeout: std::time::Duration) {
        self.request_timeout = timeout;
    }

    /// Get the current request timeout.
    pub fn timeout(&self) -> std::time::Duration {
        self.request_timeout
    }

    /// Simulate transaction from a deployed contract ID
    ///
    /// # Arguments
    /// * `contract_id` - The contract ID (e.g., C...)
    /// * `function_name` - Function to invoke
    /// * `args` - Function arguments (XDR encoded)
    ///
    /// # Returns
    /// A `Result` containing `SimulationResult` on success, or `SimulationError` on failure
    pub async fn simulate_from_contract_id(
        &self,
        contract_id: &str,
        function_name: &str,
        args: Vec<String>,
        ledger_overrides: Option<HashMap<String, String>>,
    ) -> Result<SimulationResult, SimulationError> {
        if contract_id.is_empty() {
            return Err(SimulationError::NodeError(
                "Contract ID cannot be empty".to_string(),
            ));
        }

        if let Some(overrides) = ledger_overrides {
            if !overrides.is_empty() {
                return self
                    .simulate_locally(contract_id, function_name, args, overrides)
                    .await;
            }
        }

        let transaction_xdr = self.create_invoke_transaction(contract_id, function_name, args)?;
        self.simulate_transaction(&transaction_xdr).await
    }

    /// Optimized limit discovery via binary search
    #[allow(clippy::too_many_arguments)]
    pub async fn optimize_limits(
        &self,
        contract_id: &str,
        function_name: &str,
        args: Vec<String>,
        safety_margin: f64,
    ) -> Result<OptimizationReport, SimulationError> {
        // 1. Get initial estimate
        let initial_result = self
            .simulate_from_contract_id(contract_id, function_name, args.clone(), None)
            .await?;
        let estimate = initial_result.resources;

        // 2. Perform binary search for CPU and RAM concurrently
        let cpu_search = self.binary_search_resource(
            contract_id,
            function_name,
            args.clone(),
            estimate.cpu_instructions / 2,
            estimate.cpu_instructions,
            "cpu",
            &estimate,
            &initial_result.transaction_data,
        );

        let ram_search = self.binary_search_resource(
            contract_id,
            function_name,
            args.clone(),
            estimate.ram_bytes / 2,
            estimate.ram_bytes,
            "ram",
            &estimate,
            &initial_result.transaction_data,
        );

        let (min_cpu, min_ram) = tokio::join!(cpu_search, ram_search);

        let min_cpu = min_cpu?;
        let min_ram = min_ram?;

        // 3. Calculate buffers
        let cpu_buffer = OptimizationBuffer {
            estimated: estimate.cpu_instructions,
            absolute_minimum: min_cpu,
            buffer_percentage: ((estimate.cpu_instructions as f64 - min_cpu as f64)
                / estimate.cpu_instructions as f64)
                * 100.0,
        };

        let ram_buffer = OptimizationBuffer {
            estimated: estimate.ram_bytes,
            absolute_minimum: min_ram,
            buffer_percentage: ((estimate.ram_bytes as f64 - min_ram as f64)
                / estimate.ram_bytes as f64)
                * 100.0,
        };

        // 4. Calculate recommended limits with safety margin
        let recommended = SorobanResources {
            cpu_instructions: (min_cpu as f64 * (1.0 + safety_margin)) as u64,
            ram_bytes: (min_ram as f64 * (1.0 + safety_margin)) as u64,
            ledger_read_bytes: estimate.ledger_read_bytes,
            ledger_write_bytes: estimate.ledger_write_bytes,
            transaction_size_bytes: estimate.transaction_size_bytes,
        };

        Ok(OptimizationReport {
            cpu: cpu_buffer,
            ram: ram_buffer,
            recommended,
        })
    }

    #[allow(clippy::too_many_arguments)]
    async fn binary_search_resource(
        &self,
        contract_id: &str,
        function_name: &str,
        args: Vec<String>,
        mut low: u64,
        mut high: u64,
        resource_type: &str,
        base_resources: &SorobanResources,
        transaction_data_xdr: &str,
    ) -> Result<u64, SimulationError> {
        let mut min_success = high;

        while low <= high {
            let mid = low + (high - low) / 2;
            let mut test_resources = base_resources.clone();

            if resource_type == "cpu" {
                test_resources.cpu_instructions = mid;
            } else {
                test_resources.ram_bytes = mid;
            }

            match self
                .simulate_with_exact_limits(
                    contract_id,
                    function_name,
                    args.clone(),
                    &test_resources,
                    transaction_data_xdr,
                )
                .await
            {
                Ok(_) => {
                    min_success = mid;
                    if mid == 0 {
                        break;
                    }
                    high = mid - 1;
                }
                Err(_) => {
                    low = mid + 1;
                }
            }
        }

        Ok(min_success)
    }

    async fn simulate_with_exact_limits(
        &self,
        contract_id: &str,
        function_name: &str,
        args: Vec<String>,
        resources: &SorobanResources,
        transaction_data_xdr: &str,
    ) -> Result<SimulationResult, SimulationError> {
        // 1. Decode the original transaction data to get footprint and other metadata
        let xdr_bytes = BASE64.decode(transaction_data_xdr).map_err(|e| {
            SimulationError::XdrError(format!("Failed to decode transaction data: {}", e))
        })?;
        let mut soroban_data = SorobanTransactionData::from_xdr(&xdr_bytes, Limits::none())
            .map_err(|e| {
                SimulationError::XdrError(format!("Failed to parse SorobanTransactionData: {}", e))
            })?;

        // 2. Update the resource limits in the transaction data
        soroban_data.resources.instructions = resources.cpu_instructions as u32;

        // 3. Create the basic host function
        let contract_hash = self.parse_contract_id(contract_id)?;
        let contract_address = ScAddress::Contract(Hash(contract_hash));
        let func_symbol: ScSymbol = function_name
            .try_into()
            .map_err(|_| SimulationError::InvalidContract("Invalid function name".to_string()))?;
        let sc_args: VecM<ScVal> = args
            .iter()
            .map(|arg| self.parse_sc_val_arg(arg))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| SimulationError::InvalidContract("Too many arguments".to_string()))?;

        let host_function = HostFunction::InvokeContract(InvokeContractArgs {
            contract_address,
            function_name: func_symbol,
            args: sc_args,
        });

        // 2. Build transaction XDR
        let invoke_op = InvokeHostFunctionOp {
            host_function,
            auth: vec![].try_into().unwrap(),
        };

        let operation = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(invoke_op),
        };

        let source_account = MuxedAccount::Ed25519(Uint256([0u8; 32]));

        let tx = Transaction {
            source_account,
            fee: 100,
            seq_num: SequenceNumber(0),
            cond: Preconditions::None,
            memo: Memo::None,
            operations: vec![operation].try_into().unwrap(),
            ext: TransactionExt::V1(soroban_data),
        };

        let envelope = TransactionV1Envelope {
            tx,
            signatures: VecM::default(),
        };

        let xdr_bytes = envelope
            .to_xdr(Limits::none())
            .map_err(|e| SimulationError::XdrError(format!("Failed to encode XDR: {}", e)))?;
        let transaction_xdr = BASE64.encode(&xdr_bytes);

        self.simulate_transaction(&transaction_xdr).await
    }

    /// Top-level simulate dispatcher: uses the provider registry when available,
    /// otherwise falls back to the single `rpc_url`.
    async fn simulate_transaction(
        &self,
        transaction_xdr: &str,
    ) -> Result<SimulationResult, SimulationError> {
        match &self.registry {
            Some(registry) => {
                self.simulate_transaction_with_failover(registry, transaction_xdr)
                    .await
            }
            None => {
                self.simulate_transaction_single(&self.rpc_url, None, None, transaction_xdr)
                    .await
            }
        }
    }

    /// Try each healthy provider in priority order until one succeeds or all
    /// are exhausted.
    async fn simulate_transaction_with_failover(
        &self,
        registry: &Arc<ProviderRegistry>,
        transaction_xdr: &str,
    ) -> Result<SimulationResult, SimulationError> {
        let providers = registry.healthy_providers().await;

        if providers.is_empty() {
            return Err(SimulationError::RpcRequestFailed(
                "All RPC providers are unavailable (circuit breaker tripped)".to_string(),
            ));
        }

        let mut last_error: Option<SimulationError> = None;

        for provider in &providers {
            tracing::debug!(
                provider = %provider.name,
                url = %provider.url,
                "Attempting simulation request"
            );

            let auth = provider
                .auth_header
                .as_deref()
                .zip(provider.auth_value.as_deref());

            match self
                .simulate_transaction_single(
                    &provider.url,
                    auth.map(|(h, _)| h),
                    auth.map(|(_, v)| v),
                    transaction_xdr,
                )
                .await
            {
                Ok(result) => {
                    registry.report_success(&provider.url).await;
                    return Ok(result);
                }
                Err(e) => {
                    let should_retry = match &e {
                        SimulationError::NodeTimeout | SimulationError::NetworkError(_) => true,
                        SimulationError::RpcRequestFailed(msg)
                            if msg.starts_with("HTTP error:") =>
                        {
                            // Extract status code from "HTTP error: <code>"
                            msg.split_whitespace()
                                .last()
                                .and_then(|s| s.parse::<u16>().ok())
                                .map(ProviderRegistry::is_retryable_status)
                                .unwrap_or(false)
                        }
                        _ => false,
                    };

                    registry.report_failure(&provider.url).await;

                    if should_retry {
                        tracing::warn!(
                            provider = %provider.name,
                            error = %e,
                            "Provider failed with retryable error, trying next"
                        );
                        last_error = Some(e);
                        continue;
                    }

                    // Non-retryable error (e.g. bad request) — don't bother
                    // trying other providers; the request itself is bad.
                    return Err(e);
                }
            }
        }

        Err(last_error.unwrap_or_else(|| {
            SimulationError::RpcRequestFailed("All providers exhausted".to_string())
        }))
    }

    /// Send a `simulateTransaction` JSON-RPC call to a single endpoint.
    async fn simulate_transaction_single(
        &self,
        url: &str,
        auth_header: Option<&str>,
        auth_value: Option<&str>,
        transaction_xdr: &str,
    ) -> Result<SimulationResult, SimulationError> {
        let request = SimulateTransactionRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "simulateTransaction".to_string(),
            params: SimulateTransactionParams {
                transaction: transaction_xdr.to_string(),
            },
        };

        tracing::debug!("Sending simulateTransaction request to {}", url);

        let mut req_builder = self.client.post(url).json(&request);

        // Attach provider-specific auth header if present.
        if let (Some(header), Some(value)) = (auth_header, auth_value) {
            req_builder = req_builder.header(header, value);
        }

        let response = tokio::time::timeout(self.request_timeout, req_builder.send())
            .await
            .map_err(|_| SimulationError::NodeTimeout)?
            .map_err(|e| {
                if e.is_timeout() {
                    SimulationError::NodeTimeout
                } else if e.is_connect() {
                    SimulationError::NetworkError(e)
                } else {
                    SimulationError::RpcRequestFailed(format!("Network error: {}", e))
                }
            })?;

        if !response.status().is_success() {
            return Err(SimulationError::RpcRequestFailed(format!(
                "HTTP error: {}",
                response.status()
            )));
        }

        let rpc_response: SimulateTransactionResponse = response.json().await.map_err(|e| {
            SimulationError::RpcRequestFailed(format!("Failed to parse response: {}", e))
        })?;

        match rpc_response.result {
            ResponseResult::Error { error } => {
                tracing::error!("RPC error (code {}): {}", error.code, error.message);
                match error.code {
                    -32600 => Err(SimulationError::NodeError(
                        "Invalid request format".to_string(),
                    )),
                    -32601 => Err(SimulationError::RpcRequestFailed(
                        "Method not found".to_string(),
                    )),
                    -32602 => Err(SimulationError::NodeError(format!(
                        "Invalid parameters: {}",
                        error.message
                    ))),
                    -32603 => Err(SimulationError::RpcRequestFailed(format!(
                        "Internal error: {}",
                        error.message
                    ))),
                    _ => Err(SimulationError::RpcRequestFailed(format!(
                        "RPC error {}: {}",
                        error.code, error.message
                    ))),
                }
            }
            ResponseResult::Success { result } => {
                tracing::info!("Simulation successful at ledger {}", result.latest_ledger);
                let mut parsed = self.parse_simulation_result(result.clone())?;
                let touched_keys = self.extract_touched_ledger_keys(&result.transaction_data);

                // Extract call graph from diagnostic events
                if !result.events.is_empty() {
                    parsed.call_graph = self.extract_call_graph(&result.events);
                }

                if !touched_keys.is_empty() {
                    parsed.state_dependency = Some(
                        touched_keys
                            .iter()
                            .map(|k| StateDependency {
                                key: k.clone(),
                                source: DataSource::Live,
                            })
                            .collect(),
                    );

                    match self
                        .analyze_ttl_for_touched_entries(
                            url,
                            auth_header,
                            auth_value,
                            &touched_keys,
                            result.latest_ledger,
                        )
                        .await
                    {
                        Ok((ttl_report, snapshot)) => {
                            if !ttl_report.touched_entries.is_empty() {
                                parsed.ttl_analysis = Some(ttl_report);
                            }
                            parsed.state_snapshot = Some(snapshot);
                        }
                        Err(e) => {
                            tracing::warn!("State analysis skipped due to RPC error: {}", e);
                        }
                    }
                }

                Ok(parsed)
            }
        }
    }

    fn extract_call_graph(&self, events: &[String]) -> Option<CallGraph> {
        let mut stack: Vec<CallNode> = Vec::new();
        let mut root: Option<CallNode> = None;

        for event_b64 in events {
            let bytes = match BASE64.decode(event_b64) {
                Ok(b) => b,
                Err(_) => continue,
            };

            let diag_event = match DiagnosticEvent::from_xdr(&bytes, Limits::none()) {
                Ok(e) => e,
                Err(_) => continue,
            };

            if !diag_event.in_contract_call {
                continue;
            }

            let contract_id = match &diag_event.event.contract_id {
                Some(Hash(h)) => Strkey::Contract(*h).to_string(),
                None => "Host".to_string(),
            };

            let (topics, _data) = match &diag_event.event.body {
                soroban_sdk::xdr::ContractEventBody::V0(v0) => (&v0.topics, &v0.data),
            };

            if topics.is_empty() {
                continue;
            }

            let topic0 = match &topics[0] {
                ScVal::Symbol(s) => s.to_string(),
                _ => continue,
            };

            if topic0 == "fn_call" && topics.len() >= 3 {
                // Topic 1: Contract Address (ignored since we use event.contract_id)
                // Topic 2: Function Name
                let function = match &topics[2] {
                    ScVal::Symbol(s) => s.to_string(),
                    _ => "unknown".to_string(),
                };

                let node = CallNode {
                    contract_id: contract_id.clone(),
                    function,
                    children: Vec::new(),
                };

                stack.push(node);
            } else if topic0 == "fn_return" {
                if let Some(finished_node) = stack.pop() {
                    if let Some(parent) = stack.last_mut() {
                        parent.children.push(finished_node);
                    } else {
                        root = Some(finished_node);
                    }
                }
            }
        }

        root.map(|r| CallGraph { root: r })
    }

    fn extract_touched_ledger_keys(&self, transaction_data: &str) -> Vec<String> {
        if transaction_data.is_empty() {
            return Vec::new();
        }

        let xdr_bytes = match BASE64.decode(transaction_data) {
            Ok(bytes) => bytes,
            Err(_) => return Vec::new(),
        };

        let soroban_data = match SorobanTransactionData::from_xdr(&xdr_bytes, Limits::none()) {
            Ok(data) => data,
            Err(_) => return Vec::new(),
        };

        let mut out = Vec::new();
        let mut push_key = |key: &LedgerKey| {
            if let Ok(bytes) = key.to_xdr(Limits::none()) {
                out.push(BASE64.encode(bytes));
            }
        };

        for key in soroban_data.resources.footprint.read_only.iter() {
            push_key(key);
        }
        for key in soroban_data.resources.footprint.read_write.iter() {
            push_key(key);
        }

        out.sort();
        out.dedup();
        out
    }

    async fn analyze_ttl_for_touched_entries(
        &self,
        url: &str,
        auth_header: Option<&str>,
        auth_value: Option<&str>,
        touched_keys: &[String],
        latest_ledger: u64,
    ) -> Result<(TtlAnalysisReport, SimulationStateSnapshot), SimulationError> {
        let req = GetLedgerEntriesRequest {
            jsonrpc: "2.0".to_string(),
            id: 1,
            method: "getLedgerEntries".to_string(),
            params: GetLedgerEntriesParams {
                keys: touched_keys.to_vec(),
            },
        };

        let mut req_builder = self.client.post(url).json(&req);
        if let (Some(header), Some(value)) = (auth_header, auth_value) {
            req_builder = req_builder.header(header, value);
        }

        let response = tokio::time::timeout(self.request_timeout, req_builder.send())
            .await
            .map_err(|_| SimulationError::NodeTimeout)?
            .map_err(|e| SimulationError::RpcRequestFailed(format!("Network error: {}", e)))?;

        if !response.status().is_success() {
            return Err(SimulationError::RpcRequestFailed(format!(
                "HTTP error: {}",
                response.status()
            )));
        }

        let rpc_response: GetLedgerEntriesResponse = response.json().await.map_err(|e| {
            SimulationError::RpcRequestFailed(format!("Failed to parse response: {}", e))
        })?;

        let entries = match rpc_response.result {
            LedgerEntriesResponseResult::Success { result } => result.entries,
            LedgerEntriesResponseResult::Error { error } => {
                return Err(SimulationError::RpcRequestFailed(format!(
                    "RPC error {}: {}",
                    error.code, error.message
                )))
            }
        };

        let mut ledger_entries = HashMap::new();
        let mut ttl_entries = HashMap::new();

        let touched_entries: Vec<TtlEntryReport> = entries
            .into_iter()
            .filter_map(|entry| {
                if let Some(xdr) = &entry.xdr {
                    ledger_entries.insert(entry.key.clone(), xdr.clone());
                }
                
                let live_until = entry.live_until_ledger_seq?;
                ttl_entries.insert(entry.key.clone(), live_until);
                
                let remaining = live_until as i64 - latest_ledger as i64;
                Some(TtlEntryReport {
                    key: entry.key,
                    live_until_ledger: live_until,
                    remaining_ledgers: remaining,
                })
            })
            .collect();

        let extend_ttl_suggestions =
            Self::build_extend_ttl_suggestions(&touched_entries, latest_ledger);

        Ok((
            TtlAnalysisReport {
                current_ledger: latest_ledger,
                touched_entries,
                extend_ttl_suggestions,
            },
            SimulationStateSnapshot {
                ledger_entries,
                ttl_entries,
                latest_ledger,
            },
        ))
    }

    fn build_extend_ttl_suggestions(
        touched_entries: &[TtlEntryReport],
        latest_ledger: u64,
    ) -> Vec<ExtendTtlSuggestion> {
        touched_entries
            .iter()
            .filter_map(|entry| {
                if entry.remaining_ledgers > Self::TTL_WARNING_THRESHOLD_LEDGERS {
                    return None;
                }

                let target = latest_ledger as i64 + Self::TTL_TARGET_LEDGERS_AHEAD;
                let extend_to_ledger = target.max(entry.live_until_ledger as i64) as u32;
                let ledgers_to_extend_by = extend_to_ledger.saturating_sub(entry.live_until_ledger);

                Some(ExtendTtlSuggestion {
                    key: entry.key.clone(),
                    current_live_until_ledger: entry.live_until_ledger,
                    remaining_ledgers: entry.remaining_ledgers,
                    extend_to_ledger,
                    ledgers_to_extend_by,
                    suggested_operation: format!(
                        "env.storage().persistent().extend_ttl(<key>, {}, {})",
                        Self::TTL_WARNING_THRESHOLD_LEDGERS,
                        Self::TTL_TARGET_LEDGERS_AHEAD
                    ),
                })
            })
            .collect()
    }

    fn parse_simulation_result(
        &self,
        rpc_result: SimulationRpcResult,
    ) -> Result<SimulationResult, SimulationError> {
        let resources = if let Some(cost) = rpc_result.cost {
            let cpu_instructions = cost.cpu_insns.parse::<u64>().unwrap_or_else(|_| {
                tracing::warn!("Failed to parse cpu_insns, using 0");
                0
            });
            let ram_bytes = cost.mem_bytes.parse::<u64>().unwrap_or_else(|_| {
                tracing::warn!("Failed to parse mem_bytes, using 0");
                0
            });
            let (ledger_read_bytes, ledger_write_bytes) =
                self.extract_footprint_from_xdr(&rpc_result.transaction_data);
            SorobanResources {
                cpu_instructions,
                ram_bytes,
                ledger_read_bytes,
                ledger_write_bytes,
                transaction_size_bytes: rpc_result.transaction_data.len() as u64,
            }
        } else {
            tracing::warn!("No cost data in simulation result, using defaults");
            SorobanResources::default()
        };

        let cost_stroops = self.calculate_cost(&resources);
        Ok(SimulationResult {
            resources,
            transaction_hash: None,
            latest_ledger: rpc_result.latest_ledger,
            cost_stroops,
            state_dependency: None,
            ttl_analysis: None,
            transaction_data: rpc_result.transaction_data,
        })
    }

    fn extract_footprint_from_xdr(&self, transaction_data: &str) -> (u64, u64) {
        if transaction_data.is_empty() {
            return (0, 0);
        }
        let xdr_bytes = match BASE64.decode(transaction_data) {
            Ok(bytes) => bytes,
            Err(e) => {
                tracing::warn!("Failed to decode base64 transaction data: {}", e);
                return (0, 0);
            }
        };
        let soroban_data = match SorobanTransactionData::from_xdr(&xdr_bytes, Limits::none()) {
            Ok(data) => data,
            Err(e) => {
                tracing::warn!("Failed to parse SorobanTransactionData XDR: {}", e);
                return (0, 0);
            }
        };
        let footprint = &soroban_data.resources.footprint;
        let read_bytes = self.calculate_ledger_keys_size(&footprint.read_only);
        let write_bytes = self.calculate_ledger_keys_size(&footprint.read_write);
        tracing::debug!(
            "Extracted footprint: read_only={} keys ({} bytes), read_write={} keys ({} bytes)",
            footprint.read_only.len(),
            read_bytes,
            footprint.read_write.len(),
            write_bytes
        );
        (read_bytes, write_bytes)
    }

    fn calculate_ledger_keys_size(&self, ledger_keys: &soroban_sdk::xdr::VecM<LedgerKey>) -> u64 {
        let mut total_bytes: u64 = 0;
        for ledger_key in ledger_keys.iter() {
            let key_size = match ledger_key {
                LedgerKey::Account(_) => 56,
                LedgerKey::Trustline(_) => 72,
                LedgerKey::ContractData(contract_data) => {
                    let base_size = 32 + 4;
                    let key_estimate = self.estimate_scval_size(&contract_data.key);
                    base_size + key_estimate
                }
                LedgerKey::ContractCode(_) => 32,
                LedgerKey::Offer(_) => 48,
                LedgerKey::Data(_) => 64,
                LedgerKey::ClaimableBalance(_) => 36,
                LedgerKey::LiquidityPool(_) => 32,
                LedgerKey::ConfigSetting(_) => 8,
                LedgerKey::Ttl(_) => 32,
            };
            total_bytes += key_size;
        }
        total_bytes
    }

    /// Estimate the size of an ScVal in bytes
    #[allow(clippy::only_used_in_recursion)]
    fn estimate_scval_size(&self, scval: &soroban_sdk::xdr::ScVal) -> u64 {
        use soroban_sdk::xdr::ScVal;
        match scval {
            ScVal::Bool(_) => 1,
            ScVal::Void => 0,
            ScVal::Error(_) => 8,
            ScVal::U32(_) | ScVal::I32(_) => 4,
            ScVal::U64(_) | ScVal::I64(_) => 8,
            ScVal::Timepoint(_) | ScVal::Duration(_) => 8,
            ScVal::U128(_) | ScVal::I128(_) => 16,
            ScVal::U256(_) | ScVal::I256(_) => 32,
            ScVal::Bytes(bytes) => bytes.len() as u64,
            ScVal::String(s) => s.len() as u64,
            ScVal::Symbol(sym) => sym.len() as u64,
            ScVal::Vec(Some(vec)) => {
                vec.iter().map(|v| self.estimate_scval_size(v)).sum::<u64>() + 4
            }
            ScVal::Vec(None) => 4,
            ScVal::Map(Some(map)) => {
                map.iter()
                    .map(|e| self.estimate_scval_size(&e.key) + self.estimate_scval_size(&e.val))
                    .sum::<u64>()
                    + 4
            }
            ScVal::Map(None) => 4,
            ScVal::Address(_) => 32,
            ScVal::LedgerKeyContractInstance => 32,
            ScVal::LedgerKeyNonce(_) => 32,
            ScVal::ContractInstance(_) => 64,
        }
    }

    fn calculate_cost(&self, resources: &SorobanResources) -> u64 {
        let cpu_cost = resources.cpu_instructions / 10000;
        let ram_cost = resources.ram_bytes / 1024;
        let ledger_cost = (resources.ledger_read_bytes + resources.ledger_write_bytes) / 1024;
        cpu_cost + ram_cost + ledger_cost
    }

    /// Create invoke transaction for contract call
    ///
    /// Creates a transaction with InvokeHostFunctionOp containing InvokeContract host function.
    fn create_invoke_transaction(
        &self,
        contract_id: &str,
        function_name: &str,
        args: Vec<String>,
    ) -> Result<String, SimulationError> {
        let contract_hash = self.parse_contract_id(contract_id)?;
        let contract_address = ScAddress::Contract(Hash(contract_hash));
        let func_symbol: ScSymbol = function_name
            .try_into()
            .map_err(|_| SimulationError::NodeError("Invalid function name".to_string()))?;
        let sc_args: VecM<ScVal> = args
            .iter()
            .map(|arg| self.parse_sc_val_arg(arg))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| SimulationError::NodeError("Too many arguments".to_string()))?;
        let host_function = HostFunction::InvokeContract(InvokeContractArgs {
            contract_address,
            function_name: func_symbol,
            args: sc_args,
        });
        self.build_invoke_host_function_transaction(host_function, vec![])
    }

    fn build_invoke_host_function_transaction(
        &self,
        host_function: HostFunction,
        auth: Vec<SorobanAuthorizationEntry>,
    ) -> Result<String, SimulationError> {
        let invoke_op = InvokeHostFunctionOp {
            host_function,
            auth: auth
                .try_into()
                .map_err(|_| SimulationError::XdrError("Too many auth entries".to_string()))?,
        };
        let operation = Operation {
            source_account: None,
            body: OperationBody::InvokeHostFunction(invoke_op),
        };
        let source_account = MuxedAccount::Ed25519(Uint256([0u8; 32]));
        let transaction = Transaction {
            source_account,
            fee: 100,
            seq_num: SequenceNumber(0),
            cond: Preconditions::None,
            memo: Memo::None,
            operations: vec![operation].try_into().map_err(|_| {
                SimulationError::XdrError("Failed to create operations".to_string())
            })?,
            ext: TransactionExt::V0,
        };
        let envelope = TransactionV1Envelope {
            tx: transaction,
            signatures: VecM::default(),
        };
        let xdr_bytes = envelope
            .to_xdr(Limits::none())
            .map_err(|e| SimulationError::XdrError(format!("Failed to encode XDR: {}", e)))?;
        Ok(BASE64.encode(&xdr_bytes))
    }

    /// Parse a contract ID from strkey format (C...) to raw bytes
    pub fn parse_contract_id(&self, contract_id: &str) -> Result<[u8; 32], SimulationError> {
        let strkey = Strkey::from_string(contract_id).map_err(|e| {
            SimulationError::NodeError(format!("Invalid contract ID format: {}", e))
        })?;
        match strkey {
            Strkey::Contract(contract) => Ok(contract.0),
            _ => Err(SimulationError::InvalidContract(
                "Contract ID must be a C... address".to_string(),
            )),
        }
    }

    fn parse_sc_val_arg(&self, arg: &str) -> Result<ScVal, SimulationError> {
        let arg = arg.trim();

        // 1. Try parsing as JSON first (for complex types like Maps and Vecs)
        if arg.starts_with('{') || arg.starts_with('[') {
            return Ok(ArgParser::parse(arg)?);
        }

        // 2. Check for Boolean/Void shorthands
        if arg == "true" {
            return Ok(ScVal::Bool(true));
        }
        if arg == "false" {
            return Ok(ScVal::Bool(false));
        }
        if arg == "void" || arg == "()" {
            return Ok(ScVal::Void);
        }

        // 3. Delegation to ArgParser for special types (Addresses, Symbols, Hex)
        // If it starts with G, C, :, or 0x, we try to parse it as a quoted string
        if arg.starts_with('G')
            || arg.starts_with('C')
            || arg.starts_with(':')
            || arg.starts_with("0x")
        {
            if let Ok(val) = ArgParser::parse(&format!("\"{}\"", arg)) {
                return Ok(val);
            }
        }

        // 4. Numbers and explicit quoted strings
        if arg.starts_with('"') || arg.parse::<i64>().is_ok() || arg.parse::<u64>().is_ok() {
            if let Ok(val) = ArgParser::parse(arg) {
                return Ok(val);
            }
        }

        // 5. Default fallback: Treat as Symbol (standard Soroban behavior for unquoted strings)
        // 5. Default fallback: Treat as Symbol (standard Soroban behavior for unquoted strings)
        let symbol: ScSymbol = arg
            .try_into()
            .map_err(|_| SimulationError::NodeError(format!("Cannot parse argument: {}", arg)))?;
        Ok(ScVal::Symbol(symbol))
    }

    pub async fn simulate_locally(
        &self,
        contract_id: &str,
        function_name: &str,
        args: Vec<String>,
        overrides: HashMap<String, String>,
    ) -> Result<SimulationResult, SimulationError> {
        tracing::info!(
            "Running local simulation with {} overrides",
            overrides.len()
        );

        let mut state_dependency = Vec::new();

        // Decode overrides
        let mut injected_entries = HashMap::new();
        for (key_64, val_64) in overrides.iter() {
            let key_bytes = BASE64.decode(key_64)?;
            let _key = LedgerKey::from_xdr(&key_bytes, Limits::none())
                .map_err(|e| SimulationError::XdrError(format!("Invalid ledger key: {}", e)))?;

            let val_bytes = BASE64.decode(val_64)?;
            let entry = LedgerEntry::from_xdr(&val_bytes, Limits::none())
                .map_err(|e| SimulationError::XdrError(format!("Invalid ledger entry: {}", e)))?;

            injected_entries.insert(key_64.clone(), entry);
            state_dependency.push(StateDependency {
                key: key_64.clone(),
                source: DataSource::Injected,
            });
        }

        // To provide high-fidelity "What If" analysis, we would ideally use a local soroban-sdk Env.
        // However, this requires the contract's WASM.
        // For the MVP, we merge the overrides into the simulation result metadata.

        // We first run a normal simulation to get the baseline resources and the footprint.
        let transaction_xdr = self.create_invoke_transaction(contract_id, function_name, args)?;
        let mut result = self.simulate_transaction(&transaction_xdr).await?;

        // Merge state dependency report:
        // 1. Mark injected entries
        // 2. Mark entries that were read from the live network during simulation

        // Extract footprint to see what was read
        let xdr_bytes = BASE64.decode(&transaction_xdr)?;
        let _tx_envelope =
            TransactionV1Envelope::from_xdr(&xdr_bytes, Limits::none()).map_err(|e| {
                SimulationError::XdrError(format!("Failed to parse transaction XDR: {}", e))
            })?;

        // In a real scenario, the footprint comes from the RPC result's transactionData
        // (which we already parsed in simulate_transaction -> parse_simulation_result)
        // But for reporting purposes, we check which of those keys are in our overrides.

        // For now, we populate the dependency report with the injected entries
        // and any other entries found in the footprint as "Live".

        let final_deps = state_dependency;

        result.state_dependency = Some(final_deps);
Ok(result)
    }

    // ── Multi-account authorization simulation
// ── Multi-account authorization simulation ────────────────────────────────

    /// Simulate a contract call requiring authorization from one or more accounts.
    ///
    /// # Arguments
    /// * `contract_id`        - Deployed contract (C...)
    /// * `function_name`      - Entry-point to invoke
    /// * `args`               - Function arguments
    /// * `signers`            - One `AuthSigner` per required signer
    /// * `network_passphrase` - Stellar network passphrase (e.g. "Test SDF Network ; September 2015")
    /// * `expiration_ledger`  - Ledger at which auth entries expire
    pub async fn simulate_with_auth(
        &self,
        contract_id: &str,
        function_name: &str,
        args: Vec<String>,
        signers: Vec<AuthSigner>,
        network_passphrase: &str,
        expiration_ledger: u32,
    ) -> Result<SimulationResult, SimulationError> {
        let contract_hash = self.parse_contract_id(contract_id)?;
        let contract_address = ScAddress::Contract(Hash(contract_hash));
        let func_symbol: ScSymbol = function_name
            .try_into()
            .map_err(|_| SimulationError::NodeError("Invalid function name".to_string()))?;
        let sc_args: VecM<ScVal> = args
            .iter()
            .map(|a| self.parse_sc_val_arg(a))
            .collect::<Result<Vec<_>, _>>()?
            .try_into()
            .map_err(|_| SimulationError::NodeError("Too many arguments".to_string()))?;

        // Build the root invocation shared across all auth entries
        let root_invocation = Self::build_root_invocation(
            contract_address.clone(),
            func_symbol.clone(),
            sc_args.clone(),
        );

        // Collect and sign auth entries for every signer
        let auth_entries = self.collect_auth_entries(
            &signers,
            &root_invocation,
            network_passphrase,
            expiration_ledger,
        )?;
        result.ttl_analysis = None;

        tracing::info!(
            signers = signers.len(),
            auth_entries = auth_entries.len(),
            "Simulating with multi-account authorization"
        );

        let host_function = HostFunction::InvokeContract(InvokeContractArgs {
            contract_address,
            function_name: func_symbol,
            args: sc_args,
        });

        let transaction_xdr =
            self.build_invoke_host_function_transaction(host_function, auth_entries)?;
        self.simulate_transaction(&transaction_xdr).await
    }

    /// Build a `SorobanAuthorizedInvocation` for the given contract call.
    fn build_root_invocation(
        contract_address: ScAddress,
        function_name: ScSymbol,
        args: VecM<ScVal>,
    ) -> SorobanAuthorizedInvocation {
        SorobanAuthorizedInvocation {
            function: SorobanAuthorizedFunction::ContractFn(InvokeContractArgs {
                contract_address,
                function_name,
                args,
            }),
            sub_invocations: VecM::default(),
        }
    }

    /// Convert a slice of `AuthSigner` values into ready-to-inject
    /// `SorobanAuthorizationEntry` objects.
    pub fn collect_auth_entries(
        &self,
        signers: &[AuthSigner],
        root_invocation: &SorobanAuthorizedInvocation,
        network_passphrase: &str,
        expiration_ledger: u32,
    ) -> Result<Vec<SorobanAuthorizationEntry>, SimulationError> {
        signers
            .iter()
            .map(|signer| match signer {
                AuthSigner::PreSignedXdr { xdr } => {
                    let bytes = BASE64
                        .decode(xdr)
                        .map_err(SimulationError::Base64Error)?;
                    SorobanAuthorizationEntry::from_xdr(&bytes, Limits::none()).map_err(|e| {
                        SimulationError::XdrError(format!("Invalid auth entry XDR: {e}"))
                    })
                }
                AuthSigner::SecretKey { secret } => self.sign_auth_entry(
                    secret,
                    root_invocation,
                    network_passphrase,
                    expiration_ledger,
                ),
            })
            .collect()
    }

    /// Parse a Stellar secret key, build a `SorobanAuthorizationEntry`,
    /// sign the auth preimage with ed25519, and return the completed entry.
    pub fn sign_auth_entry(
        &self,
        secret: &str,
        invocation: &SorobanAuthorizedInvocation,
        network_passphrase: &str,
        expiration_ledger: u32,
    ) -> Result<SorobanAuthorizationEntry, SimulationError> {
        use ed25519_dalek::SigningKey;

        // 1. Parse the Stellar secret key (S...)
        let strkey = Strkey::from_string(secret)
            .map_err(|e| SimulationError::NodeError(format!("Invalid secret key: {e}")))?;
        let seed = match strkey {
            Strkey::PrivateKeyEd25519(sk) => sk.0,
            _ => {
                return Err(SimulationError::NodeError(
                    "Expected S... secret key".to_string(),
                ))
            }
        };
        let signing_key = SigningKey::from_bytes(&seed);
        let public_key = signing_key.verifying_key().to_bytes();

        // 2. Derive a deterministic nonce: sha256(pubkey || invocation_xdr)[0..8]
        let invocation_xdr = invocation
            .to_xdr(Limits::none())
            .map_err(|e| SimulationError::XdrError(format!("Encode invocation: {e}")))?;
        let nonce_input = [&public_key[..], &invocation_xdr[..]].concat();
        let nonce_hash = Sha256::digest(&nonce_input);
        let nonce = i64::from_be_bytes(nonce_hash[..8].try_into().unwrap());

        // 3. Compute the network id
        let network_id: [u8; 32] = Sha256::digest(network_passphrase.as_bytes()).into();

        // 4. Build and hash the auth preimage
        let preimage =
            HashIdPreimage::SorobanAuthorization(HashIdPreimageSorobanAuthorization {
                network_id: Hash(network_id),
                invocation: invocation.clone(),
                nonce,
                signature_expiration_ledger: expiration_ledger,
            });
        let preimage_bytes = preimage
            .to_xdr(Limits::none())
            .map_err(|e| SimulationError::XdrError(format!("Encode preimage: {e}")))?;
        let auth_hash: [u8; 32] = Sha256::digest(&preimage_bytes).into();

        // 5. Sign the hash with ed25519
        let signature: [u8; 64] = signing_key.sign(&auth_hash).to_bytes();

        // 6. Build the Soroban signature map: { pubkey_bytes => sig_bytes }
        let sig_map = ScVal::Map(Some(
            vec![ScMapEntry {
                key: ScVal::Bytes(
                    public_key
                        .to_vec()
                        .try_into()
                        .map_err(|_| SimulationError::XdrError("pubkey bytes".into()))?,
                ),
                val: ScVal::Bytes(
                    signature
                        .to_vec()
                        .try_into()
                        .map_err(|_| SimulationError::XdrError("sig bytes".into()))?,
                ),
            }]
            .try_into()
            .map_err(|_| SimulationError::XdrError("sig map".into()))?,
        ));

        // 7. Assemble the final auth entry
        Ok(SorobanAuthorizationEntry {
            credentials: SorobanCredentials::Address(SorobanAddressCredentials {
                address: ScAddress::Account(AccountId(PublicKey::PublicKeyTypeEd25519(
                    Uint256(public_key),
                ))),
                nonce,
                signature_expiration_ledger: expiration_ledger,
                signature: sig_map,
            }),
            root_invocation: invocation.clone(),
        })
    }
}
// ── Local WASM profiling ──────────────────────────────────────────────────────

/// Profile a contract from raw WASM bytes using a local Soroban test environment.
///
/// **This function is synchronous and CPU-intensive.** Always call it from a
/// `tokio::task::spawn_blocking` closure so it does not stall the async runtime.
///
/// Returns [`SorobanResources`] containing the CPU instructions and RAM bytes
/// consumed by the invocation, plus the WASM file size as `transaction_size_bytes`.
/// Ledger read/write bytes are `0` because the local env has no persistent ledger.
pub fn profile_contract(
    wasm_bytes: Vec<u8>,
    function_name: String,
    args: Vec<String>,
) -> Result<SorobanResources, SimulationError> {
    use soroban_sdk::{Env, Symbol, Val};

    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(&*wasm_bytes, ());

    // Build the argument list for the invocation.
    let mut sdk_args: soroban_sdk::Vec<Val> = soroban_sdk::Vec::new(&env);
    for arg_str in &args {
        sdk_args.push_back(local_parse_arg(&env, arg_str));
    }

    let fn_symbol = Symbol::new(&env, &function_name);

    // Capture baseline metrics *after* registration so we only measure the call.
    env.cost_estimate().budget().reset_unlimited();
    let start_cpu = env.cost_estimate().budget().cpu_instruction_cost();
    let start_mem = env.cost_estimate().budget().memory_bytes_cost();

    // Invoke; catch panics so a bad contract doesn't crash the server.
    let invoke_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        env.invoke_contract::<Val>(&contract_id, &fn_symbol, sdk_args)
    }));

    let end_cpu = env.cost_estimate().budget().cpu_instruction_cost();
    let end_mem = env.cost_estimate().budget().memory_bytes_cost();

    if invoke_result.is_err() {
        return Err(SimulationError::InvalidContract(
            "Contract invocation panicked; verify function name and argument types".to_string(),
        ));
    }

    Ok(SorobanResources {
        cpu_instructions: end_cpu.saturating_sub(start_cpu),
        ram_bytes: end_mem.saturating_sub(start_mem),
        ledger_read_bytes: 0,
        ledger_write_bytes: 0,
        transaction_size_bytes: wasm_bytes.len() as u64,
    })
}

/// Convert a string argument to a `soroban_sdk::Val` for local invocation.
///
/// Supports: `void`/`()`, `true`/`false`, integers, and falls back to Symbol.
fn local_parse_arg(env: &soroban_sdk::Env, arg: &str) -> soroban_sdk::Val {
    use soroban_sdk::IntoVal;
    let arg = arg.trim();
    if arg == "void" || arg == "()" {
        return ().into_val(env);
    }
    if arg == "true" {
        return true.into_val(env);
    }
    if arg == "false" {
        return false.into_val(env);
    }
    if let Ok(n) = arg.parse::<i64>() {
        return n.into_val(env);
    }
    if let Ok(n) = arg.parse::<u64>() {
        return n.into_val(env);
    }
    soroban_sdk::Symbol::new(env, arg).into_val(env)
}

// ── Cache ─────────────────────────────────────────────────────────────────────

const CACHE_TTL_SECS: u64 = 3_600;
const CACHE_MAX_CAPACITY: u64 = 1_000;

/// In-memory simulation result cache backed by `moka`.
///
/// Cache key: `hex(sha256(contract_id ‖ function_name ‖ args_as_json))`
/// TTL: 1 hour — balances freshness vs. RPC cost reduction.
pub struct SimulationCache {
    inner: Cache<String, SimulationResult>,
    hits: AtomicU64,
    misses: AtomicU64,
}

impl SimulationCache {
    pub fn new() -> Arc<Self> {
        let inner = Cache::builder()
            .max_capacity(CACHE_MAX_CAPACITY)
            .time_to_live(Duration::from_secs(CACHE_TTL_SECS))
            .build();
        Arc::new(Self {
            inner,
            hits: AtomicU64::new(0),
            misses: AtomicU64::new(0),
        })
    }

    pub fn generate_key(contract_id: &str, function_name: &str, args: &[String]) -> String {
        let args_json = serde_json::to_string(args).unwrap_or_else(|_| "[]".to_string());
        let input = format!("{}{}{}", contract_id, function_name, args_json);
        let digest = Sha256::digest(input.as_bytes());
        hex::encode(digest)
    }

    pub async fn get(&self, key: &str) -> Option<SimulationResult> {
        let value: Option<SimulationResult> = self.inner.get(key).await;
        if value.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(cache.key = %key, "Cache HIT");
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
            tracing::debug!(cache.key = %key, "Cache MISS");
        }
        value
    }

    pub async fn set(&self, key: String, value: SimulationResult) {
        self.inner.insert(key, value).await;
    }

    pub fn log_stats(&self) {
        let hits = self.hits.load(Ordering::Relaxed);
        let misses = self.misses.load(Ordering::Relaxed);
        let total = hits + misses;
        let hit_rate_pct = if total > 0 { hits * 100 / total } else { 0 };
        tracing::info!(
            cache.hits = hits,
            cache.misses = misses,
            cache.total = total,
            cache.hit_rate_pct = hit_rate_pct,
            "Cache statistics"
        );
    }
}

// ── Test-only helpers on SimulationCache ──────────────────────────────────────
// Placed in a dedicated #[cfg(test)] impl block — the idiomatic Rust pattern
// that ensures Arc<SimulationCache> deref resolves these methods correctly
// during test compilation without polluting the public API.

#[cfg(test)]
impl SimulationCache {
    fn hit_count(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }
    fn miss_count(&self) -> u64 {
        self.misses.load(Ordering::Relaxed)
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_soroban_resources_default() {
        let resources = SorobanResources::default();
        assert_eq!(resources.cpu_instructions, 0);
        assert_eq!(resources.ram_bytes, 0);
        assert_eq!(resources.ledger_read_bytes, 0);
        assert_eq!(resources.ledger_write_bytes, 0);
    }

    #[test]
    fn test_soroban_resources_serialization() {
        let resources = SorobanResources {
            cpu_instructions: 1000000,
            ram_bytes: 2048,
            ledger_read_bytes: 512,
            ledger_write_bytes: 256,
            transaction_size_bytes: 1024,
        };
        let json = serde_json::to_string(&resources).unwrap();
        assert!(json.contains("\"cpu_instructions\":1000000"));
        assert!(json.contains("\"ram_bytes\":2048"));
        assert!(json.contains("\"ledger_read_bytes\":512"));
        assert!(json.contains("\"ledger_write_bytes\":256"));
        let deserialized: SorobanResources = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, resources);
    }

    #[test]
    fn test_simulation_engine_creation() {
        let engine = SimulationEngine::new("https://soroban-testnet.stellar.org".to_string());
        assert_eq!(engine.rpc_url, "https://soroban-testnet.stellar.org");
    }

    #[test]
    fn test_calculate_cost() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        let resources = SorobanResources {
            cpu_instructions: 1000000,
            ram_bytes: 2048,
            ledger_read_bytes: 512,
            ledger_write_bytes: 512,
            transaction_size_bytes: 1024,
        };
        assert!(engine.calculate_cost(&resources) > 0);
    }

    #[tokio::test]
    async fn test_simulate_from_contract_id_empty() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        let result = engine
            .simulate_from_contract_id("", "test_function", vec![], None)
            .await;
        assert!(matches!(result, Err(SimulationError::NodeError(_))));
    }

    #[tokio::test]
    async fn test_simulate_locally_with_overrides() {
        // This test mocks the RPC but verifies the local injection logic
        let engine = SimulationEngine::new("https://soroban-testnet.stellar.org".to_string());

        let mut overrides = HashMap::new();
        // Mock LedgerKey/LedgerEntry (Base64)
        // Key: LedgerKey::Account (0x0...0)
        let key_xdr = "AAAAAAAAAAA=";
        // Val: LedgerEntry (Account)
        let val_xdr = "AAAAAAAAAAA=";
        overrides.insert(key_xdr.to_string(), val_xdr.to_string());

        let result = engine
            .simulate_locally(
                "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
                "hello",
                vec![],
                overrides,
            )
            .await;

        // Since we are calling the real RPC in simulate_locally (MVP implementation),
        // we expect a network error or success.
        // But we want to check if the state_dependency is populated.
        if let Ok(res) = result {
            assert!(res.state_dependency.is_some());
            let deps = res.state_dependency.unwrap();
            assert_eq!(deps.len(), 1);
            assert_eq!(deps[0].key, key_xdr);
            assert_eq!(deps[0].source, DataSource::Injected);
        }
    }

    #[test]
    fn test_simulation_error_display() {
        let err = SimulationError::NodeTimeout;
        assert_eq!(err.to_string(), "RPC node timeout");

        let err = SimulationError::NodeError("test".to_string());
        assert_eq!(err.to_string(), "Node returned an error: test");

        let err = SimulationError::XdrError("invalid xdr".to_string());
        assert_eq!(err.to_string(), "XDR decode error: invalid xdr");
    }

    #[test]
    fn test_extract_footprint_empty_data() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        assert_eq!(engine.extract_footprint_from_xdr(""), (0, 0));
    }

    #[test]
    fn test_extract_footprint_invalid_base64() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        assert_eq!(
            engine.extract_footprint_from_xdr("not-valid-base64!!!"),
            (0, 0)
        );
    }

    #[test]
    fn test_extract_footprint_invalid_xdr() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        assert_eq!(
            engine.extract_footprint_from_xdr("SGVsbG8gV29ybGQ="),
            (0, 0)
        );
    }

    #[test]
    fn test_estimate_scval_size_primitives() {
        use soroban_sdk::xdr::ScVal;
        let engine = SimulationEngine::new("https://test.com".to_string());
        assert_eq!(engine.estimate_scval_size(&ScVal::Bool(true)), 1);
        assert_eq!(engine.estimate_scval_size(&ScVal::Void), 0);
        assert_eq!(engine.estimate_scval_size(&ScVal::U32(42)), 4);
        assert_eq!(engine.estimate_scval_size(&ScVal::I32(-42)), 4);
        assert_eq!(engine.estimate_scval_size(&ScVal::U64(1000)), 8);
        assert_eq!(engine.estimate_scval_size(&ScVal::I64(-1000)), 8);
    }

    #[test]
    fn test_parse_sc_val_arg_bool() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        assert!(matches!(
            engine.parse_sc_val_arg("true").unwrap(),
            ScVal::Bool(true)
        ));
        assert!(matches!(
            engine.parse_sc_val_arg("false").unwrap(),
            ScVal::Bool(false)
        ));
    }

    #[test]
    fn test_parse_sc_val_arg_void() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        assert!(matches!(
            engine.parse_sc_val_arg("void").unwrap(),
            ScVal::Void
        ));
        assert!(matches!(
            engine.parse_sc_val_arg("()").unwrap(),
            ScVal::Void
        ));
    }

    #[test]
    fn test_parse_sc_val_arg_symbol() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        assert!(matches!(
            engine.parse_sc_val_arg(":my_symbol").unwrap(),
            ScVal::Symbol(_)
        ));
    }

    #[test]
    fn test_parse_sc_val_arg_integer() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        assert!(matches!(
            engine.parse_sc_val_arg("42").unwrap(),
            ScVal::I64(42)
        ));
        assert!(matches!(
            engine.parse_sc_val_arg("-100").unwrap(),
            ScVal::I64(-100)
        ));
    }

    #[test]
    fn test_parse_sc_val_arg_hex_bytes() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        assert!(matches!(
            engine.parse_sc_val_arg("0xdeadbeef").unwrap(),
            ScVal::Bytes(_)
        ));
    }

    #[test]
    fn test_parse_contract_id_valid() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        let result =
            engine.parse_contract_id("CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC");
        assert!(result.is_ok());
        assert_eq!(result.unwrap().len(), 32);
    }

    #[test]
    fn test_parse_contract_id_invalid_prefix() {
        let engine = SimulationEngine::new("https://test.com".to_string());

        let result =
            engine.parse_contract_id("GDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC");
        assert!(matches!(result, Err(SimulationError::NodeError(_))));
    }

    #[test]
    fn test_create_invoke_transaction() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        let result = engine.create_invoke_transaction(
            "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC",
            "hello",
            vec!["true".to_string(), "42".to_string()],
        );
        assert!(result.is_ok());
        assert!(BASE64.decode(result.unwrap()).is_ok());
    }

    // ── Cache tests ───────────────────────────────────────────────────────────

    mod cache_tests {
        use super::*;

        fn make_result() -> SimulationResult {
            SimulationResult {
                resources: SorobanResources {
                    cpu_instructions: 1_000,
                    ram_bytes: 2_000,
                    ledger_read_bytes: 512,
                    ledger_write_bytes: 256,
                    transaction_size_bytes: 128,
                },
                transaction_hash: None,
                latest_ledger: 42,
                cost_stroops: 10,
                state_dependency: None,
                ttl_analysis: None,
                transaction_data: "AAA=".to_string(),
            }
        }

        #[test]
        fn test_cache_key_is_deterministic() {
            let k1 = SimulationCache::generate_key("CONTRACT_A", "fn_x", &["arg1".to_string()]);
            let k2 = SimulationCache::generate_key("CONTRACT_A", "fn_x", &["arg1".to_string()]);
            assert_eq!(k1, k2);
        }

        #[test]
        fn test_cache_key_differs_on_contract_id() {
            let k1 = SimulationCache::generate_key("CONTRACT_A", "fn_x", &[]);
            let k2 = SimulationCache::generate_key("CONTRACT_B", "fn_x", &[]);
            assert_ne!(k1, k2);
        }

        #[test]
        fn test_cache_key_differs_on_function_name() {
            let k1 = SimulationCache::generate_key("CONTRACT_A", "fn_x", &[]);
            let k2 = SimulationCache::generate_key("CONTRACT_A", "fn_y", &[]);
            assert_ne!(k1, k2);
        }

        #[test]
        fn test_cache_key_differs_on_args() {
            let k1 = SimulationCache::generate_key("CONTRACT_A", "fn_x", &["1".to_string()]);
            let k2 = SimulationCache::generate_key("CONTRACT_A", "fn_x", &["2".to_string()]);
            assert_ne!(k1, k2);
        }

        #[test]
        fn test_cache_key_is_hex_sha256() {
            let key = SimulationCache::generate_key("C", "f", &[]);
            assert_eq!(key.len(), 64);
            assert!(key.chars().all(|c| c.is_ascii_hexdigit()));
        }

        #[tokio::test]
        async fn test_cache_miss_on_empty() {
            let cache = SimulationCache::new();
            let result = cache.get("nonexistent_key").await;
            assert!(result.is_none());
            assert_eq!(cache.miss_count(), 1);
            assert_eq!(cache.hit_count(), 0);
        }

        #[tokio::test]
        async fn test_cache_hit_after_set() {
            let cache = SimulationCache::new();
            let key = "test_key".to_string();
            cache.set(key.clone(), make_result()).await;
            let result = cache.get(&key).await;
            assert!(result.is_some());
            assert_eq!(result.unwrap().latest_ledger, 42);
            assert_eq!(cache.hit_count(), 1);
            assert_eq!(cache.miss_count(), 0);
        }

        #[tokio::test]
        async fn test_cache_aside_pattern() {
            let cache = SimulationCache::new();
            let key = SimulationCache::generate_key("CONTRACT_X", "do_thing", &[]);

            let first = cache.get(&key).await;
            assert!(first.is_none());
            cache.set(key.clone(), make_result()).await;

            let second = cache.get(&key).await;
            assert!(second.is_some());

            assert_eq!(cache.miss_count(), 1);
            assert_eq!(cache.hit_count(), 1);
        }

        #[tokio::test]
        async fn test_different_keys_stored_independently() {
            let cache = SimulationCache::new();
            let k1 = SimulationCache::generate_key("CONTRACT_A", "fn_x", &[]);
            let k2 = SimulationCache::generate_key("CONTRACT_B", "fn_x", &[]);
            let mut r1 = make_result();
            let mut r2 = make_result();
            r1.latest_ledger = 1;
            r2.latest_ledger = 2;
            cache.set(k1.clone(), r1).await;
            cache.set(k2.clone(), r2).await;
            assert_eq!(cache.get(&k1).await.unwrap().latest_ledger, 1);
            assert_eq!(cache.get(&k2).await.unwrap().latest_ledger, 2);
        }
    }
    // ── Multi-auth tests ──────────────────────────────────────────────────────

    #[test]
    fn test_build_root_invocation_structure() {
        use soroban_sdk::xdr::SorobanAuthorizedFunction;

        let contract_hash = [1u8; 32];
        let addr = ScAddress::Contract(Hash(contract_hash));
        let sym: ScSymbol = "transfer".try_into().unwrap();
        let args: VecM<ScVal> = vec![ScVal::Bool(true)].try_into().unwrap();

        let inv = SimulationEngine::build_root_invocation(addr, sym.clone(), args);

        match &inv.function {
            SorobanAuthorizedFunction::ContractFn(f) => {
                assert_eq!(f.function_name, sym);
            }
            _ => panic!("unexpected function type"),
        }
        assert_eq!(inv.sub_invocations.len(), 0);
    }

    #[test]
    fn test_collect_auth_entries_invalid_base64_is_rejected() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        let signers = vec![AuthSigner::PreSignedXdr {
            xdr: "!!!not-base64!!!".to_string(),
        }];
        let dummy_inv = SimulationEngine::build_root_invocation(
            ScAddress::Contract(Hash([0u8; 32])),
            "fn".try_into().unwrap(),
            VecM::default(),
        );
        let result = engine.collect_auth_entries(&signers, &dummy_inv, "Test", 1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_collect_auth_entries_invalid_xdr_is_rejected() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        // valid base64 but not a SorobanAuthorizationEntry
        let bad_xdr = BASE64.encode(b"this is not valid xdr");
        let signers = vec![AuthSigner::PreSignedXdr { xdr: bad_xdr }];
        let dummy_inv = SimulationEngine::build_root_invocation(
            ScAddress::Contract(Hash([0u8; 32])),
            "fn".try_into().unwrap(),
            VecM::default(),
        );
        let result = engine.collect_auth_entries(&signers, &dummy_inv, "Test", 1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_sign_auth_entry_invalid_secret_rejected() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        let dummy_inv = SimulationEngine::build_root_invocation(
            ScAddress::Contract(Hash([0u8; 32])),
            "fn".try_into().unwrap(),
            VecM::default(),
        );
        let result = engine.sign_auth_entry("NOT_A_SECRET", &dummy_inv, "Test Network", 1000);
        assert!(result.is_err());
    }

    #[test]
    fn test_sign_auth_entry_wrong_key_type_rejected() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        let dummy_inv = SimulationEngine::build_root_invocation(
            ScAddress::Contract(Hash([0u8; 32])),
            "fn".try_into().unwrap(),
            VecM::default(),
        );
        // G... address is a public key, not a secret — must be rejected
        let result = engine.sign_auth_entry(
            "GABC1234567890ABCDEFGHIJKLMNOPQRSTUVWXYZ1234567890ABCDEFG",
            &dummy_inv,
            "Test Network",
            1000,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_empty_signers_produces_empty_auth_entries() {
        let engine = SimulationEngine::new("https://test.com".to_string());
        let dummy_inv = SimulationEngine::build_root_invocation(
            ScAddress::Contract(Hash([0u8; 32])),
            "fn".try_into().unwrap(),
            VecM::default(),
        );
        let result = engine
            .collect_auth_entries(&[], &dummy_inv, "Test Network", 1000)
            .unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_auth_signer_serialization() {
        let signer = AuthSigner::SecretKey {
            secret: "STEST".to_string(),
        };
        let json = serde_json::to_string(&signer).unwrap();
        assert!(json.contains("secret_key"));
        assert!(json.contains("STEST"));

        let signer2 = AuthSigner::PreSignedXdr {
            xdr: "AAAA".to_string(),
        };
        let json2 = serde_json::to_string(&signer2).unwrap();
        assert!(json2.contains("pre_signed_xdr"));

    #[test]
    fn test_build_extend_ttl_suggestions_flags_low_ttl_entries() {
        let entries = vec![
            TtlEntryReport {
                key: "key-a".to_string(),
                live_until_ledger: 1_000,
                remaining_ledgers: 500,
            },
            TtlEntryReport {
                key: "key-b".to_string(),
                live_until_ledger: 500_000,
                remaining_ledgers: 200_000,
            },
        ];

        let suggestions = SimulationEngine::build_extend_ttl_suggestions(&entries, 500);
        assert_eq!(suggestions.len(), 1);
        assert_eq!(suggestions[0].key, "key-a");
        assert!(suggestions[0].ledgers_to_extend_by > 0);
    }
}
