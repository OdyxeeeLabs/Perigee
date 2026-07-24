-- Migration 003: Create reconciliation tables
-- Stores fee prediction vs actual reconciliation reports and discrepancies

CREATE TABLE IF NOT EXISTS reconciliation_reports (
    id TEXT PRIMARY KEY,
    from_ledger BIGINT NOT NULL,
    to_ledger BIGINT NOT NULL,
    tolerance_pct REAL NOT NULL DEFAULT 5.0,
    total_ledgers INTEGER NOT NULL DEFAULT 0,
    discrepancies_count INTEGER NOT NULL DEFAULT 0,
    avg_delta_pct REAL NOT NULL DEFAULT 0.0,
    max_delta_pct REAL NOT NULL DEFAULT 0.0,
    summary JSONB,
    created_at TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP
);

CREATE TABLE IF NOT EXISTS reconciliation_discrepancies (
    id TEXT PRIMARY KEY,
    report_id TEXT NOT NULL,
    ledger_sequence BIGINT NOT NULL,
    expected_fee BIGINT NOT NULL,
    actual_fee BIGINT NOT NULL,
    delta BIGINT NOT NULL,
    delta_pct REAL NOT NULL,
    severity VARCHAR(20) NOT NULL DEFAULT 'warning',
    FOREIGN KEY (report_id) REFERENCES reconciliation_reports(id)
);

CREATE INDEX IF NOT EXISTS idx_recon_reports_created ON reconciliation_reports(created_at);
CREATE INDEX IF NOT EXISTS idx_recon_reports_ledgers ON reconciliation_reports(from_ledger, to_ledger);
CREATE INDEX IF NOT EXISTS idx_recon_disc_report ON reconciliation_discrepancies(report_id);
CREATE INDEX IF NOT EXISTS idx_recon_disc_severity ON reconciliation_discrepancies(severity);
