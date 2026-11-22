//! Distributed tracing (Issue #591) and structured logging (Issue #592).
//!
//! # Distributed tracing
//! - `TracingConfig` with service name, OTLP exporter endpoint, and sampling
//!   strategy.
//! - `SpanGuard` RAII type that auto-exports on drop.
//! - `TraceContext` for W3C-compatible trace/span ID propagation and baggage.
//! - Default 10% probabilistic sampling; errors are always sampled.
//!
//! # Structured logging
//! - `LogEntry` JSON schema: timestamp, level, service, trace_id, span_id,
//!   message, fields.
//! - `redact_fields()` sanitises sensitive keys (password, secret, token, etc.).
//! - `is_sensitive()` helper with case-insensitive substring matching.

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    time::{SystemTime, UNIX_EPOCH},
};

// =============================================================================
// Structured logging — Issue #592
// =============================================================================

/// Standard structured log entry matching the required JSON schema.
///
/// Every log entry includes: timestamp (ISO8601), level, service, trace_id,
/// span_id, message, and arbitrary structured fields.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    /// ISO 8601 timestamp, e.g. `"2026-10-18T21:00:00Z"`.
    pub timestamp: String,
    /// Log level: `"debug"`, `"info"`, `"warn"`, or `"error"`.
    pub level: String,
    /// Emitting service name, e.g. `"perigee-core"`.
    pub service: String,
    /// OpenTelemetry trace id — correlates with distributed traces.
    pub trace_id: Option<String>,
    /// OpenTelemetry span id.
    pub span_id: Option<String>,
    /// Human-readable message.
    pub message: String,
    /// Arbitrary structured fields. Sensitive keys are redacted before storage.
    pub fields: HashMap<String, serde_json::Value>,
}

impl LogEntry {
    /// Build a new log entry, automatically redacting sensitive fields.
    pub fn new(
        level: impl Into<String>,
        service: impl Into<String>,
        message: impl Into<String>,
        trace_id: Option<String>,
        span_id: Option<String>,
        mut fields: HashMap<String, serde_json::Value>,
    ) -> Self {
        redact_fields(&mut fields);
        Self {
            timestamp: iso8601_now(),
            level: level.into(),
            service: service.into(),
            trace_id,
            span_id,
            message: message.into(),
            fields,
        }
    }

    /// Serialise to a compact JSON string suitable for log aggregation.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".into())
    }
}

/// Fields whose values must never appear in plain-text log output.
const SENSITIVE_KEYS: &[&str] = &[
    "password",
    "secret",
    "token",
    "api_key",
    "apikey",
    "private_key",
    "privatekey",
    "ssn",
    "credit_card",
    "creditcard",
    "cvv",
    "authorization",
];

/// Returns `true` if the field key matches any known sensitive pattern.
pub fn is_sensitive(key: &str) -> bool {
    let lower = key.to_lowercase();
    SENSITIVE_KEYS.iter().any(|s| lower.contains(s))
}

/// Replace all sensitive field values with `"[REDACTED]"` in-place.
pub fn redact_fields(fields: &mut HashMap<String, serde_json::Value>) {
    for (k, v) in fields.iter_mut() {
        if is_sensitive(k) {
            *v = serde_json::Value::String("[REDACTED]".to_string());
        }
    }
}

/// Log sampling decision for high-throughput paths.
///
/// For endpoints like `/health/live` that are called very frequently,
/// sampling avoids flooding the log aggregator.
pub struct LogSampler {
    /// Only log 1 in every `sample_rate` calls when sampling is active.
    sample_rate: u64,
    counter: std::sync::atomic::AtomicU64,
}

impl LogSampler {
    pub fn new(sample_rate: u64) -> Self {
        Self {
            sample_rate,
            counter: std::sync::atomic::AtomicU64::new(0),
        }
    }

    /// Returns `true` if this call should be logged.
    pub fn should_log(&self) -> bool {
        let n = self
            .counter
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        n % self.sample_rate == 0
    }
}

