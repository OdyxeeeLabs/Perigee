//! Health check endpoints for Kubernetes liveness and readiness probes.
//!
//! # Issue #593 — Create health check endpoints
//!
//! Provides:
//! - `GET /health/live`  — liveness probe: process is alive, no deadlocks.
//! - `GET /health/ready` — readiness probe: all dependencies reachable.
//!
//! Responses are structured JSON: `{status, checks, timestamp}`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::time::Instant;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Overall health status of the service or an individual dependency.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HealthStatus {
    /// All checks passed; service is fully operational.
    Healthy,
    /// One or more non-critical checks failed; service is degraded but still
    /// serving traffic.
    Degraded,
    /// One or more critical checks failed; service should not receive traffic.
    Unhealthy,
}

impl std::fmt::Display for HealthStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HealthStatus::Healthy => write!(f, "healthy"),
            HealthStatus::Degraded => write!(f, "degraded"),
            HealthStatus::Unhealthy => write!(f, "unhealthy"),
        }
    }
}

/// Result of probing a single downstream dependency.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DependencyCheck {
    /// Human-readable name of the dependency (e.g. `"postgres"`, `"redis"`).
    pub name: String,
    /// Whether the dependency is reachable.
    pub status: HealthStatus,
    /// Round-trip latency in milliseconds, or `None` if the probe failed
    /// before a response was received.
    pub latency_ms: Option<u64>,
    /// Error message if the probe failed.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

impl DependencyCheck {
    /// Construct a successful check result.
    pub fn ok(name: impl Into<String>, latency_ms: u64) -> Self {
        Self {
            name: name.into(),
            status: HealthStatus::Healthy,
            latency_ms: Some(latency_ms),
            error: None,
        }
    }

    /// Construct a failed check result.
    pub fn fail(name: impl Into<String>, latency_ms: Option<u64>, error: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            status: HealthStatus::Unhealthy,
            latency_ms,
            error: Some(error.into()),
        }
    }
}

/// Top-level health check response returned by both `/health/live` and
/// `/health/ready`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HealthCheck {
    /// Aggregated status across all checks.
    pub status: HealthStatus,
    /// Per-dependency check results, keyed by dependency name.
    pub checks: HashMap<String, DependencyCheck>,
    /// UTC timestamp at which the check was performed.
    pub timestamp: DateTime<Utc>,
}

impl HealthCheck {
    /// Create a new health check response, computing the aggregate status
    /// from the individual dependency results.
    pub fn new(checks: Vec<DependencyCheck>) -> Self {
        let aggregate = aggregate_status(&checks);
        let checks_map = checks.into_iter().map(|c| (c.name.clone(), c)).collect();
        Self {
            status: aggregate,
            checks: checks_map,
            timestamp: Utc::now(),
        }
    }

    /// Convenience: create a healthy response with no sub-checks (used for
    /// the liveness probe which only confirms the process is alive).
    pub fn liveness_ok() -> Self {
        Self {
            status: HealthStatus::Healthy,
            checks: HashMap::new(),
            timestamp: Utc::now(),
        }
    }
}

fn aggregate_status(checks: &[DependencyCheck]) -> HealthStatus {
    if checks.iter().any(|c| c.status == HealthStatus::Unhealthy) {
        HealthStatus::Unhealthy
    } else if checks.iter().any(|c| c.status == HealthStatus::Degraded) {
        HealthStatus::Degraded
    } else {
        HealthStatus::Healthy
    }
}

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// Connection strings / addresses injected into `HealthService` so it knows
/// where to probe each dependency.
#[derive(Debug, Clone)]
pub struct HealthConfig {
    /// PostgreSQL connection string, e.g. `postgres://user:pass@host/db`.
    pub database_url: Option<String>,
    /// Redis connection string, e.g. `redis://127.0.0.1:6379`.
    pub redis_url: Option<String>,
    /// List of Stellar / Soroban RPC node base URLs to probe.
    pub rpc_node_urls: Vec<String>,
}

