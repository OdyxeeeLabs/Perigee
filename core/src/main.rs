mod auth;
mod benchmarks;
mod errors;
mod insights;
mod parser;
mod simulation;

use crate::errors::AppError;
use crate::insights::{Insight, InsightsEngine, Severity};
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
    soroban_rpc_url: String,
    jwt_secret: String,
    network_passphrase: String,
    /// Redis URL reserved for future distributed cache migration.
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

/// Shared application state injected into Axum handlers.
struct AppState {
    engine: SimulationEngine,
    insights_engine: InsightsEngine,
    cache: Arc<SimulationCache>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct AnalyzeRequest {
    #[schema(example = "CDLZFC3SYJYDZT7K67VZ75HPJVIEUVNIXF47ZG2FB2RMQQVU2HHGCYSC")]
    pub contract_id: String,
    #[schema(example = "hello")]
    pub function_name: String,
    #[schema(example = "[]")]
    pub args: Option<Vec<String>>,
    /// Map of Key-Base64 to Value-Base64 ledger entry overrides.
    pub ledger_overrides: Option<HashMap<String, String>>,
}

#[derive(Serialize, ToSchema)]
pub struct ResourceReport {
    /// CPU instructions consumed.
    #[schema(example = 1500)]
    pub cpu_instructions: u64,
    /// RAM bytes consumed.
    #[schema(example = 3000)]
    pub ram_bytes: u64,
    /// Ledger read bytes.
    #[schema(example = 1024)]
    pub ledger_read_bytes: u64,
    /// Ledger write bytes.
    #[schema(example = 512)]
    pub ledger_write_bytes: u64,
    /// Transaction size in bytes.
    #[schema(example = 450)]
    pub transaction_size_bytes: u64,
    /// Number of ledger keys in the footprint.
    #[schema(example = 5)]
    pub footprint_size: u32,
    /// Potential optimization insights.
    pub insights: Vec<Insight>,
    /// Efficiency score (0-100).
    #[schema(example = 85)]
    pub efficiency_score: u8,
    /// Report showing which data was injected vs live.
    pub state_dependency: Option<Vec<StateDependencyReport>>,
}

#[derive(Serialize, ToSchema)]
pub struct StateDependencyReport {
    pub key: String,
    pub source: String,
}

/// Convert a library simulation result into an API resource report.
fn to_report(result: &SimulationResult, insights_engine: &InsightsEngine) -> ResourceReport {
    ResourceReport {
        cpu_instructions: result.resources.cpu_instructions,
        ram_bytes: result.resources.ram_bytes,
        ledger_read_bytes: result.resources.ledger_read_bytes,
        ledger_write_bytes: result.resources.ledger_write_bytes,
        transaction_size_bytes: result.resources.transaction_size_bytes,
        footprint_size: result.resources.footprint_size,
        insights: insights_engine.get_insights(&result.resources),
        efficiency_score: insights_engine.calculate_efficiency_score(&result.resources),
        state_dependency: result.state_dependency.as_ref().map(|deps| {
            deps.iter()
                .map(|d| StateDependencyReport {
                    key: d.key.clone(),
                    source: format!("{d:?}"),
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
        (status = 200, description = "Analysis successful", body = ResourceReport),
        (status = 401, description = "Unauthorized"),
        (status = 500, description = "Internal error")
    ),
    security(("jwt" = [])),
    tag = "Analysis"
)]
async fn analyze(
    State(state): State<Arc<AppState>>,
    Json(payload): Json<AnalyzeRequest>,
) -> Result<(HeaderMap, Json<ResourceReport>), AppError> {
    let args = payload.args.clone().unwrap_or_default();
    let cache_key =
        SimulationCache::generate_key(&payload.contract_id, &payload.function_name, &args);

    let (result, cache_status) = if let Some(cached) = state.cache.get(&cache_key).await {
        (cached, "HIT")
    } else {
        let sim = state
            .engine
            .simulate_from_contract_id(
                &payload.contract_id,
                &payload.function_name,
                args,
                payload.ledger_overrides.clone(),
            )
            .await
            .map_err(|e| AppError::Internal(format!("Simulation failed: {e}")))?;
        state.cache.set(cache_key, sim.clone()).await;
        (sim, "MISS")
    };

    let mut headers = HeaderMap::new();
    headers.insert(
        HeaderName::from_static("x-soroscope-cache"),
        HeaderValue::from_static(cache_status),
    );

    Ok((headers, Json(to_report(&result, &state.insights_engine))))
}

#[derive(OpenApi)]
#[openapi(
    paths(analyze, auth::challenge_handler, auth::verify_handler),
    components(schemas(
        AnalyzeRequest, ResourceReport, Insight, Severity,
        auth::ChallengeRequest, auth::ChallengeResponse,
        auth::VerifyRequest, auth::VerifyResponse
    )),
    tags(
        (name = "Analysis", description = "Contract analysis endpoints"),
        (name = "Auth", description = "SEP-10 auth")
    ),
    info(title = "SoroScope API", version = "0.1.0")
)]
struct ApiDoc;

#[tokio::main]
async fn main() {
    tracing_subscriber::registry()
        .with(EnvFilter::from_default_env())
        .with(tracing_subscriber::fmt::layer())
        .init();

    let config = load_config().expect("Failed to load config");
    let app_state = Arc::new(AppState {
        engine: SimulationEngine::new(config.soroban_rpc_url.clone()),
        insights_engine: InsightsEngine::new(),
        cache: SimulationCache::new(),
    });

    let auth_state = Arc::new(auth::AuthState::new(
        config.jwt_secret.clone(),
        None,
        config.network_passphrase.clone(),
    ));

    let protected = Router::new()
        .route("/analyze", post(analyze))
        .route_layer(middleware::from_fn(auth::auth_middleware));

    let app = Router::new()
        .merge(SwaggerUi::new("/swagger-ui").url("/api-docs/openapi.json", ApiDoc::openapi()))
        .route("/", get(|| async { "SoroScope API" }))
        .route("/auth/challenge", post(auth::challenge_handler))
        .route("/auth/verify", post(auth::verify_handler))
        .merge(protected)
        .layer(Extension(auth_state))
        .layer(CorsLayer::new().allow_origin(Any))
        .layer(TraceLayer::new_for_http())
        .with_state(app_state);

    let addr = format!("0.0.0.0:{}", config.server_port);
    let listener = tokio::net::TcpListener::bind(&addr).await.unwrap();
    tracing::info!("Listening on http://{}", addr);
    axum::serve(listener, app).await.unwrap();
}
