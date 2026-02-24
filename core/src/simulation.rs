use crate::parser::ArgParser;
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use soroban_sdk::xdr::{
    Hash, HostFunction, InvokeContractArgs, InvokeHostFunctionOp, LedgerEntry, LedgerKey, Limits,
    Memo, MuxedAccount, Operation, OperationBody, Preconditions, ReadXdr, ScAddress, ScSymbol,
    ScVal, SequenceNumber, SorobanAuthorizationEntry, SorobanTransactionData, Transaction,
    TransactionExt, TransactionV1Envelope, Uint256, VecM, WriteXdr,
};
use stellar_strkey::Strkey;
use thiserror::Error;

use moka::future::Cache;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

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

    #[error("Parse error: {0}")]
    ParseError(#[from] crate::parser::ParserError),
}

/// Soroban resource consumption data
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Default)]
pub struct SorobanResources {
    pub cpu_instructions: u64,
    pub ram_bytes: u64,
    pub ledger_read_bytes: u64,
    pub ledger_write_bytes: u64,
    pub transaction_size_bytes: u64,
    pub footprint_size: u32,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SimulationRpcResult {
    #[serde(default)]
    transaction_data: String,
    #[serde(default)]
    latest_ledger: u64,
    #[serde(default)]
    cost: Option<ResourceCost>,
    #[serde(default)]
    #[allow(dead_code)]
    results: Vec<serde_json::Value>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ResourceCost {
    cpu_insns: String,
    mem_bytes: String,
}

pub struct SimulationEngine {
    rpc_url: String,
    client: Client,
    request_timeout: std::time::Duration,
}

impl SimulationEngine {
    pub fn new(rpc_url: String) -> Self {
        Self {
            rpc_url,
            client: Client::new(),
            request_timeout: std::time::Duration::from_secs(30),
        }
    }

    /// Simulate transaction from a deployed contract ID
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

    async fn simulate_transaction(
        &self,
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

        let response = tokio::time::timeout(
            self.request_timeout,
            self.client.post(&self.rpc_url).json(&request).send(),
        )
        .await
        .map_err(|_| SimulationError::NodeTimeout)?
        .map_err(|e| {
            if e.is_timeout() {
                SimulationError::NodeTimeout
            } else if e.is_connect() {
                SimulationError::NetworkError(e)
            } else {
                SimulationError::RpcRequestFailed(format!("Network error: {e}"))
            }
        })?;

        if !response.status().is_success() {
            return Err(SimulationError::RpcRequestFailed(format!(
                "HTTP error: {}",
                response.status()
            )));
        }

        let rpc_response: SimulateTransactionResponse = response.json().await.map_err(|e| {
            SimulationError::RpcRequestFailed(format!("Failed to parse response: {e}"))
        })?;

        match rpc_response.result {
            ResponseResult::Error { error } => {
                match error.code {
                    -32600 => Err(SimulationError::NodeError("Invalid request format".to_string())),
                    -32601 => Err(SimulationError::RpcRequestFailed("Method not found".to_string())),
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
            ResponseResult::Success { result } => self.parse_simulation_result(result),
        }
    }

    fn count_footprint_keys(&self, transaction_data: &str) -> u32 {
        if transaction_data.is_empty() {
            return 0;
        }
        let xdr_bytes = match BASE64.decode(transaction_data) {
            Ok(bytes) => bytes,
            Err(_) => return 0,
        };
        match SorobanTransactionData::from_xdr(&xdr_bytes, Limits::none()) {
            Ok(data) => {
                (data.resources.footprint.read_only.len() + data.resources.footprint.read_write.len())
                    as u32
            }
            Err(_) => 0,
        }
    }

    fn parse_simulation_result(
        &self,
        rpc_result: SimulationRpcResult,
    ) -> Result<SimulationResult, SimulationError> {
        let resources = if let Some(cost) = rpc_result.cost {
            let cpu_instructions = cost.cpu_insns.parse::<u64>().unwrap_or(0);
            let ram_bytes = cost.mem_bytes.parse::<u64>().unwrap_or(0);
            let (ledger_read_bytes, ledger_write_bytes) =
                self.extract_footprint_from_xdr(&rpc_result.transaction_data);
            SorobanResources {
                cpu_instructions,
                ram_bytes,
                ledger_read_bytes,
                ledger_write_bytes,
                transaction_size_bytes: rpc_result.transaction_data.len() as u64,
                footprint_size: self.count_footprint_keys(&rpc_result.transaction_data),
            }
        } else {
            SorobanResources::default()
        };

        let cost_stroops = self.calculate_cost(&resources);
        Ok(SimulationResult {
            resources,
            transaction_hash: None,
            latest_ledger: rpc_result.latest_ledger,
            cost_stroops,
            state_dependency: None,
        })
    }

    fn extract_footprint_from_xdr(&self, transaction_data: &str) -> (u64, u64) {
        if transaction_data.is_empty() {
            return (0, 0);
        }
        let xdr_bytes = match BASE64.decode(transaction_data) {
            Ok(bytes) => bytes,
            Err(_) => return (0, 0),
        };
        let soroban_data = match SorobanTransactionData::from_xdr(&xdr_bytes, Limits::none()) {
            Ok(data) => data,
            Err(_) => return (0, 0),
        };
        let footprint = &soroban_data.resources.footprint;
        let read_bytes = self.calculate_ledger_keys_size(&footprint.read_only);
        let write_bytes = self.calculate_ledger_keys_size(&footprint.read_write);
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
            .map_err(|e| SimulationError::XdrError(format!("Failed to encode XDR: {e}")))?;
        Ok(BASE64.encode(&xdr_bytes))
    }

    fn parse_contract_id(&self, contract_id: &str) -> Result<[u8; 32], SimulationError> {
        if !contract_id.starts_with('C') {
            return Err(SimulationError::NodeError(
                "Contract ID must start with 'C'".to_string(),
            ));
        }
        let strkey = Strkey::from_string(contract_id).map_err(|e| {
            SimulationError::NodeError(format!("Invalid contract ID format: {e}"))
        })?;
        match strkey {
            Strkey::Contract(contract) => Ok(contract.0),
            _ => Err(SimulationError::NodeError(
                "Expected contract address".to_string(),
            )),
        }
    }

    fn parse_sc_val_arg(&self, arg: &str) -> Result<ScVal, SimulationError> {
        let arg = arg.trim();
        if arg.starts_with('{') || arg.starts_with('[') {
            return Ok(ArgParser::parse(arg)?);
        }
        if arg == "true" {
            return Ok(ScVal::Bool(true));
        }
        if arg == "false" {
            return Ok(ScVal::Bool(false));
        }
        if arg == "void" || arg == "()" {
            return Ok(ScVal::Void);
        }
        if arg.starts_with('G')
            || arg.starts_with('C')
            || arg.starts_with(':')
            || arg.starts_with("0x")
        {
            if let Ok(val) = ArgParser::parse(&format!("\"{arg}\"")) {
                return Ok(val);
            }
        }
        if arg.starts_with('"') || arg.parse::<i64>().is_ok() || arg.parse::<u64>().is_ok() {
            if let Ok(val) = ArgParser::parse(arg) {
                return Ok(val);
            }
        }
        let symbol: ScSymbol = arg.try_into().map_err(|_| {
            SimulationError::NodeError(format!("Cannot parse argument: {arg}"))
        })?;
        Ok(ScVal::Symbol(symbol))
    }

    pub async fn simulate_locally(
        &self,
        contract_id: &str,
        function_name: &str,
        args: Vec<String>,
        overrides: HashMap<String, String>,
    ) -> Result<SimulationResult, SimulationError> {
        let mut state_dependency = Vec::new();
        for (key_64, val_64) in overrides.iter() {
            let key_bytes = BASE64.decode(key_64)?;
            let _key = LedgerKey::from_xdr(&key_bytes, Limits::none())
                .map_err(|e| SimulationError::XdrError(format!("Invalid ledger key: {e}")))?;
            let val_bytes = BASE64.decode(val_64)?;
            let entry = LedgerEntry::from_xdr(&val_bytes, Limits::none())
                .map_err(|e| SimulationError::XdrError(format!("Invalid ledger entry: {e}")))?;
            state_dependency.push(StateDependency {
                key: key_64.clone(),
                source: DataSource::Injected,
            });
        }
        let transaction_xdr = self.create_invoke_transaction(contract_id, function_name, args)?;
        let mut result = self.simulate_transaction(&transaction_xdr).await?;
        result.state_dependency = Some(state_dependency);
        Ok(result)
    }
}

const CACHE_TTL_SECS: u64 = 3_600;
const CACHE_MAX_CAPACITY: u64 = 1_000;

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
        let input = format!("{contract_id}{function_name}{args_json}");
        let digest = Sha256::digest(input.as_bytes());
        hex::encode(digest)
    }

    pub async fn get(&self, key: &str) -> Option<SimulationResult> {
        let value: Option<SimulationResult> = self.inner.get(key).await;
        if value.is_some() {
            self.hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.misses.fetch_add(1, Ordering::Relaxed);
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
        assert_eq!(resources.transaction_size_bytes, 0);
        assert_eq!(resources.footprint_size, 0);
    }

    #[test]
    fn test_soroban_resources_serialization() {
        let resources = SorobanResources {
            cpu_instructions: 1000000,
            ram_bytes: 2048,
            ledger_read_bytes: 512,
            ledger_write_bytes: 256,
            transaction_size_bytes: 1024,
            footprint_size: 5,
        };
        let json = serde_json::to_string(&resources).unwrap();
        assert!(json.contains("\"cpu_instructions\":1000000"));
        let deserialized: SorobanResources = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized, resources);
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
            footprint_size: 5,
        };
        assert!(engine.calculate_cost(&resources) > 0);
    }
}
