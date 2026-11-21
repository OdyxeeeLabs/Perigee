-- Data archival (#607): what has been exported to cold storage, and the async
-- retrieval requests for it.
CREATE TABLE IF NOT EXISTS archive_manifest (
    object_key TEXT PRIMARY KEY,
    table_name TEXT NOT NULL,
    tenant TEXT NOT NULL,
    vault TEXT NOT NULL,
    year INTEGER NOT NULL,
    month INTEGER NOT NULL,
    row_count INTEGER NOT NULL,
    checksum TEXT NOT NULL,
    archived_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
CREATE INDEX IF NOT EXISTS idx_archive_manifest_lookup
    ON archive_manifest(table_name, tenant, vault, year, month);

-- Data subjects present in each archived object, for GDPR erasure.
CREATE TABLE IF NOT EXISTS archive_manifest_subjects (
    object_key TEXT NOT NULL REFERENCES archive_manifest(object_key) ON DELETE CASCADE,
    subject_id TEXT NOT NULL,
    PRIMARY KEY (object_key, subject_id)
);
CREATE INDEX IF NOT EXISTS idx_archive_subjects_subject ON archive_manifest_subjects(subject_id);

CREATE TABLE IF NOT EXISTS archive_retrievals (
    id TEXT PRIMARY KEY,
    table_name TEXT NOT NULL,
    tenant TEXT NOT NULL,
    vault TEXT,
    from_period TEXT NOT NULL,   -- YYYY-MM
    to_period TEXT NOT NULL,     -- YYYY-MM
    requested_by TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'QUEUED',
    result_url TEXT,
    error TEXT,
    requested_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    expires_at TEXT
);
CREATE INDEX IF NOT EXISTS idx_archive_retrievals_tenant ON archive_retrievals(tenant, status);