impl Default for HealthConfig {
    fn default() -> Self {
        Self {
            database_url: std::env::var("DATABASE_URL").ok(),
            redis_url: std::env::var("REDIS_URL").ok(),
            rpc_node_urls: std::env::var("RPC_NODE_URLS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string())
                .collect(),
        }
    }
}

// ---------------------------------------------------------------------------
// HealthService
// ---------------------------------------------------------------------------

/// Service that performs health probes and returns structured results.
///
/// Designed to be constructed once at application startup and shared via
/// `Arc<HealthService>`.
#[derive(Debug, Clone)]
pub struct HealthService {
    config: HealthConfig,
}

impl HealthService {
    /// Create a new `HealthService` with the supplied configuration.
    pub fn new(config: HealthConfig) -> Self {
        Self { config }
    }

    /// Create a `HealthService` using defaults read from environment variables.
    pub fn from_env() -> Self {
        Self::new(HealthConfig::default())
    }

    // -----------------------------------------------------------------------
    // Liveness probe — GET /health/live
    // -----------------------------------------------------------------------

    /// Liveness probe: the process is alive and not deadlocked.
    ///
    /// This is intentionally cheap — it just confirms that the async runtime
    /// is scheduling work and that no catastrophic state has been reached.
    /// It does **not** probe external dependencies; use `check_ready` for
    /// that.
    pub async fn check_live(&self) -> HealthCheck {
        // Yield to the scheduler once to prove we are not deadlocked.
        tokio::task::yield_now().await;
        HealthCheck::liveness_ok()
    }

    // -----------------------------------------------------------------------
    // Readiness probe — GET /health/ready
    // -----------------------------------------------------------------------

    /// Readiness probe: all external dependencies are reachable.
    ///
    /// Checks PostgreSQL, Redis, and every configured RPC node concurrently
    /// and aggregates the results.
    pub async fn check_ready(&self) -> HealthCheck {
        let mut futs = Vec::new();

        if let Some(db_url) = self.config.database_url.clone() {
            futs.push(tokio::spawn(async move { probe_postgres(db_url).await }));
        }
        if let Some(redis_url) = self.config.redis_url.clone() {
            futs.push(tokio::spawn(async move { probe_redis(redis_url).await }));
        }
        for url in &self.config.rpc_node_urls {
            let url = url.clone();
            futs.push(tokio::spawn(async move { probe_rpc_node(url).await }));
        }

        let mut checks = Vec::new();
        for handle in futs {
            match handle.await {
                Ok(check) => checks.push(check),
                Err(e) => checks.push(DependencyCheck::fail(
                    "unknown",
                    None,
                    format!("task panicked: {e}"),
                )),
            }
        }

        HealthCheck::new(checks)
    }
}

// ---------------------------------------------------------------------------
// Probe implementations
// ---------------------------------------------------------------------------

/// Probe PostgreSQL by executing a trivial query.
async fn probe_postgres(url: String) -> DependencyCheck {
    use sqlx::PgPool;

    let t0 = Instant::now();
    match PgPool::connect(&url).await {
        Err(e) => DependencyCheck::fail("postgres", Some(t0.elapsed().as_millis() as u64), e.to_string()),
        Ok(pool) => {
            match sqlx::query("SELECT 1").execute(&pool).await {
                Ok(_) => DependencyCheck::ok("postgres", t0.elapsed().as_millis() as u64),
                Err(e) => DependencyCheck::fail("postgres", Some(t0.elapsed().as_millis() as u64), e.to_string()),
            }
        }
    }
}

/// Probe Redis with a PING command.
async fn probe_redis(url: String) -> DependencyCheck {
    use redis::AsyncCommands;

    let t0 = Instant::now();
    let client = match redis::Client::open(url.as_str()) {
        Ok(c) => c,
        Err(e) => return DependencyCheck::fail("redis", Some(t0.elapsed().as_millis() as u64), e.to_string()),
    };
    match client.get_multiplexed_async_connection().await {
        Err(e) => DependencyCheck::fail("redis", Some(t0.elapsed().as_millis() as u64), e.to_string()),
        Ok(mut conn) => {
            let result: redis::RedisResult<String> = redis::cmd("PING").query_async(&mut conn).await;
            let latency = t0.elapsed().as_millis() as u64;
            match result {
                Ok(_) => DependencyCheck::ok("redis", latency),
                Err(e) => DependencyCheck::fail("redis", Some(latency), e.to_string()),
            }
        }
    }
}

/// Probe an RPC node by issuing an HTTP GET to its health or root endpoint.
async fn probe_rpc_node(url: String) -> DependencyCheck {
    let name = format!("rpc_node:{}", sanitize_url_for_name(&url));
    let t0 = Instant::now();
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(5))
        .build()
        .unwrap_or_default();

