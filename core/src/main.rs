#![allow(dead_code)]
// Prevent regressions: production code must not use .unwrap() or .expect().
#![warn(clippy::unwrap_used, clippy::expect_used)]
#![cfg_attr(test, allow(clippy::unwrap_used, clippy::expect_used))]

mod agent_fleet;
mod agent_health;
mod agent_identity;
mod audit_log;
mod auth;
mod backpressure;
mod benchmarks;
mod billing_service;
mod cache;
mod comparison;
mod config;
mod db;
mod error_codes;
mod errors;
pub mod fee_analytics;
pub mod fee_collector;
pub mod fee_store;
mod failover;
mod gas_golfing;
mod middleware;
pub mod insights;
mod input_sanitization;
mod jobs;
mod logging;
mod merkle_tree;
mod metrics;
mod parser;
mod policy_expiry;
pub mod reconciliation;
mod reputation;
mod rounding;
mod routing;
pub mod rpc_provider;
mod runner;
mod secret_hash;
mod simulation;
mod simulation_service;
mod signed_receipt;
mod stellar_service;
mod two_phase_commit;
pub mod vault_store;
mod manager_store;
mod wasm_branch_analysis;
mod ws;

use crate::cache::{ContractCache, SimulationCache};
use crate::comparison::{CompareMode, RegressionFlag, RegressionReport, ResourceDelta};
use crate::errors::{AppError, Validate, ValidatedJson};
use crate::input_sanitization::{SanitizedJson, SanitizedQuery};
use axum::{
    extract::{Json, Multipart, State},
    http::{HeaderMap, HeaderName, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::{get, post},
    Extension, Router,
};
use ::config::{Config, ConfigError};
use prometheus::{Encoder, HistogramVec, IntCounterVec, Opts, Registry, TextEncoder};
use serde::{Deserialize, Serialize};
use simulation_service::{AnalysisResult, SimulationMetric, SimulationService};
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
// CLI Argument Handling
use crate::fee_analytics::{FeeAnalyticsEngine, MarketConditions, ModelBreakdown};
use crate::fee_collector::{FeeCollector, FeeCollectorConfig};
use crate::fee_store::FeeStore;
use crate::gas_golfing::{GasGolfingAnalyzer, GasGolfingReport};
use crate::insights::InsightsEngine;
use crate::jobs::{JobQueue, JobQueueConfig, JobWorker};
use crate::rpc_provider::{ProviderRegistry, RegistryConfig, RegistrySnapshot, RpcProvider};
use crate::runner::RequestCancellation;
use crate::simulation::{SimulationEngine, SimulationMode, SimulationResult, SorobanResources};
use crate::signed_receipt::ReceiptSigner;
use crate::logging::{LogLevelSnapshot, LogLevelUpdate, RuntimeLogController};
use crate::stellar_service::{StellarService, StellarServiceConfig};
use crate::ws::SimulationBus;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{
    filter::LevelFilter,
    layer::SubscriberExt,
    reload,
    util::SubscriberInitExt,
    EnvFilter,
};
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AppConfig {
    /// Deployment environment. Set to `"production"` to enable
    /// production-safe error redaction (strips internal details from HTTP
    /// responses). Any other value — including absent — is treated as
    /// non-production so that misconfigured deployments fail safe.
    #[serde(default)]
    app_env: String,
    /// Port for the HTTP server
    server_port: u16,
    /// Rust log level (e.g., "info", "debug")
    rust_log: String,
    /// Primary RPC URL — used as a single-provider fallback when
    /// `RPC_PROVIDERS` is not set.
    soroban_rpc_url: String,
    /// Optional RSA Private Key PEM for RS256 JWTs. If missing, a dev key is generated.
    jwt_private_key: Option<String>,
    /// Stellar network passphrase
    network_passphrase: String,
    /// Redis URL reserved for the distributed cache migration (issue #65).
    /// Unused in the MVP in-memory implementation — present so the config
    /// surface is stable when Redis is wired in.
    redis_url: String,
    /// JSON-encoded array of RPC provider objects.  Example:
    /// ```json
    /// [
    ///   {"name":"stellar-testnet","url":"https://soroban-testnet.stellar.org"},
    ///   {"name":"blockdaemon","url":"https://soroban.blockdaemon.com","auth_header":"X-API-Key","auth_value":"KEY"}
    /// ]
    /// ```
    /// When empty or absent the engine falls back to `soroban_rpc_url`.
    #[serde(default)]
    rpc_providers: String,
    /// Stable node identifier used for gossip snapshots.
    #[serde(default)]
    registry_instance_id: String,
    /// Public base URL announced to peers, e.g. `https://api-a.example.com`.
    #[serde(default)]
    registry_public_url: String,
    /// Seed peers as a JSON array or comma-separated list of base URLs.
    #[serde(default)]
    registry_seed_peers: String,
    /// Health-check interval in seconds (default 30).
    #[serde(default = "default_health_check_interval")]
    health_check_interval_secs: u64,
    /// Gossip sync interval in seconds (default 30).
    #[serde(default = "default_gossip_interval_secs")]
    gossip_interval_secs: u64,
    /// Simulation timeout in seconds (default 30).
    #[serde(default = "default_simulation_timeout_secs")]
    simulation_timeout_secs: u64,
    /// Simulation execution mode: `failover` or `consensus`.
    #[serde(default = "default_simulation_mode")]
    simulation_mode: String,
    /// Database URL for job queue (PostgreSQL or SQLite)
    #[serde(default = "default_database_url")]
    database_url: String,
    /// Job timeout in seconds (default 300).
    #[serde(default = "default_job_timeout_secs")]
    job_timeout_secs: u64,
    /// Max concurrent jobs (default 10).
    #[serde(default = "default_max_concurrent_jobs")]
    max_concurrent_jobs: usize,
    /// Fee data collection interval in seconds (default 5).
    #[serde(default = "default_fee_collection_interval")]
    fee_collection_interval_secs: u64,
    /// Fee data retention period in days (default 30).
    #[serde(default = "default_fee_retention_days")]
    fee_retention_days: u32,
    /// Enable fee market analysis (default true).
    #[serde(default = "default_fee_analysis_enabled")]
    fee_analysis_enabled: bool,
    /// Emergency pause for message verification (default false).
    /// When true, all verification endpoints return an error.
    #[serde(default = "default_emergency_verification_paused")]
    emergency_verification_paused: bool,
    /// Filesystem path that backs the disk-persistent L2 cache. When
    /// empty the L2 tier is disabled and the service runs L1-only (same
    /// behaviour as before #104).
    #[serde(default = "default_disk_cache_path")]
    disk_cache_path: String,
    /// Number of ledgers a cached entry may lag the current ledger before
    /// L2 treats it as stale. Default 100 ≈ 8 minutes at 5 s/ledger.
    #[serde(default = "default_max_ledger_age")]
    max_ledger_age: u32,
    /// Comma-separated list of allowed CORS origins for white-label partner
    /// frontends.  Examples:
    ///   `https://partner-a.example.com,https://partner-b.example.com`
    ///
    /// Leave empty (the default) to allow **all** origins — suitable for local
    /// development only.  In production this **must** be set to the explicit
    /// list of trusted white-label domains; a wildcard in production would
    /// allow any web page to call the API with user credentials.
    #[serde(default)]
    cors_allowed_origins: String,
}

fn default_health_check_interval() -> u64 {
    30
}

fn default_simulation_timeout_secs() -> u64 {
    30
}

fn default_simulation_mode() -> String {
    "failover".to_string()
}

fn default_gossip_interval_secs() -> u64 {
    30
}

fn default_database_url() -> String {
    "sqlite://Perigee.db".to_string()
}

fn default_job_timeout_secs() -> u64 {
    300
}

fn default_max_concurrent_jobs() -> usize {
    10
}

fn default_fee_collection_interval() -> u64 {
    5
}

fn default_fee_retention_days() -> u32 {
    30
}

fn default_fee_analysis_enabled() -> bool {
    true
}

fn default_emergency_verification_paused() -> bool {
    false
}
fn default_disk_cache_path() -> String {
    // Empty == L2 disabled. Operators who want persistence set this in
    // env / config.toml explicitly; we don't create a hidden directory
    // in the CWD by default.
    String::new()
}

fn default_max_ledger_age() -> u32 {
    100
}

/// Build a [`CorsLayer`] from the `cors_allowed_origins` config value.
///
/// Behaviour:
/// * Empty string → `allow_origin(Any)` — permissive, for local dev only.
/// * Non-empty string → parse as a comma-separated list of exact origins
///   (`scheme://host[:port]`) and allow only those.  Invalid entries are
///   logged and skipped.  If every entry is invalid the layer falls back to
///   denying all cross-origin requests (empty list).
///
/// The layer also allows the standard headers and methods needed by the API.
fn build_cors_layer(cors_allowed_origins: &str) -> CorsLayer {
    use axum::http::{header, HeaderName, Method};

    let base = CorsLayer::new()
        .allow_headers([
            header::AUTHORIZATION,
            header::CONTENT_TYPE,
            header::ACCEPT,
        ])
        .allow_methods([
            Method::GET,
            Method::POST,
            Method::PUT,
            Method::PATCH,
            Method::DELETE,
            Method::OPTIONS,
        ])
        .expose_headers([
            HeaderName::from_static("x-perigee-receipt"),
            HeaderName::from_static("x-perigee-receipt-id"),
        ]);

    let trimmed = cors_allowed_origins.trim();
    if trimmed.is_empty() {
        // Local dev: allow all origins.
        tracing::warn!(
            "CORS_ALLOWED_ORIGINS is not set — allowing all origins (Any). \
             Set CORS_ALLOWED_ORIGINS in production."
        );
        return base.allow_origin(Any);
    }

    let origins: Vec<axum::http::HeaderValue> = trimmed
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .filter_map(|origin| {
            match origin.parse::<axum::http::HeaderValue>() {
                Ok(v) => {
                    tracing::info!(origin, "CORS: allowing origin");
                    Some(v)
                }
                Err(e) => {
                    tracing::warn!(origin, error = %e, "CORS: skipping invalid origin");
                    None
                }
            }
        })
        .collect();

    base.allow_origin(AllowOrigin::list(origins))
}

/// Validate RPC / relayer / network secret URLs at startup so a malformed
/// value fails fast and loudly rather than silently at first request.
///
/// Returns `Err(message)` describing exactly which environment variable is
/// problematic so an operator can fix it without spelunking through logs.
/// Issue #85 / NF-03 — "Secrets for RPC/relayer loaded from env without validation".
fn validate_config_secrets(config: &AppConfig) -> Result<(), String> {
    // 1. Primary RPC URL (always required, has a default from `load_config`).
    let rpc = config.soroban_rpc_url.trim();
    if rpc.is_empty() {
        return Err("SOROBAN_RPC_URL is empty".to_string());
    }
    if reqwest::Url::parse(rpc).is_err() {
        return Err(format!(
            "SOROBAN_RPC_URL is not a valid URL: '{}' \
             (must start with http:// or https://)",
            rpc
        ));
    }

    // 2. Stellar network passphrase — never empty.
    if config.network_passphrase.trim().is_empty() {
        return Err("NETWORK_PASSPHRASE must not be empty".to_string());
    }

    // 3. RPC_PROVIDERS — if set, must be valid JSON with valid URLs.
    if !config.rpc_providers.trim().is_empty() {
        let providers: Vec<RpcProvider> =
            serde_json::from_str(&config.rpc_providers).map_err(|e| {
                format!(
                    "RPC_PROVIDERS is not valid JSON: {} \
                     — expected a JSON array of {{name,url,auth_header?,auth_value?}} objects",
                    e
                )
            })?;
        for (idx, p) in providers.iter().enumerate() {
            if p.name.trim().is_empty() {
                return Err(format!(
                    "RPC_PROVIDERS[{}].name must not be empty",
                    idx
                ));
            }
            if reqwest::Url::parse(&p.url).is_err() {
                return Err(format!(
                    "RPC_PROVIDERS[{}] ('{}') has invalid URL: '{}'",
                    idx, p.name, p.url
                ));
            }
        }
    }

    // 4. REGISTRY_PUBLIC_URL — optional but, if set, must be a valid URL.
    if !config.registry_public_url.trim().is_empty()
        && reqwest::Url::parse(&config.registry_public_url).is_err() {
            return Err(format!(
                "REGISTRY_PUBLIC_URL is not a valid URL: '{}'",
                config.registry_public_url
            ));
        }

    // 5. REGISTRY_SEED_PEERS — every URL must be parseable.
    for peer in parse_seed_peers(&config.registry_seed_peers) {
        if reqwest::Url::parse(&peer).is_err() {
            return Err(format!(
                "REGISTRY_SEED_PEERS contains an invalid peer URL: '{}'",
                peer
            ));
        }
    }

    Ok(())
}

fn load_config() -> Result<AppConfig, ConfigError> {
    dotenvy::dotenv().ok();

    let settings = Config::builder()
        .add_source(::config::Environment::default())
        .set_default("server_port", 8080)?
        .set_default("rust_log", "info")?
        .set_default("soroban_rpc_url", "https://soroban-testnet.stellar.org")?
        .set_default("network_passphrase", "Test SDF Network ; September 2015")?
        .set_default("redis_url", "redis://127.0.0.1:6379")?
        .set_default("rpc_providers", "")?
        .set_default("registry_instance_id", "")?
        .set_default("registry_public_url", "")?
        .set_default("registry_seed_peers", "")?
        .set_default("health_check_interval_secs", 30)?
        .set_default("gossip_interval_secs", 30)?
        .set_default("simulation_timeout_secs", 30)?
        .set_default("simulation_mode", "failover")?
        .set_default("database_url", "sqlite://Perigee.db")?
        .set_default("job_timeout_secs", 300)?
        .set_default("max_concurrent_jobs", 10)?
        .set_default("fee_collection_interval_secs", 5)?
        .set_default("fee_retention_days", 30)?
        .set_default("fee_analysis_enabled", true)?
        .set_default("emergency_verification_paused", false)?
        .set_default("disk_cache_path", "")?
        .set_default("max_ledger_age", 100)?
        .set_default("cors_allowed_origins", "")?
        .build()?;

    settings.try_deserialize()
}

/// Parse the `RPC_PROVIDERS` env var (JSON array) or fall back to wrapping the
/// single `SOROBAN_RPC_URL` into a one-element provider list.
fn build_providers(config: &AppConfig) -> Vec<RpcProvider> {
    if !config.rpc_providers.is_empty() {
        match serde_json::from_str::<Vec<RpcProvider>>(&config.rpc_providers) {
            Ok(providers) if !providers.is_empty() => {
                tracing::info!(
                    count = providers.len(),
                    "Loaded RPC providers from RPC_PROVIDERS"
                );
                return providers;
            }
            Ok(_) => {
                tracing::warn!("RPC_PROVIDERS is empty array, falling back to SOROBAN_RPC_URL");
            }
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "Failed to parse RPC_PROVIDERS, falling back to SOROBAN_RPC_URL"
                );
            }
        }
    }

    vec![RpcProvider {
        name: "default".to_string(),
        url: config.soroban_rpc_url.clone(),
        auth_header: None,
        auth_value: None,
        advertise: None,
    }]
}

