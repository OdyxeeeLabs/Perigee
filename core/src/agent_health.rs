//! Agent health attestation service.
//!
//! Uses async I/O for health checks. All health check methods are async
//! and can be composed with `tokio::join!` or `futures::join_all` for
//! concurrent agent health monitoring.

use serde::Serialize;
use std::collections::HashMap;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct HealthStatus {
    pub agent_id: String,
    pub self_reported: bool,
    pub peer_attestations: Vec<String>,
    pub attestation_threshold: usize,
}

impl HealthStatus {
    pub fn new(agent_id: String, threshold: usize) -> Self {
        Self {
            agent_id,
            self_reported: false,
            peer_attestations: Vec::new(),
            attestation_threshold: threshold,
        }
    }

    pub fn add_peer_attestation(&mut self, peer_id: &str) {
        if !self.peer_attestations.contains(&peer_id.to_string()) {
            self.peer_attestations.push(peer_id.to_string());
        }
    }

    pub fn is_healthy(&self) -> bool {
        self.self_reported && self.peer_attestations.len() >= self.attestation_threshold
    }
}

pub struct HealthAttestationService {
    attestations: HashMap<String, HealthStatus>,
}

impl Default for HealthAttestationService {
    fn default() -> Self {
        Self::new()
    }
}

impl HealthAttestationService {
    pub fn new() -> Self {
        Self {
            attestations: HashMap::new(),
        }
    }

    pub fn register_agent(&mut self, agent_id: String, threshold: usize) {
        self.attestations
            .insert(agent_id.clone(), HealthStatus::new(agent_id, threshold));
    }

    pub fn record_self_report(&mut self, agent_id: &str) {
        if let Some(status) = self.attestations.get_mut(agent_id) {
            status.self_reported = true;
        }
    }

    pub fn record_peer_attestation(&mut self, target: &str, peer: &str) {
        if let Some(status) = self.attestations.get_mut(target) {
            status.add_peer_attestation(peer);
        }
    }

    pub fn check_health(&self, agent_id: &str) -> Option<bool> {
        self.attestations
            .get(agent_id)
            .map(|status| status.is_healthy())
    }

    pub fn remove_agent(&mut self, agent_id: &str) {
        self.attestations.remove(agent_id);
    }

    /// Async health check for a single agent.
    pub async fn check_health_async(&self, agent_id: String) -> Option<bool> {
        self.check_health(&agent_id)
    }
}

// ── Deep liveness probe (CORE-14) ───────────────────────────────────────────

/// The kind of downstream dependency a deep probe exercises.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DependencyKind {
    Database,
    Rpc,
    KeyValueStore,
}

/// Result of a single dependency check.
#[derive(Debug, Clone, Serialize)]
pub struct DependencyStatus {
    pub name: String,
    pub kind: DependencyKind,
    pub healthy: bool,
    pub detail: Option<String>,
}

impl DependencyStatus {
    /// A healthy dependency check.
    pub fn healthy(name: &str, kind: DependencyKind) -> Self {
        Self {
            name: name.to_string(),
            kind,
            healthy: true,
            detail: None,
        }
    }

    /// An unhealthy dependency check, carrying a reason.
    pub fn unhealthy(name: &str, kind: DependencyKind, detail: impl Into<String>) -> Self {
        Self {
            name: name.to_string(),
            kind,
            healthy: false,
            detail: Some(detail.into()),
        }
    }
}

/// Aggregated deep-liveness report: healthy only when every dependency answers.
#[derive(Debug, Clone, Serialize)]
pub struct DeepHealthReport {
    pub agent_id: String,
    pub healthy: bool,
    pub checks: Vec<DependencyStatus>,
}

impl DeepHealthReport {
    /// `true` when every dependency check passed.
    pub fn is_healthy(&self) -> bool {
        self.checks.iter().all(|c| c.healthy)
    }
}

/// Run a single dependency check and capture its outcome.
pub async fn probe_dependency<F, T>(name: &str, kind: DependencyKind, check: F) -> DependencyStatus
where
    F: std::future::Future<Output = Result<T, String>>,
{
    match check.await {
        Ok(_) => DependencyStatus::healthy(name, kind),
        Err(detail) => DependencyStatus::unhealthy(name, kind, detail),
    }
}

/// Aggregate dependency results into a deep-liveness report (CORE-14).
pub fn deep_liveness(agent_id: &str, checks: Vec<DependencyStatus>) -> DeepHealthReport {
    DeepHealthReport {
        agent_id: agent_id.to_string(),
        healthy: checks.iter().all(|c| c.healthy),
        checks,
    }
}

/// Handler shape for `GET /health/deep`: actual DB / RPC / key-value-store
/// checks rather than a blind `200 OK`, suitable for Kubernetes liveness
/// probes. Returns `200 OK` when every dependency passes, otherwise
/// `503 Service Unavailable`.
pub async fn deep_health_handler(
    agent_id: &str,
    checks: Vec<DependencyStatus>,
) -> (axum::http::StatusCode, axum::Json<DeepHealthReport>) {
    let report = deep_liveness(agent_id, checks);
    let status = if report.is_healthy() {
        axum::http::StatusCode::OK
    } else {
        axum::http::StatusCode::SERVICE_UNAVAILABLE
    };
    (status, axum::Json(report))
}

