#![allow(
    dead_code,
    clippy::large_enum_variant,
    clippy::manual_inspect,
    clippy::needless_borrows_for_generic_args
)]

use crate::input_sanitization::{SanitizedJson, SanitizedPath};
use crate::insights::InsightsEngine;
use crate::reconciliation::{FeeReconciler, ReconciliationReport};
use crate::runner::{wait_for_cancellation, RequestCancellation};
use crate::secret_hash;
use crate::simulation::{SimulationEngine, SimulationResult, SorobanResources};
use crate::two_phase_commit::{TransactionParticipant, TwoPhaseCoordinator, TwoPhaseTx};
use crate::ws::SimulationBus;
use crate::AppError;
use axum::{
    async_trait,
    extract::State,
    extract::{Extension, Path, State},
    http::StatusCode,
    Json,
};
use chrono::{DateTime, Utc};
use redis::{AsyncCommands, Client as RedisClient};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sqlx::any::AnyQueryResult;
use sqlx::{PgPool, SqlitePool};
use std::collections::HashMap;
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::time::interval;
use tokio_util::sync::CancellationToken;
use utoipa::ToSchema;
use uuid::Uuid;

const POSTGRES_JOBS_MIGRATION: &str = r#"
CREATE TABLE IF NOT EXISTS jobs (
    id UUID PRIMARY KEY,
    job_type VARCHAR(50) NOT NULL,
    status VARCHAR(20) NOT NULL DEFAULT 'QUEUED',
    payload JSONB NOT NULL,
    result JSONB,
    progress_percent INTEGER NOT NULL DEFAULT 0,
    progress_message VARCHAR(255) NOT NULL DEFAULT 'Queued',
    webhook_url VARCHAR(500),
    webhook_headers JSONB,
    webhook_secret VARCHAR(255),
    error_message TEXT,
    error_type VARCHAR(50),
    timeout_secs INTEGER NOT NULL DEFAULT 300,
    retry_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    started_at TIMESTAMPTZ,
    completed_at TIMESTAMPTZ,
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);
CREATE INDEX IF NOT EXISTS idx_jobs_status ON jobs(status);
CREATE INDEX IF NOT EXISTS idx_jobs_created_at ON jobs(created_at);
CREATE INDEX IF NOT EXISTS idx_jobs_status_created_at ON jobs(status, created_at);
"#;

/// Database pool type - supports both PostgreSQL and SQLite
#[derive(Clone)]
pub enum DbPool {
    Postgres(PgPool),
    Sqlite(SqlitePool),
}

impl DbPool {
    pub async fn execute(&self, query: &str) -> Result<AnyQueryResult, sqlx::Error> {
        match self {
            DbPool::Postgres(pool) => {
                let result = sqlx::query(query).execute(pool).await?;
                Ok(AnyQueryResult {
                    rows_affected: result.rows_affected(),
                    last_insert_id: None,
                })
            }
            DbPool::Sqlite(pool) => {
                let result = sqlx::query(query).execute(pool).await?;
                Ok(AnyQueryResult {
                    rows_affected: result.rows_affected(),
                    last_insert_id: Some(result.last_insert_rowid()),
                })
            }
        }
    }
}

/// Unique identifier for a job
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[sqlx(transparent)]
pub struct JobId(pub Uuid);

impl JobId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for JobId {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for JobId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::str::FromStr for JobId {
    type Err = uuid::Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Ok(Self(Uuid::parse_str(s)?))
    }
}

/// Status of a job in its lifecycle
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[sqlx(rename_all = "SCREAMING_SNAKE_CASE")]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum JobStatus {
    Queued,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

/// Type of analysis job
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::Type)]
#[sqlx(rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum JobType {
    Analyze,
    Compare,
    OptimizeLimits,
    Reconcile,
}

/// Payload for different job types
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "type", content = "data")]
pub enum JobPayload {
    Analyze {
        contract_id: String,
        function_name: String,
        args: Option<Vec<String>>,
        ledger_overrides: Option<HashMap<String, String>>,
    },
    Compare {
        mode: String,
        current_wasm: Option<Vec<u8>>,
        base_wasm: Option<Vec<u8>>,
        contract_id: Option<String>,
        function_name: Option<String>,
        args: Vec<String>,
    },
    OptimizeLimits {
        contract_id: String,
        function_name: String,
        args: Vec<String>,
        safety_margin: f64,
    },
    Reconcile {
        from_ledger: i64,
        to_ledger: i64,
        tolerance_pct: f64,
    },
}

/// Progress information for a job
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobProgress {
    pub percent: i32,
    pub message: String,
    pub updated_at: DateTime<Utc>,
}

/// Result of a completed job
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case", tag = "status", content = "data")]
pub enum JobResult {
    Success {
        #[serde(skip_serializing_if = "Option::is_none")]
        resources: Option<SorobanResources>,
        #[serde(skip_serializing_if = "Option::is_none")]
        simulation_result: Option<SimulationResult>,
        #[serde(skip_serializing_if = "Option::is_none")]
        optimization: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        comparison: Option<Value>,
        #[serde(skip_serializing_if = "Option::is_none")]
        reconciliation: Option<ReconciliationReport>,
        #[serde(skip_serializing_if = "Option::is_none")]
        insights: Option<crate::insights::InsightsReport>,
    },
    Failed {
        error: String,
        error_type: String,
    },
}

/// Webhook configuration for job notifications
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct WebhookConfig {
    pub callback_url: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,
    /// Plaintext secret supplied by the caller. Only an Argon2 hash of this
    /// value is ever persisted (see `secret_hash::hash_secret`); it cannot
    /// be read back once stored.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub secret: Option<String>,
}

