//! Tamper-evident audit log (BE-026).
//!
//! Administrative actions were recorded with `tracing::info!` and nothing
//! else. Anyone able to edit the log store could alter or delete a record and
//! leave no trace, which is the gap BE-026 describes.
//!
//! # How entries are protected
//!
//! Every entry carries the hash of the one before it, and each hash is an
//! HMAC-SHA256 over the previous hash plus the entry's own fields:
//!
//! ```text
//! entry_hash[n] = HMAC(key, entry_hash[n-1] || fields(entry[n]))
//! entry_hash[0] = HMAC(key, GENESIS_HASH   || fields(entry[0]))
//! ```
//!
//! Two properties follow. Chaining means altering entry *n* invalidates every
//! entry after it, so a tamperer must rewrite the whole tail rather than one
//! row. Keying means they cannot rewrite it at all without the signing key —
//! a plain SHA-256 chain is recomputable by anyone holding the data, which is
//! precisely the attacker this is defending against.
//!
//! HMAC rather than a bare `SHA256(key || message)`: SHA-256 is
//! length-extendable, so the naive keyed construction lets an attacker append
//! to a signed message without the key.
//!
//! # Field encoding
//!
//! Fields are length-prefixed before hashing. Plain concatenation is
//! ambiguous — `manager_id="ab", action="c"` and `manager_id="a", action="bc"`
//! would otherwise produce identical input and therefore identical hashes,
//! letting one be swapped for the other undetected.
//!
//! # Key management
//!
//! [`AuditChain::new`] takes the key as bytes, so the process can source it
//! from anywhere. [`signing_key_from_env`] reads `AUDIT_LOG_SIGNING_KEY` (hex)
//! for local and test use.
//!
//! **In production the key belongs in a KMS or HSM**, fetched at startup and
//! never written to disk. This module deliberately does not read files or call
//! a cloud API — it accepts bytes, which keeps the retrieval mechanism a
//! deployment concern rather than a hard-coded one. An unkeyed chain (no key
//! configured) still detects accidental corruption and reordering, but not a
//! determined edit; [`AuditChain::is_keyed`] reports which mode is in use.

use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use std::collections::HashMap;
use std::fmt;
use std::sync::{Mutex, OnceLock};
use thiserror::Error;
use tracing::{info, warn};

type HmacSha256 = Hmac<Sha256>;

/// The `prev_hash` of the first entry: 32 zero bytes, hex-encoded.
pub const GENESIS_HASH: &str = "0000000000000000000000000000000000000000000000000000000000000000";

/// Environment variable holding the hex-encoded signing key.
pub const SIGNING_KEY_ENV: &str = "AUDIT_LOG_SIGNING_KEY";

/// Security-sensitive event types for audit logging and security monitoring (Issue #398).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SecurityEventType {
    LoginSuccess,
    LoginFailed,
    TokenRefreshed,
    TokenRevoked,
    TokenExpired,
    UnauthorizedAccess,
    VaultAccessDenied,
    AgentDisabled,
    JwtKeyRotated,
}

impl SecurityEventType {
    pub fn as_str(&self) -> &'static str {
        match self {
            SecurityEventType::LoginSuccess => "LOGIN_SUCCESS",
            SecurityEventType::LoginFailed => "LOGIN_FAILED",
            SecurityEventType::TokenRefreshed => "TOKEN_REFRESHED",
            SecurityEventType::TokenRevoked => "TOKEN_REVOKED",
            SecurityEventType::TokenExpired => "TOKEN_EXPIRED",
            SecurityEventType::UnauthorizedAccess => "UNAUTHORIZED_ACCESS",
            SecurityEventType::VaultAccessDenied => "VAULT_ACCESS_DENIED",
            SecurityEventType::AgentDisabled => "AGENT_DISABLED",
            SecurityEventType::JwtKeyRotated => "JWT_KEY_ROTATED",
        }
    }
}