fn parse_seed_peers(raw: &str) -> Vec<String> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Vec::new();
    }

    if trimmed.starts_with('[') {
        return serde_json::from_str::<Vec<String>>(trimmed).unwrap_or_default();
    }

    trimmed
        .split(',')
        .map(|peer| peer.trim().trim_end_matches('/').to_string())
        .filter(|peer| !peer.is_empty())
        .collect()
}

fn build_registry_config(config: &AppConfig) -> RegistryConfig {
    let instance_id = if config.registry_instance_id.trim().is_empty() {
        uuid::Uuid::new_v4().to_string()
    } else {
        config.registry_instance_id.trim().to_string()
    };

    let public_base_url = if config.registry_public_url.trim().is_empty() {
        Some(format!("http://127.0.0.1:{}", config.server_port))
    } else {
        Some(
            config
                .registry_public_url
                .trim()
                .trim_end_matches('/')
                .to_string(),
        )
    };

    RegistryConfig {
        instance_id,
        public_base_url,
        seed_peers: parse_seed_peers(&config.registry_seed_peers),
    }
}

/// Shared application state injected into every Axum handler via [`State`].
pub struct AppState {
    engine: SimulationEngine,
    provider_registry: Arc<ProviderRegistry>,
    /// Process-wide Stellar RPC transport (pooled client, retry, circuit-breaker).
    stellar_service: Arc<StellarService>,
    cache: Arc<SimulationCache>,
    insights_cache: Arc<crate::cache::InsightsCache>,
    insights_engine: InsightsEngine,
    gas_golfing_analyzer: GasGolfingAnalyzer,
    /// Simulation timeout for RPC requests
    simulation_timeout: std::time::Duration,
    /// Job queue for background task processing
    #[allow(dead_code)]
    job_queue: JobQueue,
    /// Fee market analytics engine (integer-only math).
    fee_analytics_engine: FeeAnalyticsEngine,
    /// Fee data store
    fee_store: Arc<FeeStore>,
    /// Fee business-logic service. API-28: all fee/billing business logic
    /// lives here — handlers in this file are now thin transports.
    fee_service: billing_service::FeeService,
    /// Prometheus metrics collectors.
    metrics: Arc<AppMetrics>,
    /// WebSocket event bus for simulation jobs.
    simulation_bus: Arc<SimulationBus>,
    /// Fee reconciler for async reconciliation jobs
    #[allow(dead_code)]
    reconciler: Arc<reconciliation::FeeReconciler>,
    /// Typed DB store for reconciliation queries
    reconciliation_repo: db::reconciliation::ReconciliationRepo,
    /// White-label vault records with optimistic locking (API-37).
    vault_store: Arc<vault_store::VaultStore>,
    agent_fleet: Arc<agent_fleet::DefaultAgentFleet>,
    /// Manager onboarding with approval/KYC gate (API-33).
    manager_store: Arc<manager_store::ManagerStore>,
    receipt_signer: ReceiptSigner,
    log_levels: RuntimeLogController,
}

#[derive(Clone)]
struct AppMetrics {
    registry: Registry,
    simulation_latency_seconds: HistogramVec,
    rpc_error_count_total: IntCounterVec,
    simulation_requests_total: IntCounterVec,
    resource_utilization_percent: prometheus::GaugeVec,
}

