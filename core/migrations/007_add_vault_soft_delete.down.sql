DROP INDEX IF EXISTS idx_vaults_deleted_at;
ALTER TABLE vaults DROP COLUMN deleted_at;
