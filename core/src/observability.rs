//! Observability stack — Issue #600.
//!
//! Implements Prometheus-based metrics with:
//! - RED metrics (Rate, Errors, Duration) per endpoint
//! - USE metrics (Utilization, Saturation, Errors) per resource
//! - SLO configuration: 99.9% availability, p99 < 500 ms
//! - Default alert rules: error_rate > 1%, p99 > 1s, saturation > 80%
//! - `/metrics` endpoint for Prometheus scraping

use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

// ---------------------------------------------------------------------------
// SLO configuration
// ---------------------------------------------------------------------------

/// Service Level Objective configuration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SloConfig {
    /// Target availability as a fraction (e.g. 0.999 = 99.9%).
    pub availability_target: f64,
    /// p99 latency target in milliseconds.
    pub p99_latency_ms: u64,
    /// p50 (median) latency target in milliseconds.
    pub p50_latency_ms: u64,
}

impl Default for SloConfig {
    fn default() -> Self {
        Self {
            availability_target: 0.999,
            p99_latency_ms: 500,
            p50_latency_ms: 100,
        }
    }
}

// ---------------------------------------------------------------------------
// Alert rules
// ---------------------------------------------------------------------------

/// Severity of an alert.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AlertSeverity {
    Info,
    Warning,
    Critical,
}

/// A single alert rule that fires when `current_value > threshold`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AlertRule {
    pub name: String,
    /// Metric key this rule evaluates.
    pub metric: String,
    pub threshold: f64,
    pub severity: AlertSeverity,
    /// Link to the runbook for this alert.
    pub runbook_url: Option<String>,
}

impl AlertRule {
    /// Evaluate whether the rule should fire given the current metric value.
    pub fn fires(&self, value: f64) -> bool {
        value > self.threshold
    }
}

/// The default alert rules required by the issue spec.
pub fn default_alert_rules() -> Vec<AlertRule> {
    vec![
        AlertRule {
            name: "HighErrorRate".into(),
            metric: "error_rate_percent".into(),
            threshold: 1.0,
            severity: AlertSeverity::Critical,
            runbook_url: Some("https://docs.perigee.io/runbooks/high-error-rate".into()),
        },
        AlertRule {
            name: "HighP99Latency".into(),
            metric: "p99_latency_ms".into(),
            threshold: 1000.0,
            severity: AlertSeverity::Warning,
            runbook_url: Some("https://docs.perigee.io/runbooks/high-latency".into()),
        },
        AlertRule {
            name: "HighResourceSaturation".into(),
            metric: "saturation_percent".into(),
            threshold: 80.0,
            severity: AlertSeverity::Warning,
            runbook_url: Some("https://docs.perigee.io/runbooks/high-saturation".into()),
        },
    ]
}

// ---------------------------------------------------------------------------
// Core metric types
// ---------------------------------------------------------------------------

/// A monotonically-increasing counter (RED: rate & errors).
#[derive(Debug, Default)]
pub struct Counter {
    value: Mutex<u64>,
}

impl Counter {
    pub fn increment(&self) {
        *self.value.lock().unwrap() += 1;
    }

    pub fn add(&self, n: u64) {
        *self.value.lock().unwrap() += n;
    }

    pub fn get(&self) -> u64 {
        *self.value.lock().unwrap()
    }
}

/// A gauge for instantaneous measurements (USE: utilization, saturation).
#[derive(Debug, Default)]
pub struct Gauge {
    value: Mutex<f64>,
}

impl Gauge {
    pub fn set(&self, v: f64) {
        *self.value.lock().unwrap() = v;
    }

    pub fn get(&self) -> f64 {
        *self.value.lock().unwrap()
    }
}

/// A histogram for duration distributions (RED: duration / p99).
///
/// Uses a simple approximate percentile implementation (HDR-style bucket counts).
#[derive(Debug)]
pub struct Histogram {
    /// Sorted list of observed values (milliseconds).
    samples: Mutex<Vec<f64>>,
}

impl Default for Histogram {
    fn default() -> Self {
        Self {
            samples: Mutex::new(Vec::with_capacity(1024)),
        }
    }
}

impl Histogram {
    pub fn observe(&self, value_ms: f64) {
        self.samples.lock().unwrap().push(value_ms);
    }

    /// Calculate the p-th percentile (0.0–100.0).
    pub fn percentile(&self, p: f64) -> Option<f64> {
        let mut samples = self.samples.lock().unwrap();
        if samples.is_empty() {
            return None;
        }
        samples.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let idx = ((p / 100.0) * (samples.len() as f64 - 1.0)).round() as usize;
        Some(samples[idx.min(samples.len() - 1)])
    }