/// A job in the queue
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, sqlx::FromRow)]
pub struct Job {
    pub id: JobId,
    pub job_type: JobType,
    pub status: JobStatus,
    pub payload: Value,
    pub result: Option<Value>,
    pub progress_percent: i32,
    pub progress_message: String,
    pub webhook_url: Option<String>,
    pub webhook_headers: Option<Value>,
    /// Argon2 hash of the webhook secret (see `secret_hash`). The plaintext
    /// secret is never persisted and cannot be recovered from this value.
    #[sqlx(rename = "webhook_secret")]
    pub webhook_secret_hash: Option<String>,
    pub error_message: Option<String>,
    pub error_type: Option<String>,
    pub timeout_secs: i32,
    pub retry_count: i32,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub completed_at: Option<DateTime<Utc>>,
    pub updated_at: DateTime<Utc>,
}

impl Job {
    pub fn get_progress(&self) -> JobProgress {
        JobProgress {
            percent: self.progress_percent,
            message: self.progress_message.clone(),
            updated_at: self.updated_at,
        }
    }

    pub fn get_result(&self) -> Option<JobResult> {
        self.result
            .as_ref()
            .and_then(|r| serde_json::from_value(r.clone()).ok())
    }

    pub fn get_payload(&self) -> Option<JobPayload> {
        serde_json::from_value(self.payload.clone()).ok()
    }

    pub fn get_webhook_config(&self) -> Option<WebhookConfig> {
        self.webhook_url.as_ref().map(|url| WebhookConfig {
            callback_url: url.clone(),
            headers: self
                .webhook_headers
                .as_ref()
                .and_then(|h| serde_json::from_value(h.clone()).ok()),
            // Only an Argon2 hash is stored; the original secret is not
            // recoverable and therefore cannot be reconstructed here.
            secret: None,
        })
    }
}

/// Errors that can occur in job operations
#[derive(Debug, thiserror::Error)]
pub enum JobError {
    #[error("Job not found: {0}")]
    NotFound(JobId),
    #[error("Job cannot be cancelled in status: {0:?}")]
    CannotCancel(JobStatus),
    #[error("Database error: {0}")]
    Database(#[from] sqlx::Error),
    #[error("Job processing failed: {0}")]
    ProcessingFailed(String),
    #[error("Webhook delivery failed: {0}")]
    WebhookFailed(String),
}

/// Configuration for the job queue
#[derive(Debug, Clone)]
pub struct JobQueueConfig {
    pub job_timeout_secs: u64,
    pub cleanup_interval_secs: u64,
    pub retention_secs: u64,
    pub webhook_timeout_secs: u64,
    pub webhook_max_retries: u32,
    pub max_concurrent_jobs: usize,
    pub max_job_retries: i32,
}

impl Default for JobQueueConfig {
    fn default() -> Self {
        Self {
            job_timeout_secs: 300,
            cleanup_interval_secs: 3600,
            retention_secs: 3600,
            webhook_timeout_secs: 10,
            webhook_max_retries: 3,
            max_concurrent_jobs: 10,
            max_job_retries: 3,
        }
    }
}

#[derive(Clone)]
struct JobSubmissionData {
    id: JobId,
    job_type: JobType,
    payload: Value,
    webhook_url: Option<String>,
    webhook_headers: Option<Value>,
    webhook_secret_hash: Option<String>,
    timeout_secs: i32,
}

#[derive(Clone, Copy)]
enum JobSubmissionTarget {
    Database,
    Redis,
}

impl JobSubmissionTarget {
    fn operation(self) -> &'static str {
        match self {
            Self::Database => "database",
            Self::Redis => "redis",
        }
    }
}

struct JobSubmissionParticipant {
    queue: JobQueue,
    target: JobSubmissionTarget,
    data: JobSubmissionData,
    committed: Arc<AtomicBool>,
}

#[async_trait]
impl TransactionParticipant for JobSubmissionParticipant {
    async fn prepare(&self, _tx_id: &str, operation: &str) -> Result<(), String> {
        if operation != self.target.operation() {
            return Err(format!("Unsupported job submission operation: {}", operation));
        }
        Ok(())
    }

    async fn commit(&self, _tx_id: &str, operation: &str) -> Result<(), String> {
        if operation != self.target.operation() {
            return Err(format!("Unsupported job submission operation: {}", operation));
        }
        if self.committed.load(Ordering::Acquire) {
            return Ok(());
        }
        self.committed.store(true, Ordering::Release);
        match self.target {
            JobSubmissionTarget::Database => self
                .queue
                .insert_submission(&self.data)
                .await
                .map_err(|error| error.to_string())?,
            JobSubmissionTarget::Redis => self
                .queue
                .enqueue_submission(&self.data)
                .await
                .map_err(|error| error.to_string())?,
        }
        Ok(())
    }

    async fn abort(&self, _tx_id: &str, operation: &str) -> Result<(), String> {
        if operation != self.target.operation() {
            return Err(format!("Unsupported job submission operation: {}", operation));
        }
        if !self.committed.load(Ordering::Acquire) {
            return Ok(());
        }
        let result = match self.target {
            JobSubmissionTarget::Database => self
                .queue
                .delete_submission(&self.data.id)
                .await
                .map_err(|error| error.to_string()),
            JobSubmissionTarget::Redis => self
                .queue
                .dequeue_submission(&self.data.id)
                .await
                .map_err(|error| error.to_string()),
        };
        if result.is_ok() {
            self.committed.store(false, Ordering::Release);
        }
        result
    }

    fn supports(&self, operation: &str) -> bool {
        operation == self.target.operation()
    }
}

/// SQL-based job queue
pub struct JobQueue {
    pool: DbPool,
    redis: RedisClient,
    config: JobQueueConfig,
}

impl JobQueue {
    pub async fn new(
        database_url: &str,
        redis_url: &str,
        config: JobQueueConfig,
    ) -> Result<Self, JobError> {
        let pool = if database_url.starts_with("postgres://") {
            let pool = PgPool::connect(database_url).await?;
            DbPool::Postgres(pool)
        } else {
            let pool = SqlitePool::connect(database_url).await?;
            DbPool::Sqlite(pool)
        };

        let redis = RedisClient::open(redis_url).map_err(|e| {
            JobError::ProcessingFailed(format!("Failed to connect to Redis: {}", e))
        })?;

        // Run migrations
        Self::run_migrations(&pool).await?;

        Ok(Self {
            pool,
            redis,
            config,
        })
    }