    let probe_url = format!("{}/", url.trim_end_matches('/'));
    match client.get(&probe_url).send().await {
        Err(e) => DependencyCheck::fail(name, Some(t0.elapsed().as_millis() as u64), e.to_string()),
        Ok(resp) => {
            let latency = t0.elapsed().as_millis() as u64;
            if resp.status().is_success() || resp.status().as_u16() == 404 {
                // 404 still means the server is up; a 5xx means trouble.
                DependencyCheck::ok(name, latency)
            } else {
                DependencyCheck::fail(
                    name,
                    Some(latency),
                    format!("unexpected status {}", resp.status()),
                )
            }
        }
    }
}

/// Strip scheme/auth from a URL so it's safe to use as a map key.
fn sanitize_url_for_name(url: &str) -> String {
    url.trim_start_matches("https://")
        .trim_start_matches("http://")
        .split('/')
        .next()
        .unwrap_or(url)
        .to_string()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_aggregate_healthy() {
        let checks = vec![
            DependencyCheck::ok("postgres", 5),
            DependencyCheck::ok("redis", 3),
        ];
        let hc = HealthCheck::new(checks);
        assert_eq!(hc.status, HealthStatus::Healthy);
    }

    #[test]
    fn test_aggregate_unhealthy_if_any_fail() {
        let checks = vec![
            DependencyCheck::ok("postgres", 5),
            DependencyCheck::fail("redis", None, "connection refused"),
        ];
        let hc = HealthCheck::new(checks);
        assert_eq!(hc.status, HealthStatus::Unhealthy);
    }

    #[test]
    fn test_aggregate_degraded() {
        let checks = vec![
            DependencyCheck::ok("postgres", 5),
            DependencyCheck {
                name: "redis".to_string(),
                status: HealthStatus::Degraded,
                latency_ms: Some(300),
                error: None,
            },
        ];
        let hc = HealthCheck::new(checks);
        assert_eq!(hc.status, HealthStatus::Degraded);
    }

    #[test]
    fn test_liveness_ok_is_healthy() {
        let hc = HealthCheck::liveness_ok();
        assert_eq!(hc.status, HealthStatus::Healthy);
        assert!(hc.checks.is_empty());
    }

    #[test]
    fn test_health_check_serializes_to_json() {
        let checks = vec![DependencyCheck::ok("postgres", 12)];
        let hc = HealthCheck::new(checks);
        let json = serde_json::to_string(&hc).expect("serialization failed");
        assert!(json.contains("\"status\":\"healthy\""));
        assert!(json.contains("\"postgres\""));
    }

    #[test]
    fn test_dependency_check_fail_serializes_error() {
        let dc = DependencyCheck::fail("redis", Some(10), "timeout");
        let json = serde_json::to_string(&dc).unwrap();
        assert!(json.contains("\"error\":\"timeout\""));
        assert!(json.contains("\"status\":\"unhealthy\""));
    }

    #[test]
    fn test_sanitize_url() {
        assert_eq!(sanitize_url_for_name("https://rpc.example.com/path"), "rpc.example.com");
        assert_eq!(sanitize_url_for_name("http://localhost:8000"), "localhost:8000");
    }

    #[tokio::test]
    async fn test_check_live_returns_healthy() {
        let svc = HealthService::from_env();
        let result = svc.check_live().await;
        assert_eq!(result.status, HealthStatus::Healthy);
    }

    #[tokio::test]
    async fn test_check_ready_no_deps_configured() {
        // With no deps configured, the aggregate should be Healthy (vacuously).
        let svc = HealthService::new(HealthConfig {
            database_url: None,
            redis_url: None,
            rpc_node_urls: vec![],
        });
        let result = svc.check_ready().await;
        assert_eq!(result.status, HealthStatus::Healthy);
        assert!(result.checks.is_empty());
    }
}