impl AppMetrics {
    fn new() -> Result<Self, prometheus::Error> {
        let registry = Registry::new();

        let simulation_latency_seconds = HistogramVec::new(
            prometheus::HistogramOpts::new(
                "simulation_latency_seconds",
                "Latency of simulation requests in seconds",
            ),
            &["endpoint"],
        )?;
        let rpc_error_count_total = IntCounterVec::new(
            Opts::new(
                "rpc_error_count_total",
                "Total number of RPC and simulation errors",
            ),
            &["endpoint", "error_type"],
        )?;
        let simulation_requests_total = IntCounterVec::new(
            Opts::new(
                "simulation_requests_total",
                "Total number of simulation requests by endpoint and cache status",
            ),
            &["endpoint", "cache_status"],
        )?;
        let resource_utilization_percent = prometheus::GaugeVec::new(
            Opts::new(
                "resource_utilization_percent",
                "Resource utilization percentage from latest simulation sample",
            ),
            &["resource"],
        )?;

        registry.register(Box::new(simulation_latency_seconds.clone()))?;
        registry.register(Box::new(rpc_error_count_total.clone()))?;
        registry.register(Box::new(simulation_requests_total.clone()))?;
        registry.register(Box::new(resource_utilization_percent.clone()))?;

        Ok(Self {
            registry,
            simulation_latency_seconds,
            rpc_error_count_total,
            simulation_requests_total,
            resource_utilization_percent,
        })
    }
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AnalyzeRequest {
    #[schema(example = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC")]
    pub contract_id: String,
    #[schema(example = "hello")]
    pub function_name: String,
    #[schema(example = "[]")]
    pub args: Option<Vec<String>>,
    /// Map of Key-Base64 to Value-Base64 ledger entry overrides
    pub ledger_overrides: Option<HashMap<String, String>>,
    /// Protocol version to simulate (e.g. 21)
    pub protocol_version: Option<u32>,
    /// Whether to enable experimental host functions
    pub enable_experimental: Option<bool>,
    /// Whether to generate and include Merkle tree root of the state snapshot
    #[serde(default)]
    #[schema(example = false)]
    pub include_merkle_tree: Option<bool>,
}

impl Validate for AnalyzeRequest {
    fn validate(&self) -> Result<(), String> {
        if self.contract_id.trim().is_empty() {
            return Err("contract_id must be a non-empty string".to_string());
        }
        if self.function_name.trim().is_empty() {
            return Err("function_name must be a non-empty string".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, ToSchema)]
pub struct ResourceReport {
    /// CPU instructions consumed
    #[schema(example = 1500)]
    pub cpu_instructions: u64,
    /// RAM bytes consumed
    #[schema(example = 3000)]
    pub ram_bytes: u64,
    /// Ledger read bytes
    #[schema(example = 1024)]
    pub ledger_read_bytes: u64,
    /// Ledger write bytes
    #[schema(example = 512)]
    pub ledger_write_bytes: u64,
    /// Transaction size in bytes
    #[schema(example = 450)]
    pub transaction_size_bytes: u64,
    /// Estimated cost in stroops
    #[schema(example = 1000)]
    pub cost_stroops: u64,
    /// Report showing which data was injected vs live
    pub state_dependency: Option<Vec<StateDependencyReport>>,
    /// TTL status for touched ledger entries and extension suggestions.
    pub ttl_analysis: Option<TtlAnalysisApiReport>,
    /// Efficiency score (0–100) and optimisation insights.
    pub nutrition: NutritionReport,
    /// Cross-contract call graph
    pub call_graph: Option<crate::simulation::CallGraph>,
    /// Call graph in Mermaid format
    pub call_graph_mermaid: Option<String>,
    /// Snapshot of the ledger state used/touched during simulation
    pub state_snapshot: Option<crate::simulation::SimulationStateSnapshot>,
    /// Protocol version used for this simulation
    #[schema(example = 20)]
    pub protocol_version: u32,
    /// Testnet average resource usage for comparison
    pub testnet_averages: TestnetAverages,
}

#[derive(Serialize, ToSchema)]
pub struct TestnetAverages {
    /// Average CPU instructions for typical Soroban transactions
    pub cpu_instructions: u64,
    /// Average RAM bytes for typical Soroban transactions
    pub ram_bytes: u64,
    /// Average ledger read bytes for typical Soroban transactions
    pub ledger_read_bytes: u64,
    /// Average ledger write bytes for typical Soroban transactions
    pub ledger_write_bytes: u64,
    /// Average transaction size bytes for typical Soroban transactions
    pub transaction_size_bytes: u64,
    /// Merkle tree root hash (hex-encoded) of the state snapshot, if requested
    pub merkle_tree_root: Option<String>,
}

#[derive(Serialize, ToSchema)]
pub struct TtlAnalysisApiReport {
    pub current_ledger: u64,
    pub touched_entries: Vec<TtlEntryApiReport>,
    pub extend_ttl_suggestions: Vec<ExtendTtlSuggestionApi>,
}

#[derive(Serialize, ToSchema)]
pub struct TtlEntryApiReport {
    pub key: String,
    pub live_until_ledger: u32,
    pub remaining_ledgers: i64,
}

#[derive(Serialize, ToSchema)]
pub struct ExtendTtlSuggestionApi {
    pub key: String,
    pub current_live_until_ledger: u32,
    pub remaining_ledgers: i64,
    pub extend_to_ledger: u32,
    pub ledgers_to_extend_by: u32,
    pub suggested_operation: String,
}

/// "Nutrition label" for the contract invocation.
#[derive(Serialize, ToSchema)]
pub struct NutritionReport {
    /// Weighted efficiency score (0 = poor, 100 = optimal).
    pub efficiency_score: u32,
    /// Actionable optimisation insights.
    pub insights: Vec<InsightEntry>,
}

/// A single optimisation insight.
#[derive(Serialize, ToSchema)]
pub struct InsightEntry {
    pub severity: String,
    pub rule: String,
    pub message: String,
    pub suggested_fix: String,
}

#[derive(Serialize, ToSchema, Debug)]
pub struct StateDependencyReport {
    pub key: String,
    pub source: String,
}

#[derive(Deserialize, ToSchema)]
pub struct OptimizeLimitsRequest {
    #[schema(example = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC")]
    pub contract_id: String,
    #[schema(example = "hello")]
    pub function_name: String,
    #[schema(example = "[]")]
    #[serde(default)]
    pub args: Vec<String>,
    #[schema(example = 0.05)]
    #[serde(default = "default_safety_margin")]
    pub    safety_margin: f64,
}

impl Validate for OptimizeLimitsRequest {
    fn validate(&self) -> Result<(), String> {
        if self.contract_id.trim().is_empty() {
            return Err("contract_id must be a non-empty string".to_string());
        }
        if self.function_name.trim().is_empty() {
            return Err("function_name must be a non-empty string".to_string());
        }
        Ok(())
    }
}

fn default_safety_margin() -> f64 {
    0.05
}

// Keep the legacy f64 `safety_margin` request field for backward compatibility;
// the service converts it to integer basis points before any arithmetic.

#[derive(Serialize, ToSchema)]
pub struct OptimizeLimitsResponse {
    pub cpu: crate::simulation::OptimizationBuffer,
    pub ram: crate::simulation::OptimizationBuffer,
    pub ledger_read: crate::simulation::OptimizationBuffer,
    pub ledger_write: crate::simulation::OptimizationBuffer,
    pub recommended: crate::simulation::SorobanResources,
}

// ── Fee Market Types ─────────────────────────────────────────────────────

/// Request body for fee recommendation endpoint
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct FeeRecommendationRequest {
    /// Desired inclusion speed: "next_ledger", "next_3_ledgers", "economy", "standard", "priority"
    #[schema(example = "priority")]
    pub inclusion_speed: Option<String>,
    /// Custom safety margin (default 0.10 = 10%)
    #[schema(example = 0.10)]
    pub safety_margin: Option<f64>,
}

/// Response with fee recommendations
#[derive(Debug, Serialize, ToSchema)]
pub struct FeeRecommendationResponse {
    /// Recommended fee bid in stroops
    pub recommended_bid: u64,
    /// Estimated resource fee
    pub resource_fee_estimate: u64,
    /// Total estimated cost
    pub total_estimated_cost: u64,
    /// Confidence in inclusion, in basis points (`0..=10_000`). The legacy
    /// `0.0-1.0` ratio was promoted to integer bps to close API-26.
    pub inclusion_confidence_bps: u32,
    /// Expected number of ledgers for inclusion
    pub expected_inclusion_ledgers: u32,
    /// Current market conditions
    pub market_conditions: MarketConditions,
    /// Breakdown of prediction models
    pub model_breakdown: ModelBreakdown,
    /// Timestamp of prediction
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Request for historical fee data
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct FeeHistoryRequest {
    /// Number of recent ledgers to retrieve (default 50)
    #[schema(example = 50)]
    pub limit: Option<i64>,
    /// Starting ledger sequence (optional)
    #[schema(example = 1000)]
    pub from_ledger: Option<i64>,
    /// Ending ledger sequence (optional)
    #[schema(example = 1100)]
    pub to_ledger: Option<i64>,
}

/// Historical fee data response
#[derive(Debug, Serialize, ToSchema)]
pub struct FeeHistoryResponse {
    /// List of fee samples
    pub samples: Vec<crate::fee_store::LedgerFeeSample>,
    /// Total count of samples
    pub total_count: i64,
}

/// Request body for the WASM-bytes analysis endpoint.
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnalyzeWasmRequest {
    /// Base64-encoded WASM binary.
    #[schema(example = "<base64-encoded .wasm bytes>")]
    pub wasm_bytes: String,
    /// Name of the exported function to invoke.
    #[schema(example = "hello")]
    pub function_name: String,
    /// Optional function arguments (void | true | false | integers | symbols).
    #[schema(example = "[]")]
    pub args: Option<Vec<String>>,
    /// Protocol version to simulate (e.g. 21)
    pub protocol_version: Option<u32>,
    /// Whether to enable experimental host functions
    pub enable_experimental: Option<bool>,
}

impl Validate for AnalyzeWasmRequest {
    fn validate(&self) -> Result<(), String> {
        if self.wasm_bytes.trim().is_empty() {
            return Err("wasm_bytes must be a non-empty string".to_string());
        }
        if self.function_name.trim().is_empty() {
            return Err("function_name must be a non-empty string".to_string());
        }
        Ok(())
    }
}

/// Request body for the WASM profiling endpoint.
#[derive(Debug, Deserialize)]
pub struct ProfileWasmRequest {
    /// Base64-encoded WASM binary.
    pub wasm_bytes: String,
    /// Name of the exported function to invoke.
    pub function_name: String,
    /// Optional function arguments.
    #[serde(default)]
    pub args: Vec<String>,
}

impl Validate for ProfileWasmRequest {
    fn validate(&self) -> Result<(), String> {
        if self.wasm_bytes.trim().is_empty() {
            return Err("wasm_bytes must be a non-empty string".to_string());
        }
        if self.function_name.trim().is_empty() {
            return Err("function_name must be a non-empty string".to_string());
        }
        Ok(())
    }
}

/// Response body for the WASM profiling endpoint.
#[derive(Debug, Serialize)]
pub struct ProfileResponse {
    /// Flamegraph and per-function counts.
    pub profile: simulation::ProfileResult,
    /// Standard Soroban resource metrics (CPU, RAM, etc.).
    pub resources: simulation::SorobanResources,
}

/// Request body for the WASM execution-branch analysis endpoint (Issue #101).
#[derive(Debug, Deserialize, ToSchema)]
pub struct AnalyzeWasmBranchesRequest {
    /// Base64-encoded WASM binary to analyse.
    #[schema(example = "<base64-encoded .wasm bytes>")]
    pub wasm_bytes: String,
    /// Exported function whose execution branches should be enumerated.
    #[schema(example = "transfer")]
    pub function_name: String,
    /// Baseline argument vector used for the first (reference) simulation run.
    /// Additional permutations are generated automatically.
    #[schema(example = "[]")]
    pub args: Option<Vec<String>>,
}

impl Validate for AnalyzeWasmBranchesRequest {
    fn validate(&self) -> Result<(), String> {
        if self.wasm_bytes.trim().is_empty() {
            return Err("wasm_bytes must be a non-empty string".to_string());
        }
        if self.function_name.trim().is_empty() {
            return Err("function_name must be a non-empty string".to_string());
        }
        Ok(())
    }
}

/// API response for the WASM execution-branch analysis endpoint.
#[derive(Debug, Serialize, ToSchema)]
pub struct WasmBranchAnalysisResponse {
    /// Name of the analysed function.
    pub function_name: String,
    /// Total branch-generating instructions found via static analysis.
    pub total_branch_count: usize,
    /// Maximum control-flow nesting depth observed.
    pub max_nesting_depth: usize,
    /// Per-category branch counts.
    pub branch_type_breakdown: crate::wasm_branch_analysis::BranchTypeBreakdown,
    /// Conservative upper bound on distinct execution paths (capped at 64).
    pub estimated_paths: usize,
    /// Inventory of branch points from static analysis.
    pub branches: Vec<crate::wasm_branch_analysis::BranchInfo>,
    /// Per-path resource measurements from dynamic simulation.
    pub simulated_paths: Vec<crate::wasm_branch_analysis::PathResult>,
    /// Resource consumption for the provided baseline arguments.
    pub baseline_resources: crate::simulation::SorobanResources,
    /// Highest resource consumption across all simulated paths.
    pub worst_case_resources: crate::simulation::SorobanResources,
    /// Lowest resource consumption across all simulated paths.
    pub best_case_resources: crate::simulation::SorobanResources,
    /// Number of distinct resource profiles observed.
    pub distinct_profiles: usize,
    /// Human-readable note about path coverage.
    pub coverage_note: String,
}

/// Convert a `SimulationResult` (library type) into the API `ResourceReport`.
fn to_report(
    result: &SimulationResult,
    insights_engine: &InsightsEngine,
    merkle_tree_root: Option<String>,
) -> ResourceReport {
    let insights_report = insights_engine.analyze(&result.resources);

    ResourceReport {
        cpu_instructions: result.resources.cpu_instructions,
        ram_bytes: result.resources.ram_bytes,
        ledger_read_bytes: result.resources.ledger_read_bytes,
        ledger_write_bytes: result.resources.ledger_write_bytes,
        transaction_size_bytes: result.resources.transaction_size_bytes,
        cost_stroops: result.cost_stroops,
        state_dependency: result.state_dependency.as_ref().map(|deps| {
            deps.iter()
                .map(|d| StateDependencyReport {
                    key: d.key.clone(),
                    source: format!("{:?}", d.source),
                })
                .collect()
        }),
        ttl_analysis: result
            .ttl_analysis
            .as_ref()
            .map(|ttl| TtlAnalysisApiReport {
                current_ledger: ttl.current_ledger,
                touched_entries: ttl
                    .touched_entries
                    .iter()
                    .map(|e| TtlEntryApiReport {
                        key: e.key.clone(),
                        live_until_ledger: e.live_until_ledger,
                        remaining_ledgers: e.remaining_ledgers,
                    })
                    .collect(),
                extend_ttl_suggestions: ttl
                    .extend_ttl_suggestions
                    .iter()
                    .map(|s| ExtendTtlSuggestionApi {
                        key: s.key.clone(),
                        current_live_until_ledger: s.current_live_until_ledger,
                        remaining_ledgers: s.remaining_ledgers,
                        extend_to_ledger: s.extend_to_ledger,
                        ledgers_to_extend_by: s.ledgers_to_extend_by,
                        suggested_operation: s.suggested_operation.clone(),
                    })
                    .collect(),
            }),
        nutrition: NutritionReport {
            efficiency_score: insights_report.efficiency_score,
            insights: insights_report
                .insights
                .into_iter()
                .map(|i| InsightEntry {
                    severity: format!("{:?}", i.severity),
                    rule: i.rule,
                    message: i.message,
                    suggested_fix: i.suggested_fix,
                })
                .collect(),
        },
        call_graph: result.call_graph.clone(),
        call_graph_mermaid: result.call_graph.as_ref().map(|g| g.to_mermaid()),
        state_snapshot: result.state_snapshot.clone(),
        protocol_version: result.protocol_version,
        testnet_averages: TestnetAverages {
            cpu_instructions: 3_000_000,
            ram_bytes: 512_000,
            ledger_read_bytes: 2_048,
            ledger_write_bytes: 1_024,
            transaction_size_bytes: 600,
            merkle_tree_root,
        },
    }
}

#[utoipa::path(
    post,
    path = "/analyze",
    request_body = AnalyzeRequest,
    responses(
        (status = 200, description = "Resource analysis successful", body = ResourceReport),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Analysis failed")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Analysis"
)]
async fn analyze(
    State(state): State<Arc<AppState>>,
    Extension(cancellation): Extension<RequestCancellation>,
    ValidatedJson(payload): ValidatedJson<AnalyzeRequest>,
) -> Result<(HeaderMap, Json<crate::jobs::SubmitJobResponse>), AppError> {
    let span = tracing::info_span!(
        "analyze",
        contract_id = %payload.contract_id,
        function_name = %payload.function_name,
    );
    let _enter = span.enter();
    tracing::info!("Received analyze request, offloading to background task");

    let job_id = cancellation
        .wait(state.job_queue.submit(
            crate::jobs::JobType::Analyze,
            crate::jobs::JobPayload::Analyze {
                contract_id: payload.contract_id,
                function_name: payload.function_name,
                args: payload.args,
                ledger_overrides: payload.ledger_overrides,
            },
            None,
        ))
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
        .map_err(|e| AppError::Internal(e.to_string()))?;

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-perigee-job"),
        HeaderValue::from_str(&job_id.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );

    Ok((
        headers,
        Json(crate::jobs::SubmitJobResponse {
            job_id: job_id.to_string(),
            status: crate::jobs::JobStatus::Queued,
            message: "Simulation and analysis job submitted".to_string(),
        }),
    ))
}

#[utoipa::path(
    post,
    path = "/analyze/wasm",
    request_body = AnalyzeWasmRequest,
    responses(
        (status = 200, description = "Resource analysis successful", body = ResourceReport),
        (status = 400, description = "Invalid base64 or WASM data"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Analysis failed")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Analysis"
)]
async fn analyze_wasm(
    State(state): State<Arc<AppState>>,
    Extension(cancellation): Extension<RequestCancellation>,
    ValidatedJson(payload): ValidatedJson<AnalyzeWasmRequest>,
) -> Result<Json<ResourceReport>, AppError> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

    tracing::info!(
        function_name = %payload.function_name,
        "Received WASM analyze request"
    );

    let wasm_bytes = BASE64
        .decode(&payload.wasm_bytes)
        .map_err(|e| AppError::BadRequest(format!("Invalid base64 WASM data: {}", e)))?;

    let function_name = payload.function_name.clone();
    let args = payload.args.clone().unwrap_or_default();
    let protocol_version = payload.protocol_version;
    let enable_experimental = payload.enable_experimental;

    let start_time = std::time::Instant::now();
    let resources = cancellation
        .run_blocking(move || {
            simulation::profile_contract(
                wasm_bytes,
                function_name,
                args,
                protocol_version,
                enable_experimental,
            )
        })
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
        .map_err(|e| {
            state
                .metrics
                .rpc_error_count_total
                .with_label_values(&["/analyze/wasm", "panic"])
                .inc();
            join_error_to_internal("Contract profiling task", e)
        })?
    .map_err(|e| {
        state
            .metrics
            .rpc_error_count_total
            .with_label_values(&["/analyze/wasm", "wasm_profile_error"])
            .inc();
        AppError::Internal(format!("Contract profiling failed: {}", e))
    })?;
    state
        .metrics
        .simulation_latency_seconds
        .with_label_values(&["/analyze/wasm"])
        .observe(start_time.elapsed().as_secs_f64());
    state
        .metrics
        .simulation_requests_total
        .with_label_values(&["/analyze/wasm", "LOCAL"])
        .inc();

    let sim_result = simulation::SimulationResult {
        resources,
        transaction_hash: None,
        latest_ledger: 0,
        cost_stroops: 0,
        state_dependency: None,
        ttl_analysis: None,
        transaction_data: String::new(),
        call_graph: None,
        state_snapshot: None,
        protocol_version: protocol_version.unwrap_or(20),
    };

    let report = to_report(&sim_result, &state.insights_engine, None);
    state
        .metrics
        .resource_utilization_percent
        .with_label_values(&["efficiency_score"])
        .set(report.nutrition.efficiency_score as f64);

    Ok(Json(report))
}

async fn metrics_handler(
    State(state): State<Arc<AppState>>,
) -> Result<impl IntoResponse, AppError> {
    let metric_families = state.metrics.registry.gather();
    let encoder = TextEncoder::new();
    let mut buffer = Vec::new();
    encoder
        .encode(&metric_families, &mut buffer)
        .map_err(|e| AppError::Internal(format!("Failed to encode Prometheus metrics: {}", e)))?;
    let output = String::from_utf8(buffer)
        .map_err(|e| AppError::Internal(format!("Metrics output encoding error: {}", e)))?;
    Ok((
        StatusCode::OK,
        [("Content-Type", encoder.format_type().to_string())],
        output,
    ))
}

async fn analyze_wasm_profile(
    State(state): State<Arc<AppState>>,
    Extension(cancellation): Extension<RequestCancellation>,
    ValidatedJson(payload): ValidatedJson<ProfileWasmRequest>,
) -> Result<Json<ProfileResponse>, AppError> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

    tracing::info!(
        function_name = %payload.function_name,
        "Received WASM profile request"
    );

    let wasm_bytes = BASE64
        .decode(&payload.wasm_bytes)
        .map_err(|e| AppError::BadRequest(format!("Invalid base64 WASM data: {}", e)))?;

    let function_name = payload.function_name.clone();
    let args = payload.args.clone();

    let result = tokio::time::timeout(
        state.simulation_timeout,
        cancellation.run_blocking(move || {
            simulation::profile_contract_with_flamegraph(wasm_bytes, function_name, args)
        }),
    )
    .await
    .map_err(|_| {
        AppError::Internal(format!(
            "Profiling request timed out after {} seconds",
            state.simulation_timeout.as_secs()
        ))
    })?
    .map_err(|_| AppError::Internal("Request cancelled".into()))?
    .map_err(|e| join_error_to_internal("Profiling task", e))?
    .map_err(|e| AppError::BadRequest(format!("Profiling failed: {}", e)))?;

    let (resources, profile) = result;