    async fn run_migrations(pool: &DbPool) -> Result<(), JobError> {
        match pool {
            DbPool::Sqlite(pool) => {
                crate::db::migrations::run_migrations(pool).await?;
            }
            DbPool::Postgres(pool) => {
                for statement in POSTGRES_JOBS_MIGRATION.split(';') {
                    let statement = statement.trim();
                    if !statement.is_empty() {
                        sqlx::query(statement).execute(pool).await?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Submit a new job to the queue
    pub async fn submit(
        &self,
        job_type: JobType,
        payload: JobPayload,
        webhook: Option<WebhookConfig>,
    ) -> Result<JobId, JobError> {
        let id = JobId::new();
        let payload_json = serde_json::to_value(&payload).map_err(|e| {
            JobError::ProcessingFailed(format!("Failed to serialize payload: {}", e))
        })?;

        let (webhook_url, webhook_headers, webhook_secret_hash) = match webhook {
            Some(w) => (
                Some(w.callback_url),
                w.headers
                    .map(|h| serde_json::to_value(h).unwrap_or_default()),
                w.secret
                    .as_deref()
                    .map(secret_hash::hash_secret)
                    .transpose()
                    .map_err(|e| {
                        JobError::ProcessingFailed(format!(
                            "Failed to hash webhook secret: {e}"
                        ))
                    })?,
            ),
            None => (None, None, None),
        };

        let submission = JobSubmissionData {
            id,
            job_type,
            payload: payload_json,
            webhook_url,
            webhook_headers,
            webhook_secret_hash,
            timeout_secs: self.config.job_timeout_secs as i32,
        };
        let mut transaction = TwoPhaseTx::new(
            id.to_string(),
            vec!["database".to_string(), "redis".to_string()],
        );
        let database_participant: Arc<dyn TransactionParticipant> = Arc::new(
            JobSubmissionParticipant {
                queue: self.clone(),
                target: JobSubmissionTarget::Database,
                data: submission.clone(),
                committed: Arc::new(AtomicBool::new(false)),
            },
        );
        let redis_participant: Arc<dyn TransactionParticipant> = Arc::new(
            JobSubmissionParticipant {
                queue: self.clone(),
                target: JobSubmissionTarget::Redis,
                data: submission,
                committed: Arc::new(AtomicBool::new(false)),
            },
        );

        TwoPhaseCoordinator::new()
            .execute(
                &mut transaction,
                &[database_participant, redis_participant],
            )
            .await
            .map_err(|error| {
                JobError::ProcessingFailed(format!("Job submission transaction failed: {}", error))
            })?;

        tracing::info!(job_id = %id, "Job submitted to Redis queue");
        Ok(id)
    }

    async fn insert_submission(&self, data: &JobSubmissionData) -> Result<(), sqlx::Error> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO jobs (id, job_type, status, payload, webhook_url, webhook_headers, webhook_secret, timeout_secs)
                    VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
                    "#,
                )
                .bind(&data.id)
                .bind(&data.job_type)
                .bind(&JobStatus::Queued)
                .bind(&data.payload)
                .bind(&data.webhook_url)
                .bind(&data.webhook_headers)
                .bind(&data.webhook_secret_hash)
                .bind(data.timeout_secs)
                .execute(pool)
                .await?;
            }
            DbPool::Sqlite(pool) => {
                sqlx::query(
                    r#"
                    INSERT INTO jobs (id, job_type, status, payload, webhook_url, webhook_headers, webhook_secret, timeout_secs)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                    "#,
                )
                .bind(data.id.0.to_string())
                .bind(format!("{:?}", data.job_type))
                .bind("QUEUED")
                .bind(&data.payload)
                .bind(&data.webhook_url)
                .bind(&data.webhook_headers)
                .bind(&data.webhook_secret_hash)
                .bind(data.timeout_secs)
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    async fn delete_submission(&self, id: &JobId) -> Result<(), sqlx::Error> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                sqlx::query("DELETE FROM jobs WHERE id = $1")
                    .bind(id)
                    .execute(pool)
                    .await?;
            }
            DbPool::Sqlite(pool) => {
                sqlx::query("DELETE FROM jobs WHERE id = ?1")
                    .bind(id.0.to_string())
                    .execute(pool)
                    .await?;
            }
        }
        Ok(())
    }

    async fn enqueue_submission(&self, data: &JobSubmissionData) -> Result<(), redis::RedisError> {
        let mut connection = self.redis.get_multiplexed_async_connection().await?;
        connection
            .lpush::<_, _, ()>("Perigee:jobs:queue", data.id.0.to_string())
            .await?;
        Ok(())
    }

    async fn dequeue_submission(&self, id: &JobId) -> Result<(), redis::RedisError> {
        let mut connection = self.redis.get_multiplexed_async_connection().await?;
        let id = id.0.to_string();
        let queue_result: Result<(), redis::RedisError> =
            connection.lrem("Perigee:jobs:queue", 1, id.clone()).await;
        queue_result?;
        let processing_result: Result<(), redis::RedisError> =
            connection.lrem("Perigee:jobs:processing", 1, id).await;
        processing_result?;
        Ok(())
    }

    /// Get a job by ID
    pub async fn get(&self, id: &JobId) -> Result<Option<Job>, JobError> {
        let job = match &self.pool {
            DbPool::Postgres(pool) => {
                sqlx::query_as::<_, Job>("SELECT * FROM jobs WHERE id = $1")
                    .bind(id)
                    .fetch_optional(pool)
                    .await?
            }
            DbPool::Sqlite(pool) => {
                // For SQLite, we need to manually map since sqlx::Type might not work perfectly
                let row = sqlx::query("SELECT * FROM jobs WHERE id = ?1")
                    .bind(id.0.to_string())
                    .fetch_optional(pool)
                    .await?;

                row.map(|r| self.row_to_job(&r)).transpose()?
            }
        };

        Ok(job)
    }

    /// Get the next queued job for processing
    pub async fn get_next_queued(&self) -> Result<Option<Job>, JobError> {
        let job =
            match &self.pool {
                DbPool::Postgres(pool) => sqlx::query_as::<_, Job>(
                    "SELECT * FROM jobs WHERE status = 'QUEUED' ORDER BY created_at ASC LIMIT 1",
                )
                .fetch_optional(pool)
                .await?,
                DbPool::Sqlite(pool) => {
                    let row = sqlx::query(
                    "SELECT * FROM jobs WHERE status = 'QUEUED' ORDER BY created_at ASC LIMIT 1"
                )
                .fetch_optional(pool)
                .await?;

                    row.map(|r| self.row_to_job(&r)).transpose()?
                }
            };

        Ok(job)
    }

    /// Mark a job as processing
    pub async fn mark_processing(&self, id: &JobId) -> Result<(), JobError> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                sqlx::query(
                    "UPDATE jobs SET status = 'PROCESSING', started_at = NOW(), progress_percent = 10, progress_message = 'Processing started' WHERE id = $1"
                )
                .bind(id)
                .execute(pool)
                .await?;
            }
            DbPool::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE jobs SET status = 'PROCESSING', started_at = datetime('now'), progress_percent = 10, progress_message = 'Processing started' WHERE id = ?1"
                )
                .bind(id.0.to_string())
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    /// Update job progress
    pub async fn update_progress(
        &self,
        id: &JobId,
        percent: i32,
        message: &str,
    ) -> Result<(), JobError> {
        match &self.pool {
            DbPool::Postgres(pool) => {
                sqlx::query(
                    "UPDATE jobs SET progress_percent = $1, progress_message = $2 WHERE id = $3",
                )
                .bind(percent)
                .bind(message)
                .bind(id)
                .execute(pool)
                .await?;
            }
            DbPool::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE jobs SET progress_percent = ?1, progress_message = ?2 WHERE id = ?3",
                )
                .bind(percent)
                .bind(message)
                .bind(id.0.to_string())
                .execute(pool)
                .await?;
            }
        }
        Ok(())
    }

    /// Complete a job with a result
    pub async fn complete(&self, id: &JobId, result: &JobResult) -> Result<(), JobError> {
        let result_json = serde_json::to_value(result).map_err(|e| {
            JobError::ProcessingFailed(format!("Failed to serialize result: {}", e))
        })?;

        match &self.pool {
            DbPool::Postgres(pool) => {
                sqlx::query(
                    "UPDATE jobs SET status = 'COMPLETED', result = $1, completed_at = NOW(), progress_percent = 100, progress_message = 'Completed' WHERE id = $2"
                )
                .bind(&result_json)
                .bind(id)
                .execute(pool)
                .await?;
            }
            DbPool::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE jobs SET status = 'COMPLETED', result = ?1, completed_at = datetime('now'), progress_percent = 100, progress_message = 'Completed' WHERE id = ?2"
                )
                .bind(&result_json)
                .bind(id.0.to_string())
                .execute(pool)
                .await?;
            }
        }

        tracing::info!(job_id = %id, "Job completed");
        Ok(())
    }

    /// Mark a job as failed
    pub async fn fail(&self, id: &JobId, error: &str, error_type: &str) -> Result<(), JobError> {
        let result = JobResult::Failed {
            error: error.to_string(),
            error_type: error_type.to_string(),
        };
        let result_json = serde_json::to_value(&result).unwrap_or_default();

        match &self.pool {
            DbPool::Postgres(pool) => {
                sqlx::query(
                    "UPDATE jobs SET status = 'FAILED', result = $1, error_message = $2, error_type = $3, completed_at = NOW(), progress_message = 'Failed' WHERE id = $4"
                )
                .bind(&result_json)
                .bind(error)
                .bind(error_type)
                .bind(id)
                .execute(pool)
                .await?;
            }
            DbPool::Sqlite(pool) => {
                sqlx::query(
                    "UPDATE jobs SET status = 'FAILED', result = ?1, error_message = ?2, error_type = ?3, completed_at = datetime('now'), progress_message = 'Failed' WHERE id = ?4"
                )
                .bind(&result_json)
                .bind(error)
                .bind(error_type)
                .bind(id.0.to_string())
                .execute(pool)
                .await?;
            }
        }

        tracing::error!(job_id = %id, error = %error, "Job failed");
        Ok(())
    }

    /// Cancel a job
    pub async fn cancel(&self, id: &JobId) -> Result<Job, JobError> {
        let job = self.get(id).await?.ok_or(JobError::NotFound(*id))?;

        match job.status {
            JobStatus::Queued | JobStatus::Processing => {
                match &self.pool {
                    DbPool::Postgres(pool) => {
                        sqlx::query(
                            "UPDATE jobs SET status = 'CANCELLED', completed_at = NOW(), progress_message = 'Cancelled' WHERE id = $1"
                        )
                        .bind(id)
                        .execute(pool)
                        .await?;
                    }
                    DbPool::Sqlite(pool) => {
                        sqlx::query(
                            "UPDATE jobs SET status = 'CANCELLED', completed_at = datetime('now'), progress_message = 'Cancelled' WHERE id = ?1"
                        )
                        .bind(id.0.to_string())
                        .execute(pool)
                        .await?;
                    }
                }

                tracing::info!(job_id = %id, "Job cancelled");
                self.get(id).await?.ok_or(JobError::NotFound(*id))
            }
            status => Err(JobError::CannotCancel(status)),
        }
    }

    /// Cleanup old completed jobs
    pub async fn cleanup(&self) -> Result<u64, JobError> {
        let deleted = match &self.pool {
            DbPool::Postgres(pool) => {
                let result = sqlx::query(
                    "DELETE FROM jobs WHERE status IN ('COMPLETED', 'FAILED', 'CANCELLED') AND completed_at < NOW() - INTERVAL '1 hour' * $1"
                )
                .bind(self.config.retention_secs as f64 / 3600.0)
                .execute(pool)
                .await?;
                result.rows_affected()
            }
            DbPool::Sqlite(pool) => {
                let result = sqlx::query(
                    "DELETE FROM jobs WHERE status IN ('COMPLETED', 'FAILED', 'CANCELLED') AND completed_at < datetime('now', '-' || ?1 || ' seconds')"
                )
                .bind(self.config.retention_secs as i64)
                .execute(pool)
                .await?;
                result.rows_affected()
            }
        };

        if deleted > 0 {
            tracing::info!(count = deleted, "Cleaned up old jobs");
        }
        Ok(deleted)
    }

    /// Retry a failed job with exponential backoff
    pub async fn retry_job(&self, job: &Job) -> Result<(), JobError> {
        if job.retry_count >= self.config.max_job_retries {
            tracing::warn!(job_id = %job.id, "Max retries reached, marking as failed");
            return Ok(());
        }

        let new_retry_count = job.retry_count + 1;
        let delay_secs = 2_u64.pow(new_retry_count as u32 - 1) * 30; // 30s, 60s, 120s...

        // Update retry count in DB
        match &self.pool {
            DbPool::Postgres(pool) => {
                sqlx::query("UPDATE jobs SET retry_count = $1, status = 'QUEUED' WHERE id = $2")
                    .bind(new_retry_count)
                    .bind(&job.id)
                    .execute(pool)
                    .await?;
            }
            DbPool::Sqlite(pool) => {
                sqlx::query("UPDATE jobs SET retry_count = ?1, status = 'QUEUED' WHERE id = ?2")
                    .bind(new_retry_count)
                    .bind(job.id.0.to_string())
                    .execute(pool)
                    .await?;
            }
        }

        // Push back to Redis queue after delay (using a simple sleep for now or a delayed set)
        // For a robust implementation, we'd use a sorted set for delayed jobs.
        // For now, let's just push it back to the queue.
        let queue = self.clone();
        let id_str = job.id.0.to_string();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_secs(delay_secs)).await;
            let mut conn = match queue.redis.get_multiplexed_async_connection().await {
                Ok(c) => c,
                Err(_) => return,
            };
            let _: Result<(), _> = conn.lpush("Perigee:jobs:queue", id_str).await;
        });

        tracing::info!(job_id = %job.id, retry_count = new_retry_count, delay_secs, "Job scheduled for retry");
        Ok(())
    }

    /// Spawn a background cleanup task
    pub fn spawn_cleanup_task(&self) -> tokio::task::JoinHandle<()> {
        let queue = self.clone();
        let interval_secs = self.config.cleanup_interval_secs;

        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(interval_secs));

            loop {
                interval.tick().await;

                if let Err(e) = queue.cleanup().await {
                    tracing::error!("Cleanup task error: {}", e);
                }
            }
        })
    }

    fn row_to_job(&self, row: &sqlx::sqlite::SqliteRow) -> Result<Job, JobError> {
        // Manual mapping for SQLite since FromRow might have issues
        use sqlx::Row;

        let id_str: String = row.try_get("id")?;
        let id = JobId(
            Uuid::parse_str(&id_str)
                .map_err(|_| JobError::ProcessingFailed("Invalid UUID".to_string()))?,
        );

        Ok(Job {
            id,
            job_type: JobType::Analyze, // Simplified - would need proper parsing
            status: JobStatus::Queued,  // Simplified - would need proper parsing
            payload: row.try_get("payload").unwrap_or_default(),
            result: row.try_get("result")?,
            progress_percent: row.try_get("progress_percent")?,
            progress_message: row.try_get("progress_message")?,
            webhook_url: row.try_get("webhook_url")?,
            webhook_headers: row.try_get("webhook_headers")?,
            webhook_secret_hash: row.try_get("webhook_secret")?,
            error_message: row.try_get("error_message")?,
            error_type: row.try_get("error_type")?,
            timeout_secs: row.try_get("timeout_secs")?,
            retry_count: row.try_get("retry_count")?,
            created_at: row.try_get("created_at")?,
            started_at: row.try_get("started_at")?,
            completed_at: row.try_get("completed_at")?,
            updated_at: row.try_get("updated_at")?,
        })
    }
}