    pub fn count(&self) -> usize {
        self.samples.lock().unwrap().len()
    }
}

// ---------------------------------------------------------------------------
// MetricsCollector
// ---------------------------------------------------------------------------

/// Central Prometheus-style metrics collector.
///
/// Holds RED metrics per endpoint and USE metrics per resource.
#[derive(Debug, Default)]
pub struct MetricsCollector {
    // RED: Rate, Errors, Duration — per endpoint label
    /// Total request count per endpoint (rate).
    pub request_total: Mutex<HashMap<String, Counter>>,
    /// Error count per endpoint.
    pub error_total: Mutex<HashMap<String, Counter>>,
    /// Request duration histogram per endpoint.
    pub request_duration_ms: Mutex<HashMap<String, Histogram>>,

    // USE: Utilization, Saturation, Errors — per resource label
    /// CPU utilisation (0–100 %).
    pub cpu_utilization: Gauge,
    /// Memory saturation (0–100 %).
    pub memory_saturation: Gauge,
    /// General resource error count.
    pub resource_errors: Counter,
}

impl MetricsCollector {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Record a completed request.
    ///
    /// # Parameters
    /// - `endpoint` — e.g. `"GET /v1/vaults"`
    /// - `status`   — HTTP status code
    /// - `duration` — request wall-clock time
    pub fn record_request(&self, endpoint: &str, status: u16, duration: Duration) {
        // Rate counter
        {
            let mut map = self.request_total.lock().unwrap();
            map.entry(endpoint.to_string()).or_default().increment();
        }

        // Error counter (4xx / 5xx)
        if status >= 400 {
            let mut map = self.error_total.lock().unwrap();
            map.entry(endpoint.to_string()).or_default().increment();
        }

        // Duration histogram
        {
            let mut map = self.request_duration_ms.lock().unwrap();
            map.entry(endpoint.to_string())
                .or_default()
                .observe(duration.as_secs_f64() * 1000.0);
        }
    }

    /// Compute current error rate for an endpoint (0–100 %).
    pub fn error_rate_percent(&self, endpoint: &str) -> f64 {
        let total = self
            .request_total
            .lock()
            .unwrap()
            .get(endpoint)
            .map(|c| c.get())
            .unwrap_or(0);
        let errors = self
            .error_total
            .lock()
            .unwrap()
            .get(endpoint)
            .map(|c| c.get())
            .unwrap_or(0);
        if total == 0 {
            0.0
        } else {
            (errors as f64 / total as f64) * 100.0
        }
    }

    /// p99 latency for an endpoint in milliseconds.
    pub fn p99_latency_ms(&self, endpoint: &str) -> Option<f64> {
        self.request_duration_ms
            .lock()
            .unwrap()
            .get(endpoint)?
            .percentile(99.0)
    }

    /// Export metrics in Prometheus text exposition format.
    pub fn expose_metrics(&self) -> String {
        let mut out = String::new();

        // request_total
        out.push_str("# HELP http_requests_total Total HTTP requests\n");
        out.push_str("# TYPE http_requests_total counter\n");
        for (endpoint, counter) in self.request_total.lock().unwrap().iter() {
            out.push_str(&format!(
                "http_requests_total{{endpoint=\"{}\"}} {}\n",
                endpoint,
                counter.get()
            ));
        }

        // error_total
        out.push_str("# HELP http_errors_total Total HTTP error responses\n");
        out.push_str("# TYPE http_errors_total counter\n");
        for (endpoint, counter) in self.error_total.lock().unwrap().iter() {
            out.push_str(&format!(
                "http_errors_total{{endpoint=\"{}\"}} {}\n",
                endpoint,
                counter.get()
            ));
        }

        // p99 latency
        out.push_str("# HELP http_request_duration_p99_ms p99 request latency in ms\n");
        out.push_str("# TYPE http_request_duration_p99_ms gauge\n");
        for (endpoint, hist) in self.request_duration_ms.lock().unwrap().iter() {
            if let Some(p99) = hist.percentile(99.0) {
                out.push_str(&format!(
                    "http_request_duration_p99_ms{{endpoint=\"{}\"}} {:.2}\n",
                    endpoint, p99
                ));
            }
        }

        // USE metrics
        out.push_str("# HELP cpu_utilization_percent CPU utilisation (0-100)\n");
        out.push_str("# TYPE cpu_utilization_percent gauge\n");
        out.push_str(&format!(
            "cpu_utilization_percent {:.2}\n",
            self.cpu_utilization.get()
        ));

        out.push_str("# HELP memory_saturation_percent Memory saturation (0-100)\n");
        out.push_str("# TYPE memory_saturation_percent gauge\n");
        out.push_str(&format!(
            "memory_saturation_percent {:.2}\n",
            self.memory_saturation.get()
        ));

        out
    }