fn iso8601_now() -> String {
    use std::fmt::Write as _;
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    // Basic ISO-8601 without sub-seconds for simplicity.
    let mut s = String::new();
    let _ = write!(s, "{}Z", unix_to_rfc3339(secs));
    s
}

fn unix_to_rfc3339(secs: u64) -> String {
    // Minimal RFC3339 formatter (avoids pulling in chrono just for a timestamp).
    let days_since_epoch = secs / 86400;
    let time_of_day = secs % 86400;
    let h = time_of_day / 3600;
    let m = (time_of_day % 3600) / 60;
    let s_rem = time_of_day % 60;

    // Gregorian calendar computation (accurate for 1970–2100).
    let mut year = 1970u64;
    let mut days = days_since_epoch;
    loop {
        let y_days = if is_leap(year) { 366 } else { 365 };
        if days < y_days {
            break;
        }
        days -= y_days;
        year += 1;
    }
    let months = [31u64, if is_leap(year) { 29 } else { 28 }, 31, 30, 31, 30, 31, 31, 30, 31, 30, 31];
    let mut month = 1u64;
    for m_days in &months {
        if days < *m_days {
            break;
        }
        days -= m_days;
        month += 1;
    }
    let day = days + 1;
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        year, month, day, h, m, s_rem
    )
}

fn is_leap(y: u64) -> bool {
    (y % 4 == 0 && y % 100 != 0) || y % 400 == 0
}

// =============================================================================
// Distributed tracing — Issue #591
// =============================================================================

/// Sampling strategy for OpenTelemetry spans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SamplingStrategy {
    /// Record every span — useful during development.
    AlwaysOn,
    /// Drop all spans.
    AlwaysOff,
    /// Sample the given fraction of traces (0.0–1.0).
    /// Errors and explicitly marked spans are always sampled.
    Probabilistic(f64),
}

impl Default for SamplingStrategy {
    fn default() -> Self {
        // 10% sampling in production as required by the issue spec.
        Self::Probabilistic(0.10)
    }
}

impl SamplingStrategy {
    /// Decide whether a given trace should be sampled.
    ///
    /// `is_error` overrides probabilistic sampling — errors are always
    /// recorded regardless of the rate.
    pub fn should_sample(&self, trace_fraction: f64, is_error: bool) -> bool {
        if is_error {
            return true; // always-on for error paths
        }
        match self {
            Self::AlwaysOn => true,
            Self::AlwaysOff => false,
            Self::Probabilistic(rate) => trace_fraction < *rate,
        }
    }
}

/// Configuration for the tracing subsystem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TracingConfig {
    /// Service name reported in every span (e.g. `"perigee-core"`).
    pub service_name: String,
    /// OTLP exporter endpoint (e.g. `"http://jaeger:4318/v1/traces"`).
    /// `None` uses the no-op exporter.
    pub exporter_endpoint: Option<String>,
    /// Sampling strategy.
    pub sampling: SamplingStrategy,
}

impl Default for TracingConfig {
    fn default() -> Self {
        Self {
            service_name: "perigee-core".into(),
            exporter_endpoint: None,
            sampling: SamplingStrategy::default(),
        }
    }
}

// ---------------------------------------------------------------------------
// TraceContext
// ---------------------------------------------------------------------------

/// W3C-compatible trace context propagated across service boundaries.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct TraceContext {
    /// 128-bit trace id as a 32-character hex string.
    pub trace_id: String,
    /// 64-bit span id as a 16-character hex string.
    pub span_id: String,
    /// Parent span id when this is a child span.
    pub parent_span_id: Option<String>,
    /// W3C baggage for cross-service key-value propagation.
    pub baggage: HashMap<String, String>,
}

impl TraceContext {
    /// Returns `true` if this trace represents an error path (always sampled).
    pub fn is_error(&self) -> bool {
        self.baggage
            .get("error")
            .map(|v| v == "true")
            .unwrap_or(false)
    }