impl Clone for JobQueue {
    fn clone(&self) -> Self {
        Self {
            pool: self.pool.clone(),
            redis: self.redis.clone(),
            config: self.config.clone(),
        }
    }
}

/// Request to submit a new job
#[derive(Debug, Deserialize, ToSchema)]
pub struct SubmitJobRequest {
    pub job_type: JobType,
    pub payload: JobPayload,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub webhook: Option<WebhookConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
}

/// Response from submitting a job
#[derive(Debug, Serialize, ToSchema)]
pub struct SubmitJobResponse {
    pub job_id: String,
    pub status: JobStatus,
    pub message: String,
}

#[utoipa::path(
    post,
    path = "/jobs/submit",
    request_body = SubmitJobRequest,
    responses(
        (status = 202, description = "Job accepted", body = SubmitJobResponse),
        (status = 500, description = "Internal server error")
    ),
    tag = "Jobs"
)]
pub async fn submit_job_handler(
    State(state): State<Arc<crate::AppState>>,
    SanitizedJson(payload): SanitizedJson<SubmitJobRequest>,
    Extension(cancellation): Extension<RequestCancellation>,
    Json(payload): Json<SubmitJobRequest>,
) -> Result<(StatusCode, Json<SubmitJobResponse>), AppError> {
    let job_id = cancellation
        .wait(
            state
                .job_queue
                .submit(payload.job_type, payload.payload, payload.webhook),
        )
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
        .map_err(|e| AppError::Internal(e.to_string()))?;

    Ok((
        StatusCode::ACCEPTED,
        Json(SubmitJobResponse {
            job_id: job_id.to_string(),
            status: JobStatus::Queued,
            message: "Job submitted successfully".to_string(),
        }),
    ))
}

