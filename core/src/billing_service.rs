//! Fee-market / billing service.
//!
//! API-28 (Perigee): Business logic was previously living inside the HTTP
//! controllers in `main.rs`. This service exposes that logic through plain
//! async methods so the Axum handlers only do request → service → response
//! translation and error mapping.
//!
//! API-26 (Perigee): Monetary values use integer minor units (stroops =
//! 1e-7 XLM) end-to-end. Statistical ratios (confidence, volatility,
//! transaction pressure) are stored as basis points (0..=10000) so the
//! representations stay integer/deterministic. No f64 multiplications
//! are performed on fee/NAV amounts.

use crate::errors::AppError;
use crate::fee::analytics::FeeAnalyticsEngine;
use crate::fee::calculation::select_bid;
use crate::fee::persistence::FeeStore;
use crate::fee::validation::{
    safety_margin_to_bps as validate_safety_margin, validate_charge_request,
    validate_safety_margin_bps as validate_margin_bps,
};
pub use crate::fee::calculation::{
    FeeAnalyticsResult, FeeHistoryQuery, FeeHistoryResult, FeeRecommendationInputs,
    FeeRecommendationResult, InclusionSpeed, DEFAULT_SAFETY_MARGIN_BPS, SAFETY_MARGIN_MAX_BPS,
    SAFETY_MARGIN_MIN_BPS,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use utoipa::ToSchema;

// ── BE-021: idempotent fee charges ───────────────────────────────────────────

/// How long a completed charge stays replayable.
///
/// Twenty-four hours, as BE-021 specifies. The window only has to outlast a
/// client's retry budget: a caller that times out and retries does so within
/// seconds or minutes, and anything replayed a day later is a new intent
/// rather than a retry.
pub const IDEMPOTENCY_TTL: chrono::Duration = chrono::Duration::hours(24);

/// A request to charge a fee.
#[derive(Debug, Clone, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
pub struct ChargeRequest {
    /// Caller-supplied key identifying this charge attempt.
    ///
    /// Two requests carrying the same key are the same intent: the second is
    /// a retry of the first, not a second charge.
    pub idempotency_key: String,

    /// Who is being charged.
    pub payer: String,

    /// Amount in stroops (1e-7 XLM), matching the integer minor-unit
    /// convention described at the top of this module.
    pub amount_stroops: i64,

    /// What the charge is for.
    pub reason: String,
}

/// The result of a charge.
#[derive(Debug, Clone, Serialize, Deserialize, ToSchema, PartialEq, Eq)]
pub struct ChargeReceipt {
    /// Identifier derived from the idempotency key, so a replay returns the
    /// same id as the original rather than a new one.
    pub charge_id: String,
    pub payer: String,
    pub amount_stroops: i64,
    pub reason: String,
    pub charged_at: DateTime<Utc>,

    /// True when this receipt was served from a previous identical request
    /// rather than produced by a fresh charge.
    ///
    /// Callers generally do not need this — the point of idempotency is that
    /// they should not have to care — but it makes the behaviour observable
    /// in logs and tests.
    pub replayed: bool,
}

/// An entry retained so a duplicate key can be answered without recharging.
#[derive(Debug, Clone)]
struct StoredCharge {
    receipt: ChargeReceipt,

    /// Fingerprint of the request that produced `receipt`, excluding the key.
    ///
    /// Held so that reusing one key for a *different* charge is refused
    /// rather than silently answered with the wrong receipt — a client that
    /// recycles keys has a bug, and returning someone else's receipt would
    /// hide it.
    fingerprint: String,

    expires_at: DateTime<Utc>,
}

/// In-memory store of completed charges, keyed by idempotency key.
///
/// **Durability:** entries live in this process only. A restart forgets them,
/// so a retry that spans a restart can charge twice. Closing that gap needs a
/// shared store — the `db` module's typed tables are the natural home, and
/// `vault_store` already persists idempotency keys that way. That is a
/// deliberate follow-up rather than an oversight: the mechanism, its TTL and
/// its conflict semantics are settled here, and swapping the backing store
/// does not change them.
#[derive(Debug)]
pub struct IdempotencyStore {
    ttl: chrono::Duration,
    entries: std::sync::Mutex<std::collections::HashMap<String, StoredCharge>>,
}

impl IdempotencyStore {
    pub fn new(ttl: chrono::Duration) -> Self {
        Self {
            ttl,
            entries: std::sync::Mutex::new(std::collections::HashMap::new()),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<String, StoredCharge>> {
        // A poisoned lock means some other caller panicked mid-charge. The
        // map itself is still consistent, and refusing every subsequent charge
        // would turn one failure into an outage.
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Look up a live entry, dropping it if it has expired.
    fn lookup(&self, key: &str, now: DateTime<Utc>) -> Option<StoredCharge> {
        let mut entries = self.lock();

        match entries.get(key) {
            Some(entry) if entry.expires_at > now => Some(entry.clone()),
            Some(_) => {
                // Expired: remove it so the key is reusable.
                entries.remove(key);
                None
            }
            None => None,
        }
    }

    fn store(&self, key: String, fingerprint: String, receipt: ChargeReceipt, now: DateTime<Utc>) {
        self.lock().insert(
            key,
            StoredCharge {
                receipt,
                fingerprint,
                expires_at: now + self.ttl,
            },
        );
    }

    /// Drop every expired entry, returning how many were removed.
    ///
    /// `lookup` already expires entries it touches, but a key that is never
    /// retried would otherwise be retained forever. Call this periodically.
    pub fn prune(&self, now: DateTime<Utc>) -> usize {
        let mut entries = self.lock();
        let before = entries.len();

        entries.retain(|_, entry| entry.expires_at > now);

        before - entries.len()
    }

    /// Number of retained entries, expired ones included until pruned.
    pub fn len(&self) -> usize {
        self.lock().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for IdempotencyStore {
    fn default() -> Self {
        Self::new(IDEMPOTENCY_TTL)
    }
}

/// Fingerprint of a charge request, ignoring the idempotency key.
///
/// Length-prefixed so that field boundaries are unambiguous: without it,
/// `payer="ab", reason="c"` and `payer="a", reason="bc"` would hash alike and
/// a key could be reused across them undetected.
fn charge_fingerprint(req: &ChargeRequest) -> String {
    use sha2::{Digest, Sha256};

    let mut hasher = Sha256::new();

    for field in [req.payer.as_bytes(), req.reason.as_bytes()] {
        hasher.update((field.len() as u64).to_be_bytes());
        hasher.update(field);
    }

    hasher.update(req.amount_stroops.to_be_bytes());

    hex::encode(hasher.finalize())
}

/// Charge id derived from the idempotency key.
///
/// Deterministic on purpose: a replay must return the id the original
/// returned, and deriving it removes any chance of the two drifting.
fn charge_id_for(idempotency_key: &str) -> String {
    use sha2::{Digest, Sha256};

    let digest = Sha256::digest(idempotency_key.as_bytes());

    format!("chg_{}", hex::encode(&digest[..16]))
}

/// Fee business logic. Decoupled from the HTTP/transport layer so it can be
/// reused from CLI subcommands, job runners, JSON-RPC adapters, etc.
pub struct FeeService {
    store: Arc<FeeStore>,
    engine: FeeAnalyticsEngine,

    /// Completed charges, retained so a retry does not charge twice (BE-021).
    idempotency: IdempotencyStore,
}

impl FeeService {
    pub fn new(store: Arc<FeeStore>, engine: FeeAnalyticsEngine) -> Self {
        Self {
            store,
            engine,
            idempotency: IdempotencyStore::default(),
        }
    }

    /// Charge a fee, at most once per idempotency key (BE-021).
    ///
    /// A repeat of a request already seen returns the original receipt
    /// without charging again. Reusing a key for a *different* charge is a
    /// conflict, not a replay.
    pub async fn charge(&self, req: ChargeRequest) -> Result<ChargeReceipt, AppError> {
        self.charge_at(req, Utc::now()).await
    }

    /// [`charge`](Self::charge) against an explicit clock.
    ///
    /// Exists so expiry is testable without waiting a day.
    pub async fn charge_at(
        &self,
        req: ChargeRequest,
        now: DateTime<Utc>,
    ) -> Result<ChargeReceipt, AppError> {
        validate_charge_request(
            &req.idempotency_key,
            &req.payer,
            req.amount_stroops,
        )
        .map_err(|error| AppError::BadRequest(error.to_string()))?;

        let fingerprint = charge_fingerprint(&req);

        if let Some(existing) = self.idempotency.lookup(&req.idempotency_key, now) {
            if existing.fingerprint != fingerprint {
                // Same key, different charge. Answering with the stored
                // receipt would report a charge the caller did not ask for,
                // and charging anyway would defeat the key entirely.
                return Err(AppError::Conflict(format!(
                    "idempotency_key '{}' was already used for a different charge",
                    req.idempotency_key
                )));
            }

            return Ok(ChargeReceipt {
                replayed: true,
                ..existing.receipt
            });
        }

        let receipt = ChargeReceipt {
            charge_id: charge_id_for(&req.idempotency_key),
            payer: req.payer.clone(),
            amount_stroops: req.amount_stroops,
            reason: req.reason.clone(),
            charged_at: now,
            replayed: false,
        };

        self.idempotency.store(
            req.idempotency_key.clone(),
            fingerprint,
            receipt.clone(),
            now,
        );

        Ok(receipt)
    }

    /// Drop expired idempotency entries. Returns how many were removed.
    pub fn prune_idempotency(&self, now: DateTime<Utc>) -> usize {
        self.idempotency.prune(now)
    }

    /// Number of retained idempotency entries.
    pub fn idempotency_len(&self) -> usize {
        self.idempotency.len()
    }

    /// Validate a basis-point safety margin against the supported range.
    pub fn validate_safety_margin_bps(bps: u32) -> Result<u32, AppError> {
        validate_margin_bps(bps).map_err(|error| AppError::BadRequest(error.to_string()))
    }

    /// Backwards-compatible conversion from the legacy `f64` safety margin
    /// (e.g. 1.10 → 11000 bps) used in older API requests.
    pub fn safety_margin_to_bps(safety_margin: f64) -> Result<u32, AppError> {
        validate_safety_margin(safety_margin).map_err(|error| AppError::BadRequest(error.to_string()))
    }

    /// Build a fee recommendation from the most-recent fee samples.
    pub async fn recommend(
        &self,
        inputs: FeeRecommendationInputs,
    ) -> Result<FeeRecommendationResult, AppError> {
        let safety_margin_bps = validate_margin_bps(inputs.safety_margin_bps)
            .map_err(|error| AppError::BadRequest(error.to_string()))?;
        let engine = self.engine.with_safety_margin_bps(safety_margin_bps);
        let samples = self
            .store
            .get_recent_samples(100)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to fetch fee data: {}", e)))?;
        let current_ledger = samples
            .first()
            .map(|s| s.ledger_sequence as u64)
            .unwrap_or(0);

        let prediction = engine.predict(&samples, current_ledger);
        let market_conditions = engine.get_market_conditions(&samples, current_ledger);
        let model_breakdown = engine.get_model_breakdown(&samples);

        let (recommended_bid, expected_inclusion_ledgers) =
            select_bid(&prediction, inputs.inclusion_speed);

        Ok(FeeRecommendationResult {
            recommended_bid,
            resource_fee_estimate: 0,
            total_estimated_cost: recommended_bid,
            inclusion_confidence_bps: prediction.confidence_bps,
            expected_inclusion_ledgers,
            market_conditions,
            model_breakdown,
            timestamp: Utc::now(),
        })
    }

    /// Recent fee samples plus the historical total in the table.
    pub async fn history(&self, query: FeeHistoryQuery) -> Result<FeeHistoryResult, AppError> {
        if let (Some(from), Some(to)) = (query.from_ledger, query.to_ledger) {
            if from > to {
                return Err(AppError::BadRequest(
                    "from_ledger must not exceed to_ledger".to_string(),
                ));
            }
        }
        let limit = query.limit.unwrap_or(50).clamp(1, 1_000);
        let samples = if let (Some(from), Some(to)) = (query.from_ledger, query.to_ledger) {
            self.store
                .get_samples_in_range(from, to)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to fetch fee history: {}", e)))?
        } else {
            self.store
                .get_recent_samples(limit)
                .await
                .map_err(|e| AppError::Internal(format!("Failed to fetch fee history: {}", e)))?
        };

        let total_count = self
            .store
            .get_sample_count()
            .await
            .map_err(|e| AppError::Internal(format!("Failed to get sample count: {}", e)))?;
        Ok(FeeHistoryResult {
            samples,
            total_count,
        })
    }

    /// Composite fee-market analytics (prediction + market + model breakdown).
    pub async fn analytics(&self) -> Result<FeeAnalyticsResult, AppError> {
        let samples = self
            .store
            .get_recent_samples(200)
            .await
            .map_err(|e| AppError::Internal(format!("Failed to fetch fee data: {}", e)))?;
        let current_ledger = samples
            .first()
            .map(|s| s.ledger_sequence as u64)
            .unwrap_or(0);
        let prediction = self.engine.predict(&samples, current_ledger);
        let market_conditions = self.engine.get_market_conditions(&samples, current_ledger);
        let model_breakdown = self.engine.get_model_breakdown(&samples);

        Ok(FeeAnalyticsResult {
            current_ledger,
            prediction,
            market_conditions,
            model_breakdown,
            sample_count: samples.len(),
            timestamp: Utc::now(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_validate_safety_margin_bps_in_range() {
        assert!(FeeService::validate_safety_margin_bps(11_000).is_ok());
        assert!(FeeService::validate_safety_margin_bps(5_000).is_ok());
        assert!(FeeService::validate_safety_margin_bps(50_000).is_ok());
    }

    #[test]
    fn test_validate_safety_margin_bps_out_of_range() {
        assert!(FeeService::validate_safety_margin_bps(4_999).is_err());
        assert!(FeeService::validate_safety_margin_bps(50_001).is_err());
    }

    #[test]
    fn test_safety_margin_to_bps() {
        assert_eq!(FeeService::safety_margin_to_bps(1.10).unwrap(), 11_000);
        assert_eq!(FeeService::safety_margin_to_bps(1.0).unwrap(), 10_000);
        assert_eq!(FeeService::safety_margin_to_bps(2.0).unwrap(), 20_000);
    }

    #[test]
    fn test_safety_margin_to_bps_rejects_invalid() {
        assert!(FeeService::safety_margin_to_bps(0.0).is_err());
        assert!(FeeService::safety_margin_to_bps(-1.0).is_err());
        assert!(FeeService::safety_margin_to_bps(f64::NAN).is_err());
        assert!(FeeService::safety_margin_to_bps(10.0).is_err()); // 100_000 bps > max
    }

    #[test]
    fn test_inclusion_speed_parse() {
        assert_eq!(
            InclusionSpeed::parse(Some("economy")),
            InclusionSpeed::Economy
        );
        assert_eq!(
            InclusionSpeed::parse(Some("priority")),
            InclusionSpeed::Priority
        );
        assert_eq!(
            InclusionSpeed::parse(Some("next_ledger")),
            InclusionSpeed::NextLedger
        );
        assert_eq!(
            InclusionSpeed::parse(Some("next_3_ledgers")),
            InclusionSpeed::Next3Ledgers
        );
        assert_eq!(
            InclusionSpeed::parse(Some("standard")),
            InclusionSpeed::Standard
        );
        // Unknown → Priority (matches existing handler behaviour).
        assert_eq!(
            InclusionSpeed::parse(Some("garbage")),
            InclusionSpeed::Priority
        );
        assert_eq!(InclusionSpeed::parse(None), InclusionSpeed::Priority);
    }
}

#[cfg(test)]
mod idempotency_tests {
    use super::*;

    async fn service() -> FeeService {
        // The charge path touches neither the store nor the engine, but
        // `FeeService::new` needs both, so this builds them against an
        // anonymous in-memory SQLite database.
        let pool = sqlx::SqlitePool::connect("sqlite::memory:")
            .await
            .expect("in-memory sqlite");

        FeeService::new(
            Arc::new(FeeStore::new(crate::db::MonitoredPool::from_inner(pool))),
            FeeAnalyticsEngine::new(),
        )
    }

    fn request(key: &str) -> ChargeRequest {
        ChargeRequest {
            idempotency_key: key.to_string(),
            payer: "GABC".to_string(),
            amount_stroops: 1_000,
            reason: "simulation".to_string(),
        }
    }

    fn at(hours: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + hours * 3_600, 0).expect("valid timestamp")
    }

    #[tokio::test]
    async fn a_first_charge_is_not_a_replay() {
        let svc = service().await;

        let receipt = svc.charge_at(request("k1"), at(0)).await.unwrap();

        assert!(!receipt.replayed);
        assert_eq!(receipt.payer, "GABC");
        assert_eq!(receipt.amount_stroops, 1_000);
        assert_eq!(receipt.charged_at, at(0));
    }

    /// The behaviour BE-021 exists for: a retried request must not charge
    /// twice.
    #[tokio::test]
    async fn a_repeated_key_replays_the_original_receipt() {
        let svc = service().await;

        let first = svc.charge_at(request("k1"), at(0)).await.unwrap();
        let second = svc.charge_at(request("k1"), at(1)).await.unwrap();

        assert!(second.replayed);
        assert_eq!(second.charge_id, first.charge_id);
        assert_eq!(second.amount_stroops, first.amount_stroops);

        // The replay reports when the charge actually happened, not when it
        // was replayed — otherwise a retry would appear to be a fresh charge.
        assert_eq!(second.charged_at, at(0));

        // And only one entry was ever stored.
        assert_eq!(svc.idempotency_len(), 1);
    }

    #[tokio::test]
    async fn different_keys_are_independent_charges() {
        let svc = service().await;

        let a = svc.charge_at(request("k1"), at(0)).await.unwrap();
        let b = svc.charge_at(request("k2"), at(0)).await.unwrap();

        assert!(!a.replayed);
        assert!(!b.replayed);
        assert_ne!(a.charge_id, b.charge_id);
        assert_eq!(svc.idempotency_len(), 2);
    }

    /// Reusing a key for a different charge is a client bug. Replaying the
    /// stored receipt would report a charge they did not ask for; charging
    /// anyway would defeat the key.
    #[tokio::test]
    async fn reusing_a_key_for_a_different_charge_is_a_conflict() {
        let svc = service().await;

        svc.charge_at(request("k1"), at(0)).await.unwrap();

        let mut different = request("k1");
        different.amount_stroops = 9_999;

        let err = svc.charge_at(different, at(0)).await.unwrap_err();

        assert!(matches!(err, AppError::Conflict(_)), "{err:?}");
    }

    #[tokio::test]
    async fn a_changed_payer_is_also_a_conflict() {
        let svc = service().await;

        svc.charge_at(request("k1"), at(0)).await.unwrap();

        let mut different = request("k1");
        different.payer = "GXYZ".to_string();

        assert!(matches!(
            svc.charge_at(different, at(0)).await.unwrap_err(),
            AppError::Conflict(_)
        ));
    }

    /// Field boundaries must be unambiguous, or a key could be reused across
    /// two different charges that happen to concatenate identically.
    #[tokio::test]
    async fn field_boundaries_are_unambiguous_in_the_fingerprint() {
        let a = ChargeRequest {
            idempotency_key: "k".to_string(),
            payer: "ab".to_string(),
            amount_stroops: 1,
            reason: "c".to_string(),
        };
        let b = ChargeRequest {
            payer: "a".to_string(),
            reason: "bc".to_string(),
            ..a.clone()
        };

        assert_ne!(charge_fingerprint(&a), charge_fingerprint(&b));
    }

    // ── TTL ──────────────────────────────────────────────────────────────

    #[tokio::test]
    async fn a_key_still_replays_just_before_the_ttl_expires() {
        let svc = service().await;

        svc.charge_at(request("k1"), at(0)).await.unwrap();

        let just_inside = svc.charge_at(request("k1"), at(23)).await.unwrap();

        assert!(just_inside.replayed);
    }

    /// After the window, the same key is a new charge rather than a replay —
    /// a day later it is a fresh intent, not a retry.
    #[tokio::test]
    async fn a_key_is_reusable_once_the_ttl_has_passed() {
        let svc = service().await;

        let first = svc.charge_at(request("k1"), at(0)).await.unwrap();
        let later = svc.charge_at(request("k1"), at(25)).await.unwrap();

        assert!(!later.replayed);
        assert_eq!(later.charged_at, at(25));

        // The id is derived from the key, so it is stable across the window.
        assert_eq!(later.charge_id, first.charge_id);
    }

    /// An expired key that is never retried would otherwise be retained
    /// forever, so the store has to be prunable.
    #[tokio::test]
    async fn pruning_drops_expired_entries_and_keeps_live_ones() {
        let svc = service().await;

        svc.charge_at(request("old"), at(0)).await.unwrap();
        svc.charge_at(request("new"), at(20)).await.unwrap();
        assert_eq!(svc.idempotency_len(), 2);

        // At hour 25 the first has expired and the second has not.
        let removed = svc.prune_idempotency(at(25));

        assert_eq!(removed, 1);
        assert_eq!(svc.idempotency_len(), 1);
    }

    #[tokio::test]
    async fn the_default_ttl_is_twenty_four_hours() {
        assert_eq!(IDEMPOTENCY_TTL, chrono::Duration::hours(24));
    }

    // ── Validation ───────────────────────────────────────────────────────

    #[tokio::test]
    async fn an_empty_key_is_rejected() {
        let svc = service().await;

        for key in ["", "   "] {
            let err = svc.charge_at(request(key), at(0)).await.unwrap_err();

            assert!(matches!(err, AppError::BadRequest(_)), "{key:?}: {err:?}");
        }
    }

    #[tokio::test]
    async fn a_non_positive_amount_is_rejected() {
        let svc = service().await;

        for amount in [0, -1] {
            let mut req = request("k1");
            req.amount_stroops = amount;

            assert!(matches!(
                svc.charge_at(req, at(0)).await.unwrap_err(),
                AppError::BadRequest(_)
            ));
        }
    }

    #[tokio::test]
    async fn an_empty_payer_is_rejected() {
        let svc = service().await;

        let mut req = request("k1");
        req.payer = "  ".to_string();

        assert!(matches!(
            svc.charge_at(req, at(0)).await.unwrap_err(),
            AppError::BadRequest(_)
        ));
    }

    /// A rejected charge must not consume the key.
    #[tokio::test]
    async fn a_rejected_charge_stores_nothing() {
        let svc = service().await;

        let mut bad = request("k1");
        bad.amount_stroops = -5;

        assert!(svc.charge_at(bad, at(0)).await.is_err());
        assert_eq!(svc.idempotency_len(), 0);

        // The key is still usable for a valid charge.
        assert!(!svc.charge_at(request("k1"), at(0)).await.unwrap().replayed);
    }

    #[tokio::test]
    async fn the_charge_id_is_derived_from_the_key() {
        assert_eq!(charge_id_for("k1"), charge_id_for("k1"));
        assert_ne!(charge_id_for("k1"), charge_id_for("k2"));
        assert!(charge_id_for("k1").starts_with("chg_"));
    }
}
