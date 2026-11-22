//! Immutable comparison of persisted fee history with indexed fee-engine events.

use super::service::FeeRecordView;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

pub const MAX_RECONCILIATION_LEDGER_SPAN: i64 = 10_000;

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct TenantFeeRecord {
    pub tenant_id: String,
    pub record: FeeRecordView,
}

#[derive(Debug, Deserialize, ToSchema)]
pub struct FeeReconciliationRequest {
    pub from_ledger: u64,
    pub to_ledger: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeeReconciliationStatus {
    Clean,
    DiscrepanciesFound,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum FeeDiscrepancyKind {
    MissingFeeHistory,
    MissingLedgerEvent,
    ConflictingRecord,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FeeReconciliationDiscrepancy {
    pub ledger_sequence: u64,
    pub vault: String,
    pub payer: String,
    pub kind: u32,
    pub discrepancy: FeeDiscrepancyKind,
    pub fee_history: Vec<FeeRecordView>,
    pub ledger_events: Vec<FeeRecordView>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct FeeReconciliationReport {
    pub id: String,
    pub tenant_id: String,
    pub actor_id: String,
    pub from_ledger: u64,
    pub to_ledger: u64,
    pub total_ledgers: u64,
    pub matched_records: u64,
    pub status: FeeReconciliationStatus,
    pub discrepancies: Vec<FeeReconciliationDiscrepancy>,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum FeeReconciliationError {
    #[error("ledger range must be non-negative, ordered, and no wider than {MAX_RECONCILIATION_LEDGER_SPAN} ledgers")]
    InvalidRange,
    #[error("tenant_id and actor_id are required")]
    MissingAuditIdentity,
    #[error("fee reconciliation found an invalid empty record group")]
    InvalidRecordGroup,
    #[error("fee reconciliation database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("fee reconciliation report serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct RecordKey {
    sequence: u64,
}

impl RecordKey {
    fn from(record: &FeeRecordView) -> Self {
        Self {
            sequence: record.seq,
        }
    }
}

/// Compare two tenant-tagged snapshots without mutating either source.
pub fn reconcile_fee_history(
    tenant_id: &str,
    actor_id: &str,
    from_ledger: u64,
    to_ledger: u64,
    fee_history: &[TenantFeeRecord],
    ledger_events: &[TenantFeeRecord],
) -> Result<FeeReconciliationReport, FeeReconciliationError> {
    let span = to_ledger
        .checked_sub(from_ledger)
        .and_then(|distance| distance.checked_add(1))
        .filter(|span| *span <= MAX_RECONCILIATION_LEDGER_SPAN as u64);
    if tenant_id.trim().is_empty()
        || actor_id.trim().is_empty()
        || from_ledger > to_ledger
        || to_ledger > i64::MAX as u64
        || span.is_none()
    {
        return Err(if tenant_id.trim().is_empty() || actor_id.trim().is_empty() {
            FeeReconciliationError::MissingAuditIdentity
        } else {
            FeeReconciliationError::InvalidRange
        });
    }
    let total_ledgers = span.ok_or(FeeReconciliationError::InvalidRange)?;

    let history = records_by_key(tenant_id, from_ledger, to_ledger, fee_history);
    let events = records_by_key(tenant_id, from_ledger, to_ledger, ledger_events);
    let keys: BTreeSet<_> = history.keys().chain(events.keys()).cloned().collect();
    let mut matched_records = 0;
    let mut discrepancies = Vec::new();

    for key in keys {
        let history_records = history.get(&key).map(Vec::as_slice).unwrap_or_default();
        let event_records = events.get(&key).map(Vec::as_slice).unwrap_or_default();
        let discrepancy = match (history_records, event_records) {
            ([history_record], [event_record]) if history_record == event_record => {
                matched_records += 1;
                continue;
            }
            ([], [_]) => FeeDiscrepancyKind::MissingFeeHistory,
            ([_], []) => FeeDiscrepancyKind::MissingLedgerEvent,
            _ => FeeDiscrepancyKind::ConflictingRecord,
        };
        let representative = event_records
            .first()
            .or_else(|| history_records.first())
            .ok_or(FeeReconciliationError::InvalidRecordGroup)?;
        discrepancies.push(FeeReconciliationDiscrepancy {
            ledger_sequence: key.sequence,
            vault: representative.vault.clone(),
            payer: representative.payer.clone(),
            kind: representative.kind,
            discrepancy,
            fee_history: history_records
                .iter()
                .map(|record| (**record).clone())
                .collect(),
            ledger_events: event_records
                .iter()
                .map(|record| (**record).clone())
                .collect(),
        });
    }

    let status = if discrepancies.is_empty() {
        FeeReconciliationStatus::Clean
    } else {
        FeeReconciliationStatus::DiscrepanciesFound
    };

    Ok(FeeReconciliationReport {
        id: Uuid::new_v4().to_string(),
        tenant_id: tenant_id.to_string(),
        actor_id: actor_id.to_string(),
        from_ledger,
        to_ledger,
        total_ledgers,
        matched_records,
        status,
        discrepancies,
        created_at: Utc::now(),
    })
}

fn records_by_key<'a>(
    tenant_id: &str,
    from_ledger: u64,
    to_ledger: u64,
    records: &'a [TenantFeeRecord],
) -> BTreeMap<RecordKey, Vec<&'a FeeRecordView>> {
    let mut indexed = BTreeMap::new();
    for tagged in records {
        if tagged.tenant_id != tenant_id
            || tagged.record.seq < from_ledger
            || tagged.record.seq > to_ledger
        {
            continue;
        }
        indexed
            .entry(RecordKey::from(&tagged.record))
            .or_insert_with(Vec::new)
            .push(&tagged.record);
    }
    indexed
}

/// Append-only audit storage. Reports are inserted once and listed only within
/// the requesting tenant; no operation updates fee history or prior reports.
pub struct FeeReconciliationRepository {
    pool: SqlitePool,
}

impl FeeReconciliationRepository {
    pub fn new(pool: SqlitePool) -> Self {
        Self { pool }
    }

    pub fn pool(&self) -> &SqlitePool {
        &self.pool
    }

    pub async fn reconcile_and_persist(
        &self,
        tenant_id: &str,
        actor_id: &str,
        from_ledger: u64,
        to_ledger: u64,
        fee_history: &[TenantFeeRecord],
        ledger_events: &[TenantFeeRecord],
    ) -> Result<FeeReconciliationReport, FeeReconciliationError> {
        let report = reconcile_fee_history(
            tenant_id,
            actor_id,
            from_ledger,
            to_ledger,
            fee_history,
            ledger_events,
        )?;
        self.persist(&report).await?;
        Ok(report)
    }

    pub async fn persist(
        &self,
        report: &FeeReconciliationReport,
    ) -> Result<(), FeeReconciliationError> {
        let result_json = serde_json::to_string(report)?;
        sqlx::query(
            "INSERT INTO fee_reconciliation_reports \
             (id, tenant_id, actor_id, from_ledger, to_ledger, status, result_json, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        )
        .bind(&report.id)
        .bind(&report.tenant_id)
        .bind(&report.actor_id)
        .bind(report.from_ledger as i64)
        .bind(report.to_ledger as i64)
        .bind(match report.status {
            FeeReconciliationStatus::Clean => "clean",
            FeeReconciliationStatus::DiscrepanciesFound => "discrepancies_found",
        })
        .bind(result_json)
        .bind(report.created_at.to_rfc3339())
        .execute(&self.pool)
        .await?;
        Ok(())
    }

    pub async fn find_for_tenant(
        &self,
        tenant_id: &str,
        report_id: &str,
    ) -> Result<Option<FeeReconciliationReport>, FeeReconciliationError> {
        let row = sqlx::query_scalar::<_, String>(
            "SELECT result_json FROM fee_reconciliation_reports \
             WHERE tenant_id = ?1 AND id = ?2",
        )
        .bind(tenant_id)
        .bind(report_id)
        .fetch_optional(&self.pool)
        .await?;
        row.map(|json| serde_json::from_str(&json).map_err(FeeReconciliationError::from))
            .transpose()
    }

    pub async fn list_for_tenant(
        &self,
        tenant_id: &str,
        limit: i64,
    ) -> Result<Vec<FeeReconciliationReport>, FeeReconciliationError> {
        let rows = sqlx::query_scalar::<_, String>(
            "SELECT result_json FROM fee_reconciliation_reports \
             WHERE tenant_id = ?1 ORDER BY created_at DESC, id DESC LIMIT ?2",
        )
        .bind(tenant_id)
        .bind(limit.clamp(1, 200))
        .fetch_all(&self.pool)
        .await?;
        rows.into_iter()
            .map(|row| serde_json::from_str(&row).map_err(FeeReconciliationError::from))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    fn record(seq: u64, net: i128) -> FeeRecordView {
        FeeRecordView {
            seq,
            vault: "vault-a".to_string(),
            payer: "payer-a".to_string(),
            gross: 120,
            discount: 20,
            net,
            vault_share: 80,
            protocol_share: net - 80,
            timestamp: 1_700_000_000,
            kind: 1,
        }
    }

    fn tagged(tenant_id: &str, record: FeeRecordView) -> TenantFeeRecord {
        TenantFeeRecord {
            tenant_id: tenant_id.to_string(),
            record,
        }
    }

    #[test]
    fn matching_records_produce_a_clean_report() {
        let history = [tagged("tenant-a", record(10, 100))];
        let events = [tagged("tenant-a", record(10, 100))];

        let report = reconcile_fee_history("tenant-a", "operator-a", 10, 10, &history, &events)
            .expect("valid fixture");

        assert_eq!(report.status, FeeReconciliationStatus::Clean);
        assert_eq!(report.matched_records, 1);
        assert!(report.discrepancies.is_empty());
        assert_eq!(
            serde_json::to_value(&report)
                .expect("report JSON")["status"],
            "clean"
        );
    }

    #[test]
    fn missing_history_and_conflicting_records_are_reported() {
        let history = [
            tagged("tenant-a", record(10, 100)),
            tagged("tenant-a", record(11, 100)),
            tagged("tenant-a", record(13, 100)),
        ];
        let events = [
            tagged("tenant-a", record(10, 100)),
            tagged("tenant-a", record(11, 90)),
            tagged("tenant-a", record(12, 100)),
        ];

        let report = reconcile_fee_history("tenant-a", "operator-a", 10, 13, &history, &events)
            .expect("valid fixture");

        assert_eq!(
            report.status,
            FeeReconciliationStatus::DiscrepanciesFound
        );
        assert_eq!(report.matched_records, 1);
        assert_eq!(report.discrepancies.len(), 3);
        assert_eq!(
            report.discrepancies[0].discrepancy,
            FeeDiscrepancyKind::ConflictingRecord
        );
        assert_eq!(
            report.discrepancies[1].discrepancy,
            FeeDiscrepancyKind::MissingFeeHistory
        );
        assert_eq!(
            report.discrepancies[2].discrepancy,
            FeeDiscrepancyKind::MissingLedgerEvent
        );
        assert_eq!(report.discrepancies[1].ledger_events.len(), 1);
        assert_eq!(
            serde_json::to_value(&report)
                .expect("report JSON")["status"],
            "discrepancies_found"
        );
    }

    #[test]
    fn reconciliation_is_range_and_tenant_scoped() {
        let history = [
            tagged("tenant-a", record(10, 100)),
            tagged("tenant-b", record(10, 100)),
            tagged("tenant-a", record(11, 100)),
        ];
        let events = [tagged("tenant-a", record(10, 100))];

        let report = reconcile_fee_history("tenant-a", "operator-a", 10, 10, &history, &events)
            .expect("valid fixture");

        assert_eq!(report.status, FeeReconciliationStatus::Clean);
        assert_eq!(report.total_ledgers, 1);
    }

    #[test]
    fn reconciliation_rejects_ranges_over_the_limit() {
        let result = reconcile_fee_history(
            "tenant-a",
            "operator-a",
            1,
            MAX_RECONCILIATION_LEDGER_SPAN as u64 + 1,
            &[],
            &[],
        );

        assert!(matches!(
            result,
            Err(FeeReconciliationError::InvalidRange)
        ));
    }

    #[tokio::test]
    async fn audit_reports_are_append_only_and_tenant_scoped() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .expect("in-memory database");
        sqlx::query(
            "CREATE TABLE fee_reconciliation_reports (\
             id TEXT PRIMARY KEY, tenant_id TEXT NOT NULL, actor_id TEXT NOT NULL, \
             from_ledger INTEGER NOT NULL, to_ledger INTEGER NOT NULL, status TEXT NOT NULL, \
             result_json TEXT NOT NULL, created_at TEXT NOT NULL)",
        )
        .execute(&pool)
        .await
        .expect("audit table");
        let repository = FeeReconciliationRepository::new(pool);
        let report = repository
            .reconcile_and_persist("tenant-a", "operator-a", 10, 10, &[], &[])
            .await
            .expect("persist report");
        assert!(repository.persist(&report).await.is_err());
        let tenant_reports = repository
            .list_for_tenant("tenant-a", 10)
            .await
            .expect("list tenant reports");
        let other_tenant_reports = repository
            .list_for_tenant("tenant-b", 10)
            .await
            .expect("list other tenant reports");

        assert_eq!(tenant_reports.len(), 1);
        assert_eq!(tenant_reports[0].id, report.id);
        assert!(other_tenant_reports.is_empty());
        assert!(repository
            .find_for_tenant("tenant-b", &report.id)
            .await
            .expect("tenant-scoped lookup")
            .is_none());
        assert_eq!(
            sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM fee_reconciliation_reports")
                .fetch_one(repository.pool())
                .await
                .expect("audit row count"),
            1
        );
    }
}
