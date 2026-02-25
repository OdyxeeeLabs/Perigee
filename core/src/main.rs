mod auth;
mod benchmarks;
mod errors;
mod parser;
mod simulation;

use crate::errors::AppError;
use crate::simulation::{SimulationCache, SimulationEngine, SimulationResult};
use axum::{
<<<<<<< HEAD
    extract::{Json, Multipart},
=======
    extract::{Json, State},
    http::{HeaderMap, HeaderName, HeaderValue},
>>>>>>> origin/main
    middleware,
    routing::{get, post},
    Extension, Router,
};
use config::{Config, ConfigError};
use serde::{Deserialize, Serialize};
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
    soroban_rpc_url: String,
    jwt_secret: String,
    network_passphrase: String,
    /// Redis URL reserved for the distributed cache migration (issue #65).
    /// Unused in the MVP in-memory implementation — present so the config
    /// surface is stable when Redis is wired in.
    redis_url: String,
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
        .build()?;

    settings.try_deserialize()
}

/// Shared application state injected into every Axum handler via [`State`].
struct AppState {
    #[allow(dead_code)] // will be used when RPC simulation is wired into analyze handler
    engine: SimulationEngine,
    cache: Arc<SimulationCache>,
}

#[derive(Deserialize, ToSchema)]
struct AnalyzeRequest {
    #[schema(example = "0x1234...")]
    contract_id: String,
    #[schema(example = "invoke")]
    function_name: String,
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
}

/// Convert a `SimulationResult` (library type) into the API `ResourceReport`.
fn to_report(result: &SimulationResult) -> ResourceReport {
    ResourceReport {
        cpu_instructions: result.resources.cpu_instructions,
        ram_bytes: result.resources.ram_bytes,
        ledger_read_bytes: result.resources.ledger_read_bytes,
        ledger_write_bytes: result.resources.ledger_write_bytes,
        transaction_size_bytes: result.resources.transaction_size_bytes,
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

    let args: Vec<String> = vec![];
    let cache_key =
        SimulationCache::generate_key(&payload.contract_id, &payload.function_name, &args);

    let (result, cache_status): (SimulationResult, &'static str) =
        if let Some(cached) = state.cache.get(&cache_key).await {
            (cached, "HIT")
        } else {
            let sim = state
                .engine
                .simulate_from_contract_id(&payload.contract_id, &payload.function_name, args)
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

#[derive(Serialize, ToSchema)]
struct CompareApiResponse {
    report: RegressionReport,
}

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

        match name.as_str() {
            "mode" => {
                mode = field.text().await.unwrap_or_default();
            }
            "contract_id" => {
                contract_id = field.text().await.unwrap_or_default();
            }
            "function_name" => {
                function_name = field.text().await.unwrap_or_default();
            }
            "args" => {
                args_raw = field.text().await.unwrap_or_default();
            }
            "current_wasm" => {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(e.to_string()))?;
                let mut temp_file =
                    NamedTempFile::new().map_err(|e| AppError::Internal(e.to_string()))?;
                temp_file
                    .write_all(&data)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                current_wasm = Some(temp_file);
            }
            "base_wasm" => {
                let data = field
                    .bytes()
                    .await
                    .map_err(|e| AppError::BadRequest(e.to_string()))?;
                let mut temp_file =
                    NamedTempFile::new().map_err(|e| AppError::Internal(e.to_string()))?;
                temp_file
                    .write_all(&data)
                    .map_err(|e| AppError::Internal(e.to_string()))?;
                base_wasm = Some(temp_file);
            }
            _ => {}
        }
    }

    let current_wasm_path = current_wasm
        .ok_or_else(|| AppError::BadRequest("Missing current_wasm".to_string()))?
        .into_temp_path()
        .to_path_buf();

    let compare_mode = match mode.as_str() {
        "local_vs_local" => {
            let base_wasm_path = base_wasm
                .ok_or_else(|| AppError::BadRequest("Missing base_wasm".to_string()))?
                .into_temp_path()
                .to_path_buf();
            CompareMode::LocalVsLocal {
                current_wasm_path,
                base_wasm_path,
            }
        }
        "local_vs_deployed" => {
            if contract_id.is_empty() || function_name.is_empty() {
                return Err(AppError::BadRequest(
                    "Missing contract_id or function_name".to_string(),
                ));
            }
            let args = if args_raw.is_empty() {
                vec![]
            } else {
                args_raw.split(',').map(|s| s.trim().to_string()).collect()
            };
            CompareMode::LocalVsDeployed {
                current_wasm_path,
                contract_id,
                function_name,
                args,
            }
        }
        _ => return Err(AppError::BadRequest("Invalid mode".to_string())),
    };

    let engine = SimulationEngine::new(config.soroban_rpc_url.clone());

    let report = run_comparison(&engine, compare_mode)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok(Json(CompareApiResponse { report }))
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

<<<<<<< HEAD
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
=======
>>>>>>> origin/main
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
    let app_state = Arc::new(AppState {
        engine: SimulationEngine::new(config.soroban_rpc_url.clone()),
        cache: SimulationCache::new(),
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
        .route("/auth/challenge", post(auth::challenge_handler))
        .route("/auth/verify", post(auth::verify_handler))
        .merge(protected)
        .layer(Extension(auth_state))
        .layer(Extension(Arc::new(config.clone())))
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
