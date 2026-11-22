//! # Idempotency keys
//!
//! Prevents duplicate processing of mutating requests. A client sends a
//! UUID in the `Idempotency-Key` header; the first request with that key runs,
//! and its response is stored for 24 hours. A repeat of the same request gets
//! the stored response back instead of running again.
//!
//! * **Scope**: keys are namespaced per user *and* per endpoint (method +
//!   path), so the same key may be reused on a different endpoint or by a
//!   different user without interference.
//! * **Collision**: the same key with a *different* request (method, path or
//!   body) is a client bug and is rejected with `409`.
//! * **Concurrency**: the key is claimed atomically before the handler runs
//!   (`SET NX` on Redis), so of N simultaneous requests exactly one proceeds;
//!   the others get `409` while it is in flight and the stored response once
//!   it has finished.
//! * **Storage**: [`RedisBackend`] in production (keys expire through a TTL),
//!   [`InMemoryBackend`] for tests and single-process development.
//!
//! See `docs/IDEMPOTENCY.md` for the API consumer guide.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use thiserror::Error;

/// Request header carrying the client-generated key.
pub const IDEMPOTENCY_HEADER: &str = "Idempotency-Key";
/// How long a key and its response are remembered.
pub const DEFAULT_TTL_SECS: u64 = 24 * 60 * 60;

/// A response captured for replay.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoredResponse {
    pub status: u16,
    pub headers: Vec<(String, String)>,
    pub body: String,
}

impl StoredResponse {
    /// Server errors are transient, so they are not worth replaying.
    pub fn is_cacheable(&self) -> bool {
        self.status < 500
    }
}

/// What the backend remembers about a key.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Entry {
    /// A request claimed the key and is still running.
    InProgress { fingerprint: String },
    /// The request finished; `response` is replayed on repeats.
    Completed {
        fingerprint: String,
        response: StoredResponse,
    },
}

impl Entry {
    fn fingerprint(&self) -> &str {
        match self {
            Entry::InProgress { fingerprint } | Entry::Completed { fingerprint, .. } => fingerprint,
        }
    }
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum IdempotencyError {
    #[error("Idempotency-Key must be a UUID")]
    InvalidKey,
    #[error("Idempotency-Key was already used with a different request")]
    KeyReuse,
    #[error("A request with this Idempotency-Key is still being processed")]
    InProgress,
    #[error("Idempotency store unavailable: {0}")]
    Backend(String),
}

impl IdempotencyError {
    /// HTTP status to answer with.
    pub fn http_status(&self) -> u16 {
        match self {
            IdempotencyError::InvalidKey => 400,
            IdempotencyError::KeyReuse | IdempotencyError::InProgress => 409,
            IdempotencyError::Backend(_) => 503,
        }
    }
}

/// Who and what a key is scoped to.
#[derive(Debug, Clone)]
pub struct Scope {
    pub user_id: String,
    pub method: String,
    pub path: String,
}

impl Scope {
    pub fn new(user_id: &str, method: &str, path: &str) -> Self {
        Scope {
            user_id: user_id.to_string(),
            method: method.to_uppercase(),
            path: path.to_string(),
        }
    }

    /// Storage key. The user id is hashed-free but length-prefixed so that
    /// `("ab", "c")` and `("a", "bc")` cannot produce the same key.
    fn storage_key(&self, key: &str) -> String {
        format!(
            "idem:{}:{}:{}:{}:{}:{}",
            self.user_id.len(),
            self.user_id,
            self.method,
            self.path.len(),
            self.path,
            key
        )
    }
}

/// Fingerprint of the request itself: method, path and body.
pub fn fingerprint(method: &str, path: &str, body: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(method.to_uppercase().as_bytes());
    hasher.update([0]);
    hasher.update(path.as_bytes());
    hasher.update([0]);
    hasher.update(body);
    hex::encode(hasher.finalize())
}

/// Accepts the canonical 8-4-4-4-12 hexadecimal UUID form.
pub fn validate_key(key: &str) -> Result<(), IdempotencyError> {
    let parts: Vec<&str> = key.split('-').collect();
    let lengths = [8usize, 4, 4, 4, 12];
    let ok = parts.len() == lengths.len()
        && parts
            .iter()
            .zip(lengths)
            .all(|(part, len)| part.len() == len && part.chars().all(|c| c.is_ascii_hexdigit()));
    if ok {
        Ok(())
    } else {
        Err(IdempotencyError::InvalidKey)
    }
}

/// Storage for idempotency entries. Implementations must make
/// [`try_begin`](IdempotencyBackend::try_begin) atomic.
pub trait IdempotencyBackend: Send + Sync {
    /// Store `entry` under `key` if the key is free. Returns `None` when the
    /// caller claimed it, or the entry already there.
    fn try_begin(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<Option<Entry>, IdempotencyError>;
    /// Overwrite the entry (used to attach the response), restarting the TTL.
    fn put(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<(), IdempotencyError>;
    /// Forget the key so the client can retry.
    fn remove(&self, key: &str) -> Result<(), IdempotencyError>;
}

type Entries = HashMap<String, (Entry, u64)>;

/// Process-local backend with an injectable clock (seconds).
pub struct InMemoryBackend {
    entries: Mutex<Entries>,
    clock: Arc<dyn Fn() -> u64 + Send + Sync>,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(|| {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0)
        }))
    }

