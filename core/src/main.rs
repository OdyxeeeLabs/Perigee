mod auth;
mod benchmarks;
mod errors;
mod jobs;
mod parser;
pub mod rpc_provider;
mod simulation;

use crate::errors::AppError;
use crate::jobs::{JobQueue, JobResponse, SubmitJobRequest};
use crate::rpc_provider::{ProviderRegistry, RpcProvider};
use crate::simulation::{SimulationCache, SimulationEngine, SimulationResult};
use axum::{
    extract::{Json, State},
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
    job_queue: Arc<JobQueue>,
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
}

#[derive(Serialize, ToSchema, Debug)]
pub struct StateDependencyReport {
    pub key: String,
    pub source: String,
}

/// Convert a `SimulationResult` (library type) into the API `ResourceReport`.
fn to_report(result: &SimulationResult) -> ResourceReport {
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

    Ok((headers, Json(to_report(&result))))
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
    paths(analyze, auth::challenge_handler, auth::verify_handler, submit_job, get_job),
    components(schemas(
        AnalyzeRequest, ResourceReport,
        auth::ChallengeRequest, auth::ChallengeResponse,
        auth::VerifyRequest, auth::VerifyResponse,
        jobs::SubmitJobRequest, jobs::JobResponse
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
        job_queue: Arc::new(JobQueue::new()),
    });

    // Start the background cleanup task for expired jobs
    let _cleanup_handle = tokio::spawn(jobs::start_cleanup_task(
        (*app_state.job_queue).clone(),
        3600,
    ));

    let cors = CorsLayer::new().allow_origin(Any);

    let protected = Router::new()
        .route("/analyze", post(analyze))
        .route("/jobs/submit", post(submit_job))
        .route("/jobs/:id", get(get_job))
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