    Ok(Json(ProfileResponse { profile, resources }))
}

// ── WASM branch analysis handler (Issue #101) ─────────────────────────────────

#[utoipa::path(
    post,
    path = "/analyze/wasm/branches",
    request_body = AnalyzeWasmBranchesRequest,
    responses(
        (status = 200, description = "Branch analysis successful", body = WasmBranchAnalysisResponse),
        (status = 400, description = "Invalid base64 or WASM data"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Branch analysis failed")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Analysis"
)]
async fn analyze_wasm_branches(
    State(_state): State<Arc<AppState>>,
    Extension(cancellation): Extension<RequestCancellation>,
    ValidatedJson(payload): ValidatedJson<AnalyzeWasmBranchesRequest>,
) -> Result<Json<WasmBranchAnalysisResponse>, AppError> {
    use crate::wasm_branch_analysis::analyze_wasm_branches_with_cancellation as run_analysis;
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

    tracing::info!(
        function_name = %payload.function_name,
        "Received WASM branch analysis request"
    );

    let wasm_bytes = BASE64
        .decode(&payload.wasm_bytes)
        .map_err(|e| AppError::BadRequest(format!("Invalid base64 WASM data: {}", e)))?;

    let function_name = payload.function_name.clone();
    let args = payload.args.clone().unwrap_or_default();

    let cancellation_token = cancellation.token();
    let report = cancellation
        .run_blocking(move || {
            run_analysis(wasm_bytes, function_name, args, cancellation_token)
        })
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
        .map_err(|e| join_error_to_internal("Branch analysis task", e))?
        .map_err(|e| AppError::Internal(format!("Branch analysis failed: {}", e)))?;

    tracing::info!(
        function_name = %payload.function_name,
        total_branch_count = report.total_branch_count,
        simulated_paths = report.simulated_paths.len(),
        distinct_profiles = report.distinct_profiles,
        worst_cpu = report.worst_case_resources.cpu_instructions,
        worst_ram = report.worst_case_resources.ram_bytes,
        "Branch analysis completed"
    );

    Ok(Json(WasmBranchAnalysisResponse {
        function_name: report.function_name,
        total_branch_count: report.total_branch_count,
        max_nesting_depth: report.max_nesting_depth,
        branch_type_breakdown: report.branch_type_breakdown,
        estimated_paths: report.estimated_paths,
        branches: report.branches,
        simulated_paths: report.simulated_paths,
        baseline_resources: report.baseline_resources,
        worst_case_resources: report.worst_case_resources,
        best_case_resources: report.best_case_resources,
        distinct_profiles: report.distinct_profiles,
        coverage_note: report.coverage_note,
    }))
}

#[utoipa::path(
    post,
    path = "/analyze/optimize-limits",
    request_body = OptimizeLimitsRequest,
    responses(
        (status = 200, description = "Resource optimization successful", body = OptimizeLimitsResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Optimization failed")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Analysis"
)]
async fn optimize_limits(
    State(state): State<Arc<AppState>>,
    Extension(cancellation): Extension<RequestCancellation>,
    ValidatedJson(payload): ValidatedJson<OptimizeLimitsRequest>,
) -> Result<Json<OptimizeLimitsResponse>, AppError> {
    tracing::info!(
        "Optimizing limits for contract: {}, function: {}",
        payload.contract_id,
        payload.function_name
    );

    let report = state
        .engine
        .optimize_limits_with_cancellation(
            &payload.contract_id,
            &payload.function_name,
            payload.args,
            payload.safety_margin,
            cancellation.token(),
        )
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(OptimizeLimitsResponse {
        cpu: report.cpu,
        ram: report.ram,
        ledger_read: report.ledger_read,
        ledger_write: report.ledger_write,
        recommended: report.recommended,
    }))
}

// ── Compare types ────────────────────────────────────────────────────────────

#[derive(Serialize, ToSchema)]
pub struct CompareApiResponse {
    pub report: RegressionReport,
}

// ── Compare handler ──────────────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/analyze/compare",
    request_body(content_type = "multipart/form-data", content = String,
        description = "Multipart form with fields: mode (local_vs_local|local_vs_deployed), current_wasm, base_wasm (files), contract_id, function_name, args (text)"
    ),
    responses(
        (status = 200, description = "Comparison report", body = CompareApiResponse),
        (status = 400, description = "Invalid request"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Comparison failed")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Analysis"
)]
async fn compare_handler(
    State(state): State<Arc<AppState>>,
    Extension(cancellation): Extension<RequestCancellation>,
    mut multipart: Multipart,
) -> Result<Json<CompareApiResponse>, AppError> {
    let mut mode_str: Option<String> = None;
    let mut current_wasm_bytes: Option<Vec<u8>> = None;
    let mut base_wasm_bytes: Option<Vec<u8>> = None;
    let mut contract_id: Option<String> = None;
    let mut function_name: Option<String> = None;
    let mut args: Vec<String> = Vec::new();

    while let Some(field) = cancellation
        .wait(multipart.next_field())
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
        .map_err(|e| AppError::BadRequest(format!("Failed to read multipart field: {}", e)))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "mode" => {
                let value = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("Invalid mode field: {}", e)))?;
                mode_str = Some(input_sanitization::sanitize_multipart_text(&value, "mode")?);
                mode_str = Some(
                    cancellation
                        .wait(field.text())
                        .await
                        .map_err(|_| AppError::Internal("Request cancelled".into()))?
                        .map_err(|e| AppError::BadRequest(format!("Invalid mode field: {}", e)))?,
                );
            }
            "current_wasm" => {
                current_wasm_bytes = Some(
                    cancellation
                        .wait(field.bytes())
                        .await
                        .map_err(|_| AppError::Internal("Request cancelled".into()))?
                        .map_err(|e| {
                            AppError::BadRequest(format!("Failed to read current_wasm: {}", e))
                        })?
                        .to_vec(),
                );
            }
            "base_wasm" => {
                base_wasm_bytes = Some(
                    cancellation
                        .wait(field.bytes())
                        .await
                        .map_err(|_| AppError::Internal("Request cancelled".into()))?
                        .map_err(|e| {
                            AppError::BadRequest(format!("Failed to read base_wasm: {}", e))
                        })?
                        .to_vec(),
                );
            }
            "contract_id" => {
                let value = field.text().await.map_err(|e| {
                    AppError::BadRequest(format!("Invalid contract_id: {}", e))
                })?;
                contract_id = Some(input_sanitization::sanitize_multipart_text(
                    &value,
                    "contract_id",
                )?);
            }
            "function_name" => {
                let value = field.text().await.map_err(|e| {
                    AppError::BadRequest(format!("Invalid function_name: {}", e))
                })?;
                function_name = Some(input_sanitization::sanitize_multipart_text(
                    &value,
                    "function_name",
                )?);
                contract_id = Some(
                    cancellation
                        .wait(field.text())
                        .await
                        .map_err(|_| AppError::Internal("Request cancelled".into()))?
                        .map_err(|e| {
                            AppError::BadRequest(format!("Invalid contract_id: {}", e))
                        })?,
                );
            }
            "function_name" => {
                function_name = Some(
                    cancellation
                        .wait(field.text())
                        .await
                        .map_err(|_| AppError::Internal("Request cancelled".into()))?
                        .map_err(|e| {
                            AppError::BadRequest(format!("Invalid function_name: {}", e))
                        })?,
                );
            }
            "args" => {
                let args_json = cancellation
                    .wait(field.text())
                    .await
                    .map_err(|_| AppError::Internal("Request cancelled".into()))?
                    .map_err(|e| AppError::BadRequest(format!("Invalid args: {}", e)))?;
                let args_value: serde_json::Value = serde_json::from_str(&args_json)
                    .map_err(|e| AppError::BadRequest(format!("Invalid args JSON: {}", e)))?;
                let args_value = input_sanitization::sanitize_json(&args_value)
                    .map_err(input_sanitization::sanitization_error)?;
                args = serde_json::from_value(args_value)
                    .map_err(|e| AppError::BadRequest(format!("Invalid args: {}", e)))?;
            }
            _ => { /* ignore unknown fields */ }
        }
    }

    let mode = mode_str.unwrap_or_else(|| "local_vs_local".to_string());
    if cancellation.is_cancelled() {
        return Err(AppError::Internal("Request cancelled".into()));
    }

    let compare_mode = match mode.as_str() {
        "local_vs_local" => {
            let current_bytes = current_wasm_bytes
                .ok_or_else(|| AppError::BadRequest("Missing current_wasm file".to_string()))?;
            let base_bytes = base_wasm_bytes
                .ok_or_else(|| AppError::BadRequest("Missing base_wasm file".to_string()))?;

            let current_tmp = write_temp_wasm(&current_bytes)?;
            let base_tmp = match write_temp_wasm(&base_bytes) {
                Ok(path) => path,
                Err(error) => {
                    let _ = std::fs::remove_file(&current_tmp);
                    return Err(error);
                }
            };

            CompareMode::LocalVsLocal {
                current_wasm: current_tmp,
                base_wasm: base_tmp,
            }
        }
        "local_vs_deployed" => {
            let current_bytes = current_wasm_bytes
                .ok_or_else(|| AppError::BadRequest("Missing current_wasm file".to_string()))?;
            let cid = contract_id
                .ok_or_else(|| AppError::BadRequest("Missing contract_id".to_string()))?;
            let fname = function_name
                .ok_or_else(|| AppError::BadRequest("Missing function_name".to_string()))?;

            let current_tmp = write_temp_wasm(&current_bytes)?;

            CompareMode::LocalVsDeployed {
                current_wasm: current_tmp,
                contract_id: cid,
                function_name: fname,
                args,
            }
        }
        other => {
            return Err(AppError::BadRequest(format!(
                "Unknown mode '{}'. Use 'local_vs_local' or 'local_vs_deployed'",
                other
            )));
        }
    };
    let cleanup_paths = match &compare_mode {
        CompareMode::LocalVsLocal {
            current_wasm,
            base_wasm,
        } => vec![current_wasm.clone(), base_wasm.clone()],
        CompareMode::LocalVsDeployed { current_wasm, .. } => vec![current_wasm.clone()],
    };
    let _cleanup = TempWasmCleanup {
        paths: cleanup_paths,
    };

    let report = comparison::run_comparison_with_cancellation(
        &state.engine,
        compare_mode,
        cancellation.token(),
    )
    .await
    .map_err(|e| AppError::Internal(format!("Comparison failed: {}", e)))?;

    Ok(Json(CompareApiResponse { report }))
}

/// Sanitize a [`tokio::task::JoinError`] into an [`AppError::Internal`].
///
/// A panic payload's `Display` can expose source file paths and line numbers
/// (e.g. `"task panicked at 'assertion failed', src/simulation.rs:412"`).
/// In production we emit only the static category string and log the full
/// detail server-side.  In development the full message is preserved for
/// easier debugging.
fn join_error_to_internal(context: &str, e: tokio::task::JoinError) -> AppError {
    if e.is_panic() {
        tracing::error!(
            context = context,
            panic_detail = %e,
            "spawn_blocking task panicked"
        );
        if crate::errors::is_production() {
            AppError::Internal(format!("{}: task panicked", context))
        } else {
            AppError::Internal(format!("{}: task panicked — {}", context, e))
        }
    } else {
        // Cancellation — safe to surface
        AppError::Internal(format!("{}: task was cancelled", context))
    }
}

/// Write WASM bytes to a temporary file and return the path.
fn write_temp_wasm(bytes: &[u8]) -> Result<std::path::PathBuf, AppError> {
    use std::io::Write;
    let mut tmp = tempfile::Builder::new()
        .suffix(".wasm")
        .tempfile()
        .map_err(|e| AppError::Internal(format!("Failed to create temp file: {}", e)))?;
    tmp.write_all(bytes)
        .map_err(|e| AppError::Internal(format!("Failed to write temp file: {}", e)))?;
    let (_, path) = tmp
        .keep()
        .map_err(|e| AppError::Internal(format!("Failed to persist temp file: {}", e)))?;
    Ok(path)
}

struct TempWasmCleanup {
    paths: Vec<PathBuf>,
}

impl Drop for TempWasmCleanup {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = std::fs::remove_file(path);
        }
    }
}

// ── Gas Golfing Types ─────────────────────────────────────────────────────

#[derive(Debug, Deserialize, ToSchema)]
pub struct GasGolfingRequest {
    /// Base64-encoded WASM bytecode
    #[schema(example = "AGFzbQEAAAABBgFgAX8BfwMCAQAFAwMADAEAAQgBAUcBAQABAQgBAUcBAQACAgcABAEGCw==")]
    pub wasm_bytes: String,
    /// Contract name for identification
    #[schema(example = "my_contract")]
    pub contract_name: String,
    /// Protocol version whose Soroban resource prices should be used.
    pub protocol_version: Option<u32>,
    /// Measured resources from a Soroban simulation. Without this, savings
    /// remain unquantified because static WASM analysis cannot measure hosts.
    pub measured_resources: Option<SorobanResources>,
}

impl Validate for GasGolfingRequest {
    fn validate(&self) -> Result<(), String> {
        if self.wasm_bytes.trim().is_empty() {
            return Err("wasm_bytes must be a non-empty string".to_string());
        }
        if self.contract_name.trim().is_empty() {
            return Err("contract_name must be a non-empty string".to_string());
        }
        Ok(())
    }
}

#[derive(Serialize, ToSchema)]
pub struct GasGolfingResponse {
    pub report: GasGolfingReport,
}

// ── Gas Golfing Handler ───────────────────────────────────────────────────

#[utoipa::path(
    post,
    path = "/analyze/gas-golfing",
    request_body = GasGolfingRequest,
    responses(
        (status = 200, description = "Gas golfing analysis completed", body = GasGolfingResponse),
        (status = 400, description = "Invalid WASM data"),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Analysis failed")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Analysis"
)]
async fn analyze_gas_golfing(
    State(state): State<Arc<AppState>>,
    Extension(cancellation): Extension<RequestCancellation>,
    ValidatedJson(payload): ValidatedJson<GasGolfingRequest>,
) -> Result<Json<GasGolfingResponse>, AppError> {
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine};

    tracing::info!(
        contract_name = %payload.contract_name,
        "Received gas golfing analysis request"
    );

    let wasm_bytes = BASE64
        .decode(&payload.wasm_bytes)
        .map_err(|e| AppError::BadRequest(format!("Invalid base64 WASM data: {}", e)))?;

    let contract_name = payload.contract_name.clone();

    let analyzer = match payload.protocol_version {
        Some(version) => GasGolfingAnalyzer::for_protocol_version(version)
            .map_err(|e| AppError::BadRequest(e.to_string()))?,
        None => state.gas_golfing_analyzer.clone(),
    };
    let measured_resources = payload.measured_resources;

    let report = cancellation
        .run_blocking(move || {
            analyzer.analyze_wasm_with_measurement(
                &wasm_bytes,
                &contract_name,
                measured_resources.as_ref(),
            )
        })
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
        .map_err(|e| AppError::Internal(format!("Gas golfing analysis task panicked: {}", e)))?;

    Ok(Json(GasGolfingResponse { report }))
}

// ── Fee Market API Handlers ──────────────────────────────────────────────

