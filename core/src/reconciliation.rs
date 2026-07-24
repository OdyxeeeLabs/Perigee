#![allow(dead_code)]

use crate::fee_analytics::FeeAnalyticsEngine;
use crate::fee_store::FeeStore;
use crate::jobs::{JobId, JobQueue};
use crate::AppError;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::sync::Arc;
use utoipa::ToSchema;

/// Severity of a fee discrepancy
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum DiscrepancySeverity {
    /// Delta within acceptable range
    Low,
    /// Delta exceeds warning threshold
    Warning,
    /// Delta exceeds critical threshold
    Critical,
}

/// A single fee discrepancy between predicted and actual
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Discrepancy {
    pub id: String,
    pub report_id: String,
    pub ledger_sequence: i64,
    pub expected_fee: i64,
    pub actual_fee: i64,
    pub delta: i64,
    pub delta_pct: f64,
    pub severity: String,
}

/// Summary statistics for a reconciliation report
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReconciliationSummary {
    pub mean_delta_pct: f64,
    pub median_delta_pct: f64,
    pub std_dev_delta_pct: f64,
    pub ledgers_with_critical: i64,
    pub ledgers_with_warning: i64,
}

/// A completed reconciliation report
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReconciliationReport {
    pub id: String,
    pub from_ledger: i64,
    pub to_ledger: i64,
    pub tolerance_pct: f64,
    pub total_ledgers: i32,
    pub discrepancies_count: i32,
    pub avg_delta_pct: f64,
    pub max_delta_pct: f64,
    pub summary: Option<ReconciliationSummary>,
    pub created_at: String,
}

/// Request to start a reconciliation job
#[derive(Debug, Deserialize, ToSchema)]
pub struct ReconcileRequest {
    pub from_ledger: i64,
    pub to_ledger: i64,
    #[serde(default = "default_tolerance")]
    pub tolerance_pct: f64,
}

fn default_tolerance() -> f64 {
    5.0
}

/// Response from submitting a reconciliation job
#[derive(Debug, Serialize, ToSchema)]
pub struct ReconcileResponse {
    pub job_id: String,
    pub status: String,
    pub message: String,
}

/// Query params for listing reconciliation reports
#[derive(Debug, Deserialize)]
pub struct ListReportsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    10
}

/// Fee reconciliation engine that compares predictions against actuals
pub struct FeeReconciler {
    store: Arc<FeeStore>,
    analytics: FeeAnalyticsEngine,
    pool: SqlitePool,
}

impl FeeReconciler {
    pub fn new(store: Arc<FeeStore>, pool: SqlitePool) -> Self {
        Self {
            store,
            analytics: FeeAnalyticsEngine::new(),
            pool,
        }
    }

