mod auth;
mod benchmarks;
mod comparison;
mod errors;
pub mod fee_analytics;
pub mod fee_collector;
pub mod fee_store;
pub mod insights;
mod jobs;
mod parser;
pub mod rpc_provider;
mod simulation;

use crate::comparison::{CompareMode, RegressionFlag, RegressionReport, ResourceDelta};
use crate::errors::AppError;
use crate::fee_analytics::{FeeAnalyticsEngine, MarketConditions, ModelBreakdown};
use crate::fee_collector::{FeeCollector, FeeCollectorConfig};
use crate::fee_store::FeeStore;
use crate::insights::InsightsEngine;
use crate::jobs::{
    JobId, JobQueue, JobQueueConfig, JobWorker, SubmitJobRequest, SubmitJobResponse,
};
use crate::rpc_provider::{ProviderRegistry, RpcProvider};
use crate::simulation::{SimulationCache, SimulationEngine, SimulationResult};
use axum::{
    extract::{Json, Multipart, Path, State},
    http::{HeaderMap, HeaderName, HeaderValue},
    middleware,
    routing::{get, post},
    Extension, Router,
};
use config::{Config, ConfigError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::path::PathBuf;
use std::sync::Arc;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

#[derive(Debug, Deserialize)]
#[allow(dead_code)]
struct AppConfig {
    server_port: u16,
    rust_log: String,
    /// Primary RPC URL — used as a single-provider fallback when
    /// `RPC_PROVIDERS` is not set.
    soroban_rpc_url: String,
    jwt_secret: String,
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
    /// Health-check interval in seconds (default 30).
    #[serde(default = "default_health_check_interval")]
    health_check_interval_secs: u64,
    /// Simulation timeout in seconds (default 30).
    #[serde(default = "default_simulation_timeout_secs")]
    simulation_timeout_secs: u64,
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
}

fn default_health_check_interval() -> u64 {
    30
}

fn default_simulation_timeout_secs() -> u64 {
    30
}

fn default_database_url() -> String {
    "sqlite://soroscope.db".to_string()
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

fn load_config() -> Result<AppConfig, ConfigError> {
    dotenvy::dotenv().ok();

    let settings = Config::builder()
        .add_source(config::Environment::default())
        .set_default("server_port", 8080)?
        .set_default("rust_log", "info")?
        .set_default("soroban_rpc_url", "https://soroban-testnet.stellar.org")?
        .set_default("jwt_secret", "dev-secret-change-in-production")?
        .set_default("network_passphrase", "Test SDF Network ; September 2015")?
        .set_default("redis_url", "redis://127.0.0.1:6379")?
        .set_default("rpc_providers", "")?
        .set_default("health_check_interval_secs", 30)?
        .set_default("simulation_timeout_secs", 30)?
        .set_default("database_url", "sqlite://soroscope.db")?
        .set_default("job_timeout_secs", 300)?
        .set_default("max_concurrent_jobs", 10)?
        .set_default("fee_collection_interval_secs", 5)?
        .set_default("fee_retention_days", 30)?
        .set_default("fee_analysis_enabled", true)?
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
    }]
}

/// Shared application state injected into every Axum handler via [`State`].
struct AppState {
    engine: SimulationEngine,
    cache: Arc<SimulationCache>,
    insights_engine: InsightsEngine,
    /// Simulation timeout for RPC requests
    simulation_timeout: std::time::Duration,
    /// Job queue for background task processing
    job_queue: JobQueue,
    /// Fee market analytics engine
    fee_analytics_engine: FeeAnalyticsEngine,
    /// Fee data store
    fee_store: Arc<FeeStore>,
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
    pub safety_margin: f64,
}

fn default_safety_margin() -> f64 {
    0.05
}

#[derive(Serialize, ToSchema)]
pub struct OptimizeLimitsResponse {
    pub cpu: crate::simulation::OptimizationBuffer,
    pub ram: crate::simulation::OptimizationBuffer,
    pub recommended: crate::simulation::SorobanResources,
}

// ── Fee Market Types ─────────────────────────────────────────────────────

/// Request body for fee recommendation endpoint
#[derive(Debug, Deserialize, ToSchema)]
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
    /// Confidence in inclusion (0.0-1.0)
    pub inclusion_confidence: f64,
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
#[derive(Debug, Deserialize, ToSchema)]
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
}