#[utoipa::path(
    get,
    path = "/fees/recommend",
    params(
        ("inclusion_speed" = Option<String>, Query, description = "Desired inclusion speed: next_ledger, next_3_ledgers, economy, standard, priority"),
        ("safety_margin" = Option<f64>, Query, description = "Custom safety margin (default 0.10)")
    ),
    responses(
        (status = 200, description = "Fee recommendation successful", body = FeeRecommendationResponse),
        (status = 500, description = "Failed to generate recommendation")
    ),
    tag = "Fee Market"
)]
async fn fee_recommend(
    State(state): State<Arc<AppState>>,
    SanitizedQuery(req): SanitizedQuery<FeeRecommendationRequest>,
    Extension(cancellation): Extension<RequestCancellation>,
    Query(req): Query<FeeRecommendationRequest>,
) -> Result<Json<FeeRecommendationResponse>, AppError> {
    tracing::info!("Generating fee recommendation");

    let inclusion_speed = billing_service::InclusionSpeed::parse(req.inclusion_speed.as_deref());
    let safety_margin_bps = match req.safety_margin {
        Some(m) => billing_service::FeeService::safety_margin_to_bps(m)?,
        None => billing_service::DEFAULT_SAFETY_MARGIN_BPS,
    };
    let inputs = billing_service::FeeRecommendationInputs {
        inclusion_speed,
        safety_margin_bps,
    };
    let result = cancellation
        .wait(state.fee_service.recommend(inputs))
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))??;
    Ok(Json(FeeRecommendationResponse {
        recommended_bid: result.recommended_bid,
        resource_fee_estimate: result.resource_fee_estimate,
        total_estimated_cost: result.total_estimated_cost,
        inclusion_confidence_bps: result.inclusion_confidence_bps,
        expected_inclusion_ledgers: result.expected_inclusion_ledgers,
        market_conditions: result.market_conditions,
        model_breakdown: result.model_breakdown,
        timestamp: result.timestamp,
    }))
}

#[utoipa::path(
    get,
    path = "/fees/history",
    params(
        ("limit" = Option<i64>, Query, description = "Number of recent ledgers to retrieve"),
        ("from_ledger" = Option<i64>, Query, description = "Starting ledger sequence"),
        ("to_ledger" = Option<i64>, Query, description = "Ending ledger sequence")
    ),
    responses(
        (status = 200, description = "Fee history retrieved successfully", body = FeeHistoryResponse),
        (status = 500, description = "Failed to fetch fee history")
    ),
    tag = "Fee Market"
)]
async fn fee_history(
    State(state): State<Arc<AppState>>,
    SanitizedQuery(req): SanitizedQuery<FeeHistoryRequest>,
    Extension(cancellation): Extension<RequestCancellation>,
    Query(req): Query<FeeHistoryRequest>,
) -> Result<Json<FeeHistoryResponse>, AppError> {
    tracing::info!("Fetching fee history");

    let result = cancellation
        .wait(state.fee_service.history(billing_service::FeeHistoryQuery {
            limit: req.limit,
            from_ledger: req.from_ledger,
            to_ledger: req.to_ledger,
        }))
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))??;
    Ok(Json(FeeHistoryResponse {
        samples: result.samples,
        total_count: result.total_count,
    }))
}

#[utoipa::path(
    get,
    path = "/fees/analytics",
    responses(
        (status = 200, description = "Fee analytics retrieved successfully", body = FeeAnalyticsEnvelope),
        (status = 500, description = "Failed to fetch analytics")
    ),
    tag = "Fee Market"
)]
async fn fee_analytics(
    State(state): State<Arc<AppState>>,
    Extension(cancellation): Extension<RequestCancellation>,
) -> Result<Json<FeeAnalyticsEnvelope>, AppError> {
    tracing::info!("Fetching fee analytics");

    let result = cancellation
        .wait(state.fee_service.analytics())
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))??;
    Ok(Json(FeeAnalyticsEnvelope {
        current_ledger: result.current_ledger,
        prediction: result.prediction,
        market_conditions: result.market_conditions,
        model_breakdown: result.model_breakdown,
        sample_count: result.sample_count,
        timestamp: result.timestamp,
    }))
}

/// Envelope returned by `GET /fees/analytics`. Mirrors
/// `billing_service::FeeAnalyticsResult` but is exposed in the OpenAPI
/// schema as a single named object.
#[derive(Debug, Serialize, ToSchema)]
pub struct FeeAnalyticsEnvelope {
    pub current_ledger: u64,
    pub prediction: crate::fee_analytics::FeePrediction,
    pub market_conditions: crate::fee_analytics::MarketConditions,
    pub model_breakdown: crate::fee_analytics::ModelBreakdown,
    pub sample_count: usize,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearerAuth",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::HttpBuilder::new()
                        .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
            components.add_security_scheme(
                "jwt",
                utoipa::openapi::security::SecurityScheme::Http(
                    utoipa::openapi::security::HttpBuilder::new()
                        .scheme(utoipa::openapi::security::HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}

#[derive(OpenApi)]
#[openapi(
    paths(
        analyze, analyze_wasm, analyze_wasm_branches, optimize_limits, compare_handler, analyze_gas_golfing,
        auth::challenge_handler, auth::verify_handler, auth::refresh_handler,
        auth::revoke_handler, auth::emergency_pause_handler, auth::jwks_handler,
        auth::issue_scoped_token_handler,
        fee_recommend, fee_history, fee_analytics,
        vault_store::create_vault_handler, vault_store::get_vault_handler,
        vault_store::update_vault_handler, vault_store::soft_delete_vault_handler,
        vault_store::restore_vault_handler, vault_store::list_vaults_handler,
        vault_store::list_deleted_vaults_handler,
        manager_store::register_manager_handler, manager_store::list_managers_handler,
        manager_store::get_manager_handler, manager_store::approve_manager_handler,
        manager_store::reject_manager_handler, manager_store::check_manager_status_handler,
        reconciliation::reconcile_handler, reconciliation::get_reconcile_job_handler,
        reconciliation::list_reports_handler,
        ws::ws_handler
    ),
    components(schemas(
        AnalyzeRequest, AnalyzeWasmRequest, AnalyzeWasmBranchesRequest,
        WasmBranchAnalysisResponse, ResourceReport, GasGolfingRequest, GasGolfingResponse,
        OptimizeLimitsRequest, OptimizeLimitsResponse,
        CompareApiResponse, RegressionReport, ResourceDelta, RegressionFlag,
        crate::wasm_branch_analysis::BranchInfo,
        crate::wasm_branch_analysis::BranchType,
        crate::wasm_branch_analysis::BranchTypeBreakdown,
        crate::wasm_branch_analysis::PathResult,
        auth::Role, auth::ScopedTokenRequest, auth::ScopedTokenResponse,
        auth::ChallengeRequest, auth::ChallengeResponse,
        auth::VerifyRequest, auth::VerifyResponse, auth::RefreshRequest,
        auth::RevokeResponse, auth::EmergencyPauseRequest, auth::EmergencyPauseResponse,
        auth::JwkSetResponse, auth::JwkResponse,
        crate::simulation::OptimizationBuffer,
        crate::simulation::SorobanResources,
        FeeRecommendationRequest, FeeRecommendationResponse,
        FeeHistoryRequest, FeeHistoryResponse,
        crate::fee_store::LedgerFeeSample,
        crate::fee_analytics::MarketConditions,
        crate::fee_analytics::ModelBreakdown,
        crate::fee_analytics::TrendDirection,
        FeeAnalyticsEnvelope,
        vault_store::VaultRecord, vault_store::CreateVaultRequest,
        vault_store::UpdateVaultRequest, vault_store::ListVaultsQuery,
        vault_store::ListDeletedVaultsQuery,
        manager_store::ManagerRecord, manager_store::RegisterManagerRequest,
        manager_store::ApproveManagerRequest, manager_store::ManagerStatusResponse,
        reconciliation::ReconcileRequest, reconciliation::ReconcileResponse,
        reconciliation::ReconciliationReport, reconciliation::ListReportsQuery,
        crate::jobs::Job, crate::jobs::JobStatus, crate::jobs::JobType
    )),
    modifiers(&SecurityAddon),
    tags(
        (name = "Analysis", description = "Soroban contract resource analysis endpoints"),
        (name = "Auth", description = "SEP-10 wallet authentication"),
        (name = "Fee Market", description = "Stellar/Soroban fee market analysis and prediction"),
        (name = "Vaults", description = "White-label vault records with optimistic locking"),
        (name = "Managers", description = "Manager onboarding and KYC approval gate"),
        (name = "Reconciliation", description = "Fee reconciliation background engine"),
        (name = "Streaming", description = "WebSocket real-time simulation progress streaming")
    ),
    info(
        title = "Perigee API",
        version = "0.1.0",
        description = "API for analyzing Soroban smart contract resource consumption, fee market predictions, policy vaults, and manager onboarding"
    )
)]
struct ApiDoc;

async fn get_log_levels(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<auth::AuthenticatedUser>,
) -> Result<Json<LogLevelSnapshot>, AppError> {
    if !user.is_admin() {
        return Err(AppError::Forbidden("Admin privileges are required".into()));
    }
    state
        .log_levels
        .snapshot()
        .map(Json)
        .map_err(|error| AppError::Internal(error.to_string()))
}

async fn set_log_level(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<auth::AuthenticatedUser>,
    Json(update): Json<LogLevelUpdate>,
) -> Result<Json<LogLevelSnapshot>, AppError> {
    if !user.is_admin() {
        return Err(AppError::Forbidden("Admin privileges are required".into()));
    }
    let module = update.module.trim();
    let result = if module == "*" {
        state.log_levels.set_global_level(&update.level)
    } else {
        state.log_levels.set_module(module, &update.level)
    };
    if result.is_ok() {
        crate::audit_log::log_audit_event(
            &user.stellar_address,
            "runtime_log_level_update",
            &format!("module={module};level={}", update.level),
        );
    }
    result
        .map(Json)
        .map_err(|error| AppError::BadRequest(error.to_string()))
}

async fn remove_log_level(
    State(state): State<Arc<AppState>>,
    Extension(user): Extension<auth::AuthenticatedUser>,
    axum::extract::Path(module): axum::extract::Path<String>,
) -> Result<Json<LogLevelSnapshot>, AppError> {
    if !user.is_admin() {
        return Err(AppError::Forbidden("Admin privileges are required".into()));
    }
    let module = module.trim();
    let result = if module == "*" {
        state.log_levels.clear_global_level()
    } else {
        state.log_levels.remove_module(module)
    };
    result
        .map(Json)
        .map_err(|error| AppError::BadRequest(error.to_string()))
}

async fn health_check() -> &'static str {
    "OK"
}

/// Fallback handler for all unmatched routes.
///
/// Returns a uniform JSON `{"error":"NOT_FOUND","message":"..."}` body
/// instead of axum's default plain-text response, which could expose
/// routing internals or framework version strings.
async fn not_found_handler(request: axum::extract::Request) -> impl IntoResponse {
    let path = request.uri().path().to_owned();
    tracing::debug!(path = %path, "Unmatched route");
    AppError::NotFound(format!("No route for {}", path))
}

async fn ready_check(
    State(state): State<Arc<AppState>>,
    Extension(cancellation): Extension<RequestCancellation>,
) -> axum::response::Response {
    use axum::response::IntoResponse;
    
    let db_ok = cancellation
        .wait(sqlx::query("SELECT 1").execute(state.reconciliation_repo.pool()))
        .await
        .is_ok_and(|result| result.is_ok());
        
    let rpc_ok = !state.provider_registry.healthy_providers().await.is_empty();
    let agents_ok = state.agent_fleet.is_operational("server");
    let rpc_ok = cancellation
        .wait(state.provider_registry.healthy_providers())
        .await
        .is_ok_and(|providers| !providers.is_empty());

    if db_ok && rpc_ok && agents_ok {
        (axum::http::StatusCode::OK, "OK").into_response()
    } else {
        (axum::http::StatusCode::SERVICE_UNAVAILABLE, "Service Unavailable").into_response()
    }
}

