CREATE TABLE IF NOT EXISTS ledger_fee_samples (
    ledger_sequence INTEGER PRIMARY KEY,
    collected_at TEXT NOT NULL,
    base_reserve INTEGER NOT NULL,
    base_fee INTEGER NOT NULL,
    max_fee INTEGER NOT NULL,
    fee_charged INTEGER NOT NULL,
    transaction_count INTEGER NOT NULL,
    ledger_close_time TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS transaction_fee_records (
    id TEXT PRIMARY KEY,
    ledger_sequence INTEGER NOT NULL,
    tx_hash TEXT NOT NULL,
    fee_bid INTEGER NOT NULL,
    fee_charged INTEGER NOT NULL,
    resource_fee INTEGER NOT NULL,
    inclusion_success BOOLEAN NOT NULL,
    recorded_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY (ledger_sequence) REFERENCES ledger_fee_samples(ledger_sequence)
);

CREATE INDEX IF NOT EXISTS idx_fee_samples_sequence ON ledger_fee_samples(ledger_sequence);
CREATE INDEX IF NOT EXISTS idx_fee_samples_close_time ON ledger_fee_samples(ledger_close_time);
CREATE INDEX IF NOT EXISTS idx_tx_records_ledger ON transaction_fee_records(ledger_sequence);
CREATE INDEX IF NOT EXISTS idx_tx_records_hash ON transaction_fee_records(tx_hash);
CREATE INDEX IF NOT EXISTS idx_tx_records_recorded_at ON transaction_fee_records(recorded_at);
