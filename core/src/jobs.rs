use dashmap::DashMap;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{timeout, Duration};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::errors::AppError;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum JobStatus {
    Queued,
    Processing,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMetadata {
    pub id: String,
    pub status: JobStatus,
    pub created_at: u64,
    pub updated_at: u64,
    pub expires_at: Option<u64>,
    pub callback_url: Option<String>,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct SubmitJobRequest {
    pub callback_url: Option<String>,
    pub expires_in_seconds: Option<u64>,
    pub job_data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, ToSchema)]
pub struct JobResponse {
    pub job_id: String,
    pub status: JobStatus,
    pub created_at: u64,
    pub expires_at: Option<u64>,
}

#[derive(Clone)]
pub struct JobQueue {
    jobs: Arc<DashMap<String, JobMetadata>>,
}

impl JobQueue {
    pub fn new() -> Self {
        Self {
            jobs: Arc::new(DashMap::new()),
        }
    }

    pub fn submit_job(&self, request: SubmitJobRequest) -> Result<JobResponse, AppError> {
        let job_id = Uuid::new_v4().to_string();

        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| AppError::Internal(format!("Time error: {}", e)))?
            .as_secs();

        let expires_at = request.expires_in_seconds.map(|secs| now + secs);

        let job_metadata = JobMetadata {
            id: job_id.clone(),
            status: JobStatus::Queued,
            created_at: now,
            updated_at: now,
            expires_at,
            callback_url: request.callback_url,
            result: None,
            error: None,
        };

        self.jobs.insert(job_id.clone(), job_metadata);

        Ok(JobResponse {
            job_id,
            status: JobStatus::Queued,
            created_at: now,
            expires_at,
        })
    }

    pub async fn get_job(&self, job_id: &str) -> Result<JobResponse, AppError> {
        let job = self
            .jobs
            .get(job_id)
            .ok_or_else(|| AppError::NotFound(format!("Job with ID {} not found", job_id)))?;

        Ok(JobResponse {
            job_id: job.id.clone(),
            status: job.status.clone(),
            created_at: job.created_at,
            expires_at: job.expires_at,
        })
    }

    pub fn update_job_status(
        &self,
        job_id: &str,
        status: JobStatus,
        result: Option<serde_json::Value>,
        error: Option<String>,
    ) -> Result<(), AppError> {
        if let Some(mut job) = self.jobs.get_mut(job_id) {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| AppError::Internal(format!("Time error: {}", e)))?
                .as_secs();

            job.status = status;
            job.updated_at = now;
            job.result = result;
            job.error = error;

            // If job is completed, trigger webhook if callback URL exists
            if matches!(job.status, JobStatus::Completed | JobStatus::Failed) {
                if let Some(ref callback_url) = job.callback_url.clone() {
                    // Clone into fully-owned values so the async task is 'static
                    let owned_url: String = callback_url.clone();
                    let job_snapshot: JobMetadata = job.clone();
                    tokio::spawn(async move {
                        Self::trigger_webhook(owned_url, job_snapshot).await;
                    });
                }
            }
        } else {
            return Err(AppError::NotFound(format!(
                "Job with ID {} not found",
                job_id
            )));
        }

        Ok(())
    }

    pub fn cancel_job(&self, job_id: &str) -> Result<(), AppError> {
        self.update_job_status(
            job_id,
            JobStatus::Cancelled,
            None,
            Some("Job cancelled".to_string()),
        )
    }

    pub fn cleanup_expired_jobs(&self) {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        self.jobs.retain(|_, job| {
            // Keep job if it doesn't expire or hasn't expired yet
            job.expires_at.map_or(true, |exp| exp > now)
        });
    }

    pub async fn trigger_webhook(callback_url: String, job: JobMetadata) {
        let client = reqwest::Client::new();
        let payload = serde_json::json!({
            "job_id": job.id,
            "status": format!("{:?}", job.status),
            "result": job.result,
            "error": job.error,
            "completed_at": job.updated_at
        });

        match timeout(
            Duration::from_secs(30),
            client.post(&callback_url).json(&payload).send(),
        )
        .await
        {
            Ok(Ok(response)) => {
                if response.status().is_success() {
                    tracing::info!(
                        "Webhook sent successfully to {} for job {}",
                        callback_url,
                        job.id
                    );
                } else {
                    tracing::warn!(
                        "Webhook failed with status {} for job {} at {}",
                        response.status(),
                        job.id,
                        callback_url
                    );
                }
            }
            Ok(Err(e)) => {
                tracing::error!("Failed to send webhook for job {}: {}", job.id, e);
            }
            Err(_) => {
                tracing::error!(
                    "Webhook request timed out for job {} at {}",
                    job.id,
                    callback_url
                );
            }
        }
    }

    pub fn get_job_result(&self, job_id: &str) -> Result<Option<serde_json::Value>, AppError> {
        let job = self
            .jobs
            .get(job_id)
            .ok_or_else(|| AppError::NotFound(format!("Job with ID {} not found", job_id)))?;

        Ok(job.result.clone())
    }

    pub fn is_job_expired(&self, job_id: &str) -> Result<bool, AppError> {
        let job = self
            .jobs
            .get(job_id)
            .ok_or_else(|| AppError::NotFound(format!("Job with ID {} not found", job_id)))?;

        match job.expires_at {
            Some(expiry) => {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .map_err(|e| AppError::Internal(format!("Time error: {}", e)))?
                    .as_secs();

                Ok(now > expiry)
            }
            None => Ok(false),
        }
    }
}

// Background cleanup task to remove expired jobs
pub async fn start_cleanup_task(job_queue: JobQueue, interval_seconds: u64) {
    let job_queue_clone = job_queue.clone();
    tokio::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(interval_seconds)).await;
            job_queue_clone.cleanup_expired_jobs();
            tracing::debug!("Expired jobs cleanup completed");
        }
    });
}
