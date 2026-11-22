DROP INDEX IF EXISTS idx_deletion_audit_record;
DROP TABLE IF EXISTS deletion_audit;
DROP INDEX IF EXISTS idx_managers_deleted_at;
ALTER TABLE managers DROP COLUMN deleted_by;
ALTER TABLE managers DROP COLUMN deleted_at;
ALTER TABLE vaults DROP COLUMN deleted_by;
