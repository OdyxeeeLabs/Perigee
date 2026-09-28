CREATE TABLE IF NOT EXISTS vaults (
    id TEXT PRIMARY KEY,
    manager_id TEXT NOT NULL,
    name TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    config_json TEXT NOT NULL DEFAULT '{}',
    version INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE INDEX IF NOT EXISTS idx_vaults_manager ON vaults(manager_id);
CREATE INDEX IF NOT EXISTS idx_vaults_status ON vaults(status);
