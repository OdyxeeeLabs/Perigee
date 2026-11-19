-- Transactional outbox (#610): events are written in the same transaction as the
-- business change and relayed to the broker by a separate process.
CREATE TABLE IF NOT EXISTS outbox_events (
    id TEXT PRIMARY KEY,
    aggregate_type TEXT NOT NULL,
    aggregate_id TEXT NOT NULL,
    event_type TEXT NOT NULL,
    payload TEXT NOT NULL,
    idempotency_key TEXT NOT NULL UNIQUE,
    sequence INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'PENDING',
    attempts INTEGER NOT NULL DEFAULT 0,
    last_error TEXT,
    created_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    published_at TEXT,
    quarantined_at TEXT
);

-- Per-aggregate ordering: one sequence number per event, no gaps from concurrent writers.
CREATE UNIQUE INDEX IF NOT EXISTS idx_outbox_aggregate_sequence
    ON outbox_events(aggregate_type, aggregate_id, sequence);
-- The relay's polling query.
CREATE INDEX IF NOT EXISTS idx_outbox_status_created_at ON outbox_events(status, created_at);

-- Consumer-side deduplication: a consumer handles each idempotency key once.
CREATE TABLE IF NOT EXISTS processed_messages (
    consumer TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    processed_at TEXT NOT NULL DEFAULT CURRENT_TIMESTAMP,
    PRIMARY KEY (consumer, idempotency_key)
);
