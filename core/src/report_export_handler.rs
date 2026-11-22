//! Axum handler for the tenant-scoped CSV report export (API-21, #782).
//!
//! This module lives in the binary crate because it depends on `AppState`.
//! All CSV rendering and escaping logic lives in the library crate's
//! `csv_export` module so it can be unit-tested independently.

use crate::auth::AuthenticatedUser;
use crate::csv_export::{reports_to_csv, MAX_EXPORT_BYTES, MAX_EXPORT_REPORTS, MAX_EXPORT_ROWS};
use crate::error_codes::ErrorCode;
use crate::errors::AppError;
use axum::{
    extract::{Query, State},
    http::{header, HeaderName, StatusCode},
    response::{IntoResponse, Response},
    Extension,
};
use serde::Deserialize;
use std::sync::Arc;
use utoipa::ToSchema;

fn default_export_limit() -> usize {
    50
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CsvExportQuery {
    /// Number of most recent reports to include (1-200, default 50).
    #[serde(default = "default_export_limit")]
    pub limit: usize,
}

/// GET /reports/fee-reconciliation.csv
#[utoipa::path(
    get,
    path = "/reports/fee-reconciliation.csv",
    params(
        ("limit" = Option<usize>, Query, description = "Most recent reports to include (1-200, default 50)")
    ),
    responses(
        (status = 200, description = "Tenant-scoped CSV export", content_type = "text/csv", body = String),
        (status = 400, description = "Export exceeds row or byte limit"),
        (status = 401, description = "Unauthorized")
    ),
    security(("bearerAuth" = []), ("jwt" = [])),
    tag = "Reports"
)]
pub async fn export_reports_csv_handler(
    State(state): State<Arc<crate::AppState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Query(query): Query<CsvExportQuery>,
) -> Result<Response, AppError> {
    let limit = query.limit.clamp(1, MAX_EXPORT_REPORTS);
    let reports = state
        .report_export_repo
        .list_for_tenant(&user.stellar_address, limit as i64)
        .await
        .map_err(|error| AppError::Internal(error.to_string()))?;

    let csv = reports_to_csv(&user.stellar_address, &reports);
    if csv.rows > MAX_EXPORT_ROWS {
        return Err(AppError::with_code(
            ErrorCode::InvalidInput,
            format!(
                "CSV export contains {} rows; maximum is {}",
                csv.rows, MAX_EXPORT_ROWS
            ),
        ));
    }
    if csv.body.len() > MAX_EXPORT_BYTES {
        return Err(AppError::with_code(
            ErrorCode::InvalidInput,
            format!(
                "CSV export is {} bytes; maximum is {}",
                csv.body.len(),
                MAX_EXPORT_BYTES
            ),
        ));
    }

    Ok((
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"perigee-fee-reconciliation-reports.csv\"",
            ),
            (header::CACHE_CONTROL, "no-store"),
            (HeaderName::from_static("x-content-type-options"), "nosniff"),
        ],
        csv.body,
    )
        .into_response())
}
