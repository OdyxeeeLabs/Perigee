//! Session management, TOTP-based MFA and attribute-based access control (ABAC).
//!
//! Complements `auth` (RS256 access tokens, rotating hashed refresh tokens, RBAC):
//! - [`SessionStore`] tracks per-device sessions and supports listing/revocation.
//! - [`totp_code`] / [`verify_totp`] implement RFC 6238 TOTP (HMAC-SHA256).
//! - [`AbacContext`] / [`authorize`] add vault-ownership and tier attributes on top of roles.

use hmac::{Hmac, Mac};
use rand::RngCore;
use sha2::Sha256;
use std::collections::HashMap;
use std::sync::RwLock;
use std::time::{SystemTime, UNIX_EPOCH};
use subtle::ConstantTimeEq;

const TOTP_STEP_SECS: u64 = 30;
const TOTP_DIGITS: u32 = 6;
const BASE32_ALPHABET: &[u8; 32] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// Device metadata recorded when a session is created.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeviceInfo {
    pub user_agent: String,
    pub ip_address: String,
}

/// A tracked login session.
#[derive(Clone, Debug)]
pub struct Session {
    pub id: String,
    pub subject: String,
    pub device: DeviceInfo,
    pub created_at: u64,
    pub last_seen_at: u64,
    pub expires_at: u64,
    pub mfa_verified: bool,
    pub revoked: bool,
}

impl Session {
    pub fn is_active(&self, now: u64) -> bool {
        !self.revoked && now < self.expires_at
    }
}

/// In-memory session registry keyed by session id.
#[derive(Default)]
pub struct SessionStore {
    sessions: RwLock<HashMap<String, Session>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn create(
        &self,
        subject: &str,
        device: DeviceInfo,
        ttl_secs: u64,
        mfa_verified: bool,
    ) -> Session {
        let mut bytes = [0u8; 16];
        rand::thread_rng().fill_bytes(&mut bytes);
        let now = now_secs();
        let session = Session {
            id: hex::encode(bytes),
            subject: subject.to_string(),
            device,
            created_at: now,
            last_seen_at: now,
            expires_at: now + ttl_secs,
            mfa_verified,
            revoked: false,
        };
        let mut map = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        map.retain(|_, s| s.is_active(now) || s.revoked);
        map.insert(session.id.clone(), session.clone());
        session
    }

    /// Return the session if it exists and is still active, updating `last_seen_at`.
    pub fn touch(&self, session_id: &str) -> Option<Session> {
        let now = now_secs();
        let mut map = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        let session = map.get_mut(session_id)?;
        if !session.is_active(now) {
            return None;
        }
        session.last_seen_at = now;
        Some(session.clone())
    }

    /// Active sessions (one per device) for a subject.
    pub fn list_for_subject(&self, subject: &str) -> Vec<Session> {
        let now = now_secs();
        let map = self.sessions.read().unwrap_or_else(|e| e.into_inner());
        let mut out: Vec<Session> = map
            .values()
            .filter(|s| s.subject == subject && s.is_active(now))
            .cloned()
            .collect();
        out.sort_by_key(|s| s.created_at);
        out
    }

    /// Revoke one session; only the owning subject may revoke it. Returns true if revoked.
    pub fn revoke(&self, subject: &str, session_id: &str) -> bool {
        let mut map = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        match map.get_mut(session_id) {
            Some(s) if s.subject == subject && !s.revoked => {
                s.revoked = true;
                true
            }
            _ => false,
        }
    }

    /// Revoke every session of a subject; returns how many were revoked.
    pub fn revoke_all(&self, subject: &str) -> usize {
        let mut map = self.sessions.write().unwrap_or_else(|e| e.into_inner());
        let mut count = 0;
        for s in map.values_mut().filter(|s| s.subject == subject && !s.revoked) {
            s.revoked = true;
            count += 1;
        }
        count
    }
}

/// Generate a random base32 TOTP secret (160 bits).
pub fn generate_totp_secret() -> String {
    let mut bytes = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    base32_encode(&bytes)
}

fn base32_encode(data: &[u8]) -> String {
    let mut out = String::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for &b in data {
        buffer = (buffer << 8) | b as u32;
        bits += 8;
        while bits >= 5 {
            out.push(BASE32_ALPHABET[((buffer >> (bits - 5)) & 31) as usize] as char);
            bits -= 5;
        }
    }
    if bits > 0 {
        out.push(BASE32_ALPHABET[((buffer << (5 - bits)) & 31) as usize] as char);
    }
    out
}

fn base32_decode(s: &str) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    let (mut buffer, mut bits) = (0u32, 0u32);
    for c in s.trim_end_matches('=').bytes() {
        let v = BASE32_ALPHABET.iter().position(|&a| a == c.to_ascii_uppercase())? as u32;
        buffer = (buffer << 5) | v;
        bits += 5;
        if bits >= 8 {
            out.push((buffer >> (bits - 8)) as u8);
            bits -= 8;
            buffer &= (1 << bits) - 1;
        }
    }
    Some(out)
}