impl fmt::Display for SecurityEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Structured security audit event matching Issue #398 specification.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SecurityAuditEvent {
    pub event: SecurityEventType,
    #[serde(rename = "agentId", skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(rename = "vaultId", skip_serializing_if = "Option::is_none")]
    pub vault_id: Option<String>,
    pub timestamp: DateTime<Utc>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ip: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<HashMap<String, String>>,
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum AuditChainError {
    #[error("audit entry {sequence} has hash {found}, expected {expected}")]
    HashMismatch {
        sequence: u64,
        expected: String,
        found: String,
    },

    #[error("audit entry {sequence} links to previous hash {found}, expected {expected}")]
    BrokenLink {
        sequence: u64,
        expected: String,
        found: String,
    },

    #[error("audit entry at position {position} has sequence {sequence}, expected {expected}")]
    SequenceGap {
        position: usize,
        sequence: u64,
        expected: u64,
    },
}

/// One recorded administrative action.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AuditEvent {
    /// Position in the chain, starting at 0.
    pub sequence: u64,
    pub manager_id: String,
    pub action: String,
    pub actor: String,
    pub timestamp: DateTime<Utc>,
    /// Hash of the preceding entry, or [`GENESIS_HASH`] for the first.
    pub prev_hash: String,
    /// HMAC over `prev_hash` and this entry's fields.
    pub entry_hash: String,
}

/// An append-only chain of audit entries.
pub struct AuditChain {
    entries: Vec<AuditEvent>,
    key: Option<Vec<u8>>,
}

impl AuditChain {
    /// Create an empty chain signed with `key`.
    ///
    /// `None` produces an unkeyed chain — tamper-*evident* against accidental
    /// corruption, but not against an attacker who can recompute hashes.
    pub fn new(key: Option<Vec<u8>>) -> Self {
        Self {
            entries: Vec::new(),
            key,
        }
    }

    /// Whether this chain is signed with a key.
    pub fn is_keyed(&self) -> bool {
        self.key.is_some()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entries(&self) -> &[AuditEvent] {
        &self.entries
    }

    /// The hash of the most recent entry, or [`GENESIS_HASH`] when empty.
    pub fn head_hash(&self) -> String {
        self.entries
            .last()
            .map(|e| e.entry_hash.clone())
            .unwrap_or_else(|| GENESIS_HASH.to_string())
    }

    /// Append an entry and return it.
    pub fn append(
        &mut self,
        manager_id: &str,
        action: &str,
        actor: &str,
        timestamp: DateTime<Utc>,
    ) -> &AuditEvent {
        let sequence = self.entries.len() as u64;
        let prev_hash = self.head_hash();

        let entry_hash = Self::compute_hash(
            self.key.as_deref(),
            &prev_hash,
            sequence,
            manager_id,
            action,
            actor,
            timestamp,
        );

        self.entries.push(AuditEvent {
            sequence,
            manager_id: manager_id.to_string(),
            action: action.to_string(),
            actor: actor.to_string(),
            timestamp,
            prev_hash,
            entry_hash,
        });

        self.entries.last().expect("an entry was just pushed")
    }

    /// Verify the whole chain.
    ///
    /// Checks three things per entry: that sequence numbers are contiguous
    /// from zero (so a deleted entry is caught, not just a modified one), that
    /// each `prev_hash` matches the actual predecessor, and that each
    /// `entry_hash` still matches a recomputation of its own contents.
    pub fn verify(&self) -> Result<(), AuditChainError> {
        let mut expected_prev = GENESIS_HASH.to_string();

        for (position, entry) in self.entries.iter().enumerate() {
            let expected_sequence = position as u64;

            if entry.sequence != expected_sequence {
                return Err(AuditChainError::SequenceGap {
                    position,
                    sequence: entry.sequence,
                    expected: expected_sequence,
                });
            }

            if entry.prev_hash != expected_prev {
                return Err(AuditChainError::BrokenLink {
                    sequence: entry.sequence,
                    expected: expected_prev,
                    found: entry.prev_hash.clone(),
                });
            }

            let recomputed = Self::compute_hash(
                self.key.as_deref(),
                &entry.prev_hash,
                entry.sequence,
                &entry.manager_id,
                &entry.action,
                &entry.actor,
                entry.timestamp,
            );

            if recomputed != entry.entry_hash {
                return Err(AuditChainError::HashMismatch {
                    sequence: entry.sequence,
                    expected: recomputed,
                    found: entry.entry_hash.clone(),
                });
            }

            expected_prev = entry.entry_hash.clone();
        }

        Ok(())
    }

    /// HMAC over the previous hash and this entry's fields.
    ///
    /// An unkeyed chain uses an empty key, which is still a well-defined HMAC
    /// — just not a secret one.
    fn compute_hash(
        key: Option<&[u8]>,
        prev_hash: &str,
        sequence: u64,
        manager_id: &str,
        action: &str,
        actor: &str,
        timestamp: DateTime<Utc>,
    ) -> String {
        let mut mac = HmacSha256::new_from_slice(key.unwrap_or(&[]))
            .expect("HMAC accepts keys of any length");

        // Length-prefix every field so no two different field splits can
        // produce the same input. See the module docs.
        let mut absorb = |bytes: &[u8]| {
            mac.update(&(bytes.len() as u64).to_be_bytes());
            mac.update(bytes);
        };

        absorb(prev_hash.as_bytes());
        absorb(&sequence.to_be_bytes());
        absorb(manager_id.as_bytes());
        absorb(action.as_bytes());
        absorb(actor.as_bytes());
        absorb(timestamp.to_rfc3339().as_bytes());

        hex::encode(mac.finalize().into_bytes())
    }
}

/// Read the signing key from [`SIGNING_KEY_ENV`], if set and valid hex.
///
/// A malformed key is a configuration error worth surfacing loudly rather than
/// silently degrading to an unkeyed chain, so it is logged at warn level.
pub fn signing_key_from_env() -> Option<Vec<u8>> {
    let raw = std::env::var(SIGNING_KEY_ENV).ok()?;

    match hex::decode(raw.trim()) {
        Ok(key) if !key.is_empty() => Some(key),
        Ok(_) => {
            warn!(
                target: "audit_log",
                "{SIGNING_KEY_ENV} is set but empty; audit chain will be unkeyed"
            );
            None
        }
        Err(e) => {
            warn!(
                target: "audit_log",
                error = %e,
                "{SIGNING_KEY_ENV} is not valid hex; audit chain will be unkeyed"
            );
            None
        }
    }
}

/// Process-wide chain backing [`log_audit_event`].
fn global_chain() -> &'static Mutex<AuditChain> {
    static CHAIN: OnceLock<Mutex<AuditChain>> = OnceLock::new();

