// core/src/engine/real_provider.rs
use super::traits::{LedgerEntryInfo, ProviderError, SimulationProvider, SimulationRpcResult};
use crate::rpc_provider::{ProviderRegistry, RpcProvider};
use crate::stellar_service::{StellarService, StellarServiceConfig, StellarServiceError};
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;

/// Converts a [`StellarServiceError`] into the engine-level [`ProviderError`].
fn map_err(e: StellarServiceError) -> ProviderError {
    match e {
        StellarServiceError::Timeout { .. } => ProviderError::NodeTimeout,
        StellarServiceError::Network { source, .. } => {
            ProviderError::NetworkError(source.to_string())
        }
        StellarServiceError::HttpError { status, url } => {
            ProviderError::RpcRequestFailed(format!("HTTP {status} from {url}"))
        }
        other => ProviderError::RpcRequestFailed(other.to_string()),
    }
}

pub struct RealRpcProvider {
    provider: RpcProvider,
    stellar_service: Arc<StellarService>,
}

impl RealRpcProvider {
    /// Construct a provider backed by a single RPC URL.
    ///
    /// A throwaway [`ProviderRegistry`] and [`StellarService`] are created
    /// internally.  When running inside a full application, prefer
    /// [`Self::with_registry`] so the process-wide pool and circuit-breaker
    /// scores are shared.
    pub fn new(rpc_url: String) -> Self {
        Self::with_timeout(rpc_url, Duration::from_secs(30))
    }

    /// Same as [`Self::new`] but with a custom per-call timeout.
    pub fn with_timeout(rpc_url: String, timeout: Duration) -> Self {
        let provider = RpcProvider {
            name: "default".to_string(),
            url: rpc_url.clone(),
            auth_header: None,
            auth_value: None,
            advertise: None,
        };
        let registry = ProviderRegistry::new(vec![provider.clone()]);
        let config = StellarServiceConfig::default().with_timeout(timeout);
        let stellar_service = Arc::new(StellarService::new(registry, config));
        Self {
            provider,
            stellar_service,
        }
    }

    /// Construct a provider that shares the process-wide registry and service.
    pub fn with_registry(
        provider: RpcProvider,
        registry: Arc<ProviderRegistry>,
        timeout: Duration,
    ) -> Self {
        let config = StellarServiceConfig::default().with_timeout(timeout);
        let stellar_service = Arc::new(StellarService::new(registry, config));
        Self {
            provider,
            stellar_service,
        }
    }
}

#[async_trait]
impl SimulationProvider for RealRpcProvider {
    async fn simulate_transaction(
        &self,
        transaction_xdr: &str,
    ) -> Result<SimulationRpcResult, ProviderError> {
        let raw = self
            .stellar_service
            .call_rpc(
                &self.provider,
                "simulateTransaction",
                serde_json::json!({ "transaction": transaction_xdr }),
            )
            .await
            .map_err(map_err)?;

        #[derive(serde::Deserialize)]
        struct RpcResponse {
            result: SimulateResult,
        }

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct SimulateResult {
            transaction_data: String,
            latest_ledger: u64,
            cost: Option<Cost>,
        }

        #[derive(serde::Deserialize)]
        struct Cost {
            cpu_insns: String,
            mem_bytes: String,
        }

        let rpc_response: RpcResponse = serde_json::from_value(raw)
            .map_err(|e| ProviderError::RpcRequestFailed(format!("Parse error: {e}")))?;

        Ok(SimulationRpcResult {
            transaction_data: rpc_response.result.transaction_data,
            latest_ledger: rpc_response.result.latest_ledger,
            cpu_insns: rpc_response
                .result
                .cost
                .as_ref()
                .and_then(|c| c.cpu_insns.parse().ok()),
            mem_bytes: rpc_response
                .result
                .cost
                .as_ref()
                .and_then(|c| c.mem_bytes.parse().ok()),
        })
    }

    async fn get_ledger_entries(
        &self,
        keys: &[String],
    ) -> Result<Vec<LedgerEntryInfo>, ProviderError> {
        let raw = self
            .stellar_service
            .call_rpc(
                &self.provider,
                "getLedgerEntries",
                serde_json::json!({ "keys": keys }),
            )
            .await
            .map_err(map_err)?;

        #[derive(serde::Deserialize)]
        struct RpcResponse {
            result: GetLedgerEntriesResult,
        }

        #[derive(serde::Deserialize)]
        struct GetLedgerEntriesResult {
            entries: Vec<LedgerEntry>,
        }

        #[derive(serde::Deserialize)]
        #[serde(rename_all = "camelCase")]
        struct LedgerEntry {
            key: String,
            live_until_ledger_seq: Option<u32>,
        }

        let rpc_response: RpcResponse = serde_json::from_value(raw)
            .map_err(|e| ProviderError::RpcRequestFailed(format!("Parse error: {e}")))?;

        Ok(rpc_response
            .result
            .entries
            .into_iter()
            .map(|entry| LedgerEntryInfo {
                key: entry.key,
                live_until_ledger_seq: entry.live_until_ledger_seq,
            })
            .collect())
    }
}