/// Compute the TOTP code for `secret_b32` at unix time `at` (RFC 6238, HMAC-SHA256).
pub fn totp_code(secret_b32: &str, at: u64) -> Option<String> {
    let key = base32_decode(secret_b32)?;
    if key.is_empty() {
        return None;
    }
    let counter = (at / TOTP_STEP_SECS).to_be_bytes();
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).ok()?;
    mac.update(&counter);
    let digest = mac.finalize().into_bytes();
    let offset = (digest[digest.len() - 1] & 0x0f) as usize;
    let bin = u32::from_be_bytes([
        digest[offset] & 0x7f,
        digest[offset + 1],
        digest[offset + 2],
        digest[offset + 3],
    ]);
    Some(format!(
        "{:0width$}",
        bin % 10u32.pow(TOTP_DIGITS),
        width = TOTP_DIGITS as usize
    ))
}

/// Verify a TOTP code, allowing one step of clock skew either side.
pub fn verify_totp(secret_b32: &str, code: &str, at: u64) -> bool {
    (-1i64..=1).any(|skew| {
        let t = at as i64 + skew * TOTP_STEP_SECS as i64;
        t >= 0
            && totp_code(secret_b32, t as u64)
                .is_some_and(|expected| expected.as_bytes().ct_eq(code.as_bytes()).into())
    })
}

/// Subscription tier attribute used by ABAC policies.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Tier {
    Free,
    Pro,
    Enterprise,
}

/// Attributes of the subject and target resource evaluated by [`authorize`].
#[derive(Clone, Debug)]
pub struct AbacContext<'a> {
    pub subject: &'a str,
    pub is_admin: bool,
    pub tier: Tier,
    pub vault_owner: &'a str,
    pub required_tier: Tier,
    pub mfa_verified: bool,
    pub require_mfa: bool,
}

/// ABAC decision: admins pass; others must own the vault, meet the tier, and satisfy MFA.
pub fn authorize(ctx: &AbacContext<'_>) -> Result<(), &'static str> {
    if ctx.require_mfa && !ctx.mfa_verified {
        return Err("MFA required");
    }
    if ctx.is_admin {
        return Ok(());
    }
    if ctx.subject != ctx.vault_owner {
        return Err("not the vault owner");
    }
    if ctx.tier < ctx.required_tier {
        return Err("tier too low");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn device() -> DeviceInfo {
        DeviceInfo {
            user_agent: "test".into(),
            ip_address: "127.0.0.1".into(),
        }
    }

    #[test]
    fn session_lifecycle_and_revocation() {
        let store = SessionStore::new();
        let a = store.create("GA", device(), 60, false);
        let b = store.create("GA", device(), 60, true);
        assert_eq!(store.list_for_subject("GA").len(), 2);
        assert!(store.touch(&a.id).is_some());
        assert!(!store.revoke("GOTHER", &a.id));
        assert!(store.revoke("GA", &a.id));
        assert!(store.touch(&a.id).is_none());
        assert_eq!(store.revoke_all("GA"), 1);
        assert!(store.touch(&b.id).is_none());
        assert!(store.list_for_subject("GA").is_empty());
    }

    #[test]
    fn expired_session_is_inactive() {
        let store = SessionStore::new();
        let s = store.create("GA", device(), 0, false);
        assert!(store.touch(&s.id).is_none());
    }

    #[test]
    fn base32_roundtrip() {
        let data = b"perigee-secret!!";
        assert_eq!(base32_decode(&base32_encode(data)).unwrap(), data);
    }

    #[test]
    fn totp_verifies_with_skew_and_rejects_wrong() {
        let secret = generate_totp_secret();
        let t = 1_700_000_000;
        let code = totp_code(&secret, t).unwrap();
        assert_eq!(code.len(), 6);
        assert!(verify_totp(&secret, &code, t));
        assert!(verify_totp(&secret, &code, t + 30));
        assert!(!verify_totp(&secret, &code, t + 300));
        assert!(!verify_totp(&secret, "abcdef", t));
    }

    #[test]
    fn abac_rules() {
        let base = AbacContext {
            subject: "GA",
            is_admin: false,
            tier: Tier::Pro,
            vault_owner: "GA",
            required_tier: Tier::Pro,
            mfa_verified: true,
            require_mfa: true,
        };
        assert!(authorize(&base).is_ok());
        assert!(authorize(&AbacContext { vault_owner: "GB", ..base.clone() }).is_err());
        assert!(authorize(&AbacContext { tier: Tier::Free, ..base.clone() }).is_err());
        assert!(authorize(&AbacContext { mfa_verified: false, is_admin: true, ..base.clone() }).is_err());
        assert!(authorize(&AbacContext { vault_owner: "GB", is_admin: true, ..base }).is_ok());
    }
}
