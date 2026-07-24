//! `StellarService` — a single, shared HTTP transport for all Stellar RPC calls.
//!
//! # Why this exists
//!
//! Every subsystem (simulation engine, fee collector, real-provider) previously
//! allocated its own `reqwest::Client` and duplicated the same retry / timeout /
//! auth logic in an ad-hoc way.  That means:
//!
//! - No connection pool sharing — each `Client::new()` creates an independent
//!   pool.  Under load this multiplies open TCP connections.
//! - No consistent retry policy — some callers retried, some didn't, and the
//!   retry delays varied.
//! - No per-call observability (tracing spans, failure counters).
//!
//! `StellarService` centralises all of that:
//!
//! - **One shared `reqwest::Client`** (and therefore one connection pool) for
//!   the entire process.
//! - **Per-call timeout** — configurable via [`StellarServiceConfig`].
//! - **Exponential back-off with jitter** — capped at [`MAX_BACKOFF`].
//! - **Circuit-breaker integration** — delegates success/failure tracking to
//!   the existing [`ProviderRegistry`] so the registry's health scores stay
//!   accurate.
//! - **Tracing** — every call is wrapped in a span; failures are logged.
//!
//! # Usage
//!
//! ```rust,ignore
//! let svc = StellarService::new(Arc::clone(&registry), StellarServiceConfig::default());
//!
//! let result: serde_json::Value = svc
//!     .call_rpc(
//!         &provider,
//!         "simulateTransaction",
//!         serde_json::json!({ "transaction": xdr }),
//!     )
//!     .await?;
//! ```
//!
//! The service is cheaply `Clone`able (it wraps an `Arc` internally) and
//! `Send + Sync`, so it can be stored in `AppState` and injected into every
//! handler / background task.

use crate::rpc_provider::{ProviderRegistry, RpcProvider};
use reqwest::Client;
use serde_json::Value;
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;

// ── Constants ─────────────────────────────────────────────────────────────────

/// Maximum number of attempts per RPC call (1 initial + retries).
pub const DEFAULT_MAX_ATTEMPTS: u32 = 3;

/// Base delay before the first retry.
pub const DEFAULT_BASE_DELAY: Duration = Duration::from_millis(200);

/// Hard ceiling on any individual back-off delay.
pub const MAX_BACKOFF: Duration = Duration::from_secs(10);

/// Default per-call network timeout.
pub const DEFAULT_CALL_TIMEOUT: Duration = Duration::from_secs(30);

// ── Error type ────────────────────────────────────────────────────────────────

/// Errors returned by [`StellarService::call_rpc`].
#[derive(Error, Debug)]
pub enum StellarServiceError {
    /// The server returned a non-2xx HTTP status.
    #[error("RPC HTTP error {status} from {url}")]
    HttpError { status: u16, url: String },

    /// The call timed out (per-call deadline exceeded).
    #[error("RPC call timed out after {timeout_ms}ms to {url}")]
    Timeout { timeout_ms: u64, url: String },

    /// A transport-level error (TCP, DNS, TLS …).
    #[error("RPC network error to {url}: {source}")]
    Network {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    /// The response body could not be decoded as JSON.
    #[error("RPC response parse error from {url}: {source}")]
    ParseError {
        url: String,
        #[source]
        source: reqwest::Error,
    },

    /// All providers are unavailable (circuit breakers tripped).
    #[error("No healthy RPC providers available")]
    NoHealthyProviders,

    /// Every retry attempt failed.
    #[error("All {attempts} RPC attempt(s) to {url} failed; last error: {last_error}")]
    AllAttemptsFailed {
        attempts: u32,
        url: String,
        last_error: String,
    },
}

impl StellarServiceError {
    /// Returns `true` for transient errors that are worth retrying.
    pub fn is_retryable(&self) -> bool {
        match self {
            Self::Timeout { .. } | Self::Network { .. } => true,
            Self::HttpError { status, .. } => {
                // 429 Too Many Requests and all 5xx server errors are retryable.
                *status == 429 || *status >= 500
            }
            _ => false,
        }
    }

    /// Returns `true` when the RTT sample should be recorded in the registry.
    ///
    /// Transport failures (timeout, TCP error) don't reflect the provider's
    /// own latency, so we skip them to avoid poisoning the EMA.
    fn should_record_rtt(&self) -> bool {
        !matches!(self, Self::Timeout { .. } | Self::Network { .. })
    }
}

// ── Configuration ─────────────────────────────────────────────────────────────

/// Tuning knobs for [`StellarService`].
#[derive(Debug, Clone)]
pub struct StellarServiceConfig {
    /// Wall-clock timeout applied to each individual HTTP send.
    pub call_timeout: Duration,
    /// Maximum total attempts (initial attempt + retries).
    pub max_attempts: u32,
    /// Base delay for exponential back-off.  The actual delay for retry *n*
    /// is `base_delay × 2^(n−1)` capped at [`MAX_BACKOFF`], with ±25% jitter.
    pub base_delay: Duration,
}

impl Default for StellarServiceConfig {
    fn default() -> Self {
        Self {
            call_timeout: DEFAULT_CALL_TIMEOUT,
            max_attempts: DEFAULT_MAX_ATTEMPTS,
            base_delay: DEFAULT_BASE_DELAY,
        }
    }
}

impl StellarServiceConfig {
    /// Set the per-call network timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.call_timeout = timeout;
        self
    }