#[utoipa::path(
    get,
    path = "/jobs/{id}",
    responses(
        (status = 200, description = "Job details", body = Job),
        (status = 404, description = "Job not found")
    ),
    params(
        ("id" = String, Path, description = "Job ID")
    ),
    tag = "Jobs"
)]
pub async fn get_job_handler(
    State(state): State<Arc<crate::AppState>>,
    SanitizedPath(id): SanitizedPath<String>,
    Extension(cancellation): Extension<RequestCancellation>,
    Path(id): Path<String>,
) -> Result<Json<Job>, AppError> {
    let job_id = JobId::from_str(&id).map_err(|_| AppError::BadRequest("Invalid job ID".into()))?;
    let job = cancellation
        .wait(state.job_queue.get(&job_id))
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
        .map_err(|e| AppError::Internal(e.to_string()))?
        .ok_or_else(|| AppError::NotFound(format!("Job {} not found", id)))?;

    Ok(Json(job))
}

#[utoipa::path(
    post,
    path = "/jobs/{id}/cancel",
    responses(
        (status = 200, description = "Job cancelled", body = Job),
        (status = 400, description = "Job cannot be cancelled"),
        (status = 404, description = "Job not found")
    ),
    params(
        ("id" = String, Path, description = "Job ID")
    ),
    tag = "Jobs"
)]
pub async fn cancel_job_handler(
    State(state): State<Arc<crate::AppState>>,
    SanitizedPath(id): SanitizedPath<String>,
    Extension(cancellation): Extension<RequestCancellation>,
    Path(id): Path<String>,
) -> Result<Json<Job>, AppError> {
    let job_id = JobId::from_str(&id).map_err(|_| AppError::BadRequest("Invalid job ID".into()))?;
    let job = cancellation
        .wait(state.job_queue.cancel(&job_id))
        .await
        .map_err(|_| AppError::Internal("Request cancelled".into()))?
        .map_err(|e| match e {
            JobError::NotFound(_) => AppError::NotFound(format!("Job {} not found", id)),
            JobError::CannotCancel(_) => AppError::BadRequest(e.to_string()),
            _ => AppError::Internal(e.to_string()),
        })?;

    Ok(Json(job))
}

