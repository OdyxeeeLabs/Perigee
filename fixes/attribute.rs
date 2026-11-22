//! Fix for #80: enforce a short TTL on API auth tokens and provide a
//! rotation endpoint instead of letting tokens persist indefinitely.
use std::time::{Duration, Instant};

pub const TOKEN_TTL: Duration = Duration::from_secs(3600);

pub struct AuthToken {
    pub value: String,
    issued_at: Instant,
}
impl AuthToken {
    pub fn new(value: &str) -> Self {
        Self { value: value.to_string(), issued_at: Instant::now() }
    }

    pub fn is_expired(&self) -> bool {
        self.issued_at.elapsed() > TOKEN_TTL
    }

    /// Rotation endpoint: only issues a fresh token if the current one
    /// is still valid, otherwise the caller must re-authenticate.
    pub fn rotate(&self, new_value: &str) -> Result<AuthToken, &'static str> {
        if self.is_expired() {
            return Err("token expired, re-authentication required");
        }
        Ok(AuthToken::new(new_value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_token_cannot_rotate() {
        let mut token = AuthToken::new("t1");
        token.issued_at -= TOKEN_TTL + Duration::from_secs(1);
        assert!(token.is_expired());
        assert!(token.rotate("t2").is_err());
    }

    #[test]
    fn valid_token_rotates() {
        let token = AuthToken::new("t1");
        assert!(!token.is_expired());
        assert!(token.rotate("t2").is_ok());
    }
}
