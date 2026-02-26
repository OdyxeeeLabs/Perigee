mod auth;
mod benchmarks;
mod comparison;
mod errors;
<<<<<<< main
mod jobs;
=======
pub mod insights;
>>>>>>> main
mod parser;
pub mod rpc_provider;
mod simulation;

use crate::comparison::{CompareMode, RegressionFlag, RegressionReport, ResourceDelta};
use crate::errors::AppError;
<<<<<<< main
use crate::jobs::{JobQueue, JobResponse, SubmitJobRequest};
=======
use crate::insights::InsightsEngine;
>>>>>>> main
use crate::rpc_provider::{ProviderRegistry, RpcProvider};
use crate::simulation::{SimulationCache, SimulationEngine, SimulationResult};
use axum::{
    extract::{Json, Multipart, State},
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
}

fn default_health_check_interval() -> u64 {
    30
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
    #[allow(dead_code)] // will be used when RPC simulation is wired into analyze handler
    engine: SimulationEngine,
    cache: Arc<SimulationCache>,
<<<<<<< main
    job_queue: Arc<JobQueue>,
=======
    insights_engine: InsightsEngine,
>>>>>>> main
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
    /// Report showing which data was injected vs live
    pub state_dependency: Option<Vec<StateDependencyReport>>,
    /// Efficiency score (0–100) and optimisation insights.
    pub nutrition: NutritionReport,
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

/// Convert a `SimulationResult` (library type) into the API `ResourceReport`.
fn to_report(result: &SimulationResult, insights_engine: &InsightsEngine) -> ResourceReport {
    let insights_report = insights_engine.analyze(&result.resources);

    ResourceReport {
        cpu_instructions: result.resources.cpu_instructions,
        ram_bytes: result.resources.ram_bytes,
        ledger_read_bytes: result.resources.ledger_read_bytes,
        ledger_write_bytes: result.resources.ledger_write_bytes,
        transaction_size_bytes: result.resources.transaction_size_bytes,
        state_dependency: result.state_dependency.as_ref().map(|deps| {
            deps.iter()
                .map(|d| StateDependencyReport {
                    key: d.key.clone(),
                    source: format!("{:?}", d.source),
                })
                .collect()
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
    tracing::info!(
        contract_id = %payload.contract_id,
        function_name = %payload.function_name,
        "Received analyze request"
    );

    let args = payload.args.clone().unwrap_or_default();
    let cache_key =
        SimulationCache::generate_key(&payload.contract_id, &payload.function_name, &args);

    let (result, cache_status): (SimulationResult, &'static str) =
        if let Some(cached) = state.cache.get(&cache_key).await {
            (cached, "HIT")
        } else {
            let sim: SimulationResult = state
                .engine
                .simulate_from_contract_id(
                    &payload.contract_id,
                    &payload.function_name,
                    args,
                    payload.ledger_overrides.clone(),
                )
                .await
                .map_err(|e| AppError::Internal(format!("Simulation failed: {}", e)))?;
            state.cache.set(cache_key, sim.clone()).await;
            (sim, "MISS")
        };

    state.cache.log_stats();

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-soroscope-cache"),
        HeaderValue::from_static(cache_status),
    );

    Ok((headers, Json(to_report(&result, &state.insights_engine))))
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

#[utoipa::path(
    post,
    path = "/jobs/submit",
    request_body = SubmitJobRequest,
    responses(
        (status = 200, description = "Job submitted successfully", body = JobResponse),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Submission failed")
    ),
    security(
        ("jwt" = [])
    ),
    tag = "Jobs"
)]
async fn submit_job(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<SubmitJobRequest>,
) -> Result<Json<JobResponse>, AppError> {
    tracing::info!(
        callback_url = ?payload.callback_url,
        "Received job submission request"
    );

    let response = state
        .job_queue
        .submit_job(payload)
        .map_err(|e| AppError::Internal(format!("Job submission failed: {}", e)))?;

    Ok(Json(response))
}

#[utoipa::path(
    get,
    path = "/jobs/{id}",
    params(("id" = String, description = "Job ID")),
    responses(
        (status = 200, description = "Job status retrieved successfully", body = JobResponse),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Job not found"),
        (status = 500, description = "Retrieval failed")
    ),
    security(
        ("jwt" = [])
    ),
    tag = "Jobs"
)]
async fn get_job(
    State(state): State<Arc<AppState>>,
    axum::extract::Path(job_id): axum::extract::Path<String>,
) -> Result<Json<JobResponse>, AppError> {
    tracing::info!(
        job_id = %job_id,
        "Received job status request"
    );

    let response = state
        .job_queue
        .get_job(&job_id)
        .await
        .map_err(|e| AppError::Internal(format!("Job retrieval failed: {}", e)))?;

    Ok(Json(response))
}

#[derive(OpenApi)]
#[openapi(
<<<<<<< main
    paths(analyze, auth::challenge_handler, auth::verify_handler, submit_job, get_job),
=======
    paths(analyze, optimize_limits, compare_handler, auth::challenge_handler, auth::verify_handler),
>>>>>>> main
    components(schemas(
        AnalyzeRequest, ResourceReport,
        OptimizeLimitsRequest, OptimizeLimitsResponse,
        CompareApiResponse, RegressionReport, ResourceDelta, RegressionFlag,
        auth::ChallengeRequest, auth::ChallengeResponse,
        auth::VerifyRequest, auth::VerifyResponse,
<<<<<<< main
        jobs::SubmitJobRequest, jobs::JobResponse
=======
        crate::simulation::OptimizationBuffer,
        crate::simulation::SorobanResources
>>>>>>> main
    )),
    tags(
        (name = "Analysis", description = "Soroban contract resource analysis endpoints"),
        (name = "Auth", description = "SEP-10 wallet authentication"),
        (name = "Jobs", description = "Background job processing endpoints")
    ),
    info(
        title = "SoroScope API",
        version = "0.1.0",
        description = "API for analyzing Soroban smart contract resource consumption"
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

    let app_state = Arc::new(AppState {
        engine: SimulationEngine::with_registry(Arc::clone(&registry)),
        cache: SimulationCache::new(),
<<<<<<< main
        job_queue: Arc::new(JobQueue::new()),
=======
        insights_engine: InsightsEngine::new(),
>>>>>>> main
    });

    // Start the background cleanup task for expired jobs
    let _cleanup_handle = tokio::spawn(jobs::start_cleanup_task(
        (*app_state.job_queue).clone(),
        3600,
    ));

    let cors = CorsLayer::new().allow_origin(Any);

    let protected = Router::new()
        .route("/analyze", post(analyze))
<<<<<<< main
        .route("/jobs/submit", post(submit_job))
        .route("/jobs/:id", get(get_job))
=======
        .route("/analyze/optimize-limits", post(optimize_limits))
        .route("/analyze/compare", post(compare_handler))
>>>>>>> main
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