/// Convert a `SimulationResult` (library type) into the API `ResourceReport`.
fn to_report(result: &SimulationResult, insights_engine: &InsightsEngine) -> ResourceReport {
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
        ttl_analysis: result.ttl_analysis.as_ref().map(|ttl| TtlAnalysisApiReport {
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
        ("jwt" = [])
    ),
    tag = "Analysis"
)]
async fn analyze(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AnalyzeRequest>,
) -> Result<(HeaderMap, Json<ResourceReport>), AppError> {
    // Create a tracing span with structured fields for this request
    let span = tracing::info_span!(
        "analyze",
        contract_id = %payload.contract_id,
        function_name = %payload.function_name,
    );
    let _enter = span.enter();

    tracing::info!("Received analyze request");

    let args = payload.args.clone().unwrap_or_default();
    let cache_key =
        SimulationCache::generate_key(&payload.contract_id, &payload.function_name, &args);

    // Track simulation latency
    let start_time = std::time::Instant::now();

    let (result, cache_status): (SimulationResult, &'static str) =
        if let Some(cached) = state.cache.get(&cache_key).await {
            tracing::debug!("Cache HIT for key: {}", cache_key);
            (cached, "HIT")
        } else {
            tracing::debug!("Cache MISS for key: {}", cache_key);

            // Wrap the simulation call with a timeout to prevent hanging
            let sim_result = tokio::time::timeout(
                state.simulation_timeout,
                state.engine.simulate_from_contract_id(
                    &payload.contract_id,
                    &payload.function_name,
                    args,
                    payload.ledger_overrides.clone(),
                ),
            )
            .await
            .map_err(|_| {
                tracing::error!("Simulation timed out after {:?}", state.simulation_timeout);
                AppError::Internal(format!(
                    "Simulation timed out after {} seconds",
                    state.simulation_timeout.as_secs()
                ))
            })?;

            let sim: SimulationResult = sim_result?;
            state.cache.set(cache_key, sim.clone()).await;
            (sim, "MISS")
        };

    let latency_ms = start_time.elapsed().as_millis() as u64;

    // Log comprehensive simulation metrics
    tracing::info!(
        latency_ms = latency_ms,
        cache_status = cache_status,
        cpu_instructions = result.resources.cpu_instructions,
        ram_bytes = result.resources.ram_bytes,
        ledger_read_bytes = result.resources.ledger_read_bytes,
        ledger_write_bytes = result.resources.ledger_write_bytes,
        transaction_size_bytes = result.resources.transaction_size_bytes,
        cost_stroops = result.cost_stroops,
        latest_ledger = result.latest_ledger,
        "Simulation completed successfully"
    );

    state.cache.log_stats();

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-soroscope-cache"),
        HeaderValue::from_static(cache_status),
    );
    headers.insert(
        HeaderName::from_static("x-soroscope-latency-ms"),
        HeaderValue::from_str(&latency_ms.to_string())
            .unwrap_or_else(|_| HeaderValue::from_static("0")),
    );

    Ok((headers, Json(to_report(&result, &state.insights_engine))))
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
        ("jwt" = [])
    ),
    tag = "Analysis"
)]
async fn analyze_wasm(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AnalyzeWasmRequest>,
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

    let resources = tokio::task::spawn_blocking(move || {
        simulation::profile_contract(wasm_bytes, function_name, args)
    })
    .await
    .map_err(|e| AppError::Internal(format!("Contract profiling task panicked: {}", e)))?
    .map_err(|e| AppError::Internal(format!("Contract profiling failed: {}", e)))?;

    let sim_result = simulation::SimulationResult {
        resources,
        transaction_hash: None,
        latest_ledger: 0,
        cost_stroops: 0,
        state_dependency: None,
        ttl_analysis: None,
        transaction_data: String::new(),
    };

    Ok(Json(to_report(&sim_result, &state.insights_engine)))
}