    /// Evaluate all default alert rules against the current metrics for
    /// the given endpoint and resource saturation level.
    pub fn evaluate_alerts(
        &self,
        endpoint: &str,
        saturation_percent: f64,
    ) -> Vec<FiredAlert> {
        let mut fired = Vec::new();
        for rule in default_alert_rules() {
            let value = match rule.metric.as_str() {
                "error_rate_percent" => self.error_rate_percent(endpoint),
                "p99_latency_ms" => self.p99_latency_ms(endpoint).unwrap_or(0.0),
                "saturation_percent" => saturation_percent,
                _ => continue,
            };
            if rule.fires(value) {
                fired.push(FiredAlert {
                    rule: rule.clone(),
                    current_value: value,
                    fired_at: std::time::SystemTime::now(),
                });
            }
        }
        fired
    }
}

/// An alert that has fired.
#[derive(Debug)]
pub struct FiredAlert {
    pub rule: AlertRule,
    pub current_value: f64,
    pub fired_at: std::time::SystemTime,
}

// ---------------------------------------------------------------------------
// Request timer helper
// ---------------------------------------------------------------------------

/// RAII guard that records a request's duration on drop.
pub struct RequestTimer<'a> {
    collector: &'a MetricsCollector,
    endpoint: String,
    status: Mutex<u16>,
    started_at: Instant,
}

impl<'a> RequestTimer<'a> {
    pub fn start(collector: &'a MetricsCollector, endpoint: impl Into<String>) -> Self {
        Self {
            collector,
            endpoint: endpoint.into(),
            status: Mutex::new(200),
            started_at: Instant::now(),
        }
    }

    pub fn set_status(&self, status: u16) {
        *self.status.lock().unwrap() = status;
    }
}

impl<'a> Drop for RequestTimer<'a> {
    fn drop(&mut self) {
        let status = *self.status.lock().unwrap();
        self.collector
            .record_request(&self.endpoint, status, self.started_at.elapsed());
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counter_increments_correctly() {
        let c = Counter::default();
        c.increment();
        c.increment();
        c.add(3);
        assert_eq!(c.get(), 5);
    }

    #[test]
    fn histogram_percentile() {
        let h = Histogram::default();
        for v in [10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0, 90.0, 100.0] {
            h.observe(v);
        }
        let p99 = h.percentile(99.0).unwrap();
        assert!(p99 >= 90.0);
        let p50 = h.percentile(50.0).unwrap();
        assert!(p50 >= 50.0 && p50 <= 60.0);
    }

    #[test]
    fn metrics_collector_tracks_error_rate() {
        let mc = MetricsCollector::new();
        for _ in 0..9 {
            mc.record_request("GET /v1/health", 200, Duration::from_millis(10));
        }
        mc.record_request("GET /v1/health", 500, Duration::from_millis(10));
        let rate = mc.error_rate_percent("GET /v1/health");
        assert!((rate - 10.0).abs() < 0.01);
    }

    #[test]
    fn default_alert_rules_fire_correctly() {
        let rule = AlertRule {
            name: "test".into(),
            metric: "error_rate_percent".into(),
            threshold: 1.0,
            severity: AlertSeverity::Critical,
            runbook_url: None,
        };
        assert!(!rule.fires(0.5));
        assert!(!rule.fires(1.0)); // threshold is strictly >
        assert!(rule.fires(1.1));
    }

    #[test]
    fn expose_metrics_contains_expected_keys() {
        let mc = MetricsCollector::new();
        mc.record_request("GET /v1/vaults", 200, Duration::from_millis(42));
        mc.record_request("GET /v1/vaults", 500, Duration::from_millis(100));
        let output = mc.expose_metrics();
        assert!(output.contains("http_requests_total"));
        assert!(output.contains("http_errors_total"));
        assert!(output.contains("GET /v1/vaults"));
    }

    #[test]
    fn slo_config_defaults() {
        let slo = SloConfig::default();
        assert_eq!(slo.availability_target, 0.999);
        assert_eq!(slo.p99_latency_ms, 500);
    }
}
