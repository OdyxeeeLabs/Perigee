//! Durable idempotency storage (API-06).
//!
//! [`SqliteBackend`] persists entries so a retry after a restart, or on another
//! process sharing the database file, still replays the original response.
//! [`TieredBackend`] keeps the in-memory backend as the fast path and treats
//! SQLite as the source of truth for the atomic claim.
//!
//! * **Expiry**: rows carry `expires_at`; an expired row is treated as absent
//!   and replaced on the next claim (same TTL as the in-memory path).
//! * **Concurrency**: claims run in an `IMMEDIATE` transaction, so concurrent
//!   same-key requests produce exactly one winner.

use crate::idempotency::{Entry, IdempotencyBackend, IdempotencyError, InMemoryBackend};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use std::sync::{Arc, Mutex};

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS idempotency_keys (
    key TEXT PRIMARY KEY,
    entry TEXT NOT NULL,
    expires_at INTEGER NOT NULL
)";

fn backend_err(e: impl std::fmt::Display) -> IdempotencyError {
    IdempotencyError::Backend(e.to_string())
}

pub struct SqliteBackend {
    conn: Mutex<Connection>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl SqliteBackend {
    /// Open (creating if needed) the database file at `path`.
    pub fn open(path: &str) -> Result<Self, IdempotencyError> {
        let conn = Connection::open(path).map_err(backend_err)?;
        conn.execute_batch(SCHEMA).map_err(backend_err)?;
        Ok(SqliteBackend {
            conn: Mutex::new(conn),
            clock: Arc::new(|| {
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            }),
        })
    }

    /// Replace the clock (seconds); used by tests to exercise expiry.
    pub fn with_clock(mut self, clock: Arc<dyn Fn() -> u64 + Send + Sync>) -> Self {
        self.clock = clock;
        self
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Connection>, IdempotencyError> {
        self.conn
            .lock()
            .map_err(|_| IdempotencyError::Backend("idempotency store lock poisoned".into()))
    }
}

impl IdempotencyBackend for SqliteBackend {
    fn try_begin(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<Option<Entry>, IdempotencyError> {
        let now = (self.clock)() as i64;
        let mut conn = self.lock()?;
        let tx = conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(backend_err)?;
        tx.execute(
            "DELETE FROM idempotency_keys WHERE key = ?1 AND expires_at <= ?2",
            params![key, now],
        )
        .map_err(backend_err)?;
        let existing: Option<String> = tx
            .query_row("SELECT entry FROM idempotency_keys WHERE key = ?1", params![key], |r| r.get(0))
            .optional()
            .map_err(backend_err)?;
        if let Some(raw) = existing {
            return serde_json::from_str(&raw).map(Some).map_err(backend_err);
        }
        let raw = serde_json::to_string(entry).map_err(backend_err)?;
        tx.execute(
            "INSERT INTO idempotency_keys (key, entry, expires_at) VALUES (?1, ?2, ?3)",
            params![key, raw, now + ttl_secs as i64],
        )
        .map_err(backend_err)?;
        tx.commit().map_err(backend_err)?;
        Ok(None)
    }

    fn put(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<(), IdempotencyError> {
        let now = (self.clock)() as i64;
        let raw = serde_json::to_string(entry).map_err(backend_err)?;
        self.lock()?
            .execute(
                "INSERT OR REPLACE INTO idempotency_keys (key, entry, expires_at) VALUES (?1, ?2, ?3)",
                params![key, raw, now + ttl_secs as i64],
            )
            .map(|_| ())
            .map_err(backend_err)
    }

    fn remove(&self, key: &str) -> Result<(), IdempotencyError> {
        self.lock()?
            .execute("DELETE FROM idempotency_keys WHERE key = ?1", params![key])
            .map(|_| ())
            .map_err(backend_err)
    }
}

/// In-memory fast path in front of a durable backend.
pub struct TieredBackend<D: IdempotencyBackend> {
    fast: InMemoryBackend,
    durable: D,
}

impl<D: IdempotencyBackend> TieredBackend<D> {
    pub fn new(durable: D) -> Self {
        TieredBackend { fast: InMemoryBackend::new(), durable }
    }
}

impl<D: IdempotencyBackend> IdempotencyBackend for TieredBackend<D> {
    fn try_begin(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<Option<Entry>, IdempotencyError> {
        // Fast path: the in-memory map already knows this key.
        if let Some(hit) = self.fast.try_begin(key, entry, ttl_secs)? {
            return Ok(Some(hit));
        }
        // Miss: the claim just made in memory is provisional; durable decides.
        match self.durable.try_begin(key, entry, ttl_secs) {
            Ok(None) => Ok(None),
            Ok(Some(existing)) => {
                // Mirror only finished results; another process may still abandon an in-progress claim.
                match existing {
                    Entry::Completed { .. } => self.fast.put(key, &existing, ttl_secs)?,
                    Entry::InProgress { .. } => self.fast.remove(key)?,
                }
                Ok(Some(existing))
            }
            Err(e) => {
                self.fast.remove(key)?;
                Err(e)
            }
        }
    }

    fn put(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<(), IdempotencyError> {
        self.durable.put(key, entry, ttl_secs)?;
        self.fast.put(key, entry, ttl_secs)
    }

    fn remove(&self, key: &str) -> Result<(), IdempotencyError> {
        self.durable.remove(key)?;
        self.fast.remove(key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::idempotency::{fingerprint, IdempotencyService, Outcome, Scope, StoredResponse};
    use std::sync::atomic::{AtomicU64, Ordering};

    const KEY: &str = "123e4567-e89b-12d3-a456-426614174000";

    fn stored() -> StoredResponse {
        StoredResponse { status: 201, headers: vec![], body: "created".into() }
    }

    #[test]
    fn retry_after_restart_replays_the_original_response() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("idem.db");
        let db = db.to_str().unwrap();
        let scope = Scope::new("user-1", "POST", "/vaults");
        let fp = fingerprint("POST", "/vaults", b"{}");

        let first = IdempotencyService::new(TieredBackend::new(SqliteBackend::open(db).unwrap()));
        let Outcome::Proceed(claim) = first.begin(&scope, KEY, &fp).unwrap() else { panic!("first must proceed") };
        first.complete(claim, stored()).unwrap();
        drop(first); // "restart": new process, empty memory

        let second = IdempotencyService::new(TieredBackend::new(SqliteBackend::open(db).unwrap()));
        match second.begin(&scope, KEY, &fp).unwrap() {
            Outcome::Replay(resp) => assert_eq!(resp, stored()),
            Outcome::Proceed(_) => panic!("mutation must not repeat after restart"),
        }
    }

    #[test]
    fn same_key_has_one_winner_and_expired_keys_are_reclaimed() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("idem.db");
        let now = Arc::new(AtomicU64::new(1_000));
        let clock = { let now = now.clone(); Arc::new(move || now.load(Ordering::SeqCst)) };
        let backend = SqliteBackend::open(db.to_str().unwrap()).unwrap().with_clock(clock);
        let entry = Entry::InProgress { fingerprint: "fp".into() };

        assert!(backend.try_begin("k", &entry, 60).unwrap().is_none(), "first claim wins");
        assert!(backend.try_begin("k", &entry, 60).unwrap().is_some(), "second claim loses");
        now.store(1_061, Ordering::SeqCst);
        assert!(backend.try_begin("k", &entry, 60).unwrap().is_none(), "expired key is reclaimed");
    }
}
