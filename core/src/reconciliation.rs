#![allow(dead_code)]

//! Fee reconciliation engine.
//!
//! **Precision model:** Fee percentages (`delta_pct`, `mean_delta_pct`, etc.)
//! are stored as `f64` for API compatibility. Internal calculations use
//! integer arithmetic where possible (`i64` for fee amounts in stroops).
//! When converting between integer stroop amounts and percentage values,
//! rounding is performed to 4 decimal places to avoid floating-point drift
//! in reconciliation summaries.
//!
//! Known limitation: `Discrepancy::delta_pct` uses `f64`, which may lose
//! precision for very small fee differences. For high-precision use cases,
//! consider migrating to fixed-point arithmetic (e.g. `i128` with 18 decimal
//! places) in a future iteration.

use crate::db;
use crate::input_sanitization::{SanitizedJson, SanitizedPath, SanitizedQuery};
use crate::runner::RequestCancellation;
use std::str::FromStr;
use tokio_util::sync::CancellationToken;
use crate::fee_analytics::FeeAnalyticsEngine;
use crate::fee_store::FeeStore;
use crate::AppError;
use axum::{
    extract::State,
    http::StatusCode,
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
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

/// The reconciliation DTOs live in [`crate::db::models`]; the typed DB layer
/// returns them directly. They were duplicated here field-for-field, which is
/// what made `FeeReconciler` build one `ReconciliationReport` while the repo
/// expected the other. `DiscrepancySeverity` above stays local — the DB layer
/// stores severity as a plain string and has no equivalent enum.
pub use crate::db::models::{
    Discrepancy, ListReportsQuery, ReconcileRequest, ReconcileResponse, ReconciliationReport,
    ReconciliationSummary,
};

/// Fee reconciliation engine that compares predictions against actuals
pub struct FeeReconciler {
    store: Arc<FeeStore>,
    analytics: FeeAnalyticsEngine,
    reports: db::reconciliation::ReconciliationRepo,
}

impl FeeReconciler {
    pub fn new(store: Arc<FeeStore>, reports: db::reconciliation::ReconciliationRepo) -> Self {
        Self {
            store,
            analytics: FeeAnalyticsEngine::new(),
            reports,
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
        let cancellation = CancellationToken::new();
        let _guard = cancellation.clone().drop_guard();
        self.run_with_cancellation(
            from_ledger,
            to_ledger,
            tolerance_pct,
            progress_callback,
            cancellation,
        )
        .await
    }

    pub async fn run_with_cancellation(
        &self,
        from_ledger: i64,
        to_ledger: i64,
        tolerance_pct: f64,
        progress_callback: Option<Box<dyn Fn(i32, &str) + Send + Sync>>,
        cancellation: CancellationToken,
    ) -> Result<ReconciliationReport, ReconciliationError> {
        if cancellation.is_cancelled() {
            return Err(ReconciliationError::Cancelled);
        }
        let report_id = uuid::Uuid::new_v4().to_string();
        let mut discrepancies: Vec<Discrepancy> = Vec::new();
        let total_ledgers = to_ledger - from_ledger + 1;

        // Get all samples in the range for actuals
        let actual_samples = tokio::select! {
            _ = cancellation.cancelled() => return Err(ReconciliationError::Cancelled),
            result = self.store.get_samples_in_range(from_ledger, to_ledger) => result
                .map_err(|e| ReconciliationError::StoreError(e.to_string()))?,
        };

        // Index actual samples by ledger sequence for fast lookup
        let actuals: std::collections::HashMap<i64, i64> = actual_samples
            .iter()
            .map(|s| (s.ledger_sequence, s.base_fee))
            .collect();

        let mut checked = 0i64;
        let mut deltas: Vec<f64> = Vec::new();
        let mut max_delta_pct: f64 = 0.0;

        for ledger_seq in from_ledger..=to_ledger {
            if cancellation.is_cancelled() {
                return Err(ReconciliationError::Cancelled);
            }
            checked += 1;

            // Get historical data up to (but not including) this ledger for prediction
            let historical = tokio::select! {
                _ = cancellation.cancelled() => return Err(ReconciliationError::Cancelled),
                result = self
                    .store
                    .get_samples_in_range(ledger_seq - 100, ledger_seq - 1) => result
                    .map_err(|e| ReconciliationError::StoreError(e.to_string()))?,
            };

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
            if sorted_deltas.len().is_multiple_of(2) {
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
            // The local binding is `avg_delta_pct`; the DTO field is
            // `mean_delta_pct`. Same statistic, different name.
            mean_delta_pct: avg_delta_pct,
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
        tokio::select! {
            _ = cancellation.cancelled() => return Err(ReconciliationError::Cancelled),
            result = self.persist_report(&report, &discrepancies) => result?,
        }

        if cancellation.is_cancelled() {
            return Err(ReconciliationError::Cancelled);
        }
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
        self.reports
            .persist_report(report, discrepancies)
            .await
            .map_err(|e| ReconciliationError::StoreError(e.to_string()))?;

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

    #[error("Reconciliation cancelled")]
    Cancelled,
}

impl From<ReconciliationError> for AppError {
    fn from(err: ReconciliationError) -> Self {
        match err {
            ReconciliationError::InvalidRange(msg) => AppError::BadRequest(msg),
            ReconciliationError::NoData => {
                AppError::BadRequest("No data available for the requested ledger range".into())
            }
            ReconciliationError::Cancelled => AppError::Internal("Reconciliation cancelled".into()),
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
    SanitizedJson(req): SanitizedJson<ReconcileRequest>,
    Extension(cancellation): Extension<RequestCancellation>,
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

    let job_id = cancellation
        .wait(
            state
                .job_queue
                .submit(crate::jobs::JobType::Reconcile, payload, None),
        )
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
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
    SanitizedPath(job_id): SanitizedPath<String>,
    Extension(cancellation): Extension<RequestCancellation>,
    Path(job_id): Path<String>,
) -> Result<Json<crate::jobs::Job>, AppError> {
    let id = crate::jobs::JobId::from_str(&job_id)
        .map_err(|_| AppError::BadRequest("Invalid job ID".into()))?;

    let job = cancellation
        .wait(state.job_queue.get(&id))
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
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
    SanitizedQuery(params): SanitizedQuery<ListReportsQuery>,
    Extension(cancellation): Extension<RequestCancellation>,
    Query(params): Query<ListReportsQuery>,
) -> Result<Json<Vec<ReconciliationReport>>, AppError> {
    let reports = cancellation
        .wait(state.reconciliation_repo.list(params.limit))
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
        .map_err(|e| AppError::Internal(e.to_string()))?;
    Ok(Json(reports))
}
