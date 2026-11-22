//! # Data archival strategy
//!
//! Tiered storage keeps the primary database small without losing history:
//!
//! | Tier | Age | Where | Access |
//! | --- | --- | --- | --- |
//! | **Hot** | last 90 days | main tables | normal queries |
//! | **Warm** | up to 12 months | monthly partitions, same cluster | normal queries, partition pruned |
//! | **Cold** | older | Parquet in S3/Glacier, partitioned by tenant/vault/month | Athena/Trino, or the async retrieval API |
//!
//! A monthly job ([`plan_monthly_run`]) exports each warm partition that has
//! aged out to the cold tier, verifies the export against the row count and
//! checksum, records it in the [`ArchiveManifest`], and only then drops the
//! partition. [`RetrievalService`] gives clients an asynchronous way to ask for
//! cold data, and the manifest drives GDPR erasure of data that already left the
//! database ([`ArchiveManifest::gdpr_rewrite_tasks`]). See `docs/ARCHIVAL.md`.

use chrono::{Datelike, NaiveDate};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use thiserror::Error;

pub const HOT_DAYS: i64 = 90;
pub const WARM_MONTHS: i64 = 12;
/// How long a prepared retrieval stays downloadable.
pub const RETRIEVAL_TTL_SECS: u64 = 24 * 60 * 60;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum ArchivalError {
    #[error("`{0}` is not a valid SQL identifier")]
    InvalidIdentifier(String),
    #[error("invalid month {0}")]
    InvalidMonth(u32),
    #[error("unknown retrieval request `{0}`")]
    UnknownRequest(String),
    #[error("retrieval request is not in the `{0}` state")]
    WrongState(&'static str),
    #[error("the export for {0} failed verification: {1}")]
    VerificationFailed(String, String),
    #[error("not allowed to read another tenant's archive")]
    TenantMismatch,
}

fn ident(name: &str) -> Result<&str, ArchivalError> {
    let valid = !name.is_empty()
        && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
        && !name.starts_with(|c: char| c.is_ascii_digit());
    if valid {
        Ok(name)
    } else {
        Err(ArchivalError::InvalidIdentifier(name.to_string()))
    }
}

// ---------------------------------------------------------------------------
// Tiers
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    Hot,
    Warm,
    Cold,
}

#[derive(Debug, Clone, Copy)]
pub struct RetentionPolicy {
    pub hot_days: i64,
    pub warm_months: i64,
}

impl Default for RetentionPolicy {
    fn default() -> Self {
        RetentionPolicy { hot_days: HOT_DAYS, warm_months: WARM_MONTHS }
    }
}

fn months_between(from: NaiveDate, to: NaiveDate) -> i64 {
    (to.year() - from.year()) as i64 * 12 + to.month() as i64 - from.month() as i64
}

impl RetentionPolicy {
    /// Which tier a record created on `created` belongs to as of `today`.
    pub fn tier_for(&self, created: NaiveDate, today: NaiveDate) -> Tier {
        let age_days = (today - created).num_days();
        if age_days <= self.hot_days {
            Tier::Hot
        } else if months_between(created, today) <= self.warm_months {
            Tier::Warm
        } else {
            Tier::Cold
        }
    }
}

// ---------------------------------------------------------------------------
// Partitions
// ---------------------------------------------------------------------------

/// `vault_events_2026_10`
pub fn partition_name(table: &str, year: i32, month: u32) -> Result<String, ArchivalError> {
    if !(1..=12).contains(&month) {
        return Err(ArchivalError::InvalidMonth(month));
    }
    Ok(format!("{}_{year:04}_{month:02}", ident(table)?))
}

fn month_bounds(year: i32, month: u32) -> Result<(NaiveDate, NaiveDate), ArchivalError> {
    let start = NaiveDate::from_ymd_opt(year, month, 1).ok_or(ArchivalError::InvalidMonth(month))?;
    let (next_year, next_month) = if month == 12 { (year + 1, 1) } else { (year, month + 1) };
    let end = NaiveDate::from_ymd_opt(next_year, next_month, 1).ok_or(ArchivalError::InvalidMonth(month))?;
    Ok((start, end))
}

