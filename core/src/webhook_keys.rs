//! # Webhook signing-key rotation (API-08)
//!
//! An endpoint has one *current* secret and, during a rotation, one *retiring*
//! secret with an expiry. While the retiring key is live the delivery carries
//! both `v1=` signatures (new key first), so consumers still on the old secret
//! and consumers already on the new one both verify; [`verify_signature`]
//! already accepts several `v1` values. After `retiring_until` only the
//! current key signs and is accepted. An endpoint without a retiring secret
//! behaves exactly as before. Secrets are never formatted into errors or logs.

use crate::webhooks::{signature_header, verify_signature, Endpoint, SignatureError};

impl Endpoint {
    /// Start a rotation: keep `old_secret` valid until unix time `until`.
    /// `self.secret` is the new, current key.
    pub fn retiring_secret(mut self, old_secret: &str, until: u64) -> Self {
        self.retiring = Some((old_secret.to_string(), until));
        self
    }

    /// Secrets that are valid at `now`: current first, then the retiring one.
    pub fn active_secrets(&self, now: u64) -> Vec<&str> {
        let mut keys = vec![self.secret.as_str()];
        if let Some((old, until)) = &self.retiring {
            if now < *until {
                keys.push(old.as_str());
            }
        }
        keys
    }
}

/// `X-Perigee-Signature` value signed with every active key.
pub fn signature_header_for(endpoint: &Endpoint, timestamp: u64, body: &[u8]) -> String {
    let keys = endpoint.active_secrets(timestamp);
    let mut header = signature_header(keys[0].as_bytes(), timestamp, body);
    for old in &keys[1..] {
        let extra = signature_header(old.as_bytes(), timestamp, body);
        if let Some((_, tag)) = extra.split_once(",v1=") {
            header.push_str(",v1=");
            header.push_str(tag);
        }
    }
    header
}

/// Verify against every key active at `now`.
pub fn verify_with_keys(
    endpoint: &Endpoint,
    header: &str,
    body: &[u8],
    now: u64,
    tolerance_secs: u64,
) -> Result<(), SignatureError> {
    let mut last = SignatureError::Mismatch;
    for key in endpoint.active_secrets(now) {
        match verify_signature(key.as_bytes(), header, body, now, tolerance_secs) {
            Ok(()) => return Ok(()),
            Err(e) => last = e,
        }
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_keys_verify_during_rotation_and_only_the_new_one_after() {
        let body = b"{}";
        let ep = Endpoint::new("e", "https://example.com/h", "new").retiring_secret("old", 2_000);
        let header = signature_header_for(&ep, 1_000, body);
        assert_eq!(header.matches("v1=").count(), 2);
        // A consumer still holding only the old secret, and one holding only the new.
        assert!(verify_signature(b"old", &header, body, 1_000, 300).is_ok());
        assert!(verify_signature(b"new", &header, body, 1_000, 300).is_ok());
        assert!(verify_with_keys(&ep, &header, body, 1_000, 300).is_ok());

        // Past the retiring window: old key no longer signs or verifies.
        let old_only = signature_header(b"old", 2_100, body);
        assert_eq!(verify_with_keys(&ep, &old_only, body, 2_100, 300), Err(SignatureError::Mismatch));
        assert_eq!(signature_header_for(&ep, 2_100, body).matches("v1=").count(), 1);
    }

    #[test]
    fn a_single_key_is_unchanged() {
        let ep = Endpoint::new("e", "https://example.com/h", "only");
        assert_eq!(signature_header_for(&ep, 5, b"x"), signature_header(b"only", 5, b"x"));
    }
}
