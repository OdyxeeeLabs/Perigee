pub mod migrations;
pub mod models;
pub mod schema;

use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use chrono::Utc;
use sqlx::postgres::{PgPool, PgPoolOptions};
use sqlx::sqlite::{SqliteConnectOptions, SqlitePoolOptions};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::db::schema::{
    ManagersTable, ReconciliationDiscrepanciesTable, ReconciliationReportsTable, TypedSchema,
    VaultsTable,
};

pub type Pool = SqlitePool;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DatabasePoolConfig {
    pub max_connections: u32,
    pub min_connections: u32,
    pub acquire_timeout: Duration,
    pub idle_timeout: Option<Duration>,
    pub max_lifetime: Option<Duration>,
    pub busy_timeout: Duration,
    pub health_check_timeout: Duration,
    pub health_check_interval: Duration,
}

pub type PoolConfig = DatabasePoolConfig;

impl Default for DatabasePoolConfig {
    fn default() -> Self {
        Self {
            max_connections: 10,
            min_connections: 0,
            acquire_timeout: Duration::from_secs(5),
            idle_timeout: Some(Duration::from_secs(300)),
            max_lifetime: Some(Duration::from_secs(1_800)),
            busy_timeout: Duration::from_secs(5),
            health_check_timeout: Duration::from_secs(2),
            health_check_interval: Duration::from_secs(30),
        }
    }
}

impl DatabasePoolConfig {
    pub fn new(
        max_connections: u32,
        min_connections: u32,
        acquire_timeout_secs: u64,
        idle_timeout_secs: u64,
        max_lifetime_secs: u64,
        busy_timeout_secs: u64,
        health_check_timeout_secs: u64,
        health_check_interval_secs: u64,
    ) -> Self {
        Self {
            max_connections,
            min_connections,
            acquire_timeout: Duration::from_secs(acquire_timeout_secs),
            idle_timeout: (idle_timeout_secs > 0).then_some(Duration::from_secs(idle_timeout_secs)),
            max_lifetime: (max_lifetime_secs > 0).then_some(Duration::from_secs(max_lifetime_secs)),
            busy_timeout: Duration::from_secs(busy_timeout_secs),
            health_check_timeout: Duration::from_secs(health_check_timeout_secs),
            health_check_interval: Duration::from_secs(health_check_interval_secs),
        }
        .normalized()
    }

    pub fn from_env() -> Self {
        Self::new(
            read_env_u32(&[
                "DATABASE_MAX_CONNECTIONS",
                "DATABASE_POOL_MAX_CONNECTIONS",
                "DB_MAX_CONNECTIONS",
                "DB_POOL_MAX_CONNECTIONS",
            ], 10),
            read_env_u32(&[
                "DATABASE_MIN_CONNECTIONS",
                "DATABASE_POOL_MIN_CONNECTIONS",
                "DB_MIN_CONNECTIONS",
                "DB_POOL_MIN_CONNECTIONS",
            ], 0),
            read_env_u64(&[
                "DATABASE_ACQUIRE_TIMEOUT_SECS",
                "DATABASE_POOL_ACQUIRE_TIMEOUT_SECS",
                "DB_ACQUIRE_TIMEOUT_SECS",
                "DB_POOL_ACQUIRE_TIMEOUT_SECS",
            ], 5),
            read_env_u64(&[
                "DATABASE_IDLE_TIMEOUT_SECS",
                "DATABASE_POOL_IDLE_TIMEOUT_SECS",
                "DB_IDLE_TIMEOUT_SECS",
                "DB_POOL_IDLE_TIMEOUT_SECS",
            ], 300),
            read_env_u64(&[
                "DATABASE_MAX_LIFETIME_SECS",
                "DATABASE_POOL_MAX_LIFETIME_SECS",
                "DB_MAX_LIFETIME_SECS",
                "DB_POOL_MAX_LIFETIME_SECS",
            ], 1_800),
            read_env_u64(&[
                "DATABASE_BUSY_TIMEOUT_SECS",
                "DATABASE_POOL_BUSY_TIMEOUT_SECS",
                "DB_BUSY_TIMEOUT_SECS",
                "DB_POOL_BUSY_TIMEOUT_SECS",
            ], 5),
            read_env_u64(&[
                "DATABASE_HEALTH_CHECK_TIMEOUT_SECS",
                "DATABASE_POOL_HEALTH_CHECK_TIMEOUT_SECS",
                "DB_HEALTH_CHECK_TIMEOUT_SECS",
                "DB_POOL_HEALTH_CHECK_TIMEOUT_SECS",
            ], 2),
            read_env_u64(&[
                "DATABASE_HEALTH_CHECK_INTERVAL_SECS",
                "DATABASE_POOL_HEALTH_CHECK_INTERVAL_SECS",
                "DB_HEALTH_CHECK_INTERVAL_SECS",
                "DB_POOL_HEALTH_CHECK_INTERVAL_SECS",
            ], 30),
        )
    }

    pub fn normalized(mut self) -> Self {
        self.max_connections = self.max_connections.max(1);
        self.min_connections = self.min_connections.min(self.max_connections);
        self.acquire_timeout = self.acquire_timeout.max(Duration::from_millis(1));
        self.busy_timeout = self.busy_timeout.max(Duration::from_millis(1));
        self.health_check_timeout = self.health_check_timeout.max(Duration::from_millis(1));
        self.health_check_interval = self.health_check_interval.max(Duration::from_millis(1));
        self
    }