    /// Set the maximum number of attempts (≥ 1).
    pub fn with_max_attempts(mut self, max_attempts: u32) -> Self {
        self.max_attempts = max_attempts.max(1);
        self
    }

    /// Set the base delay for exponential back-off.
    pub fn with_base_delay(mut self, base_delay: Duration) -> Self {
        self.base_delay = base_delay;
        self
    }
}

// ── Service ───────────────────────────────────────────────────────────────────

struct Inner {
    client: Client,
    registry: Arc<ProviderRegistry>,
    config: StellarServiceConfig,
}

/// Shared HTTP transport for all Stellar JSON-RPC calls.
///
/// Cheap to `Clone` (wraps an `Arc`). Store once in `AppState` and inject
/// wherever an RPC call is needed.
#[derive(Clone)]
pub struct StellarService(Arc<Inner>);

impl StellarService {
    /// Construct a new service.
    ///
    /// One `StellarService` should be created per process — its internal
    /// `reqwest::Client` maintains the shared connection pool.
    pub fn new(registry: Arc<ProviderRegistry>, config: StellarServiceConfig) -> Self {
        let client = Client::builder()
            // Keep up to 20 idle connections per host ready for reuse.
            .pool_max_idle_per_host(20)
            // TCP keep-alive prevents silent connection drops through NAT.
            .tcp_keepalive(Duration::from_secs(60))
            // No global timeout here — we apply per-call timeouts in call_rpc.
            .build()
            .expect("reqwest::Client builder should not fail with these settings");

        Self(Arc::new(Inner {
            client,
            registry,
            config,
        }))
    }