    pub fn with_clock(clock: Arc<dyn Fn() -> u64 + Send + Sync>) -> Self {
        InMemoryBackend {
            entries: Mutex::new(HashMap::new()),
            clock,
        }
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, Entries>, IdempotencyError> {
        self.entries
            .lock()
            .map_err(|_| IdempotencyError::Backend("idempotency store lock poisoned".into()))
    }
}

impl Default for InMemoryBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl IdempotencyBackend for InMemoryBackend {
    fn try_begin(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<Option<Entry>, IdempotencyError> {
        let now = (self.clock)();
        let mut map = self.lock()?;
        if let Some((existing, expires_at)) = map.get(key) {
            if *expires_at > now {
                return Ok(Some(existing.clone()));
            }
        }
        map.insert(key.to_string(), (entry.clone(), now + ttl_secs));
        Ok(None)
    }

    fn put(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<(), IdempotencyError> {
        let now = (self.clock)();
        self.lock()?
            .insert(key.to_string(), (entry.clone(), now + ttl_secs));
        Ok(())
    }

    fn remove(&self, key: &str) -> Result<(), IdempotencyError> {
        self.lock()?.remove(key);
        Ok(())
    }
}

/// Redis-backed store: `SET key value NX EX ttl` claims a key atomically and
/// Redis expires it, so nothing needs sweeping.
pub struct RedisBackend {
    client: redis::Client,
}

impl RedisBackend {
    pub fn new(client: redis::Client) -> Self {
        RedisBackend { client }
    }

    fn connection(&self) -> Result<redis::Connection, IdempotencyError> {
        self.client
            .get_connection()
            .map_err(|e| IdempotencyError::Backend(e.to_string()))
    }
}

fn encode(entry: &Entry) -> Result<String, IdempotencyError> {
    serde_json::to_string(entry).map_err(|e| IdempotencyError::Backend(e.to_string()))
}

fn decode(raw: &str) -> Result<Entry, IdempotencyError> {
    serde_json::from_str(raw).map_err(|e| IdempotencyError::Backend(e.to_string()))
}

impl IdempotencyBackend for RedisBackend {
    fn try_begin(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<Option<Entry>, IdempotencyError> {
        let mut conn = self.connection()?;
        let claimed: Option<String> = redis::cmd("SET")
            .arg(key)
            .arg(encode(entry)?)
            .arg("NX")
            .arg("EX")
            .arg(ttl_secs)
            .query(&mut conn)
            .map_err(|e| IdempotencyError::Backend(e.to_string()))?;
        if claimed.is_some() {
            return Ok(None);
        }
        let existing: Option<String> = redis::cmd("GET")
            .arg(key)
            .query(&mut conn)
            .map_err(|e| IdempotencyError::Backend(e.to_string()))?;
        match existing {
            Some(raw) => decode(&raw).map(Some),
            // Expired between SET and GET: report it as in progress so the
            // client retries rather than risking a double run.
            None => Err(IdempotencyError::InProgress),
        }
    }

    fn put(&self, key: &str, entry: &Entry, ttl_secs: u64) -> Result<(), IdempotencyError> {
        let mut conn = self.connection()?;
        redis::cmd("SET")
            .arg(key)
            .arg(encode(entry)?)
            .arg("EX")
            .arg(ttl_secs)
            .query::<()>(&mut conn)
            .map_err(|e| IdempotencyError::Backend(e.to_string()))
    }

    fn remove(&self, key: &str) -> Result<(), IdempotencyError> {
        let mut conn = self.connection()?;
        redis::cmd("DEL")
            .arg(key)
            .query::<()>(&mut conn)
            .map_err(|e| IdempotencyError::Backend(e.to_string()))
    }
}

/// Proof that the caller owns a key; hand it back to
/// [`IdempotencyService::complete`] or [`IdempotencyService::abandon`].
#[derive(Debug)]
pub struct Claim {
    storage_key: String,
    fingerprint: String,
}

/// Result of [`IdempotencyService::begin`].
#[derive(Debug)]
pub enum Outcome {
    /// First time this key is seen: run the handler, then call `complete`.
    Proceed(Claim),
    /// Already handled: return this response without running the handler.
    Replay(StoredResponse),
}

pub struct IdempotencyService<B: IdempotencyBackend> {
    backend: B,
    ttl_secs: u64,
}

impl<B: IdempotencyBackend> IdempotencyService<B> {
    pub fn new(backend: B) -> Self {
        Self::with_ttl(backend, DEFAULT_TTL_SECS)
    }

    pub fn with_ttl(backend: B, ttl_secs: u64) -> Self {
        IdempotencyService { backend, ttl_secs }
    }

    /// Claim `key` for the request described by `scope` and `fingerprint`.
    pub fn begin(&self, scope: &Scope, key: &str, fingerprint: &str) -> Result<Outcome, IdempotencyError> {
        validate_key(key)?;
        let storage_key = scope.storage_key(key);
        let fresh = Entry::InProgress {
            fingerprint: fingerprint.to_string(),
        };

        match self.backend.try_begin(&storage_key, &fresh, self.ttl_secs)? {
            None => Ok(Outcome::Proceed(Claim {
                storage_key,
                fingerprint: fingerprint.to_string(),
            })),
            Some(existing) if existing.fingerprint() != fingerprint => Err(IdempotencyError::KeyReuse),
            Some(Entry::InProgress { .. }) => Err(IdempotencyError::InProgress),
            Some(Entry::Completed { response, .. }) => Ok(Outcome::Replay(response)),
        }
    }

    /// Record the handler's response. Server errors release the key instead,
    /// so the client may retry and get a fresh attempt.
    pub fn complete(&self, claim: Claim, response: StoredResponse) -> Result<(), IdempotencyError> {
        if !response.is_cacheable() {
            return self.backend.remove(&claim.storage_key);
        }
        let entry = Entry::Completed {
            fingerprint: claim.fingerprint,
            response,
        };
        self.backend.put(&claim.storage_key, &entry, self.ttl_secs)
    }

    /// Release a claim without storing anything (the handler panicked or the
    /// connection dropped).
    pub fn abandon(&self, claim: Claim) -> Result<(), IdempotencyError> {
        self.backend.remove(&claim.storage_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::thread;

    const KEY: &str = "123e4567-e89b-12d3-a456-426614174000";

    fn response(status: u16, body: &str) -> StoredResponse {
        StoredResponse {
            status,
            headers: vec![("content-type".into(), "application/json".into())],
            body: body.into(),
        }
    }

    fn service() -> IdempotencyService<InMemoryBackend> {
        IdempotencyService::new(InMemoryBackend::new())
    }

    fn scope() -> Scope {
        Scope::new("user-1", "post", "/vaults")
    }

    #[test]
    fn first_request_proceeds_and_repeat_replays_the_stored_response() {
        let svc = service();
        let fp = fingerprint("POST", "/vaults", b"{\"a\":1}");

        let Outcome::Proceed(claim) = svc.begin(&scope(), KEY, &fp).unwrap() else {
            panic!("first request must proceed");
        };
        svc.complete(claim, response(201, "created")).unwrap();

        match svc.begin(&scope(), KEY, &fp).unwrap() {
            Outcome::Replay(stored) => assert_eq!(stored, response(201, "created")),
            Outcome::Proceed(_) => panic!("repeat must replay"),
        }
    }

    #[test]
    fn same_key_with_a_different_request_is_a_409_collision() {
        let svc = service();
        let Outcome::Proceed(claim) = svc
            .begin(&scope(), KEY, &fingerprint("POST", "/vaults", b"one"))
            .unwrap()
        else {
            panic!()
        };
        svc.complete(claim, response(201, "ok")).unwrap();

        let err = svc
            .begin(&scope(), KEY, &fingerprint("POST", "/vaults", b"two"))
            .unwrap_err();

        assert_eq!(err, IdempotencyError::KeyReuse);
        assert_eq!(err.http_status(), 409);
    }

    #[test]
    fn a_request_still_running_is_a_409_not_a_second_run() {
        let svc = service();
        let fp = fingerprint("POST", "/vaults", b"x");
        let _claim = svc.begin(&scope(), KEY, &fp).unwrap();

        let err = svc.begin(&scope(), KEY, &fp).unwrap_err();

        assert_eq!(err, IdempotencyError::InProgress);
        assert_eq!(err.http_status(), 409);
    }

    #[test]
    fn keys_are_scoped_per_user_and_per_endpoint() {
        let svc = service();
        let fp = fingerprint("POST", "/vaults", b"x");
        let _held = svc.begin(&scope(), KEY, &fp).unwrap();

        let other_user = Scope::new("user-2", "POST", "/vaults");
        let other_endpoint = Scope::new("user-1", "POST", "/fees");
        assert!(matches!(svc.begin(&other_user, KEY, &fp), Ok(Outcome::Proceed(_))));
        assert!(matches!(svc.begin(&other_endpoint, KEY, &fp), Ok(Outcome::Proceed(_))));
    }

    #[test]
    fn scope_keys_cannot_be_forged_by_shifting_characters_between_fields() {
        let a = Scope::new("ab", "POST", "c").storage_key(KEY);
        let b = Scope::new("a", "POST", "bc").storage_key(KEY);
        assert_ne!(a, b);
    }

    #[test]
    fn invalid_keys_are_rejected_with_400() {
        let svc = service();
        for bad in ["", "not-a-uuid", "123e4567-e89b-12d3-a456-42661417400", "zzze4567-e89b-12d3-a456-426614174000"] {
            let err = svc.begin(&scope(), bad, "fp").unwrap_err();
            assert_eq!(err, IdempotencyError::InvalidKey, "{bad}");
            assert_eq!(err.http_status(), 400);
        }
    }

    #[test]
    fn entries_expire_after_the_ttl() {
        let now = Arc::new(AtomicU64::new(1_000));
        let clock = {
            let now = now.clone();
            Arc::new(move || now.load(Ordering::SeqCst))
        };
        let svc = IdempotencyService::new(InMemoryBackend::with_clock(clock));
        let fp = fingerprint("POST", "/vaults", b"x");

        let Outcome::Proceed(claim) = svc.begin(&scope(), KEY, &fp).unwrap() else {
            panic!()
        };
        svc.complete(claim, response(200, "ok")).unwrap();

        now.store(1_000 + DEFAULT_TTL_SECS - 1, Ordering::SeqCst);
        assert!(matches!(svc.begin(&scope(), KEY, &fp), Ok(Outcome::Replay(_))));

        now.store(1_000 + DEFAULT_TTL_SECS + 1, Ordering::SeqCst);
        assert!(matches!(svc.begin(&scope(), KEY, &fp), Ok(Outcome::Proceed(_))));
    }

    #[test]
    fn server_errors_release_the_key_so_the_client_can_retry() {
        let svc = service();
        let fp = fingerprint("POST", "/vaults", b"x");
        let Outcome::Proceed(claim) = svc.begin(&scope(), KEY, &fp).unwrap() else {
            panic!()
        };
        svc.complete(claim, response(503, "try later")).unwrap();

        assert!(matches!(svc.begin(&scope(), KEY, &fp), Ok(Outcome::Proceed(_))));
    }

    #[test]
    fn client_errors_are_replayed() {
        let svc = service();
        let fp = fingerprint("POST", "/vaults", b"x");
        let Outcome::Proceed(claim) = svc.begin(&scope(), KEY, &fp).unwrap() else {
            panic!()
        };
        svc.complete(claim, response(422, "bad input")).unwrap();

        assert!(matches!(svc.begin(&scope(), KEY, &fp), Ok(Outcome::Replay(r)) if r.status == 422));
    }

    #[test]
    fn abandoning_a_claim_frees_the_key() {
        let svc = service();
        let fp = fingerprint("POST", "/vaults", b"x");
        let Outcome::Proceed(claim) = svc.begin(&scope(), KEY, &fp).unwrap() else {
            panic!()
        };
        svc.abandon(claim).unwrap();

        assert!(matches!(svc.begin(&scope(), KEY, &fp), Ok(Outcome::Proceed(_))));
    }

    #[test]
    fn concurrent_requests_with_one_key_run_exactly_once() {
        let svc = Arc::new(service());
        let fp = fingerprint("POST", "/vaults", b"x");
        let runs = Arc::new(AtomicU64::new(0));

        let handles: Vec<_> = (0..16)
            .map(|_| {
                let (svc, fp, runs) = (svc.clone(), fp.clone(), runs.clone());
                thread::spawn(move || match svc.begin(&scope(), KEY, &fp) {
                    Ok(Outcome::Proceed(claim)) => {
                        runs.fetch_add(1, Ordering::SeqCst);
                        svc.complete(claim, response(201, "done")).unwrap();
                    }
                    Ok(Outcome::Replay(_)) | Err(IdempotencyError::InProgress) => {}
                    Err(other) => panic!("unexpected: {other}"),
                })
            })
            .collect();
        for handle in handles {
            handle.join().unwrap();
        }

        assert_eq!(runs.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn fingerprint_depends_on_method_path_and_body() {
        let base = fingerprint("POST", "/a", b"body");
        assert_eq!(base, fingerprint("post", "/a", b"body"));
        assert_ne!(base, fingerprint("PUT", "/a", b"body"));
        assert_ne!(base, fingerprint("POST", "/b", b"body"));
        assert_ne!(base, fingerprint("POST", "/a", b"other"));
    }
}
