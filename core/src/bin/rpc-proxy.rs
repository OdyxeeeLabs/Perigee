use axum::{
    extract::{Request, State},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::post,
    Json, Router,
};
use axum::http::{HeaderName, HeaderValue};
use clap::Parser;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::sync::Arc;
use tracing::Instrument;
use uuid::Uuid;

#[derive(Parser)]
struct Args {
    #[arg(short, long, default_value = "8545")]
    port: u16,
    #[arg(short, long)]
    rpc_url: String,
}

#[derive(Clone)]
#[allow(dead_code)]
struct Config {
    max_gas_limit: u64,
    min_gas_price: u64,
    blocked_addresses: HashSet<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            max_gas_limit: 30_000_000,
            min_gas_price: 100,
            blocked_addresses: HashSet::new(),
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
struct RpcRequest {
    jsonrpc: String,
    method: String,
    params: serde_json::Value,
    id: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct RpcResponse {
    jsonrpc: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    result: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<RpcError>,
    id: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct RpcError {
    code: i32,
    message: String,
}

#[derive(Debug, Deserialize, Serialize)]
struct Transaction {
    from: String,
    to: Option<String>,
    gas: Option<String>,
    value: Option<String>,
    data: Option<String>,
}

struct AppState {
    rpc_url: String,
    client: reqwest::Client,
    config: Config,
}

async fn request_id_middleware(mut request: Request, next: Next) -> Response {
    let request_id = request
        .headers()
        .get("x-request-id")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| {
            !value.is_empty()
                && value.len() <= 128
                && value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
        })
        .map(str::to_owned)
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let header_value = HeaderValue::from_str(&request_id)
        .unwrap_or_else(|_| HeaderValue::from_static("unknown"));
    request.headers_mut().insert(
        HeaderName::from_static("x-request-id"),
        header_value.clone(),
    );

    let method = request.method().to_string();
    let path = request.uri().path().to_owned();
    let span = tracing::info_span!(
        "rpc_request",
        request_id = %request_id,
        method = %method,
        path = %path,
    );
    let mut response = next.run(request).instrument(span.clone()).await;
    response.headers_mut().insert(
        HeaderName::from_static("x-request-id"),
        header_value,
    );
    tracing::info!(
        parent: &span,
        request_id = %request_id,
        status = %response.status(),
        "RPC request completed"
    );
    response
}

async fn shutdown_signal() {
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };

    #[cfg(unix)]
    let terminate = async {
        if let Ok(mut signal) =
            tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        {
            signal.recv().await;
        }
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("Shutdown signal received"),
        _ = terminate => tracing::info!("Shutdown signal received"),
    }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();
    let args = Args::parse();

    let state = Arc::new(AppState {
        rpc_url: args.rpc_url,
        client: reqwest::Client::new(),
        config: Config::default(),
    });

    let app = Router::new()
        .route("/", post(handle_rpc))
        .layer(axum::extract::DefaultBodyLimit::max(1024 * 1024 * 2))
        .layer(axum::middleware::from_fn(request_id_middleware))
        .with_state(state);

    let listener = match tokio::net::TcpListener::bind(format!("0.0.0.0:{}", args.port)).await {
        Ok(listener) => listener,
        Err(error) => {
            tracing::error!(error = %error, port = args.port, "RPC proxy failed to bind");
            return;
        }
    };

    tracing::info!(port = args.port, "RPC proxy started");
    if let Err(error) = axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await
    {
        tracing::error!(error = %error, "RPC proxy stopped with an error");
    }
}

async fn handle_rpc(
    State(state): State<Arc<AppState>>,
    Json(req): Json<RpcRequest>,
) -> impl IntoResponse {
    if req.method == "eth_sendTransaction" {
        tracing::info!("Intercepting sendTransaction");

        let params: Vec<Transaction> = match serde_json::from_value(req.params.clone()) {
            Ok(p) => p,
            Err(_) => {
                return Json(RpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(RpcError {
                        code: -32602,
                        message: "Invalid params".to_string(),
                    }),
                    id: req.id,
                });
            }
        };

        if params.is_empty() {
            return Json(RpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(RpcError {
                    code: -32602,
                    message: "Missing transaction params".to_string(),
                }),
                id: req.id,
            });
        }

        let tx = &params[0];

        if state.config.blocked_addresses.contains(&tx.from) {
            return Json(RpcResponse {
                jsonrpc: "2.0".to_string(),
                result: None,
                error: Some(RpcError {
                    code: -32000,
                    message: "Address is blocked".to_string(),
                }),
                id: req.id,
            });
        }

        let simulate_req = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "eth_call",
            "params": [tx, "latest"],
            "id": 1
        });

        match state
            .client
            .post(&state.rpc_url)
            .json(&simulate_req)
            .send()
            .await
        {
            Ok(resp) => {
                let result: serde_json::Value = resp.json().await.unwrap_or_default();

                if result.get("error").is_some() {
                    tracing::info!("Transaction simulation failed");
                    return Json(RpcResponse {
                        jsonrpc: "2.0".to_string(),
                        result: None,
                        error: Some(RpcError {
                            code: -32000,
                            message: "Transaction would fail".to_string(),
                        }),
                        id: req.id,
                    });
                }

                let gas_req = serde_json::json!({
                    "jsonrpc": "2.0",
                    "method": "eth_estimateGas",
                    "params": [tx],
                    "id": 1
                });

                if let Ok(gas_resp) = state
                    .client
                    .post(&state.rpc_url)
                    .json(&gas_req)
                    .send()
                    .await
                {
                    let gas_body: serde_json::Value = gas_resp.json().await.unwrap_or_default();
                    let gas_used = gas_body
                        .get("result")
                        .and_then(|r| r.as_str())
                        .and_then(|s| u64::from_str_radix(&s[2..], 16).ok())
                        .unwrap_or(0);

                    if gas_used > state.config.max_gas_limit {
                        tracing::info!(
                            gas_used,
                            max_gas_limit = state.config.max_gas_limit,
                            "Gas limit exceeded"
                        );
                        return Json(RpcResponse {
                            jsonrpc: "2.0".to_string(),
                            result: None,
                            error: Some(RpcError {
                                code: -32000,
                                message: format!(
                                    "Gas limit exceeded: {} > {}",
                                    gas_used, state.config.max_gas_limit
                                ),
                            }),
                            id: req.id,
                        });
                    }

                    tracing::info!(gas_used, "Transaction simulation passed");
                }
            }
            Err(_) => {
                return Json(RpcResponse {
                    jsonrpc: "2.0".to_string(),
                    result: None,
                    error: Some(RpcError {
                        code: -32000,
                        message: "Simulation failed".to_string(),
                    }),
                    id: req.id,
                });
            }
        }
    }

    match state.client.post(&state.rpc_url).json(&req).send().await {
        Ok(resp) => {
            let body: serde_json::Value = resp.json().await.unwrap_or_default();
            Json(RpcResponse {
                jsonrpc: "2.0".to_string(),
                result: body.get("result").cloned(),
                error: None,
                id: req.id,
            })
        }
        Err(_) => Json(RpcResponse {
            jsonrpc: "2.0".to_string(),
            result: None,
            error: Some(RpcError {
                code: -32000,
                message: "Upstream request failed".to_string(),
            }),
            id: req.id,
        }),
    }
}
