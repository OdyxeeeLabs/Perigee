#![allow(dead_code)]

use std::collections::{BTreeSet, HashMap};

// Divide before multiplying: `u64::MAX * 80` overflows u64, which is a
// compile-time error in a const context rather than a runtime surprise.
const NONCE_WARN_THRESHOLD: u64 = u64::MAX / 100 * 80;
const NONCE_MAX: u64 = u64::MAX;

/// Tracks per-domain nonce assignment for on-chain submissions.
///
/// A nonce is handed out by [`NoncePartition::next_nonce`] (or
/// [`assign_nonce`](Self::assign_nonce)) and is considered *assigned but
/// uncommitted* until the matching transaction has durably landed on-chain.
///
/// Previously a nonce was consumed the moment it was handed out, so a
/// transaction that failed after assignment left a permanent gap that blocked
/// every later transaction for the account. This version:
///
/// * keeps assigned-but-uncommitted nonces in `inflight` so they can be
///   released back to the available pool on failure (`release_nonce`), and
/// * detects gaps (assigned nonces that fell below the contiguous
///   high-water mark) and lets the caller fill them with no-op transactions
///   (`detect_gaps` / `fill_gap`).
pub struct NoncePartition {
    /// Highest nonce that has been *contiguously* committed for each domain.
    /// The next nonce the chain expects is `committed[domain] + 1`.
    committed: HashMap<String, u64>,
    /// Nonces that have been assigned (via `next_nonce`) but not yet durably
    /// committed on-chain.
    inflight: HashMap<String, BTreeSet<u64>>,
    /// Genuine gaps: nonces that were assigned, never committed, and now sit
    /// below the contiguous high-water mark, so they must be filled with a
    /// no-op transaction before the chain can advance.
    filling: HashMap<String, BTreeSet<u64>>,
    /// Nonces that have been committed, used to advance `committed`
    /// contiguously even when commits arrive out of order.
    committed_set: HashMap<String, BTreeSet<u64>>,
    exhausted: Vec<String>,
}

impl Default for NoncePartition {
    fn default() -> Self {
        Self::new()
    }
}

impl NoncePartition {
    pub fn new() -> Self {
        Self {
            committed: HashMap::new(),
            inflight: HashMap::new(),
            filling: HashMap::new(),
            committed_set: HashMap::new(),
            exhausted: Vec::new(),
        }
    }

    /// The next nonce value the chain expects for `domain` (i.e. one past the
    /// highest contiguous committed nonce). `None` committed yet means the next
    /// expected value is `0`.
    fn next_expected(&self, domain: &str) -> u64 {
        self.committed
            .get(domain)
            .copied()
            .map_or(0, |c| c.saturating_add(1))
    }

    /// The next nonce that would be handed out for `domain`: the smallest value
    /// at or above `next_expected` that is not currently held in `inflight` or
    /// `filling`.
    fn next_assignable(&self, domain: &str) -> u64 {
        let mut candidate = self.next_expected(domain);
        while self
            .inflight
            .get(domain)
            .is_some_and(|s| s.contains(&candidate))
            || self
                .filling
                .get(domain)
                .is_some_and(|s| s.contains(&candidate))
        {
            candidate = candidate.saturating_add(1);
        }
        candidate
    }