async fn registry_providers(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<crate::rpc_provider::ProviderHealthReport>> {
    Json(state.provider_registry.provider_reports().await)
}

async fn registry_peers(
    State(state): State<Arc<AppState>>,
) -> Json<Vec<crate::rpc_provider::PeerHealthReport>> {
    Json(state.provider_registry.peer_reports().await)
}

async fn registry_gossip(
    State(state): State<Arc<AppState>>,
    SanitizedJson(snapshot): SanitizedJson<RegistrySnapshot>,
) -> Json<RegistrySnapshot> {
    state.provider_registry.merge_snapshot(snapshot).await;
    Json(state.provider_registry.registry_snapshot().await)
}

/// Resolves SIGINT / SIGTERM so the HTTP server can drain in-flight
/// requests before exiting (Closes API-30: No graceful shutdown).
///
/// On Unix this listens for both signals. On non-Unix targets only
/// Ctrl-C is wired up.
async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c()
            .await
            .expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
            .expect("failed to install SIGTERM handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("SIGINT received, draining in-flight requests before shutdown"),
        _ = terminate => tracing::info!("SIGTERM received, draining in-flight requests before shutdown"),
    }
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }
    let base_log_directives = env::var("RUST_LOG").unwrap_or_else(|_| "info".to_string());
    let initial_filter = EnvFilter::builder()
        .with_regex(false)
        .with_default_directive(LevelFilter::INFO.into())
        .parse(&base_log_directives)
        .unwrap_or_else(|_| EnvFilter::new("info"));
    let base_filter_directives = initial_filter.to_string();
    let (filter_layer, filter_handle) =
        reload::Layer::<EnvFilter, tracing_subscriber::Registry>::new(initial_filter);
    if env::var("LOG_FORMAT").as_deref() == Ok("json") {
        tracing_subscriber::registry()
            .with(filter_layer)
            .with(tracing_subscriber::fmt::layer().json())
            .init();
    } else {
        tracing_subscriber::registry()
            .with(filter_layer)
            .with(tracing_subscriber::fmt::layer())
            .init();
    }
    let log_levels = RuntimeLogController::new(filter_handle, base_filter_directives);
    if let Err(error) = log_levels.load_file() {
        panic!("Invalid log level configuration: {error}");
    }

    tracing::info!("Perigee Starting...");

    let config = load_config().expect("Failed to load configuration");
    // Fail fast on malformed secrets before the server binds (issue #85 / NF-03).
    if let Err(err) = validate_config_secrets(&config) {
        tracing::error!(
            error = %err,
            "Configuration validation failed at startup. Refusing to bind."
        );
        panic!("Invalid configuration: {}", err);
    }
    tracing::info!("Perigee initialized with config: {:?}", config);
    tracing::info!(
        redis_url = %config.redis_url,
        "Cache config: using in-memory (moka) MVP; Redis URL reserved for future migration"
    );

    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && args[1] == "benchmark" {
        tracing::info!("Starting Perigee Benchmark...");

        let possible_paths = vec![
            "target/wasm32-unknown-unknown/release/soroban_token_contract.wasm",
            "../target/wasm32-unknown-unknown/release/soroban_token_contract.wasm",
        ];

        let mut wasm_path = None;
        for p in possible_paths {
            let path = PathBuf::from(p);
            if path.exists() {
                wasm_path = Some(path);
                break;
            }
        }

        if let Some(path) = wasm_path {
            let db_path = env::var("Perigee_DB_PATH")
                .unwrap_or_else(|_| "Perigee_metrics.db".to_string());
            let webhook_url = env::var("Perigee_ALERT_WEBHOOK_URL").ok();
            let simulation_service = SimulationService::new(db_path, webhook_url)
                .expect("initialize simulation service");
            // Catch up on any webhook events left pending/retrying from a
            // previous run (e.g. the process crashed or was killed mid
            // backoff) before generating new alerts. This is what makes
            // delivery durable across restarts rather than just within a
            // single process's retry loop.
            if let Err(e) = simulation_service.dispatch_due_events().await {
                tracing::warn!("Failed to drain pending webhook events: {}", e);
            }
            if let Err(e) = benchmarks::run_token_benchmark(path, &simulation_service).await {
                tracing::error!("Benchmark failed: {}", e);
            }
        } else {
            tracing::error!(
                "Could not find soroban_token_contract.wasm. Build the contract first."
            );
        }

        return;
    }

    // ── CLI: merkle subcommand ──────────────────────────────────────────
    if args.len() > 1 && args[1] == "merkle" {
        if args.len() < 4 {
            eprintln!("Usage: Perigee-core merkle <build|proof> <args>");
            eprintln!("Commands:");
            eprintln!("  build <leaf1> <leaf2> ...            Build a Merkle tree and print the root hash");
            eprintln!("  proof <leaf_index> <leaf1> <leaf2> ... Generate a Merkle proof for the given leaf index");
            std::process::exit(1);
        }

        let command = &args[2];
        match command.as_str() {
            "build" => {
                if args.len() < 4 {
                    eprintln!("Usage: Perigee-core merkle build <leaf1> <leaf2> ...");
                    std::process::exit(1);
                }
                let leaves: Vec<Vec<u8>> = args[3..]
                    .iter()
                    .map(|arg| arg.as_bytes().to_vec())
                    .collect();
                let mut tree = merkle_tree::MerkleTree::new(32);
                match tree.build(leaves) {
                    Ok(()) => println!("{}", tree.get_root_hex()),
                    Err(err) => {
                        eprintln!("Error building Merkle tree: {}", err);
                        std::process::exit(1);
                    }
                }
            }
            "proof" => {
                if args.len() < 5 {
                    eprintln!(
                        "Usage: Perigee-core merkle proof <leaf_index> <leaf1> <leaf2> ..."
                    );
                    std::process::exit(1);
                }
                let leaf_index = match args[3].parse::<usize>() {
                    Ok(index) => index,
                    Err(_) => {
                        eprintln!("Leaf index must be a non-negative integer.");
                        std::process::exit(1);
                    }
                };
                let leaves: Vec<Vec<u8>> = args[4..]
                    .iter()
                    .map(|arg| arg.as_bytes().to_vec())
                    .collect();
                let mut tree = merkle_tree::MerkleTree::new(32);
                if let Err(err) = tree.build(leaves) {
                    eprintln!("Error building Merkle tree: {}", err);
                    std::process::exit(1);
                }
                let proof = match tree.generate_proof(leaf_index) {
                    Ok(proof) => proof,
                    Err(err) => {
                        eprintln!("Error generating Merkle proof: {}", err);
                        std::process::exit(1);
                    }
                };
                let output = serde_json::json!({
                    "root": tree.get_root_hex(),
                    "leaf_index": leaf_index,
                    "leaf_count": tree.leaf_count(),
                    "proof": proof,
                });
                println!("{}", serde_json::to_string_pretty(&output).unwrap());
            }
            unknown => {
                eprintln!("Unknown merkle command: {}", unknown);
                eprintln!("Available commands: build, proof");
                std::process::exit(1);
            }
        }

        return;
    }

    // Default Web Server
    println!("Perigee CLI Initialized. Run with 'benchmark' argument to profile token contract.");

    // ── CLI: compare subcommand ──────────────────────────────────────────
    if args.len() > 1 && args[1] == "compare" {
        if args.len() < 4 {
            eprintln!("Usage: Perigee-core compare <current.wasm> <base.wasm>");
            eprintln!("\nCompare two WASM contract versions and detect resource regressions.");
            eprintln!("\nArguments:");
            eprintln!("  <current.wasm>  Path to the new (current) version WASM file");
            eprintln!("  <base.wasm>     Path to the reference (base) version WASM file");
            std::process::exit(1);
        }

        let current_path = PathBuf::from(&args[2]);
        let base_path = PathBuf::from(&args[3]);

        if !current_path.exists() {
            eprintln!(
                "Error: Current WASM file not found: {}",
                current_path.display()
            );
            std::process::exit(1);
        }
        if !base_path.exists() {
            eprintln!("Error: Base WASM file not found: {}", base_path.display());
            std::process::exit(1);
        }

        let providers = build_providers(&config);
        let registry = rpc_provider::ProviderRegistry::new(providers);
        let engine = SimulationEngine::with_registry(std::sync::Arc::clone(&registry));

        let compare_mode = comparison::CompareMode::LocalVsLocal {
            current_wasm: current_path,
            base_wasm: base_path,
        };

        match comparison::run_comparison(&engine, compare_mode).await {
            Ok(report) => {
                comparison::print_report(&report);
            }
            Err(e) => {
                eprintln!("Error: Comparison failed: {}", e);
                std::process::exit(1);
            }
        }

        return;
    }

    // ── CLI: export subcommand ──────────────────────────────────────────
    if args.len() > 1 && args[1] == "export" {
        if args.len() < 6 {
            eprintln!(
                "Usage: Perigee-core export <contract_id> <function> <args_json> <output_file>"
            );
            eprintln!("\nSimulate a transaction and export the touched state to a JSON file.");
            std::process::exit(1);
        }

        let contract_id = &args[2];
        let function = &args[3];
        let args_json = &args[4];
        let output_file = &args[5];

        let parsed_args: Vec<String> = serde_json::from_str(args_json).unwrap_or_default();

        let providers = build_providers(&config);
        let registry = rpc_provider::ProviderRegistry::new(providers);
        let engine = SimulationEngine::with_registry(std::sync::Arc::clone(&registry));

        match engine
            .simulate_from_contract_id(contract_id, function, parsed_args, None, None, None)
            .await
        {
            Ok(result) => {
                if let Some(snapshot) = result.state_snapshot {
                    let json = serde_json::to_string_pretty(&snapshot).unwrap();
                    if let Err(e) = std::fs::write(output_file, json) {
                        eprintln!("Error: Failed to write snapshot to {}: {}", output_file, e);
                        std::process::exit(1);
                    }
                    println!("State snapshot exported to {}", output_file);
                } else {
                    eprintln!("Error: No state snapshot generated.");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                eprintln!("Error: Simulation failed: {}", e);
                std::process::exit(1);
            }
        }

        return;
    }

    // ── CLI: restore subcommand ──────────────────────────────────────────
    if args.len() > 1 && args[1] == "restore" {
        if args.len() < 6 {
            eprintln!("Usage: Perigee-core restore <snapshot_file> <contract_id> <function> <args_json>");
            eprintln!("\nRestore state from a JSON file and run a simulation.");
            std::process::exit(1);
        }

        let snapshot_file = &args[2];
        let contract_id = &args[3];
        let function = &args[4];
        let args_json = &args[5];

        let snapshot_json =
            std::fs::read_to_string(snapshot_file).expect("Failed to read snapshot file");
        let snapshot: crate::simulation::SimulationStateSnapshot =
            serde_json::from_str(&snapshot_json).expect("Failed to parse snapshot JSON");

        let parsed_args: Vec<String> = serde_json::from_str(args_json).unwrap_or_default();

        let providers = build_providers(&config);
        let registry = rpc_provider::ProviderRegistry::new(providers);
        let engine = SimulationEngine::with_registry(std::sync::Arc::clone(&registry));

        match engine
            .simulate_from_contract_id(
                contract_id,
                function,
                parsed_args,
                Some(snapshot.ledger_entries),
                None,
                None,
            )
            .await
        {
            Ok(result) => {
                println!("Simulation successful with restored state.");
                println!("Resources: {:?}", result.resources);
                if let Some(deps) = result.state_dependency {
                    println!("State dependencies: {} entries", deps.len());
                }
            }
            Err(e) => {
                eprintln!("Error: Simulation failed: {}", e);
                std::process::exit(1);
            }
        }

        return;
    }

    // ── CLI: migrate subcommand ──────────────────────────────────────────
    if args.len() > 1 && args[1] == "migrate" {
        tracing::info!(database_url = %config.database_url, "Running database migrations");
        let db_pool = sqlx::SqlitePool::connect(&config.database_url)
            .await
            .expect("Failed to connect to database");
        crate::db::migrations::run_migrations(&db_pool)
            .await
            .expect("Failed to run database migrations");
        println!("Database migrations applied successfully.");
        return;
    }

    let receipt_signer = ReceiptSigner::from_env_with_app_env(Some(&config.app_env))
        .unwrap_or_else(|error| panic!("Failed to configure receipt signing: {error}"));
    tracing::info!("Starting Perigee API Server...");

    // ── Multi-node RPC setup ────────────────────────────────────────────
    let providers = build_providers(&config);
    let startup_providers = providers.clone();
    let provider_names: Vec<&str> = providers.iter().map(|p| p.name.as_str()).collect();
    tracing::info!(providers = ?provider_names, "RPC provider pool");

    let registry = ProviderRegistry::new_with_config(providers, build_registry_config(&config));
    tracing::info!(
        instance_id = registry.instance_id(),
        public_url = ?registry.public_base_url(),
        "Provider registry initialized"
    );

    // Spawn background health checker.
    let health_interval = std::time::Duration::from_secs(config.health_check_interval_secs);
    let _health_handle = registry.spawn_health_checker(health_interval);
    tracing::info!(
        interval_secs = config.health_check_interval_secs,
        "Background RPC health checker started"
    );

    let gossip_interval = std::time::Duration::from_secs(config.gossip_interval_secs);
    let _gossip_handle = registry.spawn_gossip_task(gossip_interval);
    tracing::info!(
        interval_secs = config.gossip_interval_secs,
        "Provider gossip sync started"
    );

    let simulation_timeout = std::time::Duration::from_secs(config.simulation_timeout_secs);
    let simulation_mode = SimulationMode::from_config(&config.simulation_mode)
        .expect("Invalid simulation mode configuration");
    tracing::info!(
        timeout_secs = config.simulation_timeout_secs,
        "Simulation timeout configured"
    );
    tracing::info!(mode = ?simulation_mode, "Simulation mode configured");

    // ── Process-wide Stellar RPC service ────────────────────────────────
    // One shared reqwest::Client (connection pool) and one retry policy for
    // the entire process.  Every subsystem receives an Arc clone of this.
    let stellar_service = Arc::new(
        StellarService::new(
            Arc::clone(&registry),
            StellarServiceConfig::default().with_timeout(simulation_timeout),
        )
        .unwrap_or_else(|e| panic!("Failed to build Stellar HTTP client: {e}")),
    );

    for provider in &startup_providers {
        if let Err(error) = stellar_service
            .validate_network_passphrase(provider, &config.network_passphrase)
            .await
        {
            tracing::error!(
                provider = %provider.name,
                url = %provider.url,
                error = %error,
                "Stellar network validation failed at startup; refusing to initialize signing"
            );
            panic!("Stellar network validation failed: {}", error);
        }
    }
    tracing::info!("StellarService initialized (pooled client, retry, circuit-breaker)");

    // Construct signing state only after every configured RPC provider has
    // proved that it is connected to the expected Stellar network.
    let auth_state = Arc::new(auth::AuthState::new(
        config.jwt_private_key.clone(),
        None,
        config.network_passphrase.clone(),
        config.emergency_verification_paused,
    ));
    tracing::info!(
        "SEP-10 server account: {}",
        auth_state.server_stellar_address()
    );

    // ── Fee Market Setup ────────────────────────────────────────────────
    let database_url = &config.database_url;
    tracing::info!(database_url = %database_url, "Initializing database");

    let db_pool = db::init_pool(database_url)
        .await
        .expect("Failed to initialize database");
    tracing::info!("Database migrations completed");

    // Initialize typed DB schema for the managers, vaults, and reconciliation records.
    let db_schema = db::schema::TypedSchema::new(std::sync::Arc::new(db_pool.clone()));

    let fee_store = Arc::new(FeeStore::new(db_pool.clone()));
    let vault_store = Arc::new(vault_store::VaultStore::new(db_schema.vaults()));
    let manager_store = Arc::new(manager_store::ManagerStore::new(db_schema.managers()));
    let fee_analytics_engine = FeeAnalyticsEngine::new();

    let reconciliation_repo = db::reconciliation::ReconciliationRepo::with_redis(
        db_schema.reconciliation_reports(),
        db_schema.reconciliation_discrepancies(),
        &config.redis_url,
    )
    .expect("Failed to initialize reconciliation report cache");
    let reconciler = Arc::new(reconciliation::FeeReconciler::new(
        Arc::clone(&fee_store),
        reconciliation_repo.clone(),
    ));
    // API-28: business-logic service owns fee / billing math; wired into
    // AppState so the HTTP handlers stay thin.
    let fee_service = billing_service::FeeService::new(
        Arc::clone(&fee_store),
        fee_analytics_engine.clone(),
    );
    let job_queue_config = JobQueueConfig {
        job_timeout_secs: config.job_timeout_secs,
        max_concurrent_jobs: config.max_concurrent_jobs,
        ..JobQueueConfig::default()
    };
    let job_queue = JobQueue::new(database_url, &config.redis_url, job_queue_config.clone())
        .await
        .expect("Failed to initialize job queue");
    // ── WebSocket event bus ─────────────────────────────────────────────
    let simulation_bus = SimulationBus::new();
    let agent_fleet = Arc::new(agent_fleet::DefaultAgentFleet::new());
    if agent_fleet
        .register_with_threshold(agent_fleet::AgentIdentity::new("api-server".to_string()), 0)
        .is_ok()
    {
        agent_fleet.record_self_report("api-server");
    }

    let insights_cache = crate::cache::InsightsCache::new();
    let job_worker = JobWorker::new(
        job_queue.clone(),
        SimulationEngine::with_registry_and_timeout_and_mode(
            Arc::clone(&registry),
            simulation_timeout,
            simulation_mode,
        )
        .with_stellar_service(Arc::clone(&stellar_service)),
        InsightsEngine::new(),
        insights_cache.clone(),
        job_queue_config,
    )
    .with_bus(Arc::clone(&simulation_bus))
    .with_reconciler(Arc::clone(&reconciler));

    tokio::spawn(async move {
        job_worker.run().await;
    });

    // ── Distributed Job Queue Setup ─────────────────────────────────────
    let job_config = JobQueueConfig {
        job_timeout_secs: config.job_timeout_secs,
        max_concurrent_jobs: config.max_concurrent_jobs,
        ..Default::default()
    };

    let job_queue = JobQueue::new(&config.database_url, &config.redis_url, job_config.clone())
        .await
        .expect("Failed to initialize JobQueue");

    // Spawn background cleanup task
    job_queue.spawn_cleanup_task();

    // Spawn worker
    let worker = JobWorker::new(
        job_queue.clone(),
        SimulationEngine::with_registry_and_timeout(Arc::clone(&registry), simulation_timeout)
            .with_stellar_service(Arc::clone(&stellar_service)),
        InsightsEngine::new(),
        insights_cache.clone(),
        job_config,
    );

    tokio::spawn(async move {
        worker.run().await;
    });

    tracing::info!("Job queue and worker started (Redis backend)");

    // Start background fee collector if enabled
    if config.fee_analysis_enabled {
        let collector_config = FeeCollectorConfig {
            collection_interval_secs: config.fee_collection_interval_secs,
            batch_size: 10,
            request_timeout: std::time::Duration::from_secs(10),
        };

        let collector = Arc::new(
            FeeCollector::new(
                Arc::clone(&registry),
                Arc::clone(&fee_store),
                collector_config,
            )
            .unwrap_or_else(|e| panic!("Failed to build fee collector HTTP client: {e}")),
        );

        tokio::spawn(async move {
            collector.run_collection_loop().await;
        });

        tracing::info!(
            interval_secs = config.fee_collection_interval_secs,
            "Fee market collector started"
        );

        // Schedule periodic cleanup of old fee data
        let cleanup_store = Arc::clone(&fee_store);
        let retention_days = config.fee_retention_days;
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(3600)); // Every hour
            loop {
                interval.tick().await;
                if let Err(e) = cleanup_store
                    .cleanup_old_samples(retention_days as i32)
                    .await
                {
                    tracing::error!(error = %e, "Failed to cleanup old fee samples");
                }
            }
        });
    } else {
        tracing::info!("Fee market analysis is disabled");
    }

    // ── Persistent Cache Setup (L2) ─────────────────────────────────────
    let sled_db = sled::open("Perigee_cache").expect("Failed to open sled database");
    let simulation_cache = SimulationCache::new(&sled_db);
    let contract_cache = Arc::new(ContractCache::new(&sled_db));

    let app_state = Arc::new(AppState {
        engine: SimulationEngine::with_registry_and_cache(
            Arc::clone(&registry),
            Arc::clone(&contract_cache),
        )
        .with_stellar_service(Arc::clone(&stellar_service)),
        provider_registry: Arc::clone(&registry),
        stellar_service: Arc::clone(&stellar_service),
        cache: simulation_cache,
        insights_cache,
        insights_engine: InsightsEngine::new(),
        gas_golfing_analyzer: GasGolfingAnalyzer::new(),
        simulation_timeout,
        job_queue,
        fee_analytics_engine,
        fee_store,
        fee_service,
        metrics: Arc::new(AppMetrics::new().expect("Failed to initialize Prometheus metrics")),
        simulation_bus,
        reconciler,
        reconciliation_repo,
        vault_store,
        agent_fleet,
        manager_store,
        receipt_signer: receipt_signer.clone(),
        log_levels: log_levels.clone(),
    });

    let cors = build_cors_layer(&config.cors_allowed_origins);

    let protected = Router::new()
        .route("/analyze", post(analyze))
        .route("/analyze/wasm", post(analyze_wasm))
        .route("/analyze/wasm/branches", post(analyze_wasm_branches))
        .route("/analyze/optimize-limits", post(optimize_limits))
        .route("/analyze/compare", post(compare_handler))
        .route("/analyze/gas-golfing", post(analyze_gas_golfing))
        // Scoped token issuance for role- and vault-scoped delegation
        .route("/auth/scoped-token", post(auth::issue_scoped_token_handler))
        .route(
            "/admin/log-levels",
            get(get_log_levels).post(set_log_level),
        )
        .route(
            "/admin/log-levels/:module",
            axum::routing::delete(remove_log_level),
        )
        // Vault records with tenant-scoped access (API-37)
        .route("/vaults", get(vault_store::list_vaults_handler).post(vault_store::create_vault_handler))
        .route(
            "/vaults/:id",
            get(vault_store::get_vault_handler).patch(vault_store::update_vault_handler).delete(vault_store::soft_delete_vault_handler),
        )
        .route(
            "/vaults/:id/restore",
            post(vault_store::restore_vault_handler),
        )
        .route(
            "/admin/vaults/deleted",
            get(vault_store::list_deleted_vaults_handler),
        )
        .route_layer(axum::middleware::from_fn(auth::auth_middleware));

    let api_routes = Router::new()
        .route("/health", get(health_check))
        .route("/ready", get(ready_check))
        .route("/metrics", get(metrics_handler))
        .route("/auth/challenge", post(auth::challenge_handler))
        .route("/auth/verify", post(auth::verify_handler))
        .route("/auth/refresh", post(auth::refresh_handler))
        .route("/auth/revoke", post(auth::revoke_handler))
        .route("/auth/emergency-pause", post(auth::emergency_pause_handler))
        .route("/auth/jwks", get(auth::jwks_handler))
        // Fee market routes (public access)
        .route("/fees/recommend", get(fee_recommend))
        .route("/fees/history", get(fee_history))
        .route("/fees/analytics", get(fee_analytics))
        // Manager onboarding with approval/KYC gate (API-33)
        .route("/managers/register", post(manager_store::register_manager_handler))
        .route("/managers", get(manager_store::list_managers_handler))
        .route(
            "/managers/:id",
            get(manager_store::get_manager_handler),
        )
        .route(
            "/managers/:id/approve",
            post(manager_store::approve_manager_handler),
        )
        .route(
            "/managers/:id/reject",
            post(manager_store::reject_manager_handler),
        )
        .route(
            "/managers/status/:stellar_address",
            get(manager_store::check_manager_status_handler),
        )
        // Reconciliation routes (async via job queue)
        .route("/reconcile", post(reconciliation::reconcile_handler))
        .route(
            "/reconcile/reports",
            get(reconciliation::list_reports_handler),
        )
        .route(
            "/reconcile/:job_id",
            get(reconciliation::get_reconcile_job_handler),
        )
        // WebSocket streaming (Issue #105) — no auth required on the upgrade;
        // the client passes the job_id in the path.
        .route("/ws/jobs/:job_id", get(ws::ws_handler))
        .merge(protected);

    let app = Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route(
            "/",
            get(|| async {
                "Hello from Perigee! Usage: cargo run -p Perigee-core -- benchmark"
            }),
        )
        .nest("/v1", api_routes.clone())
        .merge(api_routes)
        // Catch-all fallback: return a structured JSON 404 instead of
        // axum's default plain-text body, which could expose framework
        // version strings or routing internals.
        .fallback(not_found_handler)
        .layer(Extension(auth_state))
        .layer(cors)
        .layer(axum::middleware::from_fn(
            crate::middleware::method_not_allowed_middleware,
        ))
        .layer(axum::middleware::from_fn(
            crate::middleware::receipt_middleware,
        ))
        .layer(axum::middleware::from_fn(
            crate::middleware::correlation_id_middleware,
        ))
        .layer(axum::middleware::from_fn(
            crate::middleware::request_cancellation_middleware,
        ))
        .layer(axum::middleware::from_fn(
            crate::middleware::api_version_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .layer(axum::middleware::from_fn(
            input_sanitization::sanitize_request_middleware,
        ))
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024 * 2)) // 2 MB limit
        .layer(Extension(receipt_signer.clone()))
        .with_state(app_state); // ← thread AppState through all handlers

    let bind_addr = format!("0.0.0.0:{}", config.server_port);
    let listener = tokio::net::TcpListener::bind(&bind_addr)
        .await
        .expect("Failed to bind to address");

    tracing::info!(
        "Server listening on http://{}",
        listener.local_addr().unwrap()
    );
    tracing::info!(
        "Swagger UI available at http://{}/swagger-ui",
        listener.local_addr().unwrap()
    );

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
        .expect("Server failed to start");
}