#[utoipa::path(
    post,
    path = "/analyze/optimize-limits",
    request_body = OptimizeLimitsRequest,
    responses(
        (status = 200, description = "Resource optimization successful", body = OptimizeLimitsResponse),
        (status = 500, description = "Optimization failed")
    ),
    tag = "Analysis"
)]
async fn optimize_limits(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<OptimizeLimitsRequest>,
) -> Result<Json<OptimizeLimitsResponse>, AppError> {
    tracing::info!(
        "Optimizing limits for contract: {}, function: {}",
        payload.contract_id,
        payload.function_name
    );

    let report = state
        .engine
        .optimize_limits(
            &payload.contract_id,
            &payload.function_name,
            payload.args,
            payload.safety_margin,
        )
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(OptimizeLimitsResponse {
        cpu: report.cpu,
        ram: report.ram,
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
        (status = 500, description = "Comparison failed")
    ),
    tag = "Analysis"
)]
async fn compare_handler(
    State(state): State<Arc<AppState>>,
    mut multipart: Multipart,
) -> Result<Json<CompareApiResponse>, AppError> {
    let mut mode_str: Option<String> = None;
    let mut current_wasm_bytes: Option<Vec<u8>> = None;
    let mut base_wasm_bytes: Option<Vec<u8>> = None;
    let mut contract_id: Option<String> = None;
    let mut function_name: Option<String> = None;
    let mut args: Vec<String> = Vec::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(format!("Failed to read multipart field: {}", e)))?
    {
        let name = field.name().unwrap_or_default().to_string();
        match name.as_str() {
            "mode" => {
                mode_str = Some(
                    field
                        .text()
                        .await
                        .map_err(|e| AppError::BadRequest(format!("Invalid mode field: {}", e)))?,
                );
            }
            "current_wasm" => {
                current_wasm_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| {
                            AppError::BadRequest(format!("Failed to read current_wasm: {}", e))
                        })?
                        .to_vec(),
                );
            }
            "base_wasm" => {
                base_wasm_bytes = Some(
                    field
                        .bytes()
                        .await
                        .map_err(|e| {
                            AppError::BadRequest(format!("Failed to read base_wasm: {}", e))
                        })?
                        .to_vec(),
                );
            }
            "contract_id" => {
                contract_id =
                    Some(field.text().await.map_err(|e| {
                        AppError::BadRequest(format!("Invalid contract_id: {}", e))
                    })?);
            }
            "function_name" => {
                function_name =
                    Some(field.text().await.map_err(|e| {
                        AppError::BadRequest(format!("Invalid function_name: {}", e))
                    })?);
            }
            "args" => {
                let args_json = field
                    .text()
                    .await
                    .map_err(|e| AppError::BadRequest(format!("Invalid args: {}", e)))?;
                args = serde_json::from_str(&args_json).unwrap_or_default();
            }
            _ => { /* ignore unknown fields */ }
        }
    }

    let mode = mode_str.unwrap_or_else(|| "local_vs_local".to_string());

    let compare_mode = match mode.as_str() {
        "local_vs_local" => {
            let current_bytes = current_wasm_bytes
                .ok_or_else(|| AppError::BadRequest("Missing current_wasm file".to_string()))?;
            let base_bytes = base_wasm_bytes
                .ok_or_else(|| AppError::BadRequest("Missing base_wasm file".to_string()))?;

            let current_tmp = write_temp_wasm(&current_bytes)?;
            let base_tmp = write_temp_wasm(&base_bytes)?;

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

    let report = comparison::run_comparison(&state.engine, compare_mode)
        .await
        .map_err(|e| AppError::Internal(format!("Comparison failed: {}", e)))?;

    Ok(Json(CompareApiResponse { report }))
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
) -> Result<Json<FeeRecommendationResponse>, AppError> {
    use crate::fee_analytics::TrendDirection;

    tracing::info("Generating fee recommendation");

    // Get recent samples for analysis
    let samples = state
        .fee_store
        .get_recent_samples(100)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch fee data: {}", e)))?;

    // Get current ledger from latest sample or use 0
    let current_ledger = samples
        .first()
        .map(|s| s.ledger_sequence as u64)
        .unwrap_or(0);

    // Generate prediction
    let prediction = state.fee_analytics_engine.predict(&samples, current_ledger);
    let market_conditions = state.fee_analytics_engine.get_market_conditions(&samples, current_ledger);
    let model_breakdown = state.fee_analytics_engine.get_model_breakdown(&samples);

    // Determine recommended bid based on prediction
    let (recommended_bid, expected_ledgers) = (
        prediction.priority_bid,
        1,
    );

    Ok(Json(FeeRecommendationResponse {
        recommended_bid,
        resource_fee_estimate: 0, // Will be calculated based on transaction resources
        total_estimated_cost: recommended_bid,
        inclusion_confidence: prediction.confidence_score,
        expected_inclusion_ledgers: expected_ledgers,
        market_conditions,
        model_breakdown,
        timestamp: chrono::Utc::now(),
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
) -> Result<Json<FeeHistoryResponse>, AppError> {
    tracing::info("Fetching fee history");

    let limit = 50; // Default limit
    let samples = state
        .fee_store
        .get_recent_samples(limit)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch fee history: {}", e)))?;

    let total_count = state
        .fee_store
        .get_sample_count()
        .await
        .map_err(|e| AppError::Internal(format!("Failed to get sample count: {}", e)))?;

    Ok(Json(FeeHistoryResponse {
        samples,
        total_count,
    }))
}

#[utoipa::path(
    get,
    path = "/fees/analytics",
    responses(
        (status = 200, description = "Fee analytics retrieved successfully", body = serde_json::Value),
        (status = 500, description = "Failed to fetch analytics")
    ),
    tag = "Fee Market"
)]
async fn fee_analytics(
    State(state): State<Arc<AppState>>,
) -> Result<Json<serde_json::Value>, AppError> {
    tracing::info("Fetching fee analytics");

    // Get recent samples for analysis
    let samples = state
        .fee_store
        .get_recent_samples(200)
        .await
        .map_err(|e| AppError::Internal(format!("Failed to fetch fee data: {}", e)))?;

    let current_ledger = samples
        .first()
        .map(|s| s.ledger_sequence as u64)
        .unwrap_or(0);

    let prediction = state.fee_analytics_engine.predict(&samples, current_ledger);
    let market_conditions = state.fee_analytics_engine.get_market_conditions(&samples, current_ledger);
    let model_breakdown = state.fee_analytics_engine.get_model_breakdown(&samples);

    let response = serde_json::json!({
        "current_ledger": current_ledger,
        "prediction": prediction,
        "market_conditions": market_conditions,
        "model_breakdown": model_breakdown,
        "sample_count": samples.len(),
        "timestamp": chrono::Utc::now(),
    });

    Ok(Json(response))
}

#[derive(OpenApi)]
#[openapi(
    paths(
        analyze, analyze_wasm, optimize_limits, compare_handler,
        auth::challenge_handler, auth::verify_handler,
        fee_recommend, fee_history, fee_analytics
    ),
    components(schemas(
        AnalyzeRequest, AnalyzeWasmRequest, ResourceReport,
        OptimizeLimitsRequest, OptimizeLimitsResponse,
        CompareApiResponse, RegressionReport, ResourceDelta, RegressionFlag,
        auth::ChallengeRequest, auth::ChallengeResponse,
        auth::VerifyRequest, auth::VerifyResponse,
        crate::simulation::OptimizationBuffer,
        crate::simulation::SorobanResources,
        FeeRecommendationRequest, FeeRecommendationResponse,
        FeeHistoryRequest, FeeHistoryResponse,
        crate::fee_store::LedgerFeeSample,
        crate::fee_analytics::MarketConditions,
        crate::fee_analytics::ModelBreakdown,
        crate::fee_analytics::TrendDirection
    )),
    tags(
        (name = "Analysis", description = "Soroban contract resource analysis endpoints"),
        (name = "Auth", description = "SEP-10 wallet authentication"),
        (name = "Fee Market", description = "Stellar/Soroban fee market analysis and prediction")
    ),
    info(
        title = "SoroScope API",
        version = "0.1.0",
        description = "API for analyzing Soroban smart contract resource consumption and fee market predictions"
    )
)]
struct ApiDoc;