/// PostgreSQL declarative-partitioning DDL for one month of a table that is
/// `PARTITION BY RANGE (created_at)`.
pub fn create_partition_sql(table: &str, year: i32, month: u32) -> Result<String, ArchivalError> {
    let name = partition_name(table, year, month)?;
    let (start, end) = month_bounds(year, month)?;
    Ok(format!(
        "CREATE TABLE IF NOT EXISTS {name} PARTITION OF {table} FOR VALUES FROM ('{start}') TO ('{end}');"
    ))
}

/// DDL for the current month and the next `months_ahead`, so inserts never miss a partition.
pub fn ensure_partitions_sql(table: &str, today: NaiveDate, months_ahead: u32) -> Result<Vec<String>, ArchivalError> {
    let mut statements = Vec::new();
    let (mut year, mut month) = (today.year(), today.month());
    for _ in 0..=months_ahead {
        statements.push(create_partition_sql(table, year, month)?);
        if month == 12 {
            year += 1;
            month = 1;
        } else {
            month += 1;
        }
    }
    Ok(statements)
}

// ---------------------------------------------------------------------------
// Cold storage layout
// ---------------------------------------------------------------------------

/// Hive-style key so Athena and Trino can prune by tenant, vault and month:
/// `archive/<table>/tenant=<t>/vault=<v>/year=YYYY/month=MM/part-0000.parquet`.
pub fn object_key(table: &str, tenant: &str, vault: &str, year: i32, month: u32) -> Result<String, ArchivalError> {
    ident(table)?;
    if !(1..=12).contains(&month) {
        return Err(ArchivalError::InvalidMonth(month));
    }
    let clean = |part: &str| part.replace(['/', '='], "_");
    Ok(format!(
        "archive/{table}/tenant={}/vault={}/year={year:04}/month={month:02}/part-0000.parquet",
        clean(tenant),
        clean(vault)
    ))
}

/// Athena DDL over the archive with partition projection, so no `MSCK REPAIR` is needed.
pub fn athena_table_ddl(table: &str, columns: &[(&str, &str)], bucket: &str) -> Result<String, ArchivalError> {
    ident(table)?;
    let cols = columns
        .iter()
        .map(|(name, ty)| ident(name).map(|n| format!("  {n} {ty}")))
        .collect::<Result<Vec<_>, _>>()?
        .join(",\n");
    Ok(format!(
        "CREATE EXTERNAL TABLE IF NOT EXISTS {table}_archive (\n{cols}\n)\n\
         PARTITIONED BY (tenant string, vault string, year int, month int)\n\
         STORED AS PARQUET\n\
         LOCATION 's3://{bucket}/archive/{table}/'\n\
         TBLPROPERTIES (\n  'projection.enabled' = 'true',\n  'projection.tenant.type' = 'injected',\n  'projection.vault.type' = 'injected',\n  'projection.year.type' = 'integer',\n  'projection.year.range' = '2020,2100',\n  'projection.month.type' = 'integer',\n  'projection.month.range' = '1,12',\n  'projection.month.digits' = '2',\n  'storage.location.template' = 's3://{bucket}/archive/{table}/tenant=${{tenant}}/vault=${{vault}}/year=${{year}}/month=${{month}}/'\n);"
    ))
}

// ---------------------------------------------------------------------------
// Monthly job
// ---------------------------------------------------------------------------

/// One partition due to move to the cold tier.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArchiveTask {
    pub table: String,
    pub year: i32,
    pub month: u32,
    pub partition: String,
    /// Reads the rows to export (the exporter writes Parquet per tenant/vault).
    pub export_query: String,
    /// Counts the rows the export must contain.
    pub count_query: String,
    /// Run only after the export is verified.
    pub detach_sql: String,
    pub drop_sql: String,
}

/// Partitions older than the warm window that are not archived yet.
pub fn plan_monthly_run(
    tables: &[&str],
    today: NaiveDate,
    policy: &RetentionPolicy,
    already_archived: &BTreeSet<(String, i32, u32)>,
    oldest: (i32, u32),
) -> Result<Vec<ArchiveTask>, ArchivalError> {
    let mut tasks = Vec::new();
    let (mut year, mut month) = oldest;
    loop {
        let (start, _) = month_bounds(year, month)?;
        if months_between(start, today) <= policy.warm_months {
            break;
        }
        for table in tables {
            if already_archived.contains(&(table.to_string(), year, month)) {
                continue;
            }
            let partition = partition_name(table, year, month)?;
            tasks.push(ArchiveTask {
                table: table.to_string(),
                year,
                month,
                export_query: format!("SELECT * FROM {partition}"),
                count_query: format!("SELECT COUNT(*) FROM {partition}"),
                detach_sql: format!("ALTER TABLE {table} DETACH PARTITION {partition};"),
                drop_sql: format!("DROP TABLE {partition};"),
                partition,
            });
        }
        if month == 12 {
            year += 1;
            month = 1;
        } else {
            month += 1;
        }
    }
    Ok(tasks)
}

