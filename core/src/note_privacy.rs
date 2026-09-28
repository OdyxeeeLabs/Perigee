use std::env;
use std::future::Future;
use std::time::{Duration, Instant};

use tokio::time::timeout;
use tracing;

use crate::metrics::Metrics;

pub struct NoteEnvelope {
    pub padded_data: Vec<u8>,
    pub fixed_size: usize,
}

impl NoteEnvelope {
    pub fn new(payload: &[u8], target_size: usize) -> Self {
        let padded = Self::pad_to_fixed_size(payload, target_size);
        Self {
            padded_data: padded,
            fixed_size: target_size,
        }
    }

    pub fn from_string(text: &str, target_size: usize) -> Self {
        Self::new(text.as_bytes(), target_size)
    }

    pub fn pad_to_fixed_size(data: &[u8], target_size: usize) -> Vec<u8> {
        let mut padded = data.to_vec();
        if padded.len() < target_size {
            let padding_len = target_size - padded.len();
            // Pad with deterministic pattern to avoid leaking real content
            for i in 0..padding_len {
                padded.push((i % 256) as u8);
            }
        } else {
            padded.truncate(target_size);
        }
        padded
    }

    pub fn reveal_size_hint(&self) -> usize {
        self.fixed_size
    }
}

pub struct NoteSanitizer;

impl NoteSanitizer {
    pub fn sanitize_metadata(metadata: &str) -> String {
        metadata
            .chars()
            .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
            .collect()
    }

    pub fn redact_counterparties(metadata: &str) -> String {
        metadata
            .replace("from:", "from: [REDACTED]")
            .replace("to:", "to: [REDACTED]")
            .replace("counterparty:", "counterparty: [REDACTED]")
    }

    pub fn hash_identifier(id: &str) -> String {
        use std::collections::hash_map::DefaultHasher;
        use std::hash::{Hash, Hasher};
        let mut hasher = DefaultHasher::new();
        id.hash(&mut hasher);
        format!("{:016x}", hasher.finish())
    }
}

// ---------------------------------------------------------------------------
// Shielded-rail settlement fallback (BE-028)
// ---------------------------------------------------------------------------
//
// Fee settlements prefer the shielded tokenless rail, which keeps
// counterparty information private.  When that rail is unavailable (network
// congestion, downtime) a settlement must not block indefinitely — instead it
// waits at most `SHIELDED_RAIL_TIMEOUT_SECS` (default 30s) and then falls back
// to transparent settlement, logging a warning and recording a metric so the
// fallback frequency is observable.

/// Environment variable controlling how long a settlement waits on the
/// shielded tokenless rail before falling back to transparent settlement.
pub const SHIELDED_RAIL_TIMEOUT_ENV_VAR: &str = "SHIELDED_RAIL_TIMEOUT_SECS";

/// Default shielded-rail timeout in seconds (per BE-028).
pub const DEFAULT_SHIELDED_RAIL_TIMEOUT_SECS: u64 = 30;

/// The rail a settlement was actually settled through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SettlementRail {
    /// Settled through the shielded tokenless rail.
    Shielded,
    /// Settled through the transparent fallback rail.
    Transparent,
}

/// Why the shielded rail could not be used for a settlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RailUnavailable {
    /// The shielded rail did not respond within the configured timeout.
    Timeout,
    /// The shielded rail reported that it is unavailable (e.g. congestion).
    Unavailable(String),
}

/// Configuration for the shielded-rail fallback.
#[derive(Debug, Clone)]
pub struct ShieldedRailFallbackConfig {
    /// How long to wait on the shielded rail before falling back.
    pub timeout: Duration,
}

impl ShieldedRailFallbackConfig {
    /// Build the config from `SHIELDED_RAIL_TIMEOUT_SECS` (default 30s).
    pub fn from_env() -> Self {
        Self {
            timeout: Self::parse_timeout_env(
                env::var(SHIELDED_RAIL_TIMEOUT_ENV_VAR).ok().as_deref(),
            ),
        }
    }