async fn health_check() -> &'static str {
    "OK"
}

#[tokio::main]
async fn main() {
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("SoroScope Starting...");

    let config = load_config().expect("Failed to load configuration");
    tracing::info!("SoroScope initialized with config: {:?}", config);
    tracing::info!(
        redis_url = %config.redis_url,
        "Cache config: using in-memory (moka) MVP; Redis URL reserved for future migration"
    );

    let args: Vec<String> = env::args().collect();

    if args.len() > 1 && args[1] == "benchmark" {
        tracing::info!("Starting SoroScope Benchmark...");

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
            if let Err(e) = benchmarks::run_token_benchmark(path) {
                tracing::error!("Benchmark failed: {}", e);
            }
        } else {
            tracing::error!(
                "Could not find soroban_token_contract.wasm. Build the contract first."
            );
        }

        return;
    }

    // ── CLI: compare subcommand ──────────────────────────────────────────
    if args.len() > 1 && args[1] == "compare" {
        if args.len() < 4 {
            eprintln!("Usage: soroscope-core compare <current.wasm> <base.wasm>");
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

    tracing::info!("Starting SoroScope API Server...");

    let auth_state = Arc::new(auth::AuthState::new(
        config.jwt_secret.clone(),
        None,
        config.network_passphrase.clone(),
    ));
    tracing::info!(
        "SEP-10 server account: {}",
        auth_state.server_stellar_address()
    );
    // ── Multi-node RPC setup ────────────────────────────────────────────
    let providers = build_providers(&config);
    let provider_names: Vec<&str> = providers.iter().map(|p| p.name.as_str()).collect();
    tracing::info!(providers = ?provider_names, "RPC provider pool");

    let registry = ProviderRegistry::new(providers);

    // Spawn background health checker.
    let health_interval = std::time::Duration::from_secs(config.health_check_interval_secs);
    let _health_handle = registry.spawn_health_checker(health_interval);
    tracing::info!(
        interval_secs = config.health_check_interval_secs,
        "Background RPC health checker started"
    );

    let simulation_timeout = std::time::Duration::from_secs(config.simulation_timeout_secs);
    tracing::info!(
        timeout_secs = config.simulation_timeout_secs,
        "Simulation timeout configured"
    );

    // ── Fee Market Setup ────────────────────────────────────────────────
    let database_url = &config.database_url;
    tracing::info!(database_url = %database_url, "Initializing database");

    let db_pool = sqlx::SqlitePool::connect(database_url)
        .await
        .expect("Failed to connect to database");

    // Run migrations
    sqlx::migrate!()
        .run(&db_pool)
        .await
        .expect("Failed to run database migrations");

    tracing::info!("Database migrations completed");

    let fee_store = Arc::new(FeeStore::new(db_pool.clone()));
    let fee_analytics_engine = FeeAnalyticsEngine::new();

    // Start background fee collector if enabled
    if config.fee_analysis_enabled {
        let collector_config = FeeCollectorConfig {
            collection_interval_secs: config.fee_collection_interval_secs,
            batch_size: 10,
            request_timeout: std::time::Duration::from_secs(10),
        };

        let collector = Arc::new(FeeCollector::new(
            Arc::clone(&registry),
            Arc::clone(&fee_store),
            collector_config,
        ));

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
                if let Err(e) = cleanup_store.cleanup_old_samples(retention_days as i32).await {
                    tracing::error!(error = %e, "Failed to cleanup old fee samples");
                }
            }
        });
    } else {
        tracing::info!("Fee market analysis is disabled");
    }

    let app_state = Arc::new(AppState {
        engine: SimulationEngine::with_registry_and_timeout(
            Arc::clone(&registry),
            simulation_timeout,
        ),
        cache: SimulationCache::new(),
        insights_engine: InsightsEngine::new(),
        simulation_timeout,
        fee_analytics_engine,
        fee_store,
    });

    let cors = CorsLayer::new().allow_origin(Any);

    let protected = Router::new()
        .route("/analyze", post(analyze))
        .route("/analyze/wasm", post(analyze_wasm))
        .route("/analyze/optimize-limits", post(optimize_limits))
        .route("/analyze/compare", post(compare_handler))
        .route_layer(middleware::from_fn(auth::auth_middleware));

    let app = Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route(
            "/",
            get(|| async {
                "Hello from SoroScope! Usage: cargo run -p soroscope-core -- benchmark"
            }),
        )
        .route("/health", get(health_check))
        .route("/auth/challenge", post(auth::challenge_handler))
        .route("/auth/verify", post(auth::verify_handler))
        // Fee market routes (public access)
        .route("/fees/recommend", get(fee_recommend))
        .route("/fees/history", get(fee_history))
        .route("/fees/analytics", get(fee_analytics))
        .merge(protected)
        .layer(Extension(auth_state))
        .layer(cors)
        .layer(TraceLayer::new_for_http())
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
        .await
        .expect("Server failed to start");
}

// ─────────────────────────────────────────────────────────────────────────────
// Integration Tests
// ─────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
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
        };

        let insights_engine = InsightsEngine::new();
        let report = to_report(&sim_result, &insights_engine);

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
    fn test_simulation_engine_timeout_configurable() {
        use std::time::Duration;

        // Create a mock registry (we can't easily create one without mocking)
        // Instead, test that the SimulationEngine has timeout methods
        let engine = SimulationEngine::new("https://test.com".to_string());

        // Default should be 30 seconds
        assert_eq!(engine.timeout(), Duration::from_secs(30));
    }
}
