DROP INDEX IF EXISTS idx_vaults_idempotency;
ALTER TABLE vaults DROP COLUMN idempotency_key;
