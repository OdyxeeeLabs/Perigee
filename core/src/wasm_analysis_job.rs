//! # Asynchronous WASM analysis jobs (API-34, light pass)
//!
//! A minimal in-memory job store for submitting a WASM artifact for
//! out-of-band branch analysis (see [`crate::wasm_branch_analysis`]) and
//! polling for its result, instead of blocking on the synchronous
//! `/analyze/wasm` route.
//!
//! This is intentionally small: jobs live in a process-local
//! `HashMap<Uuid, WasmAnalysisJob>` behind a `Mutex`, not in durable storage.
//! Lookup is scoped only by knowing the job's `Uuid` — in a real deployment
//! this would additionally be keyed by tenant/API key so one caller cannot
//! enumerate another caller's jobs; that multi-tenant auth layer is out of
//! scope for this light pass.
//!
//! No HTTP route is wired to this module yet (see the API-34 issue); it is
//! ready to be exposed by a future submit/poll endpoint.

use crate::error_codes::ErrorCode;
use crate::errors::AppError;
use crate::input_sanitization::max_body_bytes;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Mutex;
use uuid::Uuid;

/// How long a finished job's result is retained before lookups report it as
/// `Expired` instead of returning the (now stale) result. A light stand-in
/// for a real retention policy.
pub const DEFAULT_RETENTION_HOURS: i64 = 24;

/// Lifecycle of a submitted WASM analysis job.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WasmAnalysisStatus {
    Queued,
    Running,
    Succeeded,
    Failed,
    Expired,
}

/// A submitted WASM analysis job and its current state.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WasmAnalysisJob {
    pub id: Uuid,
    pub status: WasmAnalysisStatus,
    pub submitted_at: DateTime<Utc>,
    pub expires_at: DateTime<Utc>,
    /// Size of the submitted artifact in bytes (the artifact bytes
    /// themselves are not retained here; only enough to validate and
    /// report on the submission).
    pub artifact_bytes: usize,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
}

impl WasmAnalysisJob {
    fn new(artifact_bytes: usize, now: DateTime<Utc>, retention_hours: i64) -> Self {
        WasmAnalysisJob {
            id: Uuid::new_v4(),
            status: WasmAnalysisStatus::Queued,
            submitted_at: now,
            expires_at: now + chrono::Duration::hours(retention_hours),
            artifact_bytes,
            result: None,
            error: None,
        }
    }

    fn is_expired(&self, now: DateTime<Utc>) -> bool {
        now >= self.expires_at
    }
}

/// In-memory store of submitted jobs, keyed by id.
///
/// Reachability is by `Uuid` alone: anyone who can guess or was handed a
/// job's id can look it up. A real deployment would additionally scope
/// lookups by tenant/API key.
#[derive(Default)]
pub struct WasmAnalysisJobStore {
    jobs: Mutex<HashMap<Uuid, WasmAnalysisJob>>,
    retention_hours: i64,
}

impl WasmAnalysisJobStore {
    pub fn new() -> Self {
        WasmAnalysisJobStore {
            jobs: Mutex::new(HashMap::new()),
            retention_hours: DEFAULT_RETENTION_HOURS,
        }
    }

    #[cfg(test)]
    fn with_retention_hours(retention_hours: i64) -> Self {
        WasmAnalysisJobStore {
            jobs: Mutex::new(HashMap::new()),
            retention_hours,
        }
    }

    /// Submit a WASM artifact for analysis. Rejects artifacts larger than
    /// the configured upload cap (reusing the API-33 body-size limit)
    /// with `ErrorCode::PayloadTooLarge`.
    pub fn submit(&self, artifact: &[u8], now: DateTime<Utc>) -> Result<Uuid, AppError> {
        let max_bytes = max_body_bytes();
        if artifact.len() > max_bytes {
            return Err(AppError::with_code(
                ErrorCode::PayloadTooLarge,
                format!(
                    "WASM artifact ({} bytes) exceeds the {} byte upload limit",
                    artifact.len(),
                    max_bytes
                ),
            ));
        }

        let job = WasmAnalysisJob::new(artifact.len(), now, self.retention_hours);
        let id = job.id;
        self.jobs
            .lock()
            .map_err(|_| AppError::with_code(ErrorCode::InternalServerError, "job store unavailable"))?
            .insert(id, job);
        Ok(id)
    }