    /// Parse a timeout value (seconds).  Missing, non-numeric or zero values
    /// fall back to the default of 30 seconds.
    pub fn parse_timeout_env(raw: Option<&str>) -> Duration {
        let Some(raw) = raw else {
            return Duration::from_secs(DEFAULT_SHIELDED_RAIL_TIMEOUT_SECS);
        };

        match raw.trim().parse::<u64>() {
            Ok(secs) if secs > 0 => Duration::from_secs(secs),
            _ => {
                tracing::warn!(
                    value_present = !raw.is_empty(),
                    env_var = SHIELDED_RAIL_TIMEOUT_ENV_VAR,
                    "Invalid shielded-rail timeout — falling back to the default of 30s",
                );
                Duration::from_secs(DEFAULT_SHIELDED_RAIL_TIMEOUT_SECS)
            }
        }
    }
}

impl Default for ShieldedRailFallbackConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_SHIELDED_RAIL_TIMEOUT_SECS),
        }
    }
}

/// Settle a payment through the shielded tokenless rail, falling back to the
/// transparent rail when the shielded rail is unavailable or does not respond
/// within `config.timeout`.
///
/// - `shielded`    — the shielded settlement attempt.  Returning
///   [`RailUnavailable::Unavailable`] means the rail is down; hanging past the
///   timeout is treated as [`RailUnavailable::Timeout`].
/// - `transparent` — the transparent settlement executed after a fallback.
/// - `metrics`     — optional Prometheus metrics; when provided, every
///   fallback is recorded on `perigee_shielded_rail_fallback_total`.
///
/// Returns which rail the settlement went through, or an error if the
/// transparent fallback itself failed.
pub async fn settle_with_fallback<F, G>(
    settlement_id: &str,
    shielded: F,
    transparent: G,
    config: &ShieldedRailFallbackConfig,
    metrics: Option<&Metrics>,
) -> Result<SettlementRail, String>
where
    F: Future<Output = Result<(), RailUnavailable>>,
    G: FnOnce() -> Result<(), String>,
{
    let started = Instant::now();

    let fallback_reason = match timeout(config.timeout, shielded).await {
        Ok(Ok(())) => return Ok(SettlementRail::Shielded),
        Ok(Err(RailUnavailable::Unavailable(reason))) => reason,
        Ok(Err(RailUnavailable::Timeout)) | Err(_) => {
            format!("no response within {}s", config.timeout.as_secs())
        }
    };

    let waited_ms = started.elapsed().as_millis() as u64;

    let redactor = crate::log_redaction::LogRedactor::new("perigee-log-redaction");
    let redacted_settlement_id = redactor.redact_address(settlement_id);
    let redacted_reason = crate::log_redaction::redact_sensitive_text(&fallback_reason);
    tracing::warn!(
        settlement_id = %redacted_settlement_id,
        reason = %redacted_reason,
        waited_ms,
        timeout_ms = config.timeout.as_millis() as u64,
        "Shielded payment rail unavailable — falling back to transparent settlement",
    );

    if let Some(metrics) = metrics {
        metrics.record_shielded_rail_fallback();
    }

    transparent().map(|()| SettlementRail::Transparent)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn metrics() -> std::sync::Arc<Metrics> {
        Metrics::new().expect("metrics registry should initialise")
    }

    #[tokio::test]
    async fn shielded_success_uses_shielded_rail_and_records_no_fallback() {
        let metrics = metrics();
        let transparent_called = AtomicUsize::new(0);

        let rail = settle_with_fallback(
            "set-1",
            async { Ok(()) },
            || {
                transparent_called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            &ShieldedRailFallbackConfig::default(),
            Some(metrics.as_ref()),
        )
        .await
        .expect("settlement succeeds");

        assert_eq!(rail, SettlementRail::Shielded);
        assert_eq!(transparent_called.load(Ordering::SeqCst), 0);
        assert_eq!(metrics.shielded_rail_fallback_total.get(), 0);
    }

    #[tokio::test]
    async fn unavailable_shielded_rail_falls_back_and_records_metric() {
        let metrics = metrics();
        let transparent_called = AtomicUsize::new(0);

        let rail = settle_with_fallback(
            "set-2",
            async { Err(RailUnavailable::Unavailable("network congestion".into())) },
            || {
                transparent_called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            &ShieldedRailFallbackConfig::default(),
            Some(metrics.as_ref()),
        )
        .await
        .expect("fallback settlement succeeds");

        assert_eq!(rail, SettlementRail::Transparent);
        assert_eq!(transparent_called.load(Ordering::SeqCst), 1);
        assert_eq!(metrics.shielded_rail_fallback_total.get(), 1);
    }

    #[tokio::test]
    async fn hanging_shielded_rail_times_out_and_falls_back() {
        let metrics = metrics();
        let transparent_called = AtomicUsize::new(0);

        // A shielded rail that hangs far past the configured timeout — the
        // timeout fires and the settlement falls back to the transparent rail.
        let config = ShieldedRailFallbackConfig {
            timeout: Duration::from_millis(100),
        };

        let rail = settle_with_fallback(
            "set-3",
            async {
                tokio::time::sleep(Duration::from_secs(5)).await;
                Ok(())
            },
            || {
                transparent_called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            &config,
            Some(metrics.as_ref()),
        )
        .await
        .expect("fallback settlement succeeds");

        assert_eq!(rail, SettlementRail::Transparent);
        assert_eq!(transparent_called.load(Ordering::SeqCst), 1);
        assert_eq!(metrics.shielded_rail_fallback_total.get(), 1);
    }

    #[tokio::test]
    async fn transparent_fallback_failure_is_propagated() {
        let result = settle_with_fallback(
            "set-4",
            async { Err(RailUnavailable::Unavailable("downtime".into())) },
            || Err("transparent rail rejected the settlement".into()),
            &ShieldedRailFallbackConfig::default(),
            None,
        )
        .await;

        assert_eq!(
            result,
            Err("transparent rail rejected the settlement".to_string())
        );
    }

    #[tokio::test]
    async fn fallback_works_without_metrics_handle() {
        let transparent_called = AtomicUsize::new(0);

        let rail = settle_with_fallback(
            "set-5",
            async { Err(RailUnavailable::Timeout) },
            || {
                transparent_called.fetch_add(1, Ordering::SeqCst);
                Ok(())
            },
            &ShieldedRailFallbackConfig::default(),
            None,
        )
        .await
        .expect("fallback settlement succeeds");

        assert_eq!(rail, SettlementRail::Transparent);
        assert_eq!(transparent_called.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn timeout_config_defaults_to_30_seconds() {
        assert_eq!(
            ShieldedRailFallbackConfig::default().timeout,
            Duration::from_secs(30)
        );
        assert_eq!(
            ShieldedRailFallbackConfig::parse_timeout_env(None),
            Duration::from_secs(30)
        );
    }

    #[test]
    fn timeout_config_parses_env_value() {
        assert_eq!(
            ShieldedRailFallbackConfig::parse_timeout_env(Some("5")),
            Duration::from_secs(5)
        );
        assert_eq!(
            ShieldedRailFallbackConfig::parse_timeout_env(Some(" 60 ")),
            Duration::from_secs(60)
        );
    }

    #[test]
    fn timeout_config_rejects_invalid_values() {
        for bad in ["0", "-1", "abc", "1.5", ""] {
            assert_eq!(
                ShieldedRailFallbackConfig::parse_timeout_env(Some(bad)),
                Duration::from_secs(30),
                "expected default for {bad:?}"
            );
        }
    }
}
