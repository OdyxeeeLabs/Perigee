mod auth;
mod benchmarks;
mod errors;
pub mod insights;
mod parser;
pub mod rpc_provider;
mod simulation;

use crate::errors::AppError;
use crate::insights::InsightsEngine;
use crate::rpc_provider::{ProviderRegistry, RpcProvider};
use crate::simulation::{SimulationCache, SimulationEngine, SimulationResult};
use axum::{
    extract::{Json, Multipart},
    middleware,
    routing::{get, post},
    Extension, Router,
};
use config::{Config, ConfigError};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::env;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;
use tempfile::NamedTempFile;
use tower_http::cors::{Any, CorsLayer};
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};
use utoipa::{OpenApi, ToSchema};
use utoipa_swagger_ui::SwaggerUi;

use soroscope_core::comparison::{run_comparison, CompareMode, RegressionReport};
use soroscope_core::simulation::SimulationEngine;

#[derive(Debug, Clone, Deserialize)]
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
    // Load .env file if present
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
    insights_engine: InsightsEngine,
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
struct ResourceReport {
    #[schema(example = 1000)]
    cpu_instructions: u64,
    #[schema(example = 2048)]
    memory_bytes: u64,
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
        (status = 500, description = "Analysis failed")
    ),
    tag = "Analysis"
)]
async fn analyze(Json(payload): Json<AnalyzeRequest>) -> Result<Json<ResourceReport>, AppError> {
    tracing::info!(
        "Analyzing request for contract: {}, function: {}",
        payload.contract_id,
        payload.function_name
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

#[derive(ToSchema)]
#[allow(dead_code)]
struct CompareApiMultipartRequest {
    #[schema(example = "local_vs_local")]
    mode: String,
    #[schema(value_type = String, format = Binary)]
    current_wasm: String,
    #[schema(value_type = String, format = Binary)]
    base_wasm: Option<String>,
    #[schema(example = "C1234...")]
    contract_id: Option<String>,
    #[schema(example = "hello")]
    function_name: Option<String>,
    #[schema(example = "[\"arg1\", \"12\"]")]
    args: Option<String>,
}

#[utoipa::path(
    post,
    path = "/analyze/compare",
    request_body(content = CompareApiMultipartRequest, content_type = "multipart/form-data", description = "Multipart form with mode, current_wasm, and base_wasm/contract details"),
    responses(
        (status = 200, description = "Comparison successful", body = CompareApiResponse),
        (status = 400, description = "Bad request"),
        (status = 500, description = "Simulation failed")
    ),
    tag = "Analysis"
)]

async fn compare_handler(
    Extension(config): Extension<Arc<AppConfig>>,
    mut multipart: Multipart,
) -> Result<Json<CompareApiResponse>, AppError> {
    let mut mode = String::new();
    let mut current_wasm: Option<NamedTempFile> = None;
    let mut base_wasm: Option<NamedTempFile> = None;
    let mut contract_id = String::new();
    let mut function_name = String::new();
    let mut args_raw = String::new();

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| AppError::BadRequest(e.to_string()))?
    {
        let name = field.name().unwrap_or_default().to_string();

    Ok((headers, Json(to_report(&result, &state.insights_engine))))
}

#[derive(OpenApi)]
#[openapi(
    paths(analyze, compare_handler, auth::challenge_handler, auth::verify_handler),
    components(schemas(
        AnalyzeRequest, ResourceReport,
        CompareApiMultipartRequest, CompareApiResponse, RegressionReport, soroscope_core::comparison::ResourceDelta, soroscope_core::comparison::RegressionFlag,
        auth::ChallengeRequest, auth::ChallengeResponse,
        auth::VerifyRequest, auth::VerifyResponse
    )),
    tags(
        (name = "Analysis", description = "Soroban contract resource analysis endpoints"),
        (name = "Auth", description = "SEP-10 wallet authentication")
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
    // -------------------------------
    // Initialize Tracing / Logging
    // -------------------------------
    if env::var("RUST_LOG").is_err() {
        env::set_var("RUST_LOG", "info");
    }

    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    tracing::info!("SoroScope Starting...");

    // -------------------------------
    // Load configuration
    // -------------------------------
    let config = load_config().expect("Failed to load configuration");
    tracing::info!("SoroScope initialized with config: {:?}", config);

    // -------------------------------
    // CLI Argument Handling (Benchmark)
    // -------------------------------
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

    if args.len() > 1 && args[1] == "compare" {
        if args.len() < 4 {
            tracing::error!(
                "Usage: cargo run -p soroscope-core -- compare path/to/v1.wasm path/to/v2.wasm"
            );
            return;
        }
        tracing::info!("Starting SoroScope Compare...");

        let path1 = PathBuf::from(&args[2]);
        let path2 = PathBuf::from(&args[3]);

        if !path1.exists() {
            tracing::error!("File not found: {:?}", path1);
            return;
        }
        if !path2.exists() {
            tracing::error!("File not found: {:?}", path2);
            return;
        }

        let engine = SimulationEngine::new(config.soroban_rpc_url.clone());
        let mode = CompareMode::LocalVsLocal {
            current_wasm_path: path1,
            base_wasm_path: path2,
        };

        // Create a local async runtime for the CLI
        let rt = tokio::runtime::Runtime::new().unwrap();
        match rt.block_on(run_comparison(&engine, mode)) {
            Ok(report) => {
                println!("\n=== Regression Report ===");
                println!("Summary: {}", report.summary);

                println!("\nMetrics (Current vs Base):");
                println!(
                    "CPU Instructions: {} vs {} ({:+.1}%)",
                    report.current.cpu_instructions,
                    report.base.cpu_instructions,
                    report.deltas.cpu_instructions
                );
                println!(
                    "RAM Bytes: {} vs {} ({:+.1}%)",
                    report.current.ram_bytes, report.base.ram_bytes, report.deltas.ram_bytes
                );
                println!(
                    "Ledger Read Bytes: {} vs {} ({:+.1}%)",
                    report.current.ledger_read_bytes,
                    report.base.ledger_read_bytes,
                    report.deltas.ledger_read_bytes
                );
                println!(
                    "Ledger Write Bytes: {} vs {} ({:+.1}%)",
                    report.current.ledger_write_bytes,
                    report.base.ledger_write_bytes,
                    report.deltas.ledger_write_bytes
                );
                println!(
                    "TX Size Bytes: {} vs {} ({:+.1}%)",
                    report.current.transaction_size_bytes,
                    report.base.transaction_size_bytes,
                    report.deltas.transaction_size_bytes
                );
            }
            Err(e) => {
                tracing::error!("Comparison failed: {}", e);
            }
        }
        return;
    }

    // -------------------------------
    // Web Server Setup
    // -------------------------------
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
        insights_engine: InsightsEngine::new(),
    });

    let cors = CorsLayer::new().allow_origin(Any);

    let protected = Router::new()
        .route("/analyze", post(analyze))
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
        .route(
            "/error",
            get(|| async { Err::<&str, AppError>(AppError::BadRequest("Test error".to_string())) }),
        )
        .route("/auth/challenge", post(auth::challenge_handler))
        .route("/auth/verify", post(auth::verify_handler))
        .merge(protected)
        .layer(Extension(auth_state))
        .layer(Extension(Arc::new(config.clone())))
        .layer(cors)
        .layer(TraceLayer::new_for_http());

    // -------------------------------
    // Run Server
    // -------------------------------
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