    /// Run reconciliation for a ledger range and persist results
    pub async fn run(
        &self,
        from_ledger: i64,
        to_ledger: i64,
        tolerance_pct: f64,
        progress_callback: Option<Box<dyn Fn(i32, &str) + Send + Sync>>,
    ) -> Result<ReconciliationReport, ReconciliationError> {
        let report_id = uuid::Uuid::new_v4().to_string();
        let mut discrepancies: Vec<Discrepancy> = Vec::new();
        let total_ledgers = to_ledger - from_ledger + 1;

        // Get all samples in the range for actuals
        let actual_samples = self
            .store
            .get_samples_in_range(from_ledger, to_ledger)
            .await
            .map_err(|e| ReconciliationError::StoreError(e.to_string()))?;

        // Index actual samples by ledger sequence for fast lookup
        let actuals: std::collections::HashMap<i64, i64> = actual_samples
            .iter()
            .map(|s| (s.ledger_sequence, s.base_fee))
            .collect();

        let mut checked = 0i64;
        let mut deltas: Vec<f64> = Vec::new();
        let mut max_delta_pct: f64 = 0.0;

        for ledger_seq in from_ledger..=to_ledger {
            checked += 1;

            // Get historical data up to (but not including) this ledger for prediction
            let historical = self
                .store
                .get_samples_in_range(ledger_seq - 100, ledger_seq - 1)
                .await
                .map_err(|e| ReconciliationError::StoreError(e.to_string()))?;

            if historical.is_empty() {
                continue;
            }

            // Predict fee for this ledger
            let prediction = self.analytics.predict(&historical, ledger_seq as u64);
            let predicted_fee = prediction.standard_bid as i64;

            // Get actual fee
            let actual_fee = match actuals.get(&ledger_seq) {
                Some(&fee) => fee,
                None => continue, // No actual data for this ledger
            };

            // Compute discrepancy
            let delta = actual_fee - predicted_fee;
            let delta_pct = if predicted_fee > 0 {
                (delta.abs() as f64 / predicted_fee as f64) * 100.0
            } else {
                0.0
            };

            if delta_pct > max_delta_pct {
                max_delta_pct = delta_pct;
            }

            deltas.push(delta_pct);

            // Only record discrepancies exceeding tolerance
            if delta_pct > tolerance_pct {
                let severity = if delta_pct > tolerance_pct * 3.0 {
                    "critical"
                } else {
                    "warning"
                };

                discrepancies.push(Discrepancy {
                    id: uuid::Uuid::new_v4().to_string(),
                    report_id: report_id.clone(),
                    ledger_sequence: ledger_seq,
                    expected_fee: predicted_fee,
                    actual_fee,
                    delta,
                    delta_pct,
                    severity: severity.to_string(),
                });
            }

            // Report progress periodically
            if checked % 10 == 0 || checked == total_ledgers {
                let percent = ((checked as f64 / total_ledgers as f64) * 90.0 + 10.0) as i32;
                let msg = format!(
                    "Processing ledger {}/{} ({} discrepancies so far)",
                    ledger_seq,
                    to_ledger,
                    discrepancies.len()
                );
                if let Some(ref cb) = progress_callback {
                    cb(percent.min(99), &msg);
                }
            }
        }

        // Compute summary statistics
        let avg_delta_pct = if deltas.is_empty() {
            0.0
        } else {
            deltas.iter().sum::<f64>() / deltas.len() as f64
        };

        let mut sorted_deltas = deltas.clone();
        sorted_deltas.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let median_delta_pct = if sorted_deltas.is_empty() {
            0.0
        } else {
            let mid = sorted_deltas.len() / 2;
            if sorted_deltas.len() % 2 == 0 {
                (sorted_deltas[mid - 1] + sorted_deltas[mid]) / 2.0
            } else {
                sorted_deltas[mid]
            }
        };

        let std_dev_delta_pct = if deltas.len() < 2 {
            0.0
        } else {
            let mean = avg_delta_pct;
            let variance: f64 = deltas.iter().map(|&d| (d - mean).powi(2)).sum::<f64>()
                / (deltas.len() - 1) as f64;
            variance.sqrt()
        };

        let ledgers_with_critical = discrepancies
            .iter()
            .filter(|d| d.severity == "critical")
            .count() as i64;
        let ledgers_with_warning = discrepancies
            .iter()
            .filter(|d| d.severity == "warning")
            .count() as i64;

        let summary = ReconciliationSummary {
            mean_delta_pct,
            median_delta_pct,
            std_dev_delta_pct,
            ledgers_with_critical,
            ledgers_with_warning,
        };

        let report = ReconciliationReport {
            id: report_id.clone(),
            from_ledger,
            to_ledger,
            tolerance_pct,
            total_ledgers: total_ledgers as i32,
            discrepancies_count: discrepancies.len() as i32,
            avg_delta_pct,
            max_delta_pct,
            summary: Some(summary),
            created_at: Utc::now().to_rfc3339(),
        };

        // Persist report and discrepancies
        self.persist_report(&report, &discrepancies).await?;

        if let Some(ref cb) = progress_callback {
            cb(100, "Reconciliation complete");
        }

        Ok(report)
    }