// ---------------------------------------------------------------------------
// Manifest and verification
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestEntry {
    pub key: String,
    pub table: String,
    pub tenant: String,
    pub vault: String,
    pub year: i32,
    pub month: u32,
    pub row_count: u64,
    pub checksum: String,
    /// Data subjects present in the object, for GDPR erasure.
    pub subjects: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RewriteTask {
    pub key: String,
    pub remove_subject: String,
}

#[derive(Debug, Default)]
pub struct ArchiveManifest {
    entries: BTreeMap<String, ManifestEntry>,
}

impl ArchiveManifest {
    /// Record an export, but only if what landed in S3 matches what was in the
    /// database. Returns the entry so the caller can proceed to detach/drop.
    pub fn record_verified(&mut self, entry: ManifestEntry, expected_rows: u64, uploaded_rows: u64, expected_checksum: &str) -> Result<(), ArchivalError> {
        if uploaded_rows != expected_rows {
            return Err(ArchivalError::VerificationFailed(entry.key, format!("expected {expected_rows} rows, found {uploaded_rows}")));
        }
        if entry.checksum != expected_checksum {
            return Err(ArchivalError::VerificationFailed(entry.key, "checksum mismatch".to_string()));
        }
        self.entries.insert(entry.key.clone(), entry);
        Ok(())
    }

    pub fn is_archived(&self, table: &str, year: i32, month: u32) -> bool {
        self.entries.values().any(|e| e.table == table && e.year == year && e.month == month)
    }

    pub fn archived_set(&self) -> BTreeSet<(String, i32, u32)> {
        self.entries.values().map(|e| (e.table.clone(), e.year, e.month)).collect()
    }

    /// Objects for `tenant` and `vault` covering `from..=to` (as `(year, month)`).
    pub fn find(&self, table: &str, tenant: &str, vault: Option<&str>, from: (i32, u32), to: (i32, u32)) -> Vec<&ManifestEntry> {
        self.entries
            .values()
            .filter(|e| e.table == table && e.tenant == tenant)
            .filter(|e| vault.is_none_or(|v| e.vault == v))
            .filter(|e| (e.year, e.month) >= from && (e.year, e.month) <= to)
            .collect()
    }

    /// Objects that must be rewritten without `subject` to honour an erasure request.
    pub fn gdpr_rewrite_tasks(&self, subject: &str) -> Vec<RewriteTask> {
        self.entries
            .values()
            .filter(|e| e.subjects.contains(subject))
            .map(|e| RewriteTask { key: e.key.clone(), remove_subject: subject.to_string() })
            .collect()
    }

    /// Call once an object has been rewritten with the subject removed.
    pub fn mark_rewritten(&mut self, key: &str, subject: &str, new_row_count: u64, new_checksum: &str) {
        if let Some(entry) = self.entries.get_mut(key) {
            entry.subjects.remove(subject);
            entry.row_count = new_row_count;
            entry.checksum = new_checksum.to_string();
        }
    }
}

// ---------------------------------------------------------------------------
// Retrieval API
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RetrievalStatus {
    Queued,
    Running,
    Ready,
    Failed,
    Expired,
}

#[derive(Debug, Clone, Serialize)]
pub struct RetrievalRequest {
    pub id: String,
    pub table: String,
    pub tenant: String,
    pub vault: Option<String>,
    pub from: (i32, u32),
    pub to: (i32, u32),
    pub requested_by: String,
    pub status: RetrievalStatus,
    pub result_url: Option<String>,
    pub expires_at: Option<u64>,
    pub error: Option<String>,
    pub requested_at: u64,
}

/// Asynchronous access to cold data: `POST /archive/retrievals` creates a
/// request and returns its id immediately; a worker fulfils it; the client
/// polls `GET /archive/retrievals/{id}` until it is `ready` and downloads from
/// the (expiring) URL.
#[derive(Default)]
pub struct RetrievalService {
    requests: BTreeMap<String, RetrievalRequest>,
    next_id: u64,
}

/// What a client asks for: a table's data for one tenant (optionally one vault)
/// over an inclusive `(year, month)` range.
#[derive(Debug, Clone)]
pub struct RetrievalQuery {
    pub table: String,
    pub tenant: String,
    pub vault: Option<String>,
    pub from: (i32, u32),
    pub to: (i32, u32),
}

impl RetrievalService {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn request(&mut self, query: RetrievalQuery, requested_by: &str, now: u64) -> Result<String, ArchivalError> {
        let RetrievalQuery { table, tenant, vault, from, to } = query;
        ident(&table)?;
        self.next_id += 1;
        let id = format!("ret_{}", self.next_id);
        self.requests.insert(
            id.clone(),
            RetrievalRequest {
                id: id.clone(),
                table,
                tenant,
                vault,
                from,
                to,
                requested_by: requested_by.to_string(),
                status: RetrievalStatus::Queued,
                result_url: None,
                expires_at: None,
                error: None,
                requested_at: now,
            },
        );
        Ok(id)
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut RetrievalRequest, ArchivalError> {
        self.requests.get_mut(id).ok_or_else(|| ArchivalError::UnknownRequest(id.to_string()))
    }

    pub fn start(&mut self, id: &str) -> Result<(), ArchivalError> {
        let request = self.get_mut(id)?;
        if request.status != RetrievalStatus::Queued {
            return Err(ArchivalError::WrongState("queued"));
        }
        request.status = RetrievalStatus::Running;
        Ok(())
    }

    pub fn complete(&mut self, id: &str, url: &str, now: u64) -> Result<(), ArchivalError> {
        let request = self.get_mut(id)?;
        if request.status != RetrievalStatus::Running {
            return Err(ArchivalError::WrongState("running"));
        }
        request.status = RetrievalStatus::Ready;
        request.result_url = Some(url.to_string());
        request.expires_at = Some(now + RETRIEVAL_TTL_SECS);
        Ok(())
    }

    pub fn fail(&mut self, id: &str, error: &str) -> Result<(), ArchivalError> {
        let request = self.get_mut(id)?;
        request.status = RetrievalStatus::Failed;
        request.error = Some(error.to_string());
        Ok(())
    }

    /// Status for the caller. A tenant can only see its own requests, and a
    /// `ready` request past its expiry is reported (and stored) as `expired`.
    pub fn status(&mut self, id: &str, tenant: &str, now: u64) -> Result<RetrievalRequest, ArchivalError> {
        let request = self.get_mut(id)?;
        if request.tenant != tenant {
            return Err(ArchivalError::TenantMismatch);
        }
        if request.status == RetrievalStatus::Ready && request.expires_at.is_some_and(|at| now >= at) {
            request.status = RetrievalStatus::Expired;
            request.result_url = None;
        }
        Ok(request.clone())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn date(y: i32, m: u32, d: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(y, m, d).unwrap()
    }

    #[test]
    fn records_move_from_hot_to_warm_to_cold() {
        let policy = RetentionPolicy::default();
        let today = date(2026, 10, 19);

        assert_eq!(policy.tier_for(date(2026, 10, 1), today), Tier::Hot);
        assert_eq!(policy.tier_for(date(2026, 7, 21), today), Tier::Hot, "exactly 90 days");
        assert_eq!(policy.tier_for(date(2026, 7, 20), today), Tier::Warm);
        assert_eq!(policy.tier_for(date(2025, 10, 5), today), Tier::Warm, "12 months back");
        assert_eq!(policy.tier_for(date(2025, 9, 30), today), Tier::Cold);
    }

    #[test]
    fn monthly_partitions_have_stable_names_and_range_bounds() {
        assert_eq!(partition_name("vault_events", 2026, 3).unwrap(), "vault_events_2026_03");
        assert_eq!(
            create_partition_sql("vault_events", 2026, 12).unwrap(),
            "CREATE TABLE IF NOT EXISTS vault_events_2026_12 PARTITION OF vault_events FOR VALUES FROM ('2026-12-01') TO ('2027-01-01');"
        );
        assert_eq!(partition_name("t", 2026, 13), Err(ArchivalError::InvalidMonth(13)));
        assert!(partition_name("t; DROP TABLE x", 2026, 1).is_err());
    }

    #[test]
    fn future_partitions_are_created_ahead_across_the_year_boundary() {
        let statements = ensure_partitions_sql("vault_events", date(2026, 11, 15), 2).unwrap();
        assert_eq!(statements.len(), 3);
        assert!(statements[0].contains("vault_events_2026_11"));
        assert!(statements[1].contains("vault_events_2026_12"));
        assert!(statements[2].contains("vault_events_2027_01") && statements[2].contains("'2027-01-01') TO ('2027-02-01')"));
    }

    #[test]
    fn object_keys_are_hive_partitioned_and_cannot_be_escaped() {
        assert_eq!(
            object_key("vault_events", "acme", "v-7", 2025, 4).unwrap(),
            "archive/vault_events/tenant=acme/vault=v-7/year=2025/month=04/part-0000.parquet"
        );
        let hostile = object_key("vault_events", "../other/tenant=x", "v=1", 2025, 4).unwrap();
        assert!(hostile.contains("tenant=.._other_tenant_x/vault=v_1/"), "{hostile}");
        assert!(object_key("vault_events", "t", "v", 2025, 0).is_err());
    }

    #[test]
    fn the_athena_table_uses_partition_projection_over_the_archive_prefix() {
        let ddl = athena_table_ddl("vault_events", &[("id", "string"), ("amount", "decimal(38,7)")], "perigee-archive").unwrap();
        assert!(ddl.contains("CREATE EXTERNAL TABLE IF NOT EXISTS vault_events_archive"));
        assert!(ddl.contains("PARTITIONED BY (tenant string, vault string, year int, month int)"));
        assert!(ddl.contains("STORED AS PARQUET"));
        assert!(ddl.contains("LOCATION 's3://perigee-archive/archive/vault_events/'"));
        assert!(ddl.contains("'storage.location.template' = 's3://perigee-archive/archive/vault_events/tenant=${tenant}/vault=${vault}/year=${year}/month=${month}/'"));
        assert!(athena_table_ddl("t", &[("a b", "string")], "b").is_err());
    }

    #[test]
    fn the_monthly_run_plans_only_partitions_past_the_warm_window_that_are_not_done() {
        let today = date(2026, 10, 19);
        let mut done = BTreeSet::new();
        done.insert(("vault_events".to_string(), 2025, 8));

        let tasks = plan_monthly_run(&["vault_events"], today, &RetentionPolicy::default(), &done, (2025, 7)).unwrap();

        // 2025-10 is 12 months back (still warm); 2025-07..=2025-09 are cold, 2025-08 already archived.
        let months: Vec<(i32, u32)> = tasks.iter().map(|t| (t.year, t.month)).collect();
        assert_eq!(months, vec![(2025, 7), (2025, 9)]);
        let first = &tasks[0];
        assert_eq!(first.partition, "vault_events_2025_07");
        assert_eq!(first.count_query, "SELECT COUNT(*) FROM vault_events_2025_07");
        assert_eq!(first.detach_sql, "ALTER TABLE vault_events DETACH PARTITION vault_events_2025_07;");
        assert_eq!(first.drop_sql, "DROP TABLE vault_events_2025_07;");
    }

    fn entry(key: &str, month: u32, subjects: &[&str]) -> ManifestEntry {
        ManifestEntry {
            key: key.into(),
            table: "vault_events".into(),
            tenant: "acme".into(),
            vault: "v1".into(),
            year: 2025,
            month,
            row_count: 10,
            checksum: "abc".into(),
            subjects: subjects.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn a_partition_is_only_recorded_when_the_export_matches_the_database() {
        let mut manifest = ArchiveManifest::default();

        assert!(matches!(manifest.record_verified(entry("k1", 7, &[]), 10, 9, "abc"), Err(ArchivalError::VerificationFailed(_, _))));
        assert!(matches!(manifest.record_verified(entry("k1", 7, &[]), 10, 10, "different"), Err(ArchivalError::VerificationFailed(_, _))));
        assert!(!manifest.is_archived("vault_events", 2025, 7), "nothing recorded, so the partition must not be dropped");

        manifest.record_verified(entry("k1", 7, &[]), 10, 10, "abc").unwrap();
        assert!(manifest.is_archived("vault_events", 2025, 7));
        assert!(manifest.archived_set().contains(&("vault_events".to_string(), 2025, 7)));
    }

    #[test]
    fn the_manifest_finds_objects_by_tenant_vault_and_period() {
        let mut manifest = ArchiveManifest::default();
        for month in 1..=6 {
            manifest.record_verified(entry(&format!("k{month}"), month, &[]), 10, 10, "abc").unwrap();
        }
        let mut other = entry("other", 3, &[]);
        other.tenant = "globex".into();
        manifest.record_verified(other, 10, 10, "abc").unwrap();

        let found = manifest.find("vault_events", "acme", Some("v1"), (2025, 2), (2025, 4));
        assert_eq!(found.iter().map(|e| e.month).collect::<Vec<_>>(), vec![2, 3, 4]);
        assert!(manifest.find("vault_events", "acme", Some("other-vault"), (2025, 1), (2025, 12)).is_empty());
        assert_eq!(manifest.find("vault_events", "globex", None, (2025, 1), (2025, 12)).len(), 1);
    }

    #[test]
    fn gdpr_erasure_finds_and_tracks_the_archived_objects_to_rewrite() {
        let mut manifest = ArchiveManifest::default();
        manifest.record_verified(entry("k1", 1, &["user-9", "user-2"]), 10, 10, "abc").unwrap();
        manifest.record_verified(entry("k2", 2, &["user-2"]), 10, 10, "abc").unwrap();

        let tasks = manifest.gdpr_rewrite_tasks("user-9");
        assert_eq!(tasks, vec![RewriteTask { key: "k1".into(), remove_subject: "user-9".into() }]);

        manifest.mark_rewritten("k1", "user-9", 8, "new-checksum");
        assert!(manifest.gdpr_rewrite_tasks("user-9").is_empty());
        assert_eq!(manifest.gdpr_rewrite_tasks("user-2").len(), 2, "other subjects untouched");
    }

    fn query(table: &str, tenant: &str, vault: Option<&str>, from: (i32, u32), to: (i32, u32)) -> RetrievalQuery {
        RetrievalQuery { table: table.into(), tenant: tenant.into(), vault: vault.map(str::to_string), from, to }
    }

    #[test]
    fn retrieval_requests_follow_queued_running_ready_and_expire() {
        let mut service = RetrievalService::new();
        let id = service.request(query("vault_events", "acme", Some("v1"), (2025, 1), (2025, 3)), "alice", 1_000).unwrap();
        assert_eq!(service.status(&id, "acme", 1_000).unwrap().status, RetrievalStatus::Queued);

        assert_eq!(service.complete(&id, "https://x", 1_000), Err(ArchivalError::WrongState("running")));
        service.start(&id).unwrap();
        assert_eq!(service.start(&id), Err(ArchivalError::WrongState("queued")));
        service.complete(&id, "https://bucket/signed", 1_100).unwrap();

        let ready = service.status(&id, "acme", 1_200).unwrap();
        assert_eq!(ready.status, RetrievalStatus::Ready);
        assert_eq!(ready.result_url.as_deref(), Some("https://bucket/signed"));
        assert_eq!(ready.expires_at, Some(1_100 + RETRIEVAL_TTL_SECS));

        let expired = service.status(&id, "acme", 1_100 + RETRIEVAL_TTL_SECS).unwrap();
        assert_eq!(expired.status, RetrievalStatus::Expired);
        assert_eq!(expired.result_url, None, "the link is withdrawn");
    }

    #[test]
    fn failures_are_reported_and_tenants_are_isolated() {
        let mut service = RetrievalService::new();
        let id = service.request(query("vault_events", "acme", None, (2025, 1), (2025, 1)), "alice", 0).unwrap();
        service.start(&id).unwrap();
        service.fail(&id, "Athena query timed out").unwrap();

        let failed = service.status(&id, "acme", 1).unwrap();
        assert_eq!((failed.status, failed.error.as_deref()), (RetrievalStatus::Failed, Some("Athena query timed out")));
        assert_eq!(service.status(&id, "globex", 1).unwrap_err(), ArchivalError::TenantMismatch);
        assert_eq!(service.status("nope", "acme", 1).unwrap_err(), ArchivalError::UnknownRequest("nope".into()));
        assert!(service.request(query("bad table", "acme", None, (2025, 1), (2025, 1)), "a", 0).is_err());
    }
}
