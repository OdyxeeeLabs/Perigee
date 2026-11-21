-- Vault reporting metadata (API-27 / #788). Kept off-chain and separate from
-- vaults.config_json so bulk metadata edits can never touch on-chain policy.
CREATE TABLE IF NOT EXISTS vault_metadata (
    vault_id TEXT PRIMARY KEY REFERENCES vaults(id),
    labels_json TEXT NOT NULL DEFAULT '[]',
    reporting_reference TEXT NULL,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);
