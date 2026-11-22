#![allow(clippy::type_complexity)]

use crate::errors::AppError;
use reqwest::Client;
use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

const DEFAULT_ZSCORE_THRESHOLD: f64 = 2.0;
const DEFAULT_SHIFT_THRESHOLD: f64 = 0.10;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SimulationMetric {
    pub contract: String,
    pub method: String,
    pub code_hash: String,
    pub cpu_instructions: u64,
    pub ram_bytes: u64,
    pub ledger_footprint: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoricalAverages {
    pub samples: usize,
    pub avg_cpu_instructions: f64,
    pub avg_ram_bytes: f64,
    pub avg_ledger_footprint: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DriftDetail {
    pub metric: String,
    pub value: u64,
    pub average: f64,
    pub percent_shift: f64,
    pub z_score: Option<f64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AnalysisResult {
    pub has_historical_baseline: bool,
    pub historical: Option<HistoricalAverages>,
    pub outliers: Vec<DriftDetail>,
    pub alert_triggered: bool,
}

#[derive(Debug, Clone)]
pub struct SimulationService {
    db_path: PathBuf,
    shift_threshold: f64,
    zscore_threshold: f64,
    webhook_url: Option<String>,
}

impl SimulationService {
    pub fn new(db_path: impl AsRef<Path>, webhook_url: Option<String>) -> Result<Self, AppError> {
        let service = Self {
            db_path: db_path.as_ref().to_path_buf(),
            shift_threshold: DEFAULT_SHIFT_THRESHOLD,
            zscore_threshold: DEFAULT_ZSCORE_THRESHOLD,
            webhook_url,
        };
        service.ensure_schema()?;
        Ok(service)
    }

    fn connect(&self) -> Result<Connection, AppError> {
        Connection::open(&self.db_path)
            .map_err(|e| AppError::Internal(format!("Failed to open metrics database: {e}")))
    }

    fn ensure_schema(&self) -> Result<(), AppError> {
        let conn = self.connect()?;
        conn.execute_batch(
            "
            CREATE TABLE IF NOT EXISTS simulation_metrics (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                contract TEXT NOT NULL,
                method TEXT NOT NULL,
                code_hash TEXT NOT NULL,
                cpu_instructions INTEGER NOT NULL,
                ram_bytes INTEGER NOT NULL,
                ledger_footprint INTEGER NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_simulation_lookup
                ON simulation_metrics(contract, method, code_hash, created_at);

            CREATE TABLE IF NOT EXISTS simulation_alerts (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                simulation_metric_id INTEGER NOT NULL,
                contract TEXT NOT NULL,
                method TEXT NOT NULL,
                code_hash TEXT NOT NULL,
                details_json TEXT NOT NULL,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                FOREIGN KEY(simulation_metric_id) REFERENCES simulation_metrics(id)
            );

            CREATE TABLE IF NOT EXISTS webhook_events (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                url TEXT NOT NULL,
                payload_json TEXT NOT NULL,
                status TEXT NOT NULL DEFAULT 'pending',
                attempts INTEGER NOT NULL DEFAULT 0,
                max_attempts INTEGER NOT NULL DEFAULT 8,
                next_attempt_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                last_error TEXT,
                created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
                updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
            );

            CREATE INDEX IF NOT EXISTS idx_webhook_events_dispatch
                ON webhook_events(status, next_attempt_at);
            ",
        )
        .map_err(|e| AppError::Internal(format!("Failed to create metrics schema: {e}")))?;
        Ok(())
    }

    pub async fn record_and_analyze(
        &self,
        metric: SimulationMetric,
    ) -> Result<AnalysisResult, AppError> {
        let baseline =
            self.load_historical_stats(&metric.contract, &metric.method, &metric.code_hash)?;
        let outliers = if let Some((ref historical, ref rows)) = baseline {
            self.detect_outliers(&metric, historical, rows)
        } else {
            Vec::new()
        };

        let metric_id = self.insert_metric(&metric)?;
        let alert_triggered = !outliers.is_empty();

        if alert_triggered {
            self.store_alert(metric_id, &metric, &outliers)?;
            self.emit_alert(&metric, &outliers).await;
        }

        Ok(AnalysisResult {
            has_historical_baseline: baseline.is_some(),
            historical: baseline.as_ref().map(|(historical, _)| historical.clone()),
            outliers,
            alert_triggered,
        })
    }

    fn insert_metric(&self, metric: &SimulationMetric) -> Result<i64, AppError> {
        let conn = self.connect()?;
        conn.execute(
            "
            INSERT INTO simulation_metrics (contract, method, code_hash, cpu_instructions, ram_bytes, ledger_footprint)
            VALUES (?, ?, ?, ?, ?, ?)
            ",
            params![
                metric.contract,
                metric.method,
                metric.code_hash,
                metric.cpu_instructions as i64,
                metric.ram_bytes as i64,
                metric.ledger_footprint as i64
            ],
        )
        .map_err(|e| AppError::Internal(format!("Failed to insert simulation metric: {e}")))?;
        Ok(conn.last_insert_rowid())
    }

    fn store_alert(
        &self,
        simulation_metric_id: i64,
        metric: &SimulationMetric,
        outliers: &[DriftDetail],
    ) -> Result<(), AppError> {
        let details_json = serde_json::to_string(outliers)
            .map_err(|e| AppError::Internal(format!("Failed to serialize alert details: {e}")))?;
        let conn = self.connect()?;
        conn.execute(
            "
            INSERT INTO simulation_alerts (simulation_metric_id, contract, method, code_hash, details_json)
            VALUES (?, ?, ?, ?, ?)
            ",
            params![
                simulation_metric_id,
                metric.contract,
                metric.method,
                metric.code_hash,
                details_json
            ],
        )
        .map_err(|e| AppError::Internal(format!("Failed to store simulation alert: {e}")))?;
        Ok(())
    }

    fn load_historical_stats(
        &self,
        contract: &str,
        method: &str,
        code_hash: &str,
    ) -> Result<Option<(HistoricalAverages, Vec<(u64, u64, u64)>)>, AppError> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "
                SELECT cpu_instructions, ram_bytes, ledger_footprint
                FROM simulation_metrics
                WHERE contract = ?1 AND method = ?2 AND code_hash = ?3
                ",
            )
            .map_err(|e| AppError::Internal(format!("Failed to prepare history query: {e}")))?;

        let rows = stmt
            .query_map(params![contract, method, code_hash], |row| {
                Ok((
                    row.get::<_, i64>(0)? as u64,
                    row.get::<_, i64>(1)? as u64,
                    row.get::<_, i64>(2)? as u64,
                ))
            })
            .map_err(|e| AppError::Internal(format!("Failed to query historical metrics: {e}")))?;

        let data: Result<Vec<_>, _> = rows.collect();
        let data = data
            .map_err(|e| AppError::Internal(format!("Failed to read historical metrics: {e}")))?;

        if data.is_empty() {
            return Ok(None);
        }

        let n = data.len() as f64;
        let cpu_sum: f64 = data.iter().map(|r| r.0 as f64).sum();
        let ram_sum: f64 = data.iter().map(|r| r.1 as f64).sum();
        let ledger_sum: f64 = data.iter().map(|r| r.2 as f64).sum();

        Ok(Some((
            HistoricalAverages {
                samples: data.len(),
                avg_cpu_instructions: cpu_sum / n,
                avg_ram_bytes: ram_sum / n,
                avg_ledger_footprint: ledger_sum / n,
            },
            data,
        )))
    }

    fn detect_outliers(
        &self,
        current: &SimulationMetric,
        historical: &HistoricalAverages,
        rows: &[(u64, u64, u64)],
    ) -> Vec<DriftDetail> {
        let cpu_values: Vec<f64> = rows.iter().map(|r| r.0 as f64).collect();
        let ram_values: Vec<f64> = rows.iter().map(|r| r.1 as f64).collect();
        let ledger_values: Vec<f64> = rows.iter().map(|r| r.2 as f64).collect();

        let cpu_z = z_score(current.cpu_instructions as f64, &cpu_values);
        let ram_z = z_score(current.ram_bytes as f64, &ram_values);
        let ledger_z = z_score(current.ledger_footprint as f64, &ledger_values);

        let mut outliers = Vec::new();

        if let Some(detail) = assess_metric_shift(
            "cpu_instructions",
            current.cpu_instructions,
            historical.avg_cpu_instructions,
            cpu_z,
            self.shift_threshold,
            self.zscore_threshold,
        ) {
            outliers.push(detail);
        }

        if let Some(detail) = assess_metric_shift(
            "ram_bytes",
            current.ram_bytes,
            historical.avg_ram_bytes,
            ram_z,
            self.shift_threshold,
            self.zscore_threshold,
        ) {
            outliers.push(detail);
        }

        if let Some(detail) = assess_metric_shift(
            "ledger_footprint",
            current.ledger_footprint,
            historical.avg_ledger_footprint,
            ledger_z,
            self.shift_threshold,
            self.zscore_threshold,
        ) {
            outliers.push(detail);
        }

        outliers
    }

    async fn emit_alert(&self, metric: &SimulationMetric, outliers: &[DriftDetail]) {
        tracing::warn!(
            contract = %crate::log_redaction::redact_sensitive_text(&metric.contract),
            method = %metric.method,
            code_hash = %metric.code_hash,
            outliers = ?outliers,
            "Resource shift alert"
        );

        let Some(url) = &self.webhook_url else {
            return;
        };

        let payload = serde_json::json!({
            "event": "simulation_resource_shift",
            "contract": metric.contract,
            "method": metric.method,
            "code_hash": metric.code_hash,
            "cpu_instructions": metric.cpu_instructions,
            "ram_bytes": metric.ram_bytes,
            "ledger_footprint": metric.ledger_footprint,
            "outliers": outliers,
        });

        // Persist the event *before* attempting delivery. Even if the
        // process crashes right after this line, the event survives and
        // will be picked up by the background dispatcher (or the next
        // `dispatch_due_events` sweep) instead of being silently lost.
        let event_id = match self.enqueue_webhook_event(url, &payload) {
            Ok(id) => id,
            Err(err) => {
                tracing::error!(
                    error = %crate::log_redaction::redact_display(&err),
                    "Failed to persist webhook alert"
                );
                return;
            }
        };

        // Best-effort immediate delivery so a healthy webhook endpoint
        // still gets near-real-time notifications, rather than always
        // waiting for the next poll cycle. If this fails, the event is
        // already durably queued and `dispatch_due_events` will retry it
        // with backoff.
        if let Err(err) = self.deliver_event(event_id, url, &payload).await {
            tracing::warn!(
                event_id,
                error = %crate::log_redaction::redact_display(&err),
                "Immediate webhook delivery failed; event queued for retry"
            );
        }
    }

    /// Insert a webhook event into the durable queue. Returns the new
    /// event's row id.
    fn enqueue_webhook_event(
        &self,
        url: &str,
        payload: &serde_json::Value,
    ) -> Result<i64, AppError> {
        let payload_json = serde_json::to_string(payload)
            .map_err(|e| AppError::Internal(format!("Failed to serialize webhook payload: {e}")))?;
        let conn = self.connect()?;
        conn.execute(
            "INSERT INTO webhook_events (url, payload_json, max_attempts) VALUES (?, ?, ?)",
            params![url, payload_json, WEBHOOK_MAX_ATTEMPTS as i64],
        )
        .map_err(|e| AppError::Internal(format!("Failed to enqueue webhook event: {e}")))?;
        Ok(conn.last_insert_rowid())
    }

    /// Attempt to deliver a single already-persisted event and update its
    /// row accordingly: `delivered` on success, or rescheduled with
    /// exponential backoff (eventually `dead_letter` after exhausting
    /// `max_attempts`) on failure. The event is never deleted on failure,
    /// so it always remains available for retry or manual inspection.
    async fn deliver_event(
        &self,
        event_id: i64,
        url: &str,
        payload: &serde_json::Value,
    ) -> Result<(), String> {
        let client = Client::new();
        let result = client
            .post(url)
            .timeout(std::time::Duration::from_secs(WEBHOOK_TIMEOUT_SECS))
            .json(payload)
            .send()
            .await;

        match result {
            Ok(resp) if resp.status().is_success() => {
                if let Err(err) = self.mark_event_delivered(event_id) {
                    tracing::error!(
                        event_id,
                        error = %crate::log_redaction::redact_display(&err),
                        "Delivered webhook event but failed to update its status"
                    );
                }
                Ok(())
            }
            Ok(resp) => {
                let err = format!("non-success status {}", resp.status());
                self.reschedule_event(event_id, &err);
                Err(err)
            }
            Err(err) => {
                let err = err.to_string();
                self.reschedule_event(event_id, &err);
                Err(err)
            }
        }
    }

    fn mark_event_delivered(&self, event_id: i64) -> Result<(), AppError> {
        let conn = self.connect()?;
        conn.execute(
            "UPDATE webhook_events
             SET status = 'delivered', updated_at = CURRENT_TIMESTAMP
             WHERE id = ?",
            params![event_id],
        )
        .map_err(|e| AppError::Internal(format!("Failed to mark webhook event delivered: {e}")))?;
        Ok(())
    }

    /// Bump the attempt count and either schedule the next retry with
    /// exponential backoff, or mark the event `dead_letter` once
    /// `max_attempts` is exhausted. Errors here are logged rather than
    /// propagated since this already runs inside a failure path.
    fn reschedule_event(&self, event_id: i64, error: &str) {
        let conn = match self.connect() {
            Ok(conn) => conn,
            Err(err) => {
                tracing::error!(
                    event_id,
                    error = %crate::log_redaction::redact_display(&err),
                    "Failed to open database while rescheduling webhook event"
                );
                return;
            }
        };

        let row: Result<(i64, i64), _> = conn.query_row(
            "SELECT attempts, max_attempts FROM webhook_events WHERE id = ?",
            params![event_id],
            |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)),
        );

        let (attempts, max_attempts) = match row {
            Ok(row) => row,
            Err(err) => {
                tracing::error!(
                    event_id,
                    error = %crate::log_redaction::redact_display(&err),
                    "Failed to load webhook event for reschedule"
                );
                return;
            }
        };

        let attempts = attempts + 1;

        let update_result = if attempts >= max_attempts {
            conn.execute(
                "UPDATE webhook_events
                 SET attempts = ?, status = 'dead_letter', last_error = ?, updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?",
                params![attempts, error, event_id],
            )
        } else {
            let backoff_secs = webhook_backoff_secs(attempts);
            conn.execute(
                "UPDATE webhook_events
                 SET attempts = ?, status = 'pending', last_error = ?,
                     next_attempt_at = datetime(CURRENT_TIMESTAMP, ?),
                     updated_at = CURRENT_TIMESTAMP
                 WHERE id = ?",
                params![attempts, error, format!("+{backoff_secs} seconds"), event_id],
            )
        };

        if let Err(err) = update_result {
            tracing::error!(
                event_id,
                error = %crate::log_redaction::redact_display(&err),
                "Failed to reschedule webhook event"
            );
        }
    }

    /// Sweep the queue for events that are due (`pending` with
    /// `next_attempt_at` in the past) and attempt delivery for each. Safe
    /// to call repeatedly / from multiple places: delivered or still-future
    /// events are simply skipped.
    pub async fn dispatch_due_events(&self) -> Result<usize, AppError> {
        self.dispatch_due_events_with_cancellation(CancellationToken::new())
            .await
    }

    pub async fn dispatch_due_events_with_cancellation(
        &self,
        cancellation: CancellationToken,
    ) -> Result<usize, AppError> {
        if cancellation.is_cancelled() {
            return Ok(0);
        }
        let due = self.load_due_events()?;
        let count = due.len();
        for (event_id, url, payload_json) in due {
            if cancellation.is_cancelled() {
                break;
            }
            let payload: serde_json::Value = match serde_json::from_str(&payload_json) {
                Ok(payload) => payload,
                Err(err) => {
                    tracing::error!(
                        event_id,
                        error = %crate::log_redaction::redact_display(&err),
                        "Corrupt webhook payload; event marked dead-letter"
                    );
                    self.reschedule_event(event_id, &format!("corrupt payload: {err}"));
                    continue;
                }
            };
            let _ = tokio::select! {
                _ = cancellation.cancelled() => break,
                result = self.deliver_event(event_id, &url, &payload) => result,
            };
        }
        Ok(count)
    }

    fn load_due_events(&self) -> Result<Vec<(i64, String, String)>, AppError> {
        let conn = self.connect()?;
        let mut stmt = conn
            .prepare(
                "SELECT id, url, payload_json FROM webhook_events
                 WHERE status = 'pending' AND next_attempt_at <= CURRENT_TIMESTAMP
                 ORDER BY next_attempt_at ASC
                 LIMIT ?",
            )
            .map_err(|e| AppError::Internal(format!("Failed to prepare due-events query: {e}")))?;

        let rows = stmt
            .query_map(params![WEBHOOK_DISPATCH_BATCH_SIZE as i64], |row| {
                Ok((row.get::<_, i64>(0)?, row.get::<_, String>(1)?, row.get::<_, String>(2)?))
            })
            .map_err(|e| AppError::Internal(format!("Failed to query due webhook events: {e}")))?;

        rows.collect::<Result<Vec<_>, _>>()
            .map_err(|e| AppError::Internal(format!("Failed to read due webhook events: {e}")))
    }

    /// Spawn a background task that periodically sweeps the durable queue
    /// and retries any events that are due. This is what makes retries
    /// survive process restarts: on startup, any events left `pending`
    /// from before a crash are picked up again on the very first sweep.
    pub fn spawn_dispatcher(self: std::sync::Arc<Self>) -> tokio::task::JoinHandle<()> {
        self.spawn_dispatcher_with_cancellation(CancellationToken::new())
    }

    pub fn spawn_dispatcher_with_cancellation(
        self: std::sync::Arc<Self>,
        cancellation: CancellationToken,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let mut interval =
                tokio::time::interval(std::time::Duration::from_secs(WEBHOOK_DISPATCH_POLL_SECS));
            loop {
                tokio::select! {
                    _ = cancellation.cancelled() => break,
                    _ = interval.tick() => {
                        if let Err(err) = self
                            .dispatch_due_events_with_cancellation(cancellation.clone())
                            .await
                        {
                            tracing::error!(
                                error = %crate::log_redaction::redact_display(&err),
                                "Webhook dispatcher sweep failed"
                            );
                        }
                    }
                }
            }
            tracing::debug!("Webhook dispatcher stopped");
        })
    }
}