    CHAIN.get_or_init(|| Mutex::new(AuditChain::new(signing_key_from_env())))
}

/// Record an administrative action.
///
/// Appends to the process-wide chain and emits the entry through `tracing`,
/// including its hash and link so a log aggregator holds enough to verify the
/// chain independently of the process that wrote it.
pub fn log_audit_event(manager_id: &str, action: &str, actor: &str) {
    let mut chain = global_chain()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let event = chain.append(manager_id, action, actor, Utc::now()).clone();

    let redactor = crate::log_redaction::LogRedactor::new("perigee-log-redaction");
    let redacted_manager = redactor.redact_address(&event.manager_id);
    let redacted_actor = redactor.redact_address(&event.actor);
    info!(
        target: "audit_log",
        sequence = event.sequence,
        manager_id = %redacted_manager,
        action = %event.action,
        actor = %redacted_actor,
        timestamp = %event.timestamp.to_rfc3339(),
        prev_hash = %event.prev_hash,
        entry_hash = %event.entry_hash,
        "AUDIT: {} by {}",
        event.action,
        redacted_actor
    );
}

/// Record a security-sensitive event with structured JSON logging and tamper-evident chaining.
///
/// This records events such as:
/// - `LOGIN_SUCCESS`
/// - `LOGIN_FAILED`
/// - `TOKEN_REFRESHED`
/// - `TOKEN_REVOKED`
/// - `TOKEN_EXPIRED`
/// - `UNAUTHORIZED_ACCESS`
/// - `VAULT_ACCESS_DENIED`
/// - `AGENT_DISABLED`
/// - `JWT_KEY_ROTATED`
///
/// Output format matches:
/// `{"event": "VAULT_ACCESS_DENIED","agentId": "...","vaultId": "...","timestamp": "...","ip": "...","reason": "..."}`
pub fn log_security_event(
    event: SecurityEventType,
    agent_id: Option<&str>,
    vault_id: Option<&str>,
    ip: Option<&str>,
    reason: Option<&str>,
) -> SecurityAuditEvent {
    let now = Utc::now();

    let manager_id = agent_id.unwrap_or("system");
    let action = event.as_str();
    let actor = ip.unwrap_or(agent_id.unwrap_or("unknown"));

    let mut chain = global_chain()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());

    let chain_entry = chain.append(manager_id, action, actor, now.clone()).clone();

    let sec_event = SecurityAuditEvent {
        event,
        agent_id: agent_id.map(|s| s.to_string()),
        vault_id: vault_id.map(|s| s.to_string()),
        timestamp: now,
        ip: ip.map(|s| s.to_string()),
        reason: reason.map(|s| s.to_string()),
        metadata: None,
    };

    let redactor = crate::log_redaction::LogRedactor::new("perigee-log-redaction");
    let redacted_agent_id = sec_event
        .agent_id
        .as_deref()
        .map(|value| redactor.redact_address(value));
    let redacted_vault_id = sec_event
        .vault_id
        .as_deref()
        .map(|value| redactor.redact_vault_id(value));
    let redacted_ip = sec_event.ip.as_deref().map(|value| redactor.redact_address(value));
    let redacted_reason = sec_event
        .reason
        .as_deref()
        .map(crate::log_redaction::redact_sensitive_text);

    info!(
        target: "audit_log",
        sequence = chain_entry.sequence,
        event = %sec_event.event.as_str(),
        agent_id = ?redacted_agent_id,
        vault_id = ?redacted_vault_id,
        timestamp = %sec_event.timestamp.to_rfc3339(),
        ip = ?redacted_ip,
        reason = ?redacted_reason,
        prev_hash = %chain_entry.prev_hash,
        entry_hash = %chain_entry.entry_hash,
        "SECURITY_AUDIT: {}",
        sec_event.event.as_str()
    );

    sec_event
}

