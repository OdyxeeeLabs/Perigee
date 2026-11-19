//! # Rate-limit backend failure policy (API-10)
//!
//! `ApiRateLimiter` already shares quota across instances through Redis
//! (`REDIS_URL`) and falls back to a per-process bucket when Redis is
//! unreachable. That fallback is *fail-open* toward a local limit: each replica
//! then enforces the full quota on its own, so the effective limit becomes
//! `replicas x limit`. This module makes the choice explicit and configurable:
//!
//! * default: fall back to the in-memory bucket (availability over strictness);
//! * `RATE_LIMIT_FAIL_CLOSED=true` with `REDIS_URL` set: deny requests while the
//!   shared store is unavailable instead of silently granting extra quota.

/// Pure decision so it can be tested without touching the environment.
pub fn fail_closed_from(flag: Option<&str>, redis_configured: bool) -> bool {
    redis_configured
        && flag.is_some_and(|v| matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"))
}

/// Whether to reject requests while the shared Redis store is unavailable.
pub fn fail_closed() -> bool {
    fail_closed_from(
        std::env::var("RATE_LIMIT_FAIL_CLOSED").ok().as_deref(),
        std::env::var("REDIS_URL").is_ok(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_to_the_local_fallback_and_needs_a_configured_store_to_fail_closed() {
        assert!(!fail_closed_from(None, true));
        assert!(!fail_closed_from(Some("false"), true));
        assert!(!fail_closed_from(Some("true"), false));
        assert!(fail_closed_from(Some("TRUE"), true));
    }
}