const WEBHOOK_MAX_ATTEMPTS: u32 = 8;
const WEBHOOK_BASE_BACKOFF_SECS: u64 = 2;
const WEBHOOK_MAX_BACKOFF_SECS: u64 = 300;
const WEBHOOK_TIMEOUT_SECS: u64 = 10;
const WEBHOOK_DISPATCH_BATCH_SIZE: usize = 25;
const WEBHOOK_DISPATCH_POLL_SECS: u64 = 15;

/// Exponential backoff, capped, for the given (1-indexed) attempt number.
fn webhook_backoff_secs(attempt: i64) -> u64 {
    let exp = attempt.max(1) as u32 - 1;
    WEBHOOK_BASE_BACKOFF_SECS
        .saturating_mul(1u64.wrapping_shl(exp.min(20)))
        .min(WEBHOOK_MAX_BACKOFF_SECS)
}

fn assess_metric_shift(
    metric: &str,
    value: u64,
    average: f64,
    z_score: Option<f64>,
    shift_threshold: f64,
    z_threshold: f64,
) -> Option<DriftDetail> {
    if average <= f64::EPSILON {
        return None;
    }

    let percent_shift = ((value as f64 - average) / average).abs();
    let shifted = percent_shift > shift_threshold;

    let zscore_outlier = z_score.map(|z| z.abs() > z_threshold).unwrap_or(false);

    if shifted || zscore_outlier {
        return Some(DriftDetail {
            metric: metric.to_string(),
            value,
            average,
            percent_shift,
            z_score,
        });
    }

    None
}

