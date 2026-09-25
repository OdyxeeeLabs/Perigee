use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use utoipa::ToSchema;

// ── Pagination ────────────────────────────────────────────────────────────────

/// Query parameters accepted by paginated list endpoints.
///
/// - `page` is 1-indexed; defaults to 1.
/// - `page_size` defaults to 50; maximum is 200.
#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct PaginationParams {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
}

fn default_page() -> u32 {
    1
}

fn default_page_size() -> u32 {
    50
}

impl PaginationParams {
    pub const MAX_PAGE_SIZE: u32 = 200;

    /// Returns the clamped page size and the SQL OFFSET value.
    pub fn to_limit_offset(&self) -> (i64, i64) {
        let page = self.page.max(1);
        let size = self.page_size.clamp(1, Self::MAX_PAGE_SIZE);
        let offset = (page - 1) * size;
        (size as i64, offset as i64)
    }
}

/// Paginated list response envelope.
#[derive(Debug, Serialize, ToSchema)]
pub struct PagedResponse<T> {
    pub data: Vec<T>,
    pub total_count: i64,
    pub page: u32,
    pub page_size: u32,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct ManagerRecord {
    pub id: String,
    pub stellar_address: String,
    pub name: String,
    pub email: String,
    pub status: String,
    pub kyc_document_ref: String,
    pub notes: String,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct RegisterManagerRequest {
    pub stellar_address: String,
    pub name: String,
    #[serde(default)]
    pub email: String,
    #[serde(default)]
    pub kyc_document_ref: String,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct ApproveManagerRequest {
    #[serde(default)]
    pub notes: String,
}

#[derive(Debug, Serialize, ToSchema)]
pub struct ManagerStatusResponse {
    pub id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct VaultRecord {
    pub id: String,
    pub manager_id: String,
    pub name: String,
    pub status: String,
    pub config_json: String,
    pub version: i64,
    pub idempotency_key: Option<String>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    /// Set when the vault is soft-deleted (BE-044 / issue #281). `None` means the
    /// vault is active. Default queries filter rows where this is `NULL`.
    pub deleted_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct CreateVaultRequest {
    pub manager_id: String,
    pub name: String,
    #[serde(default = "default_status")]
    pub status: String,
    #[serde(default = "default_config")]
    pub config_json: String,
    #[serde(default)]
    pub idempotency_key: Option<String>,
}

fn default_status() -> String {
    "active".to_string()
}

fn default_config() -> String {
    "{}".to_string()
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct UpdateVaultRequest {
    pub version: i64,
    pub name: Option<String>,
    pub status: Option<String>,
    pub config_json: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
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

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct ReconciliationSummary {
    pub mean_delta_pct: f64,
    pub median_delta_pct: f64,
    pub std_dev_delta_pct: f64,
    pub ledgers_with_critical: i64,
    pub ledgers_with_warning: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
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

#[derive(Debug, Serialize, ToSchema)]
pub struct ReconcileResponse {
    pub job_id: String,
    pub status: String,
    pub message: String,
}

#[derive(Debug, Deserialize, Serialize, ToSchema)]
pub struct ListReportsQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
}

fn default_limit() -> i64 {
    10
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct JobRecord {
    pub id: String,
    pub job_type: String,
    pub status: String,
    pub payload: serde_json::Value,
    pub result: Option<serde_json::Value>,
    pub progress_percent: i32,
    pub webhook_url: Option<String>,
    pub error_message: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct LedgerFeeSample {
    pub ledger_sequence: i64,
    pub collected_at: DateTime<Utc>,
    pub base_reserve: i64,
    pub base_fee: i64,
    pub max_fee: i64,
    pub fee_charged: i64,
    pub transaction_count: i64,
    pub ledger_close_time: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize, FromRow, ToSchema)]
pub struct TransactionFeeRecord {
    pub id: String,
    pub ledger_sequence: i64,
    pub tx_hash: String,
    pub fee_bid: i64,
    pub fee_charged: i64,
    pub resource_fee: i64,
    pub inclusion_success: bool,
    pub recorded_at: DateTime<Utc>,
}