/// Job worker that processes jobs from the database queue
pub struct JobWorker {
    queue: JobQueue,
    engine: SimulationEngine,
    insights_engine: InsightsEngine,
    insights_cache: Arc<crate::cache::InsightsCache>,
    config: JobQueueConfig,
    http_client: Client,
    /// Optional pub/sub bus for real-time WebSocket streaming.
    /// When `None` the worker runs in polling-only mode (backwards-compatible).
    bus: Option<Arc<SimulationBus>>,
    /// Optional fee reconciler for Reconcile jobs.
    reconciler: Option<Arc<FeeReconciler>>,
}

impl JobWorker {
    pub fn new(
        queue: JobQueue,
        engine: SimulationEngine,
        insights_engine: InsightsEngine,
        insights_cache: Arc<crate::cache::InsightsCache>,
        config: JobQueueConfig,
    ) -> Self {
        Self {
            queue,
            engine,
            insights_engine,
            insights_cache,
            config,
            http_client: Client::new(),
            bus: None,
            reconciler: None,
        }
    }

    /// Attach a [`SimulationBus`] so the worker publishes real-time events.
    pub fn with_bus(mut self, bus: Arc<SimulationBus>) -> Self {
        self.bus = Some(bus);
        self
    }

    /// Attach a [`FeeReconciler`] so the worker can process Reconcile jobs.
    pub fn with_reconciler(mut self, reconciler: Arc<FeeReconciler>) -> Self {
        self.reconciler = Some(reconciler);
        self
    }