    /// Send a JSON-RPC 2.0 request to `provider`.
    ///
    /// Retries up to `config.max_attempts` times with exponential back-off for
    /// transient errors (timeouts, 429s, 5xx).  Reports success / failure to
    /// the registry after every attempt so circuit-breaker scores stay current.
    ///
    /// # Parameters
    ///
    /// - `provider` — the specific RPC node to call (obtain from
    ///   [`ProviderRegistry::healthy_providers`] or
    ///   [`ProviderRegistry::providers_by_latency`]).
    /// - `method` — JSON-RPC method name, e.g. `"simulateTransaction"`.
    /// - `params` — the `params` value.  Use `serde_json::Value::Null` for
    ///   methods that take no parameters.
    pub async fn call_rpc(
        &self,
        provider: &RpcProvider,
        method: &str,
        params: Value,
    ) -> Result<Value, StellarServiceError> {
        let inner = &*self.0;
        let url = &provider.url;

        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id":      1,
            "method":  method,
            "params":  params,
        });

        let mut last_error: Option<StellarServiceError> = None;

        for attempt in 1..=inner.config.max_attempts {
            // Back-off before every retry (not before the first attempt).
            if attempt > 1 {
                let delay = Self::backoff_delay(&inner.config, attempt);
                tracing::debug!(
                    attempt,
                    delay_ms = delay.as_millis(),
                    url = %url,
                    method,
                    "Retrying RPC call after back-off"
                );
                tokio::time::sleep(delay).await;
            }

            let started = std::time::Instant::now();
            let result = self.do_send(provider, &body).await;
            let rtt_us = started.elapsed().as_micros() as u64;

            match result {
                Ok(value) => {
                    inner.registry.record_rtt(url, rtt_us);
                    inner.registry.report_success(url).await;
                    tracing::debug!(
                        attempt,
                        rtt_us,
                        url = %url,
                        method,
                        "RPC call succeeded"
                    );
                    return Ok(value);
                }
                Err(e) => {
                    if e.should_record_rtt() {
                        inner.registry.record_rtt(url, rtt_us);
                    }
                    inner.registry.report_failure(url).await;

                    let retryable = e.is_retryable();
                    let has_more = attempt < inner.config.max_attempts;

                    if retryable && has_more {
                        tracing::warn!(
                            attempt,
                            max = inner.config.max_attempts,
                            error = %e,
                            url = %url,
                            method,
                            "Retryable RPC error; will retry"
                        );
                        last_error = Some(e);
                        continue;
                    }

                    // Non-retryable error, or final attempt.
                    tracing::error!(
                        attempt,
                        error = %e,
                        url = %url,
                        method,
                        "RPC call failed"
                    );

                    if !has_more || retryable {
                        // Wrap as AllAttemptsFailed on exhaustion.
                        let last = e.to_string();
                        return Err(StellarServiceError::AllAttemptsFailed {
                            attempts: attempt,
                            url: url.clone(),
                            last_error: last,
                        });
                    }

                    return Err(e);
                }
            }
        }

        // Retryable path exhausted all attempts.
        let last = last_error
            .as_ref()
            .map(|e| e.to_string())
            .unwrap_or_default();
        Err(StellarServiceError::AllAttemptsFailed {
            attempts: inner.config.max_attempts,
            url: url.clone(),
            last_error: last,
        })
    }

    // ── Private helpers ───────────────────────────────────────────────────────

    /// Execute a single HTTP send without any retry logic.
    async fn do_send(
        &self,
        provider: &RpcProvider,
        body: &Value,
    ) -> Result<Value, StellarServiceError> {
        let inner = &*self.0;
        let url = &provider.url;
        let timeout = inner.config.call_timeout;

        let mut req = inner.client.post(url).json(body);

        // Attach optional API-key / bearer auth headers.
        if let (Some(header), Some(value)) = (
            provider.auth_header.as_deref(),
            provider.auth_value.as_deref(),
        ) {
            req = req.header(header, value);
        }

        let response = tokio::time::timeout(timeout, req.send())
            .await
            .map_err(|_| StellarServiceError::Timeout {
                timeout_ms: timeout.as_millis() as u64,
                url: url.clone(),
            })?
            .map_err(|e| {
                if e.is_timeout() {
                    StellarServiceError::Timeout {
                        timeout_ms: timeout.as_millis() as u64,
                        url: url.clone(),
                    }
                } else {
                    StellarServiceError::Network {
                        url: url.clone(),
                        source: e,
                    }
                }
            })?;

        let status = response.status();
        if !status.is_success() {
            return Err(StellarServiceError::HttpError {
                status: status.as_u16(),
                url: url.clone(),
            });
        }

        response
            .json::<Value>()
            .await
            .map_err(|e| StellarServiceError::ParseError {
                url: url.clone(),
                source: e,
            })
    }

    /// Compute the back-off delay for `attempt` (1-based) with ±25% jitter.
    ///
    /// delay = base × 2^(attempt−2), capped at [`MAX_BACKOFF`].
    /// (attempt=2 → base×1, attempt=3 → base×2, …)
    fn backoff_delay(config: &StellarServiceConfig, attempt: u32) -> Duration {
        let exponent = attempt.saturating_sub(2);
        let base_ms = config.base_delay.as_millis() as u64;
        let raw_ms = base_ms.saturating_mul(1u64.saturating_shl(exponent));
        let capped_ms = raw_ms.min(MAX_BACKOFF.as_millis() as u64);

        // Simple jitter: use subsecond nanoseconds of the current time as a
        // cheap pseudo-random source — avoids adding a `rand` dependency.
        let now_ns = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .subsec_nanos() as u64;

        // ±25%: shift by up to capped_ms/4 in either direction.
        let quarter = (capped_ms / 4).max(1);
        let jitter = (now_ns % (quarter * 2)) as i64 - quarter as i64;
        let final_ms = (capped_ms as i64 + jitter).max(0) as u64;

        Duration::from_millis(final_ms)
    }
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timeout_is_retryable() {
        assert!(StellarServiceError::Timeout {
            timeout_ms: 5000,
            url: "http://x".into()
        }
        .is_retryable());
    }

    #[test]
    fn http_429_and_5xx_are_retryable() {
        for status in [429u16, 500, 502, 503, 504] {
            assert!(
                StellarServiceError::HttpError {
                    status,
                    url: "http://x".into()
                }
                .is_retryable(),
                "expected {status} to be retryable"
            );
        }
    }

    #[test]
    fn http_4xx_not_retryable() {
        for status in [400u16, 401, 403, 404] {
            assert!(
                !StellarServiceError::HttpError {
                    status,
                    url: "http://x".into()
                }
                .is_retryable(),
                "expected {status} to NOT be retryable"
            );
        }
    }

    #[test]
    fn no_healthy_providers_not_retryable() {
        assert!(!StellarServiceError::NoHealthyProviders.is_retryable());
    }

    #[test]
    fn backoff_never_exceeds_max() {
        let config = StellarServiceConfig::default();
        for attempt in 2..=15u32 {
            let delay = StellarService::backoff_delay(&config, attempt);
            assert!(
                delay <= MAX_BACKOFF + Duration::from_millis(1),
                "delay {delay:?} exceeds MAX_BACKOFF at attempt {attempt}"
            );
        }
    }

    #[test]
    fn backoff_first_retry_is_near_base() {
        let config = StellarServiceConfig::default();
        // attempt=2 → first retry; raw delay = base_delay × 2^0 = base_delay
        let delay = StellarService::backoff_delay(&config, 2);
        // Allow 25% jitter on top of base_delay (200ms).
        assert!(
            delay <= Duration::from_millis(250),
            "first retry delay {delay:?} should be close to base delay"
        );
    }

    #[test]
    fn config_builder_methods() {
        let cfg = StellarServiceConfig::default()
            .with_timeout(Duration::from_secs(5))
            .with_max_attempts(5)
            .with_base_delay(Duration::from_millis(100));

        assert_eq!(cfg.call_timeout, Duration::from_secs(5));
        assert_eq!(cfg.max_attempts, 5);
        assert_eq!(cfg.base_delay, Duration::from_millis(100));
    }

    #[test]
    fn max_attempts_clamped_to_one() {
        let cfg = StellarServiceConfig::default().with_max_attempts(0);
        assert_eq!(cfg.max_attempts, 1);
    }
}