// ─────────────────────────────────────────────────────────────────────────────
// CORS unit tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod cors_tests {
    use super::build_cors_layer;

    /// Helper: ask the layer whether it would produce an `Allow-Origin` header
    /// for a given `Origin` request header value.  We inspect the layer via
    /// a synthetic request rather than spinning up a full server.
    fn allowed_origin(layer: &tower_http::cors::CorsLayer, origin: &str) -> bool {
        use axum::http::{header, Method, Request, Version};
        use tower::{Service, ServiceExt};
        use tower_http::cors::CorsLayer;

        // Build a minimal OPTIONS preflight request with the Origin header.
        let req = Request::builder()
            .method(Method::OPTIONS)
            .uri("http://localhost/")
            .version(Version::HTTP_11)
            .header(header::ORIGIN, origin)
            .header(header::ACCESS_CONTROL_REQUEST_METHOD, "GET")
            .body(axum::body::Body::empty())
            .unwrap();

        // We can't easily call an async Service in a sync test, but we can
        // inspect the CorsLayer's AllowOrigin by checking the
        // `Access-Control-Allow-Origin` header that tower-http would emit.
        // Instead we test the builder's structural output through the public
        // API — verifying the two code paths (Any vs list) produce distinct
        // CorsLayer values — and rely on the call-site integration test for
        // end-to-end validation.
        //
        // For a lightweight structural check we inspect the Debug output, which
        // differs between `Any` and `List(...)`.
        let dbg = format!("{:?}", layer);
        if is_any_origin(&dbg) {
            // Any-mode layer: every origin is "allowed" from its perspective.
            true
        } else {
            // List-mode layer: check if the origin string appears in the debug.
            dbg.contains(origin)
        }
    }

    /// Whether a `CorsLayer`'s debug rendering describes an allow-any policy.
    ///
    /// tower-http renders this as `allow_origin: Const("*")`, and older
    /// versions rendered it as `Any`. These tests inspect the Debug output
    /// because `AllowOrigin` exposes no predicate, so accept both spellings
    /// rather than pinning to one release's formatting.
    fn is_any_origin(dbg: &str) -> bool {
        dbg.contains("Any") || dbg.contains(r#"allow_origin: Const("*")"#)
    }

    #[test]
    fn empty_string_produces_any_origin() {
        let layer = build_cors_layer("");
        let dbg = format!("{:?}", layer);
        assert!(
            is_any_origin(&dbg),
            "Expected an allow-any origin policy, got: {dbg}"
        );
    }

    #[test]
    fn whitespace_only_produces_any_origin() {
        let layer = build_cors_layer("   ");
        let dbg = format!("{:?}", layer);
        assert!(
            is_any_origin(&dbg),
            "Expected an allow-any origin policy for whitespace input, got: {dbg}"
        );
    }

    #[test]
    fn single_origin_appears_in_layer() {
        let layer = build_cors_layer("https://partner.example.com");
        let dbg = format!("{:?}", layer);
        // The debug output should NOT be Any.
        assert!(
            !dbg.contains("Any"),
            "Expected list mode, got Any. Debug: {dbg}"
        );
    }

    #[test]
    fn multiple_origins_appear_in_layer() {
        let layer =
            build_cors_layer("https://partner-a.example.com,https://partner-b.example.com");
        let dbg = format!("{:?}", layer);
        assert!(
            !dbg.contains("Any"),
            "Expected list mode, got Any. Debug: {dbg}"
        );
    }

    #[test]
    fn origins_with_extra_whitespace_are_trimmed() {
        // Should not panic and should produce a list-mode layer.
        let layer = build_cors_layer("  https://a.example.com  ,  https://b.example.com  ");
        let dbg = format!("{:?}", layer);
        assert!(
            !dbg.contains("Any"),
            "Expected list mode after trimming, got Any. Debug: {dbg}"
        );
    }

    #[test]
    fn invalid_origin_is_skipped_and_does_not_panic() {
        // One valid, one invalid — should not panic; layer should be list-mode.
        let layer = build_cors_layer("https://valid.example.com,not a valid origin !!!");
        let dbg = format!("{:?}", layer);
        // At least one valid origin was parsed, so mode is list not Any.
        assert!(
            !dbg.contains("Any"),
            "Expected list mode for mixed valid/invalid, got Any. Debug: {dbg}"
        );
    }

    #[test]
    fn all_invalid_origins_produce_empty_list_not_any() {
        // If every entry is invalid the layer should be list-mode with an
        // empty list (deny all), not fall back to Any.
        let layer = build_cors_layer("not-a-valid-origin !!!, also bad !!!");
        let dbg = format!("{:?}", layer);
        assert!(
            !dbg.contains("Any"),
            "Expected empty-list mode for all-invalid origins, got Any. Debug: {dbg}"
        );
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(any())]
mod tests {
    use super::*;
    use crate::simulation::{SimulationError, SorobanResources};

    #[test]
    fn test_error_mapping_node_error() {
        let sim_err = SimulationError::NodeError("Invalid contract ID".to_string());
        let app_err: AppError = sim_err.into();

        match app_err {
            AppError::BadRequest(msg) => {
                assert!(msg.contains("Invalid contract ID"));
            }
            _ => panic!("Expected BadRequest, got {:?}", app_err),
        }
    }

    #[test]
    fn test_error_mapping_invalid_contract() {
        let sim_err = SimulationError::InvalidContract("Contract not found".to_string());
        let app_err: AppError = sim_err.into();

        match app_err {
            AppError::BadRequest(msg) => {
                assert!(msg.contains("Contract not found"));
            }
            _ => panic!("Expected BadRequest, got {:?}", app_err),
        }
    }

    #[test]
    fn test_error_mapping_timeout() {
        let sim_err = SimulationError::NodeTimeout;
        let app_err: AppError = sim_err.into();

        match app_err {
            AppError::Internal(msg) => {
                assert!(msg.contains("timed out"));
            }
            _ => panic!("Expected Internal, got {:?}", app_err),
        }
    }

    #[test]
    fn test_error_mapping_rpc_request_failed() {
        let sim_err = SimulationError::RpcRequestFailed("Connection refused".to_string());
        let app_err: AppError = sim_err.into();

        match app_err {
            AppError::Internal(msg) => {
                assert!(msg.contains("Connection refused"));
            }
            _ => panic!("Expected Internal, got {:?}", app_err),
        }
    }

    #[test]
    fn test_error_mapping_network_error() {
        // Create a mock reqwest error (we can't easily create one, so test via RpcRequestFailed)
        let sim_err = SimulationError::RpcRequestFailed("Network unreachable".to_string());
        let app_err: AppError = sim_err.into();

        match app_err {
            AppError::Internal(msg) => {
                assert!(msg.contains("Network unreachable"));
            }
            _ => panic!("Expected Internal, got {:?}", app_err),
        }
    }

    #[test]
    fn test_resource_report_includes_cost_stroops() {
        let sim_result = SimulationResult {
            resources: SorobanResources {
                cpu_instructions: 1000000,
                ram_bytes: 2048,
                ledger_read_bytes: 512,
                ledger_write_bytes: 256,
                transaction_size_bytes: 1024,
            },
            transaction_hash: None,
            latest_ledger: 12345,
            cost_stroops: 5000,
            state_dependency: None,
            ttl_analysis: None,
            transaction_data: "AAA".to_string(),
            call_graph: None,
            state_snapshot: None,
            protocol_version: 0,
        };

        let insights_engine = InsightsEngine::new();
        let report = to_report(&sim_result, &insights_engine, None);

        assert_eq!(report.cost_stroops, 5000);
        assert_eq!(report.cpu_instructions, 1000000);
        assert_eq!(report.ram_bytes, 2048);
        assert_eq!(report.ledger_read_bytes, 512);
        assert_eq!(report.ledger_write_bytes, 256);
        assert_eq!(report.transaction_size_bytes, 1024);
    }

    #[test]
    fn test_app_config_default_simulation_timeout() {
        // Verify the default timeout function returns 30 seconds
        assert_eq!(default_simulation_timeout_secs(), 30);
    }

    #[test]
    fn test_app_config_default_simulation_mode() {
        assert_eq!(default_simulation_mode(), "failover");
    }

    #[test]
    fn test_simulation_engine_timeout_configurable() {
        use std::time::Duration;

        // Create a mock registry (we can't easily create one without mocking)
        // Instead, test that the SimulationEngine has timeout methods
        let engine = SimulationEngine::new("https://test.com".to_string());

        // Default should be 30 seconds
        assert_eq!(engine.timeout(), Duration::from_secs(30));
    }
    // ── API integration tests for /analyze/wasm/profile ──────────────────────

    /// Build a minimal valid WASM module with one exported function `add` that
    /// returns i32 (i32.const 42; end). Mirrors the helper in simulation.rs.
    fn minimal_wasm_bytes() -> Vec<u8> {
        use wasm_encoder::{
            CodeSection, ExportKind, ExportSection, Function, FunctionSection, Module, TypeSection,
            ValType,
        };
        let mut module = Module::new();
        let mut types = TypeSection::new();
        types.ty().function([], [ValType::I32]);
        module.section(&types);
        let mut functions = FunctionSection::new();
        functions.function(0);
        module.section(&functions);
        let mut exports = ExportSection::new();
        exports.export("add", ExportKind::Func, 0);
        module.section(&exports);
        let mut codes = CodeSection::new();
        let mut f = Function::new(vec![]);
        f.instruction(&wasm_encoder::Instruction::I32Const(42));
        f.instruction(&wasm_encoder::Instruction::End);
        codes.function(&f);
        module.section(&codes);
        module.finish()
    }

    fn build_test_app() -> Router {
        use std::sync::Arc;
        let app_state = Arc::new(AppState {
            engine: SimulationEngine::new("https://test.example.com".to_string()),
            cache: SimulationCache::new(),
            insights_cache: crate::cache::InsightsCache::new(),
            insights_engine: InsightsEngine::new(),
            simulation_timeout: std::time::Duration::from_secs(30),
        });
        let auth_state = Arc::new(auth::AuthState::new(
            "test-secret".to_string(),
            None,
            "Test SDF Network ; September 2015".to_string(),
        ));
        let protected = Router::new()
            .route("/analyze/wasm/profile", post(analyze_wasm_profile))
            .route_layer(axum::middleware::from_fn(auth::auth_middleware));
        Router::new()
            .merge(protected)
            .layer(Extension(auth_state))
            .with_state(app_state)
    }

    fn make_jwt(secret: &str) -> String {
        use jsonwebtoken::{encode, EncodingKey, Header};
        use serde_json::json;
        let claims = json!({
            "sub": "test-user",
            "exp": 9999999999u64,
        });
        encode(
            &Header::default(),
            &claims,
            &EncodingKey::from_secret(secret.as_bytes()),
        )
        .unwrap()
    }

    #[tokio::test]
    async fn test_profile_endpoint_valid_request_returns_200() {
        use axum::body::Body;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        use http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_test_app();
        let wasm_b64 = BASE64.encode(minimal_wasm_bytes());
        let body = serde_json::json!({
            "wasm_bytes": wasm_b64,
            "function_name": "add",
            "args": []
        });
        let token = make_jwt("test-secret");
        let req = Request::builder()
            .method("POST")
            .uri("/analyze/wasm/profile")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", token))
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn test_profile_endpoint_invalid_base64_returns_400() {
        use axum::body::Body;
        use http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_test_app();
        let body = serde_json::json!({
            "wasm_bytes": "!!!not-valid-base64!!!",
            "function_name": "add",
            "args": []
        });
        let token = make_jwt("test-secret");
        let req = Request::builder()
            .method("POST")
            .uri("/analyze/wasm/profile")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", token))
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_profile_endpoint_invalid_wasm_returns_400() {
        use axum::body::Body;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        use http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_test_app();
        let bad_wasm = BASE64.encode(b"this is not wasm");
        let body = serde_json::json!({
            "wasm_bytes": bad_wasm,
            "function_name": "add",
            "args": []
        });
        let token = make_jwt("test-secret");
        let req = Request::builder()
            .method("POST")
            .uri("/analyze/wasm/profile")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", token))
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_profile_endpoint_unknown_function_returns_400() {
        use axum::body::Body;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        use http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_test_app();
        let wasm_b64 = BASE64.encode(minimal_wasm_bytes());
        let body = serde_json::json!({
            "wasm_bytes": wasm_b64,
            "function_name": "nonexistent_function",
            "args": []
        });
        let token = make_jwt("test-secret");
        let req = Request::builder()
            .method("POST")
            .uri("/analyze/wasm/profile")
            .header("content-type", "application/json")
            .header("authorization", format!("Bearer {}", token))
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn test_profile_endpoint_no_jwt_returns_401() {
        use axum::body::Body;
        use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
        use http::{Request, StatusCode};
        use tower::ServiceExt;

        let app = build_test_app();
        let wasm_b64 = BASE64.encode(minimal_wasm_bytes());
        let body = serde_json::json!({
            "wasm_bytes": wasm_b64,
            "function_name": "add",
            "args": []
        });
        let req = Request::builder()
            .method("POST")
            .uri("/analyze/wasm/profile")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }
}

async fn analyze_simulation(
    State(simulation_service): State<Arc<SimulationService>>,
    SanitizedJson(metric): SanitizedJson<SimulationMetric>,
) -> Result<Json<AnalysisResult>, AppError> {
    let result = simulation_service.record_and_analyze(metric).await?;
    Ok(Json(result))
}

// ── Unit tests for validate_config_secrets (#85 / NF-03) ────────────────────

#[cfg(test)]
mod validate_config_secrets_tests {
    use super::*;

    /// Build a baseline config whose secrets are *valid* so each test only
    /// invalidates one field at a time.
    fn good_config() -> AppConfig {
        AppConfig {
            app_env: "test".to_string(),
            server_port: 8080,
            rust_log: "info".to_string(),
            soroban_rpc_url: "https://soroban-testnet.stellar.org".to_string(),
            jwt_private_key: None,
            network_passphrase: "Test SDF Network ; September 2015".to_string(),
            redis_url: String::new(),
            rpc_providers: String::new(),
            registry_instance_id: String::new(),
            registry_public_url: String::new(),
            registry_seed_peers: String::new(),
            health_check_interval_secs: 30,
            gossip_interval_secs: 30,
            simulation_timeout_secs: 30,
            simulation_mode: "failover".to_string(),
            database_url: "sqlite://Perigee.db".to_string(),
            job_timeout_secs: 300,
            max_concurrent_jobs: 10,
            fee_collection_interval_secs: 5,
            fee_retention_days: 30,
            fee_analysis_enabled: true,
            emergency_verification_paused: false,
            disk_cache_path: String::new(),
            max_ledger_age: 100,
            cors_allowed_origins: String::new(),
        }
    }

    #[test]
    fn accepts_well_formed_config() {
        let cfg = good_config();
        assert!(validate_config_secrets(&cfg).is_ok());
    }

    #[test]
    fn rejects_empty_soroban_rpc_url() {
        let mut cfg = good_config();
        cfg.soroban_rpc_url = "".to_string();
        let err = validate_config_secrets(&cfg).unwrap_err();
        assert!(err.contains("SOROBAN_RPC_URL"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_malformed_soroban_rpc_url() {
        let mut cfg = good_config();
        cfg.soroban_rpc_url = "not-a-url".to_string();
        let err = validate_config_secrets(&cfg).unwrap_err();
        assert!(err.contains("SOROBAN_RPC_URL"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_empty_network_passphrase() {
        let mut cfg = good_config();
        cfg.network_passphrase = "   ".to_string();
        let err = validate_config_secrets(&cfg).unwrap_err();
        assert!(err.contains("NETWORK_PASSPHRASE"), "unexpected error: {err}");
    }

    #[test]
    fn accepts_rpc_providers_with_valid_urls() {
        let mut cfg = good_config();
        cfg.rpc_providers = serde_json::json!([
            {"name": "stellar-testnet", "url": "https://soroban-testnet.stellar.org"},
        ])
        .to_string();
        assert!(validate_config_secrets(&cfg).is_ok());
    }

    #[test]
    fn rejects_malformed_rpc_providers_json() {
        let mut cfg = good_config();
        cfg.rpc_providers = "{not json".to_string();
        let err = validate_config_secrets(&cfg).unwrap_err();
        assert!(err.contains("RPC_PROVIDERS"), "unexpected error: {err}");
        assert!(err.contains("JSON"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_rpc_provider_with_invalid_url() {
        let mut cfg = good_config();
        cfg.rpc_providers = serde_json::json!([
            {"name": "broken", "url": "definitely-not-a-url"},
        ])
        .to_string();
        let err = validate_config_secrets(&cfg).unwrap_err();
        assert!(err.contains("RPC_PROVIDERS"), "unexpected error: {err}");
        assert!(err.contains("broken"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_rpc_provider_with_empty_name() {
        let mut cfg = good_config();
        cfg.rpc_providers = serde_json::json!([
            {"name": "", "url": "https://example.com"},
        ])
        .to_string();
        let err = validate_config_secrets(&cfg).unwrap_err();
        assert!(err.contains("name must not be empty"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_invalid_registry_public_url() {
        let mut cfg = good_config();
        cfg.registry_public_url = "just-a-string".to_string();
        let err = validate_config_secrets(&cfg).unwrap_err();
        assert!(err.contains("REGISTRY_PUBLIC_URL"), "unexpected error: {err}");
    }

    #[test]
    fn rejects_invalid_seed_peer_urls() {
        let mut cfg = good_config();
        // parse_seed_peers accepts comma-separated values too.
        cfg.registry_seed_peers = "https://peer-a.example.com,not-a-url".to_string();
        let err = validate_config_secrets(&cfg).unwrap_err();
        assert!(err.contains("REGISTRY_SEED_PEERS"), "unexpected error: {err}");
        assert!(err.contains("not-a-url"), "unexpected error: {err}");
    }

    #[test]
    fn accepts_valid_seed_peer_json_array() {
        let mut cfg = good_config();
        cfg.registry_seed_peers =
            serde_json::json!(["https://peer-a.example.com", "https://peer-b.example.com"])
                .to_string();
        assert!(validate_config_secrets(&cfg).is_ok());
    }
}

#[cfg(test)]
mod openapi_tests {
    use super::*;

    #[test]
    fn openapi_spec_generates_valid_json_with_security_and_paths() {
        let openapi = ApiDoc::openapi();
        let json = serde_json::to_string(&openapi).expect("OpenAPI to JSON failed");

        assert!(json.contains("bearerAuth"), "OpenAPI spec should contain bearerAuth scheme");
        assert!(json.contains("jwt"), "OpenAPI spec should contain jwt scheme");
        assert!(json.contains("/analyze"), "OpenAPI spec should contain /analyze path");
        assert!(json.contains("/vaults"), "OpenAPI spec should contain /vaults path");
        assert!(json.contains("/fees/analytics"), "OpenAPI spec should contain /fees/analytics path");
        assert!(json.contains("/managers"), "OpenAPI spec should contain /managers path");
        assert!(json.contains("/reconcile"), "OpenAPI spec should contain /reconcile path");
    }
}