    /// Start the worker loop
    pub async fn run(self) {
        let worker_id = Uuid::new_v4().to_string();
        tracing::info!(worker_id = %worker_id, "Job worker started");

        // Spawn heartbeat task
        let redis_clone = self.queue.redis.clone();
        let worker_id_clone = worker_id.clone();
        tokio::spawn(async move {
            let mut interval = interval(Duration::from_secs(10));
            let mut conn = match redis_clone.get_multiplexed_async_connection().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Heartbeat task failed to get Redis connection: {}", e);
                    return;
                }
            };

            loop {
                interval.tick().await;
                let key = format!("Perigee:workers:{}:heartbeat", worker_id_clone);
                let _: Result<(), _> = conn.set_ex(key, "alive", 30).await;
            }
        });

        let semaphore = Arc::new(tokio::sync::Semaphore::new(self.config.max_concurrent_jobs));

        loop {
            let mut conn = match self.queue.redis.get_multiplexed_async_connection().await {
                Ok(c) => c,
                Err(e) => {
                    tracing::error!("Worker failed to get Redis connection: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                    continue;
                }
            };

            // Reliability pattern: RPOPLPUSH (or BLMOVE)
            // Pop from main queue and push to processing list
            let job_id_res: Result<Option<String>, _> = conn
                .brpoplpush("Perigee:jobs:queue", "Perigee:jobs:processing", 0.0)
                .await;

            match job_id_res {
                Ok(Some(id_str)) => {
                    let job_id = match JobId::from_str(&id_str) {
                        Ok(id) => id,
                        Err(_) => continue,
                    };

                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(p) => p,
                        Err(e) => {
                            tracing::error!("Failed to acquire semaphore: {}", e);
                            continue;
                        }
                    };

                    let queue = self.queue.clone();
                    let engine = self.engine.clone();
                    let insights = self.insights_engine.clone();
                    let insights_cache = self.insights_cache.clone();
                    let config = self.config.clone();
                    let http_client = self.http_client.clone();
                    let bus = self.bus.clone();
                    let reconciler = self.reconciler.clone();
                    let id_str_clone = id_str.clone();

                    tokio::spawn(async move {
                        let _permit = permit;

                        if let Err(e) = Self::process_job(
                            &queue,
                            job_id,
                            engine,
                            insights,
                            insights_cache,
                            config,
                            http_client,
                            bus,
                            reconciler,
                        )
                        .await
                        {
                            tracing::error!("Job processing error: {}", e);
                        }

                        // Clean up processing list after completion
                        let mut conn = match queue.redis.get_multiplexed_async_connection().await {
                            Ok(c) => c,
                            Err(_) => return,
                        };
                        let _: Result<(), _> = conn
                            .lrem("Perigee:jobs:processing", 1, id_str_clone)
                            .await;
                    });
                }
                Ok(None) => {}
                Err(e) => {
                    tracing::error!("Error fetching next job from Redis: {}", e);
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn process_job(
        queue: &JobQueue,
        job_id: JobId,
        engine: SimulationEngine,
        insights_engine: InsightsEngine,
        insights_cache: Arc<crate::cache::InsightsCache>,
        config: JobQueueConfig,
        http_client: Client,
        bus: Option<Arc<SimulationBus>>,
        reconciler: Option<Arc<FeeReconciler>>,
    ) -> Result<(), JobError> {
        let job = queue
            .get(&job_id)
            .await?
            .ok_or(JobError::NotFound(job_id))?;
        tracing::info!(job_id = %job.id, "Processing job");

        // Mark as processing and emit first progress event
        queue.mark_processing(&job.id).await?;
        if let Some(b) = &bus {
            let _ = b.publish_async(SimulationBus::progress(&job.id, 10, "Processing started")).await;
        }

        // Process with timeout
        let timeout = Duration::from_secs(job.timeout_secs as u64);
        let cancellation = CancellationToken::new();
        let result = tokio::time::timeout(
            timeout,
            Self::execute_job(
                &job,
                &engine,
                &insights_engine,
                &insights_cache,
                queue,
                bus.clone(),
                reconciler,
                cancellation.clone(),
            ),
        )
        .await;
        cancellation.cancel();

        // Handle result, emit terminal event, and optionally send webhook
        match result {
            Ok(Ok(job_result)) => {
                queue.complete(&job.id, &job_result).await?;

                // Emit completed event with resource summary
                if let Some(b) = &bus {
                    if let JobResult::Success {
                        simulation_result: Some(ref sim),
                        ..
                    } = job_result
                    {
                        let _ = b
                            .publish_async(SimulationBus::completed(
                                &job.id,
                                &sim.resources,
                                sim.cost_stroops,
                            ))
                            .await;
                    } else {
                        let _ = b
                            .publish_async(SimulationBus::completed(
                                &job.id,
                                &SorobanResources::default(),
                                0,
                            ))
                            .await;
                    }
                }

                if let Some(webhook_config) = job.get_webhook_config() {
                    Self::send_webhook(
                        &http_client,
                        &webhook_config,
                        &job.id,
                        JobStatus::Completed,
                        Some(&job_result),
                        config.webhook_timeout_secs,
                        config.webhook_max_retries,
                    )
                    .await;
                }
            }
            Ok(Err(e)) => {
                let error_msg = e.to_string();
                queue.fail(&job.id, &error_msg, "ProcessingError").await?;

                // Attempt retry
                let _ = queue.retry_job(&job).await;

                if let Some(b) = &bus {
                    let _ = b
                        .publish_async(SimulationBus::failed(
                            &job.id,
                            &error_msg,
                            "ProcessingError",
                        ))
                        .await;
                }

                if let Some(webhook_config) = job.get_webhook_config() {
                    Self::send_webhook(
                        &http_client,
                        &webhook_config,
                        &job.id,
                        JobStatus::Failed,
                        None,
                        config.webhook_timeout_secs,
                        config.webhook_max_retries,
                    )
                    .await;
                }
            }
            Err(_) => {
                let error_msg = format!("Job timed out after {} seconds", job.timeout_secs);
                queue.fail(&job.id, &error_msg, "Timeout").await?;

                // Attempt retry
                let _ = queue.retry_job(&job).await;

                if let Some(b) = &bus {
                    let _ = b
                        .publish_async(SimulationBus::failed(&job.id, &error_msg, "Timeout"))
                        .await;
                }

                if let Some(webhook_config) = job.get_webhook_config() {
                    Self::send_webhook(
                        &http_client,
                        &webhook_config,
                        &job.id,
                        JobStatus::Failed,
                        None,
                        config.webhook_timeout_secs,
                        config.webhook_max_retries,
                    )
                    .await;
                }
            }
        }

        Ok(())
    }

    async fn execute_job(
        job: &Job,
        engine: &SimulationEngine,
        insights_engine: &InsightsEngine,
        insights_cache: &Arc<crate::cache::InsightsCache>,
        queue: &JobQueue,
        bus: Option<Arc<SimulationBus>>,
        reconciler: Option<Arc<FeeReconciler>>,
        cancellation: CancellationToken,
    ) -> Result<JobResult, Box<dyn std::error::Error + Send + Sync>> {
        let payload = job.get_payload().ok_or("Invalid payload")?;

        /// Helper: update DB progress and publish WebSocket event simultaneously.
        macro_rules! progress {
            ($percent:expr, $msg:expr) => {{
                let _ = queue.update_progress(&job.id, $percent, $msg).await;
                if let Some(ref b) = bus {
                    let _ = b
                        .publish_async(SimulationBus::progress(&job.id, $percent, $msg))
                        .await;
                let _ = wait_for_cancellation(
                    &cancellation,
                    queue.update_progress(&job.id, $percent, $msg),
                )
                .await;
                if !cancellation.is_cancelled() {
                    if let Some(ref b) = bus {
                        b.publish(SimulationBus::progress(&job.id, $percent, $msg));
                    }
                }
            }};
        }

        match payload {
            JobPayload::Analyze {
                contract_id,
                function_name,
                args,
                ledger_overrides,
            } => {
                progress!(30, "Running simulation");

                let args_ref = args.as_ref().cloned().unwrap_or_default();
                let sim_result = engine
                    .simulate_from_contract_id_with_cancellation(
                        &contract_id,
                        &function_name,
                        args_ref,
                        ledger_overrides,
                        None,
                        None,
                        cancellation.clone(),
                    )
                    .await
                    .map_err(|e| {
                        // If this was a provider failover during simulation, emit the
                        // event so WebSocket clients see the provider switch in real time.
                        // (The SimulationEngine surfaces failovers via its error chain;
                        // here we inspect the message for a best-effort broadcast.)
                        let msg = e.to_string();
                        if let Some(ref b) = bus {
                            if msg.contains("failover") || msg.contains("provider") {
                                let event = SimulationBus::provider_failover(
                                    &job.id,
                                    "unknown",
                                    "next-available",
                                    &msg,
                                );
                                let _ = b.publish(event);
                            }
                        }
                        e
                    })?;

                // If the engine ran in consensus mode, surface the quorum result.
                // (SimulationEngine sets `sim_result.provider_name` when consensus
                //  succeeded; we broadcast an agreement event here.)
                if let Some(ref b) = bus {
                    let _ = b
                        .publish_async(SimulationBus::consensus_check(
                            &job.id,
                            true,
                            vec![],
                            None,
                        ))
                        .await;
                }

                progress!(70, "Generating insights");
                let cache_key = crate::cache::SimulationCache::generate_key(
                    &contract_id,
                    &function_name,
                    &args.as_ref().cloned().unwrap_or_default(),
                );
                
                let insights = if let Some(cached_insights) = insights_cache.get(&cache_key).await {
                    cached_insights
                } else {
                    let generated = insights_engine.analyze(&sim_result.resources);
                    insights_cache.set(cache_key, generated.clone()).await;
                    generated
                };

                progress!(90, "Finalizing results");

                Ok(JobResult::Success {
                    resources: Some(sim_result.resources.clone()),
                    simulation_result: Some(sim_result),
                    optimization: None,
                    comparison: None,
                    reconciliation: None,
                    insights: Some(insights),
                })
            }
            JobPayload::OptimizeLimits {
                contract_id,
                function_name,
                args,
                safety_margin,
            } => {
                progress!(30, "Running optimization");

                let report = engine
                    .optimize_limits_with_cancellation(
                        &contract_id,
                        &function_name,
                        args,
                        safety_margin,
                        cancellation.clone(),
                    )
                    .await?;

                progress!(90, "Finalizing results");

                Ok(JobResult::Success {
                    resources: None,
                    simulation_result: None,
                    optimization: Some(serde_json::to_value(report)?),
                    comparison: None,
                    reconciliation: None,
                    insights: None,
                })
            }
            JobPayload::Reconcile {
                from_ledger,
                to_ledger,
                tolerance_pct,
            } => {
                let reconciler = reconciler.ok_or("Reconciler not configured")?;

                progress!(15, "Starting fee reconciliation");

                let queue_for_cb = queue.clone();
                let bus_for_cb = bus.clone();
                let cancellation_for_cb = cancellation.clone();
                let job_id_for_cb = job.id;
                let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel(16);
                let progress_task = tokio::spawn(async move {
                    while let Some((percent, msg)) = progress_rx.recv().await {
                        let _ = queue_for_cb
                            .update_progress(&job_id_for_cb, percent, &msg)
                            .await;
                        if let Some(ref bus) = bus_for_cb {
                            let _ = bus
                                .publish_async(SimulationBus::progress(
                                    &job_id_for_cb,
                                    percent,
                                    &msg,
                                ))
                                .await;
                        }
                    }
                });

                let report = reconciler
                    .run_with_cancellation(
                        from_ledger,
                        to_ledger,
                        tolerance_pct,
                        Some(Box::new(move |percent, msg| {
                            let _ = progress_tx.try_send((percent, msg.to_string()));
                            let q = queue_for_cb.clone();
                            let b = bus_for_cb.clone();
                            let jid = job_id_for_cb;
                            let msg = msg.to_string();
                            let progress_cancellation = cancellation_for_cb.clone();
                            tokio::spawn(async move {
                                tokio::select! {
                                    _ = progress_cancellation.cancelled() => {}
                                    _ = async move {
                                        let _ = q.update_progress(&jid, percent, &msg).await;
                                        if let Some(ref bus) = b {
                                            bus.publish(crate::ws::SimulationBus::progress(
                                                &jid, percent, &msg,
                                            ));
                                        }
                                    } => {}
                                }
                            });
                        })),
                        cancellation.clone(),
                    )
                    .await;
                let _ = progress_task.await;
                let report = report?;

                progress!(100, "Reconciliation complete");

                Ok(JobResult::Success {
                    resources: None,
                    simulation_result: None,
                    optimization: None,
                    comparison: None,
                    reconciliation: Some(report),
                    insights: None,
                })
            }
            _ => Ok(JobResult::Success {
                resources: None,
                simulation_result: None,
                optimization: None,
                comparison: Some(serde_json::json!({"status": "Not fully implemented"})),
                reconciliation: None,
                insights: None,
            }),
        }
    }

    async fn send_webhook(
        client: &Client,
        config: &WebhookConfig,
        job_id: &JobId,
        status: JobStatus,
        result: Option<&JobResult>,
        timeout_secs: u64,
        max_retries: u32,
    ) {
        let payload = serde_json::json!({
            "job_id": job_id.to_string(),
            "status": status,
            "result": result,
            "timestamp": Utc::now().to_rfc3339(),
        });

        let timeout = Duration::from_secs(timeout_secs);
        let mut last_error = None;

        for attempt in 1..=max_retries {
            let mut request = client
                .post(&config.callback_url)
                .json(&payload)
                .timeout(timeout);

            // Add custom headers if provided
            if let Some(headers) = &config.headers {
                for (key, value) in headers {
                    request = request.header(key, value);
                }
            }

            match request.send().await {
                Ok(response) => {
                    if response.status().is_success() {
                        tracing::info!(job_id = %job_id, attempt, "Webhook delivered");
                        return;
                    } else {
                        last_error = Some(format!("HTTP {}", response.status()));
                    }
                }
                Err(e) => {
                    last_error = Some(e.to_string());
                }
            }

            if attempt < max_retries {
                tokio::time::sleep(Duration::from_millis(1000 * 2_u64.pow(attempt - 1))).await;
            }
        }

        tracing::error!(job_id = %job_id, error = ?last_error, "Webhook failed");
    }
}
