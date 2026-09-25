ALTER TABLE vaults ADD COLUMN deleted_at TEXT NULL;
CREATE INDEX IF NOT EXISTS idx_vaults_deleted_at ON vaults(deleted_at);