    /// Inject trace context into HTTP-style header maps.
    pub fn inject_headers(&self) -> HashMap<String, String> {
        let mut headers = HashMap::new();
        headers.insert(
            "traceparent".into(),
            format!("00-{}-{}-01", self.trace_id, self.span_id),
        );
        if !self.baggage.is_empty() {
            let baggage_str: String = self
                .baggage
                .iter()
                .map(|(k, v)| format!("{}={}", k, v))
                .collect::<Vec<_>>()
                .join(",");
            headers.insert("baggage".into(), baggage_str);
        }
        headers
    }
}

// ---------------------------------------------------------------------------
// SpanKind, Span, SpanExporter
// ---------------------------------------------------------------------------

/// Span kinds aligned to the OpenTelemetry specification.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum SpanKind {
    /// Outbound remote call (HTTP client, gRPC stub).
    Client,
    /// Inbound request handler (HTTP server, gRPC service).
    Server,
    /// Message queue producer.
    Producer,
    /// Message queue consumer.
    Consumer,
    /// Internal computation with no remote I/O.
    Internal,
}

/// A single recorded span with OTel-standard attributes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Span {
    /// Span name, e.g. `"POST /v1/vaults"`.
    pub name: String,
    pub kind: SpanKind,
    pub context: TraceContext,
    /// Start time as Unix microseconds.
    pub start_us: u64,
    /// End time as Unix microseconds. `None` while the span is still open.
    pub end_us: Option<u64>,
    /// Standard OTel attributes: `service.name`, `http.method`, `db.statement`, etc.
    pub attributes: HashMap<String, String>,
    /// `true` = OK, `false` = Error.
    pub status_ok: bool,
}

impl Span {
    pub fn duration_us(&self) -> Option<u64> {
        self.end_us.map(|e| e - self.start_us)
    }
}

/// Trait for span exporters (OTLP/Jaeger/Zipkin/stdout).
pub trait SpanExporter: Send + Sync {
    fn export(&self, span: &Span);
}

/// No-op exporter used when `exporter_endpoint` is `None`.
pub struct NoopExporter;

impl SpanExporter for NoopExporter {
    fn export(&self, _span: &Span) {}
}

/// Stdout exporter for local development.
pub struct StdoutExporter;

impl SpanExporter for StdoutExporter {
    fn export(&self, span: &Span) {
        if let Ok(json) = serde_json::to_string(span) {
            println!("{}", json);
        }
    }
}

// ---------------------------------------------------------------------------
// SpanGuard — RAII span lifecycle management
// ---------------------------------------------------------------------------

/// Manages the lifecycle of a span, exporting it automatically on drop.
///
/// # Example
/// ```ignore
/// let ctx = TraceContext { trace_id: "abc..".into(), span_id: "def..".into(), .. Default::default() };
/// let mut span = SpanGuard::new("GET /v1/vaults", SpanKind::Server, ctx, &exporter);
/// span.set_attribute("http.status_code", "200");
/// // exported when `span` goes out of scope
/// ```
pub struct SpanGuard<'a, E: SpanExporter> {
    pub span: Span,
    exporter: &'a E,
}

impl<'a, E: SpanExporter> SpanGuard<'a, E> {
    pub fn new(
        name: impl Into<String>,
        kind: SpanKind,
        ctx: TraceContext,
        exporter: &'a E,
    ) -> Self {
        let start_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        Self {
            span: Span {
                name: name.into(),
                kind,
                context: ctx,
                start_us,
                end_us: None,
                attributes: HashMap::new(),
                status_ok: true,
            },
            exporter,
        }
    }

    /// Add or update a span attribute.
    pub fn set_attribute(&mut self, key: impl Into<String>, value: impl Into<String>) {
        self.span.attributes.insert(key.into(), value.into());
    }

    /// Mark this span as an error span.
    pub fn set_error(&mut self) {
        self.span.status_ok = false;
        self.span
            .context
            .baggage
            .insert("error".into(), "true".into());
    }
}

impl<'a, E: SpanExporter> Drop for SpanGuard<'a, E> {
    fn drop(&mut self) {
        let end_us = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as u64;
        self.span.end_us = Some(end_us);
        self.exporter.export(&self.span);
    }
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // ── Structured logging ────────────────────────────────────────────────

