-- Soft deletes (#608): deleted_at + deleted_by on soft-deleting tables, and an
-- audit trail that survives the purge job.
ALTER TABLE vaults ADD COLUMN deleted_by TEXT NULL;
ALTER TABLE managers ADD COLUMN deleted_at TEXT NULL;
ALTER TABLE managers ADD COLUMN deleted_by TEXT NULL;

CREATE INDEX IF NOT EXISTS idx_managers_deleted_at ON managers(deleted_at);

CREATE TABLE IF NOT EXISTS deletion_audit (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    table_name TEXT NOT NULL,
    record_id TEXT NOT NULL,
    action TEXT NOT NULL,       -- DELETE | RESTORE | PURGE
    actor TEXT NOT NULL,
    occurred_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
-- Only ids and actions are stored here, never the deleted data itself, so the
-- trail is safe to keep after a GDPR purge.
CREATE INDEX IF NOT EXISTS idx_deletion_audit_record ON deletion_audit(table_name, record_id);
