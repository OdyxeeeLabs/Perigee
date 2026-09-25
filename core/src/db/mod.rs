pub mod migrations;
pub mod models;
pub mod schema;

use crate::db::schema::{
    ManagersTable, ReconciliationDiscrepanciesTable, ReconciliationReportsTable, TypedSchema,
    VaultsTable,
};
use chrono::Utc;
use sqlx::pool::PoolConnection;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{Sqlite, SqlitePool};
use std::fmt;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Clone, Copy, Error, PartialEq, Eq)]
pub enum PoolConfigError {
    #[error("database pool max connections must be greater than zero")]
    InvalidMaxConnections,
    #[error("database pool min connections must not exceed max connections")]
    InvalidMinConnections,
    #[error("database pool acquire timeout must be greater than zero")]
    InvalidAcquireTimeout,
}

#[derive(Debug, Clone, Copy)]
pub struct PoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: Duration,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 10,
            min_connections: 1,
            acquire_timeout: Duration::from_secs(30),
        }
    }
}

impl PoolConfig {
    pub fn new(
        max_connections: u32,
        min_connections: u32,
        acquire_timeout: Duration,
    ) -> Result<Self, PoolConfigError> {
        if max_connections == 0 {
            return Err(PoolConfigError::InvalidMaxConnections);
        }
        if min_connections > max_connections {
            return Err(PoolConfigError::InvalidMinConnections);
        }
        if acquire_timeout.is_zero() {
            return Err(PoolConfigError::InvalidAcquireTimeout);
        }
        Ok(Self {
            max_connections,
            min_connections,
            acquire_timeout,
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PoolSnapshot {
    pub size: u32,
    pub active: u32,
    pub idle: u32,
    pub waiting: usize,
    pub max_connections: u32,
}

impl PoolSnapshot {
    pub fn utilization_percent(&self) -> f64 {
        if self.max_connections == 0 {
            return 0.0;
        }
        (self.active as f64 / self.max_connections as f64) * 100.0
    }
}

struct WaitingGuard {
    waiting: Arc<AtomicUsize>,
}

impl Drop for WaitingGuard {
    fn drop(&mut self) {
        self.waiting.fetch_sub(1, Ordering::AcqRel);
    }
}

pub struct MonitoredPool {
    inner: SqlitePool,
    waiting: Arc<AtomicUsize>,
}

impl Clone for MonitoredPool {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            waiting: Arc::clone(&self.waiting),
        }
    }
}

impl fmt::Debug for MonitoredPool {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MonitoredPool")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}

impl MonitoredPool {
    pub async fn connect(
        database_url: &str,
        config: PoolConfig,
    ) -> Result<Self, sqlx::Error> {
        let inner = SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.acquire_timeout)
            .connect(database_url)
            .await?;
        Ok(Self::from_inner(inner))
    }

    pub fn from_inner(inner: SqlitePool) -> Self {
        Self {
            inner,
            waiting: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub(crate) fn inner(&self) -> &SqlitePool {
        &self.inner
    }

    pub async fn acquire(&self) -> Result<PoolConnection<Sqlite>, sqlx::Error> {
        match self.inner.try_acquire() {
            Ok(connection) => return Ok(connection),
            Err(_) => {
                self.waiting.fetch_add(1, Ordering::AcqRel);
                let _guard = WaitingGuard {
                    waiting: Arc::clone(&self.waiting),
                };
                self.inner.acquire().await
            }
        }
    }

    pub fn snapshot(&self) -> PoolSnapshot {
        let size = self.inner.size();
        let idle = self.inner.num_idle().min(u32::MAX as usize) as u32;
        PoolSnapshot {
            size,
            active: size.saturating_sub(idle),
            idle,
            waiting: self.waiting.load(Ordering::Acquire),
            max_connections: self.inner.options().get_max_connections(),
        }
    }

    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
}

pub type Pool = MonitoredPool;

pub async fn init_pool(database_url: &str) -> Result<Pool, sqlx::Error> {
    init_pool_with_config(database_url, PoolConfig::default()).await
}

pub async fn init_pool_with_config(
    database_url: &str,
    config: PoolConfig,
) -> Result<Pool, sqlx::Error> {
    let pool = MonitoredPool::connect(database_url, config).await?;
    migrations::run_migrations(pool.inner()).await?;
    Ok(pool)
}

pub fn make_typed_schema(pool: Arc<Pool>) -> TypedSchema {
    TypedSchema::new(pool)
}

pub mod manager {
    use super::*;
    use crate::db::models;

    pub struct ManagerRepo {
        table: ManagersTable,
    }

    impl ManagerRepo {
        pub fn new(table: ManagersTable) -> Self {
            Self { table }
        }

        pub async fn register(
            &self,
            stellar_address: &str,
            name: &str,
            email: &str,
            kyc_document_ref: &str,
        ) -> Result<models::ManagerRecord, sqlx::Error> {
            let id = Uuid::new_v4().to_string();
            let now = Utc::now();
            self.table.insert(&id, stellar_address, name, email, kyc_document_ref, now).await
        }

        pub async fn get(&self, id: &str) -> Result<Option<models::ManagerRecord>, sqlx::Error> {
            self.table.find_by_id(id).await
        }

        pub async fn find_by_stellar_address(
            &self,
            address: &str,
        ) -> Result<Option<models::ManagerRecord>, sqlx::Error> {
            self.table.find_by_stellar_address(address).await
        }

        pub async fn list(
            &self,
            status_filter: Option<&str>,
        ) -> Result<Vec<models::ManagerRecord>, sqlx::Error> {
            self.table.list(status_filter).await
        }

        pub async fn approve(
            &self,
            id: &str,
            notes: &str,
        ) -> Result<models::ManagerRecord, sqlx::Error> {
            self.table.update_status(id, "approved", notes, Utc::now()).await
        }

        pub async fn reject(
            &self,
            id: &str,
            notes: &str,
        ) -> Result<models::ManagerRecord, sqlx::Error> {
            self.table.update_status(id, "rejected", notes, Utc::now()).await
        }
    }
}

pub mod vault {
    use super::*;
    use crate::db::models;

    pub struct VaultRepo {
        table: VaultsTable,
    }

    impl VaultRepo {
        pub fn new(table: VaultsTable) -> Self {
            Self { table }
        }

        pub async fn create(
            &self,
            manager_id: &str,
            name: &str,
            status: &str,
            config_json: &str,
            idempotency_key: Option<&str>,
        ) -> Result<models::VaultRecord, sqlx::Error> {
            let id = Uuid::new_v4().to_string();
            let now = Utc::now();
            let key = idempotency_key.map(str::trim).filter(|s| !s.is_empty());
            if let Some(k) = key {
                if let Some(existing) = self.table.find_by_idempotency_key(manager_id, k).await? {
                    return Ok(existing);
                }
            }
            self.table.insert(&id, manager_id, name, status, config_json, key, now).await
        }

        pub async fn get(&self, id: &str) -> Result<Option<models::VaultRecord>, sqlx::Error> {
            self.table.find_by_id(id).await
        }

        pub async fn update(
            &self,
            id: &str,
            expected_version: i64,
            name: Option<&str>,
            status: Option<&str>,
            config_json: Option<&str>,
        ) -> Result<models::VaultRecord, sqlx::Error> {
            self.table.update(id, expected_version, name, status, config_json, Utc::now()).await
        }
    }
}

pub mod reconciliation {
    use super::*;
    use crate::db::models;
    use redis::{AsyncCommands, Client as RedisClient};

    #[derive(Clone)]
    pub struct ReconciliationRepo {
        report_table: ReconciliationReportsTable,
        disc_table: ReconciliationDiscrepanciesTable,
        redis: Option<RedisClient>,
    }

    impl ReconciliationRepo {
        /// Borrow the underlying pool.
        ///
        /// The readiness probe issues a bare `SELECT 1` to prove the database
        /// is reachable, and needs a pool rather than a typed query to do it.
        pub fn pool(&self) -> &schema::DbPool {
            self.report_table.pool()
        }

        pub fn new(
            report_table: ReconciliationReportsTable,
            disc_table: ReconciliationDiscrepanciesTable,
        ) -> Self {
            Self {
                report_table,
                disc_table,
                redis: None,
            }
        }

        pub fn with_redis(
            report_table: ReconciliationReportsTable,
            disc_table: ReconciliationDiscrepanciesTable,
            redis_url: &str,
        ) -> Result<Self, redis::RedisError> {
            Ok(Self {
                report_table,
                disc_table,
                redis: Some(RedisClient::open(redis_url)?),
            })
        }

        async fn cached_report(&self, id: &str) -> Option<Option<models::ReconciliationReport>> {
            let client = self.redis.as_ref()?;
            let mut connection = client.get_multiplexed_async_connection().await.ok()?;
            let value: Option<String> = connection.get(Self::report_cache_key(id)).await.ok()?;
            value.map(|json| serde_json::from_str(&json).ok())
        }

        async fn cache_report(&self, report: &models::ReconciliationReport) {
            let Some(client) = self.redis.as_ref() else {
                return;
            };
            let Ok(json) = serde_json::to_string(report) else {
                return;
            };
            let Ok(mut connection) = client.get_multiplexed_async_connection().await else {
                return;
            };
            let _: Result<(), _> = connection
                .set_ex(Self::report_cache_key(&report.id), json, 300)
                .await;
        }

        async fn report_list_version(&self) -> Option<String> {
            let client = self.redis.as_ref()?;
            let mut connection = client.get_multiplexed_async_connection().await.ok()?;
            connection
                .get(Self::report_list_version_key())
                .await
                .ok()
        }

        async fn cached_reports(&self, limit: i64) -> Option<Vec<models::ReconciliationReport>> {
            let version = self.report_list_version().await;
            let client = self.redis.as_ref()?;
            let mut connection = client.get_multiplexed_async_connection().await.ok()?;
            let key = Self::report_list_cache_key(limit, version.as_deref().unwrap_or("0"));
            let json: Option<String> = connection.get(key).await.ok()?;
            serde_json::from_str(&json?).ok()
        }

        async fn cache_reports(
            &self,
            limit: i64,
            version: Option<&str>,
            reports: &[models::ReconciliationReport],
        ) {
            let Some(client) = self.redis.as_ref() else {
                return;
            };
            let current_version = self
                .report_list_version()
                .await
                .unwrap_or_else(|| "0".to_string());
            if version.unwrap_or("0") != current_version {
                return;
            }
            let Ok(mut connection) = client.get_multiplexed_async_connection().await else {
                return;
            };
            let key = Self::report_list_cache_key(limit, &current_version);
            let Ok(json) = serde_json::to_string(reports) else {
                return;
            };
            let _: Result<(), _> = connection.set_ex(key, json, 300).await;
        }

        fn report_cache_key(id: &str) -> String {
            format!("perigee:reconciliation:report:{id}")
        }

        fn report_list_version_key() -> &'static str {
            "perigee:reconciliation:reports:version"
        }

        fn report_list_cache_key(limit: i64, version: &str) -> String {
            format!("perigee:reconciliation:reports:{version}:{limit}")
        }

        async fn invalidate_report_cache(&self, id: &str) {
            let Some(client) = self.redis.as_ref() else {
                return;
            };
            let Ok(mut connection) = client.get_multiplexed_async_connection().await else {
                return;
            };
            let _: Result<(), _> = connection.del(Self::report_cache_key(id)).await;
            let _: Result<i64, _> = connection.incr(Self::report_list_version_key(), 1).await;
        }

        pub async fn persist_report(
            &self,
            report: &models::ReconciliationReport,
            discrepancies: &[models::Discrepancy],
        ) -> Result<(), sqlx::Error> {
            let summary_json = report.summary.as_ref().map(|s| serde_json::to_value(s).unwrap_or_default());

            self.report_table.insert(
                &report.id,
                report.from_ledger,
                report.to_ledger,
                report.tolerance_pct,
                report.total_ledgers,
                report.discrepancies_count,
                report.avg_delta_pct,
                report.max_delta_pct,
                summary_json.as_ref(),
                &report.created_at,
            ).await?;

            self.disc_table.insert_for_report(&report.id, discrepancies).await?;
            self.invalidate_report_cache(&report.id).await;

            Ok(())
        }

        pub async fn find_by_id(&self, id: &str) -> Result<Option<models::ReconciliationReport>, sqlx::Error> {
            if let Some(cached) = self.cached_report(id).await {
                return Ok(cached);
            }

            let report = self.report_table.find_by_id(id).await?;
            if let Some(ref report) = report {
                self.cache_report(report).await;
            }
            Ok(report)
        }

        pub async fn list(&self, limit: i64) -> Result<Vec<models::ReconciliationReport>, sqlx::Error> {
            if let Some(reports) = self.cached_reports(limit).await {
                return Ok(reports);
            }

            let version = self.report_list_version().await;
            let reports = self.report_table.list(limit).await?;
            self.cache_reports(limit, version.as_deref(), &reports).await;
            Ok(reports)
        }

        pub async fn find_discrepancies(
            &self,
            report_id: &str,
        ) -> Result<Vec<models::Discrepancy>, sqlx::Error> {
            self.disc_table.find_by_report_id(report_id).await
        }
    }
}