// ── Retry policy (CORE-15) ───────────────────────────────────────────────────

/// Exponential-backoff retry policy for all external calls (CORE-15).
///
/// Configured from the environment:
/// - `RETRY_MAX_ATTEMPTS`  — total attempts before giving up (default 3).
/// - `RETRY_BASE_DELAY_MS` — first retry delay (default 100).
/// - `RETRY_MAX_DELAY_MS`  — backoff ceiling (default 2_000).
/// - `RETRY_MULTIPLIER`    — exponential growth factor (default 2.0).
/// - `RETRY_JITTER`        — jitter ratio 0–1 that spreads retries (default 0.2).
///
/// Pair with [`probe_dependency_with_retry`] to make dependency probes resilient.
#[derive(Debug, Clone)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    pub base_delay: Duration,
    pub max_delay: Duration,
    pub multiplier: f64,
    pub jitter: f64,
}

impl RetryPolicy {
    /// Create a policy; `max_attempts` is clamped to at least 1.
    pub fn new(
        max_attempts: u32,
        base_delay: Duration,
        max_delay: Duration,
        multiplier: f64,
        jitter: f64,
    ) -> Self {
        Self {
            max_attempts: max_attempts.max(1),
            base_delay,
            max_delay,
            multiplier,
            jitter,
        }
    }

    /// Build the policy from the `RETRY_*` environment variables.
    pub fn from_env() -> Self {
        let env_num = |key: &str, default: f64| -> f64 {
            std::env::var(key)
                .ok()
                .and_then(|v| v.trim().parse::<f64>().ok())
                .unwrap_or(default)
        };

        let max_attempts = env_num("RETRY_MAX_ATTEMPTS", 3.0).max(1.0) as u32;
        let base_ms = env_num("RETRY_BASE_DELAY_MS", 100.0).max(1.0) as u64;
        let max_ms = env_num("RETRY_MAX_DELAY_MS", 2_000.0).max(1.0) as u64;

        Self::new(
            max_attempts,
            Duration::from_millis(base_ms),
            Duration::from_millis(max_ms),
            env_num("RETRY_MULTIPLIER", 2.0).max(1.0),
            env_num("RETRY_JITTER", 0.2).clamp(0.0, 1.0),
        )
    }

    /// Backoff delay before the `attempt`-th retry (attempts count from 0):
    ///
    /// ```text
    /// delay = min(max_delay, base * multiplier^attempt) × (1 - jitter + 2·jitter·rand)
    /// ```
    pub fn backoff(&self, attempt: u32) -> Duration {
        let exponent = attempt.min(20) as i32;
        let raw = self.base_delay.as_millis() as f64 * self.multiplier.powi(exponent);
        let capped = raw.min(self.max_delay.as_millis() as f64);
        let rnd = rand::random::<f64>();
        let jittered = capped * (1.0 - self.jitter + (2.0 * self.jitter * rnd));
        Duration::from_millis(jittered.max(1.0) as u64)
    }

    /// Run `operation`, retrying up to `max_attempts` times with exponential
    /// backoff and jitter. The last error is surfaced once attempts are spent.
    pub async fn retry<F, Fut, T>(&self, mut operation: F) -> Result<T, String>
    where
        F: FnMut() -> Fut,
        Fut: std::future::Future<Output = Result<T, String>>,
    {
        let mut attempt = 0u32;
        loop {
            match operation().await {
                Ok(value) => return Ok(value),
                Err(err) if attempt + 1 >= self.max_attempts => return Err(err),
                Err(err) => {
                    let delay = self.backoff(attempt);
                    tracing::warn!(
                        attempt,
                        delay_ms = delay.as_millis(),
                        error = %err,
                        "external call failed; retrying"
                    );
                    tokio::time::sleep(delay).await;
                    attempt += 1;
                }
            }
        }
    }
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self::new(
            3,
            Duration::from_millis(100),
            Duration::from_millis(2_000),
            2.0,
            0.2,
        )
    }
}