fn z_score(current: f64, values: &[f64]) -> Option<f64> {
    if values.len() < 2 {
        return None;
    }

    let mean = values.iter().sum::<f64>() / values.len() as f64;
    let variance = values
        .iter()
        .map(|v| {
            let diff = v - mean;
            diff * diff
        })
        .sum::<f64>()
        / values.len() as f64;

    let std_dev = variance.sqrt();
    if std_dev <= f64::EPSILON {
        return None;
    }

    Some((current - mean) / std_dev)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rusqlite::Connection;
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempDbPath(PathBuf);

    impl TempDbPath {
        fn new(test_name: &str) -> Self {
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("system clock should be after unix epoch")
                .as_nanos();
            let path = std::env::temp_dir().join(format!("Perigee_{test_name}_{nanos}.db"));
            Self(path)
        }
    }

    impl Drop for TempDbPath {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.0);
        }
    }

    fn metric(
        contract: &str,
        method: &str,
        code_hash: &str,
        cpu: u64,
        ram: u64,
        ledger: u64,
    ) -> SimulationMetric {
        SimulationMetric {
            contract: contract.to_string(),
            method: method.to_string(),
            code_hash: code_hash.to_string(),
            cpu_instructions: cpu,
            ram_bytes: ram,
            ledger_footprint: ledger,
        }
    }

    fn alert_count(db_path: &Path) -> usize {
        let conn = Connection::open(db_path).expect("open sqlite database");
        conn.query_row("SELECT COUNT(*) FROM simulation_alerts", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("count alerts") as usize
    }

    fn webhook_event_count(db_path: &Path) -> usize {
        let conn = Connection::open(db_path).expect("open sqlite database");
        conn.query_row("SELECT COUNT(*) FROM webhook_events", [], |row| {
            row.get::<_, i64>(0)
        })
        .expect("count webhook events") as usize
    }

    fn webhook_event_row(db_path: &Path, id: i64) -> (String, i64, i64) {
        let conn = Connection::open(db_path).expect("open sqlite database");
        conn.query_row(
            "SELECT status, attempts, max_attempts FROM webhook_events WHERE id = ?",
            params![id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            },
        )
        .expect("load webhook event")
    }

    #[tokio::test]
    async fn first_sample_has_no_historical_baseline() {
        let db_path = TempDbPath::new("first_sample_has_no_historical_baseline");
        let service =
            SimulationService::new(&db_path.0, None).expect("initialize simulation service");

        let result = service
            .record_and_analyze(metric("token", "mint", "hash-a", 100, 100, 100))
            .await
            .expect("record and analyze should succeed");

        assert!(!result.has_historical_baseline);
        assert!(!result.alert_triggered);
        assert!(result.outliers.is_empty());
        assert_eq!(alert_count(&db_path.0), 0);
    }

    #[tokio::test]
    async fn same_code_hash_shift_over_ten_percent_triggers_alert() {
        let db_path = TempDbPath::new("same_code_hash_shift_triggers_alert");
        let service =
            SimulationService::new(&db_path.0, None).expect("initialize simulation service");

        service
            .record_and_analyze(metric("token", "mint", "hash-a", 100, 200, 300))
            .await
            .expect("seed metric should succeed");

        let result = service
            .record_and_analyze(metric("token", "mint", "hash-a", 130, 250, 400))
            .await
            .expect("second metric should succeed");

        assert!(result.has_historical_baseline);
        assert!(result.alert_triggered);
        assert!(!result.outliers.is_empty());
        assert_eq!(alert_count(&db_path.0), 1);
    }

    #[tokio::test]
    async fn different_code_hash_does_not_trigger_no_code_change_alert() {
        let db_path = TempDbPath::new("different_code_hash_does_not_alert");
        let service =
            SimulationService::new(&db_path.0, None).expect("initialize simulation service");

        service
            .record_and_analyze(metric("token", "transfer", "hash-a", 100, 200, 300))
            .await
            .expect("seed metric should succeed");

        let result = service
            .record_and_analyze(metric("token", "transfer", "hash-b", 1_000, 2_000, 3_000))
            .await
            .expect("code-hash change metric should succeed");

        assert!(!result.has_historical_baseline);
        assert!(!result.alert_triggered);
        assert!(result.outliers.is_empty());
        assert_eq!(alert_count(&db_path.0), 0);
    }

    #[tokio::test]
    async fn z_score_outlier_triggers_even_under_ten_percent_shift() {
        let db_path = TempDbPath::new("z_score_outlier_triggers_alert");
        let service =
            SimulationService::new(&db_path.0, None).expect("initialize simulation service");

        for cpu in [95_u64, 100, 100, 102, 103] {
            service
                .record_and_analyze(metric("token", "burn", "hash-z", cpu, 1_000, 200))
                .await
                .expect("seed metric should succeed");
        }

        let result = service
            .record_and_analyze(metric("token", "burn", "hash-z", 106, 1_000, 200))
            .await
            .expect("z-score outlier metric should succeed");

        let cpu_outlier = result
            .outliers
            .iter()
            .find(|detail| detail.metric == "cpu_instructions")
            .expect("cpu outlier should be present");

        assert!(result.has_historical_baseline);
        assert!(result.alert_triggered);
        assert!(cpu_outlier.percent_shift < 0.10);
        assert!(cpu_outlier
            .z_score
            .map(|z| z.abs() > DEFAULT_ZSCORE_THRESHOLD)
            .unwrap_or(false));
        assert_eq!(alert_count(&db_path.0), 1);
    }

    #[tokio::test]
    async fn alert_durably_queues_webhook_event_even_when_delivery_fails() {
        // Nothing is listening on port 1, so delivery is guaranteed to fail
        // fast (connection refused) without needing a timeout.
        let db_path = TempDbPath::new("alert_durably_queues_webhook_event");
        let service = SimulationService::new(
            &db_path.0,
            Some("http://127.0.0.1:1/unreachable".to_string()),
        )
        .expect("initialize simulation service");

        service
            .record_and_analyze(metric("token", "mint", "hash-a", 100, 200, 300))
            .await
            .expect("seed metric should succeed");

        let result = service
            .record_and_analyze(metric("token", "mint", "hash-a", 130, 250, 400))
            .await
            .expect("second metric should succeed");

        assert!(result.alert_triggered);

        // The event must survive in the durable queue even though the
        // immediate delivery attempt failed - this is the core of the fix:
        // previously a failed send just logged to stderr and the
        // notification was gone for good.
        assert_eq!(webhook_event_count(&db_path.0), 1);
        let (status, attempts, _max_attempts) = webhook_event_row(&db_path.0, 1);
        assert_eq!(status, "pending");
        assert_eq!(attempts, 1);
    }

    #[tokio::test]
    async fn dispatch_due_events_moves_event_to_dead_letter_after_max_attempts() {
        let db_path = TempDbPath::new("dispatch_due_events_dead_letters");
        let service =
            SimulationService::new(&db_path.0, None).expect("initialize simulation service");

        // Seed an overdue pending event directly, with max_attempts = 1 so
        // the very next failed attempt exhausts its retry budget.
        {
            let conn = Connection::open(&db_path.0).expect("open sqlite database");
            conn.execute(
                "INSERT INTO webhook_events
                     (url, payload_json, status, attempts, max_attempts, next_attempt_at)
                 VALUES (?, '{}', 'pending', 0, 1, datetime('now', '-1 minute'))",
                params!["http://127.0.0.1:1/unreachable"],
            )
            .expect("seed webhook event");
        }

        let dispatched = service
            .dispatch_due_events()
            .await
            .expect("dispatch sweep should succeed");
        assert_eq!(dispatched, 1);

        let (status, attempts, max_attempts) = webhook_event_row(&db_path.0, 1);
        assert_eq!(status, "dead_letter");
        assert_eq!(attempts, 1);
        assert_eq!(max_attempts, 1);
    }

    #[tokio::test]
    async fn dispatch_due_events_skips_events_not_yet_due() {
        let db_path = TempDbPath::new("dispatch_due_events_skips_future");
        let service =
            SimulationService::new(&db_path.0, None).expect("initialize simulation service");

        {
            let conn = Connection::open(&db_path.0).expect("open sqlite database");
            conn.execute(
                "INSERT INTO webhook_events
                     (url, payload_json, status, attempts, max_attempts, next_attempt_at)
                 VALUES (?, '{}', 'pending', 1, 8, datetime('now', '+1 hour'))",
                params!["http://127.0.0.1:1/unreachable"],
            )
            .expect("seed webhook event");
        }

        let dispatched = service
            .dispatch_due_events()
            .await
            .expect("dispatch sweep should succeed");
        assert_eq!(dispatched, 0);

        let (status, attempts, _max_attempts) = webhook_event_row(&db_path.0, 1);
        assert_eq!(status, "pending");
        assert_eq!(attempts, 1);
    }

    #[test]
    fn webhook_backoff_secs_grows_exponentially_and_caps() {
        assert_eq!(webhook_backoff_secs(1), 2);
        assert_eq!(webhook_backoff_secs(2), 4);
        assert_eq!(webhook_backoff_secs(3), 8);
        assert_eq!(webhook_backoff_secs(20), WEBHOOK_MAX_BACKOFF_SECS);
    }
}
