CREATE TABLE IF NOT EXISTS fee_reconciliation_reports (
    id TEXT PRIMARY KEY,
    tenant_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    from_ledger INTEGER NOT NULL,
    to_ledger INTEGER NOT NULL,
    status TEXT NOT NULL CHECK (status IN ('clean', 'discrepancies_found')),
    result_json TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_fee_reconciliation_tenant_created
    ON fee_reconciliation_reports(tenant_id, created_at DESC);