/// Probe a dependency through [`RetryPolicy::retry`] so transient failures are
/// absorbed before the deep probe reports unhealthy (CORE-14 + CORE-15).
pub async fn probe_dependency_with_retry<F, Fut, T>(
    policy: &RetryPolicy,
    name: &str,
    kind: DependencyKind,
    operation: F,
) -> DependencyStatus
where
    F: FnMut() -> Fut,
    Fut: std::future::Future<Output = Result<T, String>>,
{
    match policy.retry(operation).await {
        Ok(_) => DependencyStatus::healthy(name, kind),
        Err(detail) => DependencyStatus::unhealthy(name, kind, detail),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_health_status_requires_both() {
        let mut status = HealthStatus::new("a".to_string(), 2);
        assert!(!status.is_healthy());
        status.self_reported = true;
        assert!(!status.is_healthy());
        status.add_peer_attestation("p1");
        assert!(!status.is_healthy());
        status.add_peer_attestation("p2");
        assert!(status.is_healthy());
    }

    #[test]
    fn test_no_duplicate_attestations() {
        let mut status = HealthStatus::new("a".to_string(), 1);
        status.add_peer_attestation("p1");
        status.add_peer_attestation("p1");
        assert_eq!(status.peer_attestations.len(), 1);
    }

    #[test]
    fn test_service_check_health() {
        let mut svc = HealthAttestationService::new();
        svc.register_agent("a".to_string(), 1);
        assert_eq!(svc.check_health("a"), Some(false));
        svc.record_self_report("a");
        svc.record_peer_attestation("a", "p1");
        assert_eq!(svc.check_health("a"), Some(true));
        assert_eq!(svc.check_health("unknown"), None);
    }

    #[tokio::test]
    async fn test_check_health_async() {
        let mut svc = HealthAttestationService::new();
        svc.register_agent("a".to_string(), 1);
        svc.record_self_report("a");
        svc.record_peer_attestation("a", "p1");
        assert_eq!(svc.check_health_async("a".to_string()).await, Some(true));
        assert_eq!(svc.check_health_async("unknown".to_string()).await, None);
    }

    #[tokio::test]
    async fn probe_dependency_captures_outcome() {
        let ok = probe_dependency("db", DependencyKind::Database, async {
            Ok::<(), String>(())
        })
        .await;
        assert!(ok.healthy);

        let bad = probe_dependency("rpc", DependencyKind::Rpc, async {
            Err::<(), String>("timeout".to_string())
        })
        .await;
        assert!(!bad.healthy);
        assert_eq!(bad.kind, DependencyKind::Rpc);
        assert_eq!(bad.detail.as_deref(), Some("timeout"));
    }

    #[tokio::test]
    async fn deep_liveness_report_is_all_or_nothing() {
        let report = deep_liveness(
            "agent-1",
            vec![
                DependencyStatus::healthy("db", DependencyKind::Database),
                DependencyStatus::healthy("rpc", DependencyKind::Rpc),
            ],
        );
        assert!(report.is_healthy());
        assert!(report.healthy);

        let degraded = deep_liveness(
            "agent-1",
            vec![
                DependencyStatus::healthy("db", DependencyKind::Database),
                DependencyStatus::unhealthy("rpc", DependencyKind::Rpc, "down"),
            ],
        );
        assert!(!degraded.is_healthy());
    }

    #[tokio::test]
    async fn deep_health_handler_reflects_dependency_state() {
        let (code, _report) = deep_health_handler(
            "agent-1",
            vec![DependencyStatus::healthy("db", DependencyKind::Database)],
        )
        .await;
        assert_eq!(code.as_u16(), 200);

        let (code, _report) = deep_health_handler(
            "agent-1",
            vec![DependencyStatus::unhealthy("kv", DependencyKind::KeyValueStore, "no conn")],
        )
        .await;
        assert_eq!(code.as_u16(), 503);
    }

    #[test]
    fn retry_backoff_is_bounded_and_grows() {
        let policy = RetryPolicy::default();
        let first = policy.backoff(0);
        let later = policy.backoff(5);

        assert!(first.as_millis() >= 1);
        // Attempt 5's raw backoff (100ms * 2^5 = 3.2s) exceeds the 2s ceiling, so
        // the delay sits in the capped jitter band [0.8·max, 1.2·max].
        let max_ms = policy.max_delay.as_millis() as f64;
        assert!(later.as_millis() as f64 >= max_ms * 0.4);
        assert!(later.as_millis() as f64 <= max_ms * 1.3);
        // Later attempts never wait less than earlier ones within the band.
        assert!(later.as_millis() >= first.as_millis());
    }

    #[tokio::test]
    async fn retry_succeeds_after_transient_failures() {
        let policy = RetryPolicy::new(
            3,
            Duration::from_millis(1),
            Duration::from_millis(1),
            2.0,
            0.0,
        );
        let mut calls = 0u32;

        let result = policy
            .retry(|| {
                calls += 1;
                async move {
                    if calls < 3 {
                        Err(format!("transient {calls}"))
                    } else {
                        Ok(calls)
                    }
                }
            })
            .await;

        assert_eq!(result, Ok(3));
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn retry_gives_up_with_last_error() {
        let policy = RetryPolicy::new(
            3,
            Duration::from_millis(1),
            Duration::from_millis(1),
            2.0,
            0.0,
        );
        let mut calls = 0u32;

        let result = policy
            .retry(|| {
                calls += 1;
                async move { Err(format!("boom {calls}")) }
            })
            .await;

        assert_eq!(result, Err("boom 3".to_string()));
        assert_eq!(calls, 3);
    }

    #[tokio::test]
    async fn probe_dependency_with_retry_reports_final_status() {
        let policy = RetryPolicy::new(
            2,
            Duration::from_millis(1),
            Duration::from_millis(1),
            2.0,
            0.0,
        );
        let status = probe_dependency_with_retry(
            &policy,
            "rpc",
            DependencyKind::Rpc,
            || async { Err::<(), _>("down".to_string()) },
        )
        .await;

        assert!(!status.healthy);
        assert_eq!(status.detail.as_deref(), Some("down"));
    }
}