/// Record a security-sensitive event and optionally increment Prometheus security metrics.
pub fn log_security_event_with_metrics(
    event: SecurityEventType,
    agent_id: Option<&str>,
    vault_id: Option<&str>,
    ip: Option<&str>,
    reason: Option<&str>,
    status: Option<&str>,
    metrics: Option<&crate::metrics::Metrics>,
) -> SecurityAuditEvent {
    let sec_event = log_security_event(event, agent_id, vault_id, ip, reason);

    if let Some(metrics) = metrics {
        let status_label = status.unwrap_or(match event {
            SecurityEventType::LoginSuccess
            | SecurityEventType::TokenRefreshed
            | SecurityEventType::TokenRevoked
            | SecurityEventType::AgentDisabled
            | SecurityEventType::JwtKeyRotated => "success",
            SecurityEventType::LoginFailed
            | SecurityEventType::TokenExpired
            | SecurityEventType::UnauthorizedAccess
            | SecurityEventType::VaultAccessDenied => "denied",
        });
        metrics
            .security_audit_events_total
            .with_label_values(&[event.as_str(), status_label])
            .inc();
    }

    sec_event
}

/// Verify the process-wide chain.
pub fn verify_global_chain() -> Result<(), AuditChainError> {
    global_chain()
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
        .verify()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000 + seconds, 0).expect("valid timestamp")
    }

    fn chain_of(n: usize) -> AuditChain {
        let mut chain = AuditChain::new(Some(b"test-key".to_vec()));

        for i in 0..n {
            chain.append("mgr-1", "vault_update", "actor-1", at(i as i64));
        }

        chain
    }

    #[test]
    fn an_empty_chain_verifies_and_heads_at_genesis() {
        let chain = AuditChain::new(Some(b"k".to_vec()));

        assert!(chain.is_empty());
        assert_eq!(chain.head_hash(), GENESIS_HASH);
        assert_eq!(chain.verify(), Ok(()));
    }

    #[test]
    fn the_first_entry_links_to_genesis() {
        let chain = chain_of(1);

        assert_eq!(chain.entries()[0].sequence, 0);
        assert_eq!(chain.entries()[0].prev_hash, GENESIS_HASH);
        assert_eq!(chain.verify(), Ok(()));
    }

    #[test]
    fn each_entry_links_to_its_predecessor() {
        let chain = chain_of(5);

        for pair in chain.entries().windows(2) {
            assert_eq!(pair[1].prev_hash, pair[0].entry_hash);
        }

        assert_eq!(chain.head_hash(), chain.entries()[4].entry_hash);
        assert_eq!(chain.verify(), Ok(()));
    }

    #[test]
    fn hashing_is_deterministic_for_identical_input() {
        let mut a = AuditChain::new(Some(b"same-key".to_vec()));
        let mut b = AuditChain::new(Some(b"same-key".to_vec()));

        a.append("mgr", "act", "who", at(0));
        b.append("mgr", "act", "who", at(0));

        assert_eq!(a.head_hash(), b.head_hash());
    }

    #[test]
    fn a_different_key_produces_a_different_hash() {
        let mut a = AuditChain::new(Some(b"key-one".to_vec()));
        let mut b = AuditChain::new(Some(b"key-two".to_vec()));

        a.append("mgr", "act", "who", at(0));
        b.append("mgr", "act", "who", at(0));

        assert_ne!(a.head_hash(), b.head_hash());
    }

    /// The core requirement: editing a stored record must be detectable.
    #[test]
    fn tampering_with_a_field_is_detected() {
        let mut chain = chain_of(3);

        chain.entries[1].actor = "attacker".to_string();

        match chain.verify() {
            Err(AuditChainError::HashMismatch { sequence, .. }) => assert_eq!(sequence, 1),
            other => panic!("expected HashMismatch at sequence 1, got {other:?}"),
        }
    }

    #[test]
    fn tampering_with_a_timestamp_is_detected() {
        let mut chain = chain_of(3);

        chain.entries[2].timestamp = at(9_999);

        assert!(matches!(
            chain.verify(),
            Err(AuditChainError::HashMismatch { sequence: 2, .. })
        ));
    }

    /// Rewriting one entry's hash to match its edited contents is not enough:
    /// the next entry still points at the old hash.
    #[test]
    fn recomputing_one_hash_without_the_rest_breaks_the_link() {
        let mut chain = chain_of(3);

        chain.entries[0].action = "vault_drain".to_string();
        chain.entries[0].entry_hash = AuditChain::compute_hash(
            Some(b"test-key"),
            &chain.entries[0].prev_hash,
            chain.entries[0].sequence,
            &chain.entries[0].manager_id,
            &chain.entries[0].action,
            &chain.entries[0].actor,
            chain.entries[0].timestamp,
        );

        // Entry 0 now hashes correctly, so the chain link is what catches it.
        assert!(matches!(
            chain.verify(),
            Err(AuditChainError::BrokenLink { sequence: 1, .. })
        ));
    }

    /// Deleting a record is the other half of tampering, and a pure hash
    /// chain would miss a deletion at the tail. Contiguous sequence numbers
    /// are what catch a removal anywhere in the chain.
    #[test]
    fn deleting_an_entry_is_detected() {
        let mut chain = chain_of(4);

        chain.entries.remove(2);

        assert!(matches!(
            chain.verify(),
            Err(AuditChainError::SequenceGap { position: 2, .. })
        ));
    }

    #[test]
    fn reordering_entries_is_detected() {
        let mut chain = chain_of(4);

        chain.entries.swap(1, 2);

        assert!(chain.verify().is_err());
    }

    /// Without the key, an attacker cannot forge a hash that verifies.
    #[test]
    fn tampering_cannot_be_covered_up_without_the_key() {
        let mut chain = chain_of(2);

        chain.entries[0].action = "vault_drain".to_string();
        chain.entries[0].entry_hash = AuditChain::compute_hash(
            Some(b"wrong-key"),
            &chain.entries[0].prev_hash,
            chain.entries[0].sequence,
            &chain.entries[0].manager_id,
            &chain.entries[0].action,
            &chain.entries[0].actor,
            chain.entries[0].timestamp,
        );

        assert!(chain.verify().is_err());
    }

    /// Length-prefixing matters: without it these two entries would hash the
    /// same input and become interchangeable.
    #[test]
    fn field_boundaries_are_unambiguous() {
        let mut a = AuditChain::new(Some(b"k".to_vec()));
        let mut b = AuditChain::new(Some(b"k".to_vec()));

        a.append("ab", "c", "actor", at(0));
        b.append("a", "bc", "actor", at(0));

        assert_ne!(a.head_hash(), b.head_hash());
    }

    #[test]
    fn an_unkeyed_chain_still_verifies_and_reports_itself() {
        let mut chain = AuditChain::new(None);
        chain.append("mgr", "act", "who", at(0));

        assert!(!chain.is_keyed());
        assert_eq!(chain.verify(), Ok(()));

        chain.entries[0].actor = "someone-else".to_string();
        assert!(chain.verify().is_err());
    }

    #[test]
    fn sequence_numbers_start_at_zero_and_increment() {
        let chain = chain_of(3);

        let sequences: Vec<u64> = chain.entries().iter().map(|e| e.sequence).collect();
        assert_eq!(sequences, vec![0, 1, 2]);
    }

    /// RFC 4231 test case 1, proving the HMAC wiring itself is correct rather
    /// than merely self-consistent.
    #[test]
    fn hmac_matches_the_rfc_4231_vector() {
        let mut mac = HmacSha256::new_from_slice(&[0x0b; 20]).unwrap();
        mac.update(b"Hi There");

        assert_eq!(
            hex::encode(mac.finalize().into_bytes()),
            "b0344c61d8db38535ca8afceaf0bf12b881dc200c9833da726e9376c2e32cff7"
        );
    }

    #[test]
    fn test_security_event_types_formatting_and_str() {
        assert_eq!(SecurityEventType::LoginSuccess.as_str(), "LOGIN_SUCCESS");
        assert_eq!(SecurityEventType::LoginFailed.as_str(), "LOGIN_FAILED");
        assert_eq!(SecurityEventType::TokenRefreshed.as_str(), "TOKEN_REFRESHED");
        assert_eq!(SecurityEventType::TokenRevoked.as_str(), "TOKEN_REVOKED");
        assert_eq!(SecurityEventType::TokenExpired.as_str(), "TOKEN_EXPIRED");
        assert_eq!(SecurityEventType::UnauthorizedAccess.as_str(), "UNAUTHORIZED_ACCESS");
        assert_eq!(SecurityEventType::VaultAccessDenied.as_str(), "VAULT_ACCESS_DENIED");
        assert_eq!(SecurityEventType::AgentDisabled.as_str(), "AGENT_DISABLED");
        assert_eq!(SecurityEventType::JwtKeyRotated.as_str(), "JWT_KEY_ROTATED");
    }

    #[test]
    fn test_log_security_event_serialization() {
        let event = log_security_event(
            SecurityEventType::VaultAccessDenied,
            Some("agent-123"),
            Some("vault-456"),
            Some("192.168.1.100"),
            Some("Agent not authorized for vault"),
        );

        assert_eq!(event.event, SecurityEventType::VaultAccessDenied);
        assert_eq!(event.agent_id.as_deref(), Some("agent-123"));
        assert_eq!(event.vault_id.as_deref(), Some("vault-456"));
        assert_eq!(event.ip.as_deref(), Some("192.168.1.100"));
        assert_eq!(event.reason.as_deref(), Some("Agent not authorized for vault"));

        let json = serde_json::to_string(&event).unwrap();
        assert!(json.contains(r#""event":"VAULT_ACCESS_DENIED""#));
        assert!(json.contains(r#""agentId":"agent-123""#));
        assert!(json.contains(r#""vaultId":"vault-456""#));
        assert!(json.contains(r#""ip":"192.168.1.100""#));
        assert!(json.contains(r#""reason":"Agent not authorized for vault""#));
        assert!(json.contains(r#""timestamp":""#));
    }

    #[test]
    fn test_log_security_event_with_metrics() {
        let metrics = crate::metrics::Metrics::new().unwrap();
        let _ = log_security_event_with_metrics(
            SecurityEventType::LoginSuccess,
            Some("GBTEST..."),
            None,
            Some("127.0.0.1"),
            None,
            None,
            Some(&metrics),
        );
        let metric_count = metrics
            .security_audit_events_total
            .with_label_values(&["LOGIN_SUCCESS", "success"])
            .get();
        assert_eq!(metric_count, 1);
    }
}
