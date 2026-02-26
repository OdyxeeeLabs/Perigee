use reqwest::Client;
use serde::Deserialize;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

// ── Configuration constants ───────────────────────────────────────────────────

/// Number of consecutive health-check failures before a provider is tripped.
const CIRCUIT_BREAKER_THRESHOLD: u64 = 3;

/// How long a tripped provider is excluded from the pool.
const CIRCUIT_BREAKER_COOLDOWN: Duration = Duration::from_secs(5 * 60); // 5 minutes

/// Timeout for the lightweight `getLatestLedger` health probe.
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(10);

// ── Types ─────────────────────────────────────────────────────────────────────

/// A single Soroban RPC endpoint with optional authentication.
#[derive(Debug, Clone, Deserialize)]
pub struct RpcProvider {
    /// Human-readable label (e.g. "stellar-testnet", "blockdaemon-mainnet").
    pub name: String,
    /// Full JSON-RPC URL.
    pub url: String,
    /// Optional authentication header name (e.g. "Authorization", "X-API-Key").
    #[serde(default)]
    pub auth_header: Option<String>,
    /// Optional authentication header value (e.g. "Bearer <token>", "<api-key>").
    #[serde(default)]
    pub auth_value: Option<String>,
}

/// Runtime health state for a single provider.
#[derive(Debug)]
struct ProviderState {
    provider: RpcProvider,
    /// Rolling count of consecutive failures (reset on success).
    consecutive_failures: AtomicU64,
    /// When the circuit breaker was tripped (None = healthy).
    tripped_at: RwLock<Option<Instant>>,
    /// Latest ledger number returned by the last successful health check.
    latest_ledger: AtomicU64,
}

/// Thread-safe registry that tracks provider health and drives failover.
pub struct ProviderRegistry {
    states: Vec<Arc<ProviderState>>,
    client: Client,
}

impl ProviderRegistry {
    /// Build a registry from a prioritized list of providers.
    ///
    /// The order matters: the first provider is preferred when healthy.
    pub fn new(providers: Vec<RpcProvider>) -> Arc<Self> {
        let states = providers
            .into_iter()
            .map(|p| {
                Arc::new(ProviderState {
                    provider: p,
                    consecutive_failures: AtomicU64::new(0),
                    tripped_at: RwLock::new(None),
                    latest_ledger: AtomicU64::new(0),
                })
            })
            .collect();

        Arc::new(Self {
            states,
            client: Client::new(),
        })
    }

    /// Return the list of providers that are currently available for requests,
    /// in priority order (skipping tripped providers whose cooldown hasn't elapsed).
    pub async fn healthy_providers(&self) -> Vec<&RpcProvider> {
        let mut available = Vec::new();
        for state in &self.states {
            if self.is_available(state).await {
                available.push(&state.provider);
            }
        }
        available
    }

    /// Report a successful request to `url`. Resets the failure counter and
    /// clears any active trip.
    pub async fn report_success(&self, url: &str) {
        if let Some(state) = self.find_by_url(url) {
            state.consecutive_failures.store(0, Ordering::Relaxed);
            let mut tripped = state.tripped_at.write().await;
            *tripped = None;
        }
    }

    /// Report a failed request to `url`. Increments the failure counter and
    /// trips the circuit breaker when the threshold is reached.
    pub async fn report_failure(&self, url: &str) {
        if let Some(state) = self.find_by_url(url) {
            let prev = state.consecutive_failures.fetch_add(1, Ordering::Relaxed);
            if prev + 1 >= CIRCUIT_BREAKER_THRESHOLD {
                let mut tripped = state.tripped_at.write().await;
                if tripped.is_none() {
                    tracing::warn!(
                        provider = %state.provider.name,
                        url = %state.provider.url,
                        failures = prev + 1,
                        "Circuit breaker TRIPPED — provider excluded for {:?}",
                        CIRCUIT_BREAKER_COOLDOWN
                    );
                }
                *tripped = Some(Instant::now());
            }
        }
    }

    /// Determine whether a request to `url` should be retried on the next
    /// provider. Returns `true` for timeouts, HTTP 429, and 5xx status codes.
    pub fn is_retryable_status(status: u16) -> bool {
        status == 429 || status >= 500
    }

    // ── Background health checker ─────────────────────────────────────────