    #[test]
    fn redact_fields_removes_sensitive_keys() {
        let mut fields = HashMap::new();
        fields.insert("username".into(), serde_json::json!("alice"));
        fields.insert("password".into(), serde_json::json!("s3cr3t"));
        fields.insert("api_key".into(), serde_json::json!("key-123"));
        fields.insert("Authorization".into(), serde_json::json!("Bearer token"));
        redact_fields(&mut fields);
        assert_eq!(fields["password"], "[REDACTED]");
        assert_eq!(fields["api_key"], "[REDACTED]");
        assert_eq!(fields["Authorization"], "[REDACTED]");
        // Non-sensitive field unchanged.
        assert_eq!(fields["username"], "alice");
    }

    #[test]
    fn sensitive_key_detection_is_case_insensitive() {
        assert!(is_sensitive("Password"));
        assert!(is_sensitive("API_KEY"));
        assert!(is_sensitive("JWT_PRIVATE_KEY"));
        assert!(!is_sensitive("user_id"));
        assert!(!is_sensitive("endpoint"));
    }

    #[test]
    fn log_entry_serialises_to_json() {
        let entry = LogEntry::new(
            "info",
            "perigee-core",
            "test message",
            Some("trace-1".into()),
            Some("span-1".into()),
            HashMap::new(),
        );
        let json = entry.to_json();
        assert!(json.contains("\"level\":\"info\""));
        assert!(json.contains("\"service\":\"perigee-core\""));
    }

    #[test]
    fn log_sampler_samples_at_correct_rate() {
        let sampler = LogSampler::new(10);
        let logged: Vec<bool> = (0..20).map(|_| sampler.should_log()).collect();
        // Calls 0 and 10 should be logged (multiples of 10).
        assert_eq!(logged[0], true);
        assert_eq!(logged[10], true);
        assert_eq!(logged[1], false);
        assert_eq!(logged[5], false);
    }

    // ── Distributed tracing ───────────────────────────────────────────────

    #[test]
    fn default_sampling_strategy_is_10_percent() {
        match SamplingStrategy::default() {
            SamplingStrategy::Probabilistic(r) => {
                assert!((r - 0.10).abs() < f64::EPSILON)
            }
            _ => panic!("expected Probabilistic"),
        }
    }

    #[test]
    fn errors_always_sampled_regardless_of_rate() {
        let strat = SamplingStrategy::Probabilistic(0.0);
        assert!(strat.should_sample(0.5, true));
    }

    #[test]
    fn probabilistic_sampling_respects_fraction() {
        let strat = SamplingStrategy::Probabilistic(0.5);
        assert!(strat.should_sample(0.3, false));   // < 0.5 → sampled
        assert!(!strat.should_sample(0.7, false));  // > 0.5 → not sampled
    }

    #[test]
    fn noop_exporter_does_not_panic() {
        let exporter = NoopExporter;
        let ctx = TraceContext {
            trace_id: "abc123".into(),
            span_id: "def456".into(),
            ..Default::default()
        };
        let _guard = SpanGuard::new("test-span", SpanKind::Internal, ctx, &exporter);
        // Drop triggered here — should not panic.
    }

    #[test]
    fn span_guard_exports_on_drop() {
        use std::sync::{Arc, Mutex};
        struct CapturingExporter(Arc<Mutex<Vec<String>>>);
        impl SpanExporter for CapturingExporter {
            fn export(&self, span: &Span) {
                self.0.lock().unwrap().push(span.name.clone());
            }
        }
        let captured = Arc::new(Mutex::new(Vec::<String>::new()));
        let exporter = CapturingExporter(Arc::clone(&captured));
        {
            let ctx = TraceContext::default();
            let _g = SpanGuard::new("my-op", SpanKind::Client, ctx, &exporter);
        }
        assert_eq!(captured.lock().unwrap()[0], "my-op");
    }

    #[test]
    fn trace_context_inject_headers() {
        let ctx = TraceContext {
            trace_id: "abc".into(),
            span_id: "def".into(),
            ..Default::default()
        };
        let headers = ctx.inject_headers();
        assert!(headers.contains_key("traceparent"));
        assert!(headers["traceparent"].contains("abc"));
    }
}
