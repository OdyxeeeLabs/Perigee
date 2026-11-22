-- Report job artifacts for asynchronous large report generation (API-22).
--
-- Each row tracks one enqueued report job, scoped to the owning tenant.
-- Generated artifacts expire at `expires_at` according to the documented
-- retention window (see docs/contributor-issue-backlog.md, API-22).

CREATE TABLE IF NOT EXISTS report_jobs (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    kind TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'queued'
        CHECK (status IN ('queued', 'running', 'completed', 'failed')),
    progress_percent INTEGER NOT NULL DEFAULT 0
        CHECK (progress_percent BETWEEN 0 AND 100),
    artifact_path TEXT,
    artifact_ready INTEGER NOT NULL DEFAULT 0 CHECK (artifact_ready IN (0, 1)),
    error_message TEXT,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
    completed_at TIMESTAMP,
    expires_at TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_report_jobs_tenant
    ON report_jobs (tenant_id, created_at);

CREATE INDEX IF NOT EXISTS idx_report_jobs_expires
    ON report_jobs (expires_at)
    WHERE expires_at IS NOT NULL;