    /// Spawn a background Tokio task that periodically probes every provider
    /// with `getLatestLedger`.
    pub fn spawn_health_checker(
        self: &Arc<Self>,
        interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                registry.run_health_checks().await;
            }
        })
    }

    /// Execute a single round of health checks against all providers.
    async fn run_health_checks(&self) {
        for state in &self.states {
            let result = self.probe_provider(state).await;
            match result {
                Ok(ledger) => {
                    state.latest_ledger.store(ledger, Ordering::Relaxed);
                    state.consecutive_failures.store(0, Ordering::Relaxed);
                    let mut tripped = state.tripped_at.write().await;
                    *tripped = None;
                    tracing::debug!(
                        provider = %state.provider.name,
                        latest_ledger = ledger,
                        "Health check OK"
                    );
                }
                Err(e) => {
                    let prev = state.consecutive_failures.fetch_add(1, Ordering::Relaxed);
                    tracing::warn!(
                        provider = %state.provider.name,
                        consecutive_failures = prev + 1,
                        error = %e,
                        "Health check FAILED"
                    );
                    if prev + 1 >= CIRCUIT_BREAKER_THRESHOLD {
                        let mut tripped = state.tripped_at.write().await;
                        if tripped.is_none() {
                            tracing::warn!(
                                provider = %state.provider.name,
                                "Circuit breaker TRIPPED by health checker"
                            );
                        }
                        *tripped = Some(Instant::now());
                    }
                }
            }
        }
    }

    /// Call `getLatestLedger` on a single provider. Returns the ledger
    /// sequence number on success.
    async fn probe_provider(&self, state: &ProviderState) -> Result<u64, String> {
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getLatestLedger",
            "params": null
        });

        let mut req = self.client.post(&state.provider.url).json(&body);

        // Attach provider-specific auth header if configured.
        if let (Some(header), Some(value)) =
            (&state.provider.auth_header, &state.provider.auth_value)
        {
            req = req.header(header.as_str(), value.as_str());
        }

        let response = tokio::time::timeout(HEALTH_CHECK_TIMEOUT, req.send())
            .await
            .map_err(|_| "timeout".to_string())?
            .map_err(|e| format!("request error: {e}"))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status().as_u16()));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|e| format!("parse error: {e}"))?;

        json["result"]["sequence"]
            .as_u64()
            .ok_or_else(|| "missing sequence in response".to_string())
    }

    // ── Internal helpers ──────────────────────────────────────────────────

    fn find_by_url(&self, url: &str) -> Option<&Arc<ProviderState>> {
        self.states.iter().find(|s| s.provider.url == url)
    }

    async fn is_available(&self, state: &ProviderState) -> bool {
        let tripped = state.tripped_at.read().await;
        match *tripped {
            None => true,
            Some(when) => when.elapsed() >= CIRCUIT_BREAKER_COOLDOWN,
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider(name: &str, url: &str) -> RpcProvider {
        RpcProvider {
            name: name.to_string(),
            url: url.to_string(),
            auth_header: None,
            auth_value: None,
        }
    }

    fn make_provider_with_auth(name: &str, url: &str) -> RpcProvider {
        RpcProvider {
            name: name.to_string(),
            url: url.to_string(),
            auth_header: Some("X-API-Key".to_string()),
            auth_value: Some("secret-key-123".to_string()),
        }
    }

    #[tokio::test]
    async fn test_all_providers_healthy_initially() {
        let registry = ProviderRegistry::new(vec![
            make_provider("a", "http://a.test"),
            make_provider("b", "http://b.test"),
        ]);
        let healthy = registry.healthy_providers().await;
        assert_eq!(healthy.len(), 2);
        assert_eq!(healthy[0].url, "http://a.test");
        assert_eq!(healthy[1].url, "http://b.test");
    }

    #[tokio::test]
    async fn test_circuit_breaker_trips_after_threshold() {
        let registry = ProviderRegistry::new(vec![
            make_provider("a", "http://a.test"),
            make_provider("b", "http://b.test"),
        ]);

        // Simulate 3 consecutive failures on provider "a"
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            registry.report_failure("http://a.test").await;
        }

        let healthy = registry.healthy_providers().await;
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].url, "http://b.test");
    }

    #[tokio::test]
    async fn test_success_resets_failure_counter() {
        let registry = ProviderRegistry::new(vec![make_provider("a", "http://a.test")]);

        // Two failures, then a success
        registry.report_failure("http://a.test").await;
        registry.report_failure("http://a.test").await;
        registry.report_success("http://a.test").await;

        // Should still be healthy (counter reset before threshold)
        let healthy = registry.healthy_providers().await;
        assert_eq!(healthy.len(), 1);
    }

    #[tokio::test]
    async fn test_success_clears_tripped_state() {
        let registry = ProviderRegistry::new(vec![make_provider("a", "http://a.test")]);

        // Trip the breaker
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            registry.report_failure("http://a.test").await;
        }
        assert_eq!(registry.healthy_providers().await.len(), 0);

        // Report success (simulating health check recovery)
        registry.report_success("http://a.test").await;
        assert_eq!(registry.healthy_providers().await.len(), 1);
    }

    #[test]
    fn test_is_retryable_status() {
        assert!(ProviderRegistry::is_retryable_status(429));
        assert!(ProviderRegistry::is_retryable_status(500));
        assert!(ProviderRegistry::is_retryable_status(502));
        assert!(ProviderRegistry::is_retryable_status(503));
        assert!(!ProviderRegistry::is_retryable_status(200));
        assert!(!ProviderRegistry::is_retryable_status(400));
        assert!(!ProviderRegistry::is_retryable_status(404));
    }

    #[tokio::test]
    async fn test_report_failure_unknown_url_is_noop() {
        let registry = ProviderRegistry::new(vec![make_provider("a", "http://a.test")]);
        registry.report_failure("http://unknown.test").await;
        assert_eq!(registry.healthy_providers().await.len(), 1);
    }

    #[tokio::test]
    async fn test_provider_with_auth_headers() {
        let provider = make_provider_with_auth("authed", "http://authed.test");
        assert_eq!(provider.auth_header.as_deref(), Some("X-API-Key"));
        assert_eq!(provider.auth_value.as_deref(), Some("secret-key-123"));

        let registry = ProviderRegistry::new(vec![provider]);
        let healthy = registry.healthy_providers().await;
        assert_eq!(healthy.len(), 1);
        assert_eq!(healthy[0].auth_header.as_deref(), Some("X-API-Key"));
    }

    #[tokio::test]
    async fn test_priority_order_preserved() {
        let registry = ProviderRegistry::new(vec![
            make_provider("primary", "http://primary.test"),
            make_provider("secondary", "http://secondary.test"),
            make_provider("tertiary", "http://tertiary.test"),
        ]);
        let healthy = registry.healthy_providers().await;
        assert_eq!(healthy[0].name, "primary");
        assert_eq!(healthy[1].name, "secondary");
        assert_eq!(healthy[2].name, "tertiary");
    }
}