    /// Look up a job by id, applying the retention/expiry check. Returns
    /// `None` if no job with that id was ever submitted.
    pub fn get(&self, id: Uuid, now: DateTime<Utc>) -> Result<Option<WasmAnalysisJob>, AppError> {
        let jobs = self
            .jobs
            .lock()
            .map_err(|_| AppError::with_code(ErrorCode::InternalServerError, "job store unavailable"))?;
        Ok(jobs.get(&id).map(|job| {
            let mut job = job.clone();
            if job.is_expired(now) && job.status != WasmAnalysisStatus::Expired {
                job.status = WasmAnalysisStatus::Expired;
            }
            job
        }))
    }

    /// Mark a job as running (claimed by a worker).
    pub fn mark_running(&self, id: Uuid) -> Result<(), AppError> {
        self.with_job_mut(id, |job| job.status = WasmAnalysisStatus::Running)
    }

    /// Record a successful result.
    pub fn complete(&self, id: Uuid, result: serde_json::Value) -> Result<(), AppError> {
        self.with_job_mut(id, |job| {
            job.status = WasmAnalysisStatus::Succeeded;
            job.result = Some(result);
        })
    }

    /// Record a failure.
    pub fn fail(&self, id: Uuid, error: impl Into<String>) -> Result<(), AppError> {
        self.with_job_mut(id, |job| {
            job.status = WasmAnalysisStatus::Failed;
            job.error = Some(error.into());
        })
    }

    fn with_job_mut(&self, id: Uuid, f: impl FnOnce(&mut WasmAnalysisJob)) -> Result<(), AppError> {
        let mut jobs = self
            .jobs
            .lock()
            .map_err(|_| AppError::with_code(ErrorCode::InternalServerError, "job store unavailable"))?;
        match jobs.get_mut(&id) {
            Some(job) => {
                f(job);
                Ok(())
            }
            None => Err(AppError::with_code(ErrorCode::NotFound, "job not found")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn oversized_artifact_is_rejected() {
        let store = WasmAnalysisJobStore::new();
        let oversized = vec![0u8; max_body_bytes() + 1];
        let err = store.submit(&oversized, Utc::now()).expect_err("must reject");
        assert_eq!(err.error_code(), ErrorCode::PayloadTooLarge);
    }

    #[test]
    fn submitted_job_is_queued_then_succeeded_after_completion() {
        let store = WasmAnalysisJobStore::new();
        let now = Utc::now();
        let id = store.submit(b"\0asm small artifact", now).unwrap();

        let job = store.get(id, now).unwrap().expect("job exists");
        assert_eq!(job.status, WasmAnalysisStatus::Queued);

        store.mark_running(id).unwrap();
        store.complete(id, json!({"worst_case_cpu": 12345})).unwrap();

        let job = store.get(id, now).unwrap().expect("job exists");
        assert_eq!(job.status, WasmAnalysisStatus::Succeeded);
        assert_eq!(job.result, Some(json!({"worst_case_cpu": 12345})));
    }

    #[test]
    fn job_older_than_retention_window_reports_as_expired() {
        let store = WasmAnalysisJobStore::with_retention_hours(1);
        let submitted_at = Utc::now() - chrono::Duration::hours(2);
        let id = store.submit(b"\0asm", submitted_at).unwrap();
        store.complete(id, json!({"ok": true})).unwrap();

        // Looked up well past its 1-hour retention window.
        let now = submitted_at + chrono::Duration::hours(3);
        let job = store.get(id, now).unwrap().expect("job exists");
        assert_eq!(job.status, WasmAnalysisStatus::Expired);
    }
}