    /// Assign the next available nonce for `domain`, marking it as assigned but
    /// uncommitted. The nonce stays reserved (in `inflight`) until the
    /// transaction either commits or is released.
    pub fn next_nonce(&mut self, domain: &str) -> Result<u64, &'static str> {
        self.assign_nonce(domain)
    }

    /// Assign the next available nonce for `domain`.
    ///
    /// Skips any nonce currently held in `inflight` or `filling` so a released
    /// or in-progress nonce is never double-assigned.
    pub fn assign_nonce(&mut self, domain: &str) -> Result<u64, &'static str> {
        let candidate = self.next_assignable(domain);

        if candidate == NONCE_MAX {
            if !self.exhausted.contains(&domain.to_string()) {
                self.exhausted.push(domain.to_string());
            }
            return Err("nonce range exhausted for domain");
        }

        self.inflight
            .entry(domain.to_string())
            .or_default()
            .insert(candidate);

        if candidate >= NONCE_WARN_THRESHOLD {
            let redacted_domain = crate::log_redaction::LogRedactor::new("perigee-log-redaction")
                .redact_address(domain);
            tracing::warn!(
                domain = %redacted_domain,
                nonce = candidate,
                "Nonce range approaching exhaustion"
            );
        }

        Ok(candidate)
    }

    /// Mark `nonce` as durably committed for `domain`.
    ///
    /// Advances the contiguous high-water mark past any now-contiguous
    /// committed nonces, which automatically closes gaps that were filled.
    pub fn commit_nonce(&mut self, domain: &str, nonce: u64) {
        if let Some(set) = self.inflight.get_mut(domain) {
            set.remove(&nonce);
        }
        if let Some(set) = self.filling.get_mut(domain) {
            set.remove(&nonce);
        }
        self.committed_set
            .entry(domain.to_string())
            .or_default()
            .insert(nonce);

        // Advance the contiguous high-water mark while the next expected nonce
        // has been committed.
        let committed = self.committed.entry(domain.to_string()).or_insert(0);
        let set = self.committed_set.get(domain).expect("inserted above");
        while set.contains(&committed.saturating_add(1)) {
            *committed = committed.saturating_add(1);
        }
    }

    /// Release an assigned-but-uncommitted nonce back into the available pool.
    ///
    /// Call this when the transaction that consumed `nonce` failed before being
    /// included on-chain, so the nonce can be reassigned instead of leaving a
    /// permanent gap (BE-045 / issue #282).
    pub fn release_nonce(&mut self, domain: &str, nonce: u64) {
        if let Some(set) = self.inflight.get_mut(domain) {
            set.remove(&nonce);
        }
        // If it was already being filled (a no-op in flight), drop that too.
        if let Some(set) = self.filling.get_mut(domain) {
            set.remove(&nonce);
        }
    }

    /// Detect assigned nonces that sit below the highest committed nonce for
    /// `domain` — i.e. a lower nonce is still uncommitted while a higher one has
    /// already landed. These gaps must be filled with no-op transactions so the
    /// contiguous high-water mark can advance.
    pub fn detect_gaps(&self, domain: &str) -> Vec<u64> {
        let Some(max_committed) = self
            .committed_set
            .get(domain)
            .and_then(|s| s.iter().max().copied())
        else {
            return Vec::new();
        };

        let mut gaps: Vec<u64> = self
            .inflight
            .get(domain)
            .into_iter()
            .flatten()
            .chain(self.filling.get(domain).into_iter().flatten())
            .copied()
            .filter(|n| *n < max_committed)
            .collect();
        gaps.sort_unstable();
        gaps
    }

    /// Begin filling a detected gap by submitting a no-op transaction for
    /// `nonce`. The nonce is moved from `inflight` to `filling`; once the
    /// no-op commits, call [`commit_nonce`](Self::commit_nonce) to close it.
    pub fn fill_gap(&mut self, domain: &str, nonce: u64) {
        if let Some(set) = self.inflight.get_mut(domain) {
            set.remove(&nonce);
        }
        self.filling.entry(domain.to_string()).or_default().insert(nonce);
    }

    /// Whether `nonce` is still unused for `domain`.
    ///
    /// A nonce is "unused" if it is strictly above the contiguous high-water
    /// mark *and* not currently held in `inflight` or `filling` (i.e. it has not
    /// been assigned yet).
    pub fn verify_unique(&self, domain: &str, nonce: u64) -> bool {
        let current = self.next_expected(domain);
        nonce > current
            && !self
                .inflight
                .get(domain)
                .is_some_and(|s| s.contains(&nonce))
            && !self
                .filling
                .get(domain)
                .is_some_and(|s| s.contains(&nonce))
    }

    pub fn current_nonce(&self, domain: &str) -> u64 {
        self.next_assignable(domain)
    }

    pub fn is_exhausted(&self, domain: &str) -> bool {
        self.exhausted.contains(&domain.to_string())
    }

    pub fn reset_domain(&mut self, domain: &str) {
        self.committed.remove(domain);
        self.inflight.remove(domain);
        self.filling.remove(domain);
        self.committed_set.remove(domain);
        self.exhausted.retain(|d| d != domain);
    }

    /// Test helper: force the contiguous high-water mark for `domain`.
    #[cfg(test)]
    pub fn force_committed(&mut self, domain: &str, value: u64) {
        self.committed.insert(domain.to_string(), value);
        self.committed_set
            .entry(domain.to_string())
            .or_default()
            .insert(value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sequential_nonces() {
        let mut np = NoncePartition::new();
        assert_eq!(np.next_nonce("rail_a").unwrap(), 0);
        assert_eq!(np.next_nonce("rail_a").unwrap(), 1);
        assert_eq!(np.next_nonce("rail_a").unwrap(), 2);
    }

    #[test]
    fn test_isolated_domains() {
        let mut np = NoncePartition::new();
        assert_eq!(np.next_nonce("rail_a").unwrap(), 0);
        assert_eq!(np.next_nonce("rail_b").unwrap(), 0);
        assert_eq!(np.next_nonce("rail_a").unwrap(), 1);
        assert_eq!(np.next_nonce("rail_b").unwrap(), 1);
    }

    #[test]
    fn test_verify_unique() {
        let mut np = NoncePartition::new();
        np.next_nonce("rail_a").unwrap();
        np.next_nonce("rail_a").unwrap();
        assert!(!np.verify_unique("rail_a", 0));
        assert!(!np.verify_unique("rail_a", 1));
        assert!(np.verify_unique("rail_a", 2));
    }

    #[test]
    fn test_nonce_exhaustion() {
        let mut np = NoncePartition::new();
        np.force_committed("test", NONCE_MAX);
        assert!(np.next_nonce("test").is_err());
        assert!(np.is_exhausted("test"));
    }

    #[test]
    fn test_reset_domain() {
        let mut np = NoncePartition::new();
        np.next_nonce("rail_a").unwrap();
        np.next_nonce("rail_a").unwrap();
        assert_eq!(np.current_nonce("rail_a"), 2);
        np.reset_domain("rail_a");
        assert_eq!(np.current_nonce("rail_a"), 0);
        assert!(!np.is_exhausted("rail_a"));
    }

    /// BE-045: a failed transaction must release its nonce so it can be reused
    /// instead of leaving a permanent gap.
    #[test]
    fn released_nonce_is_reassigned() {
        let mut np = NoncePartition::new();
        let a = np.next_nonce("rail_a").unwrap(); // 0
        let b = np.next_nonce("rail_a").unwrap(); // 1
        assert_eq!((a, b), (0, 1));

        // Transaction for nonce 0 fails before landing on-chain.
        np.release_nonce("rail_a", a);

        // Next assignment reuses the released nonce rather than skipping it.
        let c = np.next_nonce("rail_a").unwrap();
        assert_eq!(c, 0);
    }

    /// BE-045: when a higher nonce commits before a lower one, the lower nonce
    /// becomes a gap below the high-water mark and must be filled.
    #[test]
    fn detect_and_fill_gap() {
        let mut np = NoncePartition::new();
        // Assume prior committed high-water mark of 4 for this domain.
        np.force_committed("rail_a", 4);

        let n5 = np.next_nonce("rail_a").unwrap(); // 5
        let n6 = np.next_nonce("rail_a").unwrap(); // 6
        let n7 = np.next_nonce("rail_a").unwrap(); // 7
        assert_eq!((n5, n6, n7), (5, 6, 7));

        // 6 and 7 land on-chain, but 5's transaction is stuck.
        np.commit_nonce("rail_a", 6);
        np.commit_nonce("rail_a", 7);
        // High-water mark cannot advance past the missing 5; the next assignable
        // nonce skips over the still-held 5 to 6.
        assert_eq!(np.current_nonce("rail_a"), 6);

        // 5 is now a gap below the high-water mark.
        let gaps = np.detect_gaps("rail_a");
        assert_eq!(gaps, vec![5]);

        // Fill the gap with a no-op transaction, then commit it.
        np.fill_gap("rail_a", 5);
        np.commit_nonce("rail_a", 5);

        // Gap closed: high-water mark advances contiguously to 7.
        assert_eq!(np.current_nonce("rail_a"), 8);
        assert!(np.detect_gaps("rail_a").is_empty());
    }

    /// Releasing a stuck nonce lets the pipeline continue without a no-op when
    /// the original transaction can simply be retried.
    #[test]
    fn release_lets_pipeline_continue() {
        let mut np = NoncePartition::new();
        np.force_committed("rail_a", 4);

        let n5 = np.next_nonce("rail_a").unwrap();
        let n6 = np.next_nonce("rail_a").unwrap();
        assert_eq!((n5, n6), (5, 6));

        // 5 fails; release it so the caller can retry, then 6 commits.
        np.release_nonce("rail_a", 5);
        np.commit_nonce("rail_a", 6);
        // 6 committed, but 5 still missing -> high-water stuck at 5.
        assert_eq!(np.current_nonce("rail_a"), 5);

        // Retry the released nonce; it is reassigned as 5.
        let retry = np.next_nonce("rail_a").unwrap();
        assert_eq!(retry, 5);
        np.commit_nonce("rail_a", retry);
        assert_eq!(np.current_nonce("rail_a"), 7);
    }
}