    async fn persist_report(
        &self,
        report: &ReconciliationReport,
        discrepancies: &[Discrepancy],
    ) -> Result<(), ReconciliationError> {
        let summary_json =
            serde_json::to_value(&report.summary).unwrap_or_default();

        sqlx::query(
            r#"
            INSERT INTO reconciliation_reports (
                id, from_ledger, to_ledger, tolerance_pct,
                total_ledgers, discrepancies_count, avg_delta_pct,
                max_delta_pct, summary, created_at
            ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
            "#,
        )
        .bind(&report.id)
        .bind(report.from_ledger)
        .bind(report.to_ledger)
        .bind(report.tolerance_pct)
        .bind(report.total_ledgers)
        .bind(report.discrepancies_count)
        .bind(report.avg_delta_pct)
        .bind(report.max_delta_pct)
        .bind(&summary_json)
        .bind(&report.created_at)
        .execute(&self.pool)
        .await
        .map_err(|e| ReconciliationError::StoreError(e.to_string()))?;

        for disc in discrepancies {
            sqlx::query(
                r#"
                INSERT INTO reconciliation_discrepancies (
                    id, report_id, ledger_sequence, expected_fee,
                    actual_fee, delta, delta_pct, severity
                ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                "#,
            )
            .bind(&disc.id)
            .bind(&disc.report_id)
            .bind(disc.ledger_sequence)
            .bind(disc.expected_fee)
            .bind(disc.actual_fee)
            .bind(disc.delta)
            .bind(disc.delta_pct)
            .bind(&disc.severity)
            .execute(&self.pool)
            .await
            .map_err(|e| ReconciliationError::StoreError(e.to_string()))?;
        }

        Ok(())
    }
}

/// Errors during reconciliation
#[derive(Debug, thiserror::Error)]
pub enum ReconciliationError {
    #[error("Store error: {0}")]
    StoreError(String),

    #[error("Invalid range: {0}")]
    InvalidRange(String),

    #[error("No data available for the requested ledger range")]
    NoData,
}

impl From<ReconciliationError> for AppError {
    fn from(err: ReconciliationError) -> Self {
        match err {
            ReconciliationError::InvalidRange(msg) => AppError::BadRequest(msg),
            ReconciliationError::NoData => {
                AppError::BadRequest("No data available for the requested ledger range".into())
            }
            ReconciliationError::StoreError(msg) => AppError::Internal(msg),
        }
    }
}

// ── HTTP Handlers ────────────────────────────────────────────────────────────

/// Submit an async reconciliation job
#[utoipa::path(
    post,
    path = "/reconcile",
    request_body = ReconcileRequest,
    responses(
        (status = 202, description = "Reconciliation job accepted", body = ReconcileResponse),
        (status = 400, description = "Invalid request"),
        (status = 500, description = "Internal server error")
    ),
    tag = "Reconciliation"
)]
pub async fn reconcile_handler(
    State(state): State<Arc<crate::AppState>>,
    Json(req): Json<ReconcileRequest>,
) -> Result<(StatusCode, Json<ReconcileResponse>), AppError> {
    if req.from_ledger >= req.to_ledger {
        return Err(AppError::BadRequest(
            "from_ledger must be less than to_ledger".into(),
        ));
    }

    if req.tolerance_pct <= 0.0 || req.tolerance_pct > 100.0 {
        return Err(AppError::BadRequest(
            "tolerance_pct must be between 0 and 100".into(),
        ));
    }

    let payload = crate::jobs::JobPayload::Reconcile {
        from_ledger: req.from_ledger,
        to_ledger: req.to_ledger,
        tolerance_pct: req.tolerance_pct,
    };

    let job_id = state
        .job_queue
        .submit(crate::jobs::JobType::Reconcile, payload, None)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(ReconcileResponse {
            job_id: job_id.to_string(),
            status: "QUEUED".to_string(),
            message: "Reconciliation job submitted successfully".to_string(),
        }),
    ))
}

/// Get reconciliation job status/result
#[utoipa::path(
    get,
    path = "/reconcile/{job_id}",
    responses(
        (status = 200, description = "Reconciliation job details"),
        (status = 404, description = "Job not found")
    ),
    params(
        ("job_id" = String, Path, description = "Job ID")
    ),
    tag = "Reconciliation"
)]
pub async fn get_reconcile_job_handler(
    State(state): State<Arc<crate::AppState>>,
    Path(job_id): Path<String>,
) -> Result<Json<crate::jobs::Job>, AppError> {
    let id = crate::jobs::JobId::from_str(&job_id)
        .map_err(|_| AppError::BadRequest("Invalid job ID".into()))?;

    let job = state
        .job_queue
        .get(&id)
        .await
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Job {} not found", job_id)))?;

    Ok(Json(job))
}

/// List recent reconciliation reports
#[utoipa::path(
    get,
    path = "/reconcile/reports",
    params(
        ("limit" = Option<i64>, Query, description = "Max reports to return (default 10)")
    ),
    responses(
        (status = 200, description = "List of reconciliation reports")
    ),
    tag = "Reconciliation"
)]
pub async fn list_reports_handler(
    State(state): State<Arc<crate::AppState>>,
    Query(params): Query<ListReportsQuery>,
) -> Result<Json<Vec<ReconciliationReport>>, AppError> {
    let pool = &state.reconciler_pool;

    let rows = sqlx::query_as::<_, ReportRow>(
        "SELECT * FROM reconciliation_reports ORDER BY created_at DESC LIMIT ?1",
    )
    .bind(params.limit)
    .fetch_all(pool)
    .await
    .map_err(|e| AppError::Internal(e.to_string()))?;

    let reports: Vec<ReconciliationReport> = rows.into_iter().map(|r| r.into_report()).collect();
    Ok(Json(reports))
}

/// Raw DB row for deserialization
#[derive(Debug, sqlx::FromRow)]
struct ReportRow {
    id: String,
    from_ledger: i64,
    to_ledger: i64,
    tolerance_pct: f64,
    total_ledgers: i32,
    discrepancies_count: i32,
    avg_delta_pct: f64,
    max_delta_pct: f64,
    summary: Option<serde_json::Value>,
    created_at: String,
}

impl ReportRow {
    fn into_report(self) -> ReconciliationReport {
        let summary: Option<ReconciliationSummary> =
            self.summary.and_then(|v| serde_json::from_value(v).ok());

        ReconciliationReport {
            id: self.id,
            from_ledger: self.from_ledger,
            to_ledger: self.to_ledger,
            tolerance_pct: self.tolerance_pct,
            total_ledgers: self.total_ledgers,
            discrepancies_count: self.discrepancies_count,
            avg_delta_pct: self.avg_delta_pct,
            max_delta_pct: self.max_delta_pct,
            summary,
            created_at: self.created_at,
        }
    }
}