    pub fn sqlite_options(&self) -> SqlitePoolOptions {
        let config = self.clone().normalized();
        SqlitePoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.acquire_timeout)
            .idle_timeout(config.idle_timeout)
            .max_lifetime(config.max_lifetime)
            .test_before_acquire(true)
    }

    pub fn postgres_options(&self) -> PgPoolOptions {
        let config = self.clone().normalized();
        PgPoolOptions::new()
            .max_connections(config.max_connections)
            .min_connections(config.min_connections)
            .acquire_timeout(config.acquire_timeout)
            .idle_timeout(config.idle_timeout)
            .max_lifetime(config.max_lifetime)
            .test_before_acquire(true)
    }
}

fn read_env_u32(names: &[&str], default: u32) -> u32 {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

fn read_env_u64(names: &[&str], default: u64) -> u64 {
    names
        .iter()
        .find_map(|name| std::env::var(name).ok())
        .and_then(|value| value.trim().parse().ok())
        .unwrap_or(default)
}

pub async fn connect_sqlite_pool(
    database_url: &str,
    config: &DatabasePoolConfig,
) -> Result<Pool, sqlx::Error> {
    let config = config.clone().normalized();
    let connect_options = SqliteConnectOptions::from_str(database_url)?
        .create_if_missing(true)
        .busy_timeout(config.busy_timeout);
    config.sqlite_options().connect_with(connect_options).await
}

pub async fn connect_postgres_pool(
    database_url: &str,
    config: &DatabasePoolConfig,
) -> Result<PgPool, sqlx::Error> {
    config.postgres_options().connect(database_url).await
}

pub async fn init_pool_with_config(
    database_url: &str,
    config: &DatabasePoolConfig,
) -> Result<Pool, sqlx::Error> {
    let pool = connect_sqlite_pool(database_url, config).await?;
    if let Err(error) = migrations::run_migrations(&pool).await {
        pool.close().await;
        return Err(error);
    }
    Ok(pool)
}

pub async fn init_pool(database_url: &str) -> Result<Pool, sqlx::Error> {
    init_pool_with_config(database_url, &DatabasePoolConfig::from_env()).await
}

pub async fn health_check(pool: &Pool, timeout: Duration) -> bool {
    if pool.is_closed() {
        return false;
    }
    matches!(
        tokio::time::timeout(timeout, sqlx::query("SELECT 1").execute(pool)).await,
        Ok(Ok(_))
    )
}

#[derive(Clone)]
pub struct DatabaseHealth {
    healthy: Arc<AtomicBool>,
}

impl Default for DatabaseHealth {
    fn default() -> Self {
        Self {
            healthy: Arc::new(AtomicBool::new(true)),
        }
    }
}

impl DatabaseHealth {
    pub fn is_healthy(&self) -> bool {
        self.healthy.load(Ordering::Acquire)
    }

    pub fn set_healthy(&self, healthy: bool) {
        self.healthy.store(healthy, Ordering::Release);
    }
}

pub fn spawn_health_checker(
    pool: Pool,
    interval: Duration,
    timeout: Duration,
) -> (DatabaseHealth, tokio::task::JoinHandle<()>) {
    let state = DatabaseHealth::default();
    let task_state = state.clone();
    let task_pool = pool.clone();
    let interval = interval.max(Duration::from_millis(1));
    let timeout = timeout.max(Duration::from_millis(1));
    let handle = tokio::spawn(async move {
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            task_state.set_healthy(health_check(&task_pool, timeout).await);
        }
    });
    (state, handle)
}

pub fn make_typed_schema(pool: Arc<SqlitePool>) -> TypedSchema {
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
            self.table
                .create_idempotent(
                    &id,
                    manager_id,
                    name,
                    status,
                    config_json,
                    key,
                    now,
                )
                .await

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
            let mut tx = self.report_table.pool().begin().await?;

            if let Err(error) = self
                .report_table
                .insert_in_transaction(
                    &mut tx,
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
                )
                .await
            {
                let _ = tx.rollback().await;
                return Err(error);
            }

            if let Err(error) = self
                .disc_table
                .insert_for_report_in_transaction(&mut tx, &report.id, discrepancies)
                .await
            {
                let _ = tx.rollback().await;
                return Err(error);
            }

            if let Err(error) = tx.commit().await {
                return Err(error);
            }
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn pool_limits_and_timeouts_are_applied() {
        let config = DatabasePoolConfig::new(1, 4, 0, 0, 0, 0, 0, 0);
        let options = config.sqlite_options();

        assert_eq!(options.get_max_connections(), 1);
        assert_eq!(options.get_min_connections(), 1);
        assert_eq!(options.get_acquire_timeout(), Duration::from_millis(1));
        assert_eq!(options.get_idle_timeout(), None);
        assert_eq!(options.get_max_lifetime(), None);
        assert_eq!(config.health_check_interval, Duration::from_millis(1));
    }

    #[tokio::test]
    async fn exhausted_pool_does_not_wait_past_acquire_timeout() {
        let config = DatabasePoolConfig::new(1, 0, 0, 0, 0, 1, 50, 1_000);
        let pool = connect_sqlite_pool("sqlite::memory:", &config)
            .await
            .unwrap();
        let first = pool.acquire().await.unwrap();
        let exhausted = match tokio::time::timeout(
            Duration::from_millis(100),
            pool.acquire(),
        )
        .await
        {
            Ok(result) => result.is_err(),
            Err(_) => true,
        };

        assert!(exhausted);
        drop(first);
        pool.close().await;
    }
}
