//! Oracle staleness guard (BE-024).
//!
//! The oracle guard validated the *shape* of prices but not their *age*. When
//! the oracle stopped responding, the last price it had returned stayed in
//! place and strategy decisions carried on against it — indefinitely, and
//! without anything saying so. A stale price is worse than a missing one,
//! because nothing downstream can tell the difference.
//!
//! This tracks when the oracle last produced data and refuses execution once
//! that exceeds `max_staleness_seconds`.
//!
//! # Fails closed
//!
//! An oracle that has never reported ([`OracleStatus::NoData`]) blocks
//! execution, exactly as a stale one does. "No price yet" and "a price from an
//! hour ago" are both *not a current price*, and the safe answer to both is to
//! stop rather than to guess.
//!
//! # Alerts fire on the transition, not on every check
//!
//! A strategy loop may check this many times a second. Emitting an alert per
//! check would bury the signal, so alerts fire when the guard *changes* state:
//! once when data goes stale, once when it recovers. [`OracleGuard::check`]
//! is the method that does this; [`OracleGuard::status`] is a pure read.
//!
//! # Clock skew
//!
//! A timestamp in the future yields an age of zero rather than a negative
//! one. A clock a little ahead should not make data look infinitely fresh nor
//! panic the arithmetic.

use chrono::{DateTime, Utc};
use tracing::{info, warn};

/// Default staleness threshold: five minutes, as specified in BE-024.
pub const DEFAULT_MAX_STALENESS_SECONDS: u64 = 300;

/// Environment variable overriding the threshold.
pub const MAX_STALENESS_ENV: &str = "ORACLE_MAX_STALENESS_SECONDS";

/// Freshness of the oracle's most recent data.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OracleStatus {
    /// Data is within the staleness threshold.
    Fresh { age_seconds: i64 },

    /// Data is older than the threshold; execution must pause.
    Stale {
        age_seconds: i64,
        max_staleness_seconds: i64,
    },

    /// The oracle has never reported.
    NoData,
}

impl OracleStatus {
    /// Whether strategy execution may proceed.
    pub fn allows_execution(&self) -> bool {
        matches!(self, OracleStatus::Fresh { .. })
    }

    pub fn is_stale(&self) -> bool {
        matches!(self, OracleStatus::Stale { .. })
    }
}

/// Configuration for [`OracleGuard`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OracleGuardConfig {
    /// How old oracle data may be before execution pauses.
    pub max_staleness_seconds: u64,
}

impl Default for OracleGuardConfig {
    fn default() -> Self {
        Self {
            max_staleness_seconds: DEFAULT_MAX_STALENESS_SECONDS,
        }
    }
}

impl OracleGuardConfig {
    /// Read the threshold from [`MAX_STALENESS_ENV`], falling back to the
    /// default.
    ///
    /// An unparseable or zero value falls back rather than failing: a
    /// misconfigured threshold should not take the service down, and zero
    /// would make every price stale the instant it arrived.
    pub fn from_env() -> Self {
        let max_staleness_seconds = std::env::var(MAX_STALENESS_ENV)
            .ok()
            .and_then(|raw| match raw.trim().parse::<u64>() {
                Ok(0) => {
                    warn!(
                        target: "oracle_guard",
                        "{MAX_STALENESS_ENV}=0 would mark every price stale on arrival; \
                         using the default of {DEFAULT_MAX_STALENESS_SECONDS}s"
                    );
                    None
                }
                Ok(v) => Some(v),
                Err(e) => {
                    warn!(
                        target: "oracle_guard",
                        error = %crate::log_redaction::redact_display(&e),
                        "{MAX_STALENESS_ENV} is not a positive integer; \
                         using the default of {DEFAULT_MAX_STALENESS_SECONDS}s"
                    );
                    None
                }
            })
            .unwrap_or(DEFAULT_MAX_STALENESS_SECONDS);

        Self {
            max_staleness_seconds,
        }
    }
}

/// Tracks oracle freshness and gates strategy execution on it.
#[derive(Debug, Clone)]
pub struct OracleGuard {
    config: OracleGuardConfig,
    last_update: Option<DateTime<Utc>>,
    /// Whether a stale alert has already been emitted for the current episode.
    alerted: bool,
}

impl OracleGuard {
    pub fn new(config: OracleGuardConfig) -> Self {
        Self {
            config,
            last_update: None,
            alerted: false,
        }
    }

    /// Build from [`MAX_STALENESS_ENV`].
    pub fn from_env() -> Self {
        Self::new(OracleGuardConfig::from_env())
    }

    pub fn max_staleness_seconds(&self) -> u64 {
        self.config.max_staleness_seconds
    }

    pub fn last_update(&self) -> Option<DateTime<Utc>> {
        self.last_update
    }

    /// Record that the oracle produced data at `at`.
    pub fn record_update(&mut self, at: DateTime<Utc>) {
        // Never move the recorded time backwards: an out-of-order or replayed
        // update must not make the feed look older than it is.
        let is_newer = self.last_update.map(|prev| at > prev).unwrap_or(true);

        if is_newer {
            self.last_update = Some(at);
        }
    }

    /// Age of the current data in seconds, clamped at zero for clock skew.
    fn age_seconds(&self, now: DateTime<Utc>) -> Option<i64> {
        self.last_update
            .map(|last| (now - last).num_seconds().max(0))
    }

    /// Current status. Pure — emits nothing.
    pub fn status(&self, now: DateTime<Utc>) -> OracleStatus {
        let max = self.config.max_staleness_seconds as i64;

        match self.age_seconds(now) {
            None => OracleStatus::NoData,
            Some(age) if age > max => OracleStatus::Stale {
                age_seconds: age,
                max_staleness_seconds: max,
            },
            Some(age) => OracleStatus::Fresh { age_seconds: age },
        }
    }

    /// Whether strategy execution may proceed right now.
    pub fn allows_execution(&self, now: DateTime<Utc>) -> bool {
        self.status(now).allows_execution()
    }

    /// Status, emitting an alert on any change of state.
    ///
    /// Call this from the strategy loop; call [`status`](Self::status) when
    /// you only want to read.
    pub fn check(&mut self, now: DateTime<Utc>) -> OracleStatus {
        let status = self.status(now);

        match status {
            OracleStatus::Stale {
                age_seconds,
                max_staleness_seconds,
            } if !self.alerted => {
                self.alerted = true;

                warn!(
                    target: "oracle_guard",
                    event = "oracle_data_stale",
                    age_seconds,
                    max_staleness_seconds,
                    last_update = ?self.last_update,
                    "Oracle data is stale; pausing strategy execution"
                );
            }

            OracleStatus::NoData if !self.alerted => {
                self.alerted = true;

                warn!(
                    target: "oracle_guard",
                    event = "oracle_no_data",
                    max_staleness_seconds = self.config.max_staleness_seconds,
                    "Oracle has never reported; pausing strategy execution"
                );
            }

            OracleStatus::Fresh { age_seconds } if self.alerted => {
                self.alerted = false;

                info!(
                    target: "oracle_guard",
                    event = "oracle_data_recovered",
                    age_seconds,
                    "Oracle data is fresh again; resuming strategy execution"
                );
            }

            _ => {}
        }

        status
    }

    /// Whether a stale alert is currently outstanding.
    pub fn is_alerting(&self) -> bool {
        self.alerted
    }
}

impl Default for OracleGuard {
    fn default() -> Self {
        Self::new(OracleGuardConfig::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn base() -> DateTime<Utc> {
        DateTime::from_timestamp(1_700_000_000, 0).expect("valid timestamp")
    }

    fn guard(max_staleness_seconds: u64) -> OracleGuard {
        OracleGuard::new(OracleGuardConfig {
            max_staleness_seconds,
        })
    }

    #[test]
    fn the_default_threshold_is_five_minutes() {
        assert_eq!(DEFAULT_MAX_STALENESS_SECONDS, 300);
        assert_eq!(OracleGuardConfig::default().max_staleness_seconds, 300);
    }

    /// Fails closed: an oracle that has never reported blocks execution.
    #[test]
    fn no_data_blocks_execution() {
        let g = guard(300);

        assert_eq!(g.status(base()), OracleStatus::NoData);
        assert!(!g.allows_execution(base()));
    }

    #[test]
    fn fresh_data_allows_execution() {
        let mut g = guard(300);
        g.record_update(base());

        let status = g.status(base() + Duration::seconds(10));

        assert_eq!(status, OracleStatus::Fresh { age_seconds: 10 });
        assert!(status.allows_execution());
    }

    #[test]
    fn data_older_than_the_threshold_pauses_execution() {
        let mut g = guard(300);
        g.record_update(base());

        let status = g.status(base() + Duration::seconds(301));

        assert_eq!(
            status,
            OracleStatus::Stale {
                age_seconds: 301,
                max_staleness_seconds: 300,
            }
        );
        assert!(!status.allows_execution());
        assert!(status.is_stale());
    }

    /// The boundary is inclusive: data exactly at the threshold is still
    /// fresh, and goes stale the second after.
    #[test]
    fn the_threshold_boundary_is_inclusive() {
        let mut g = guard(300);
        g.record_update(base());

        assert!(g.allows_execution(base() + Duration::seconds(300)));
        assert!(!g.allows_execution(base() + Duration::seconds(301)));
    }

    #[test]
    fn a_new_update_restores_execution() {
        let mut g = guard(60);
        g.record_update(base());

        let later = base() + Duration::seconds(120);
        assert!(!g.allows_execution(later));

        g.record_update(later);
        assert!(g.allows_execution(later));
    }

    /// An out-of-order update must not make the feed look older.
    #[test]
    fn an_older_update_does_not_move_the_clock_backwards() {
        let mut g = guard(300);
        let newer = base() + Duration::seconds(100);

        g.record_update(newer);
        g.record_update(base());

        assert_eq!(g.last_update(), Some(newer));
    }

    /// A timestamp slightly in the future is clock skew, not infinite
    /// freshness, and must not produce a negative age.
    #[test]
    fn a_future_timestamp_clamps_to_zero_age() {
        let mut g = guard(300);
        g.record_update(base() + Duration::seconds(60));

        assert_eq!(g.status(base()), OracleStatus::Fresh { age_seconds: 0 });
    }

    #[test]
    fn the_alert_fires_once_per_stale_episode() {
        let mut g = guard(60);
        g.record_update(base());

        let stale_at = base() + Duration::seconds(120);

        assert!(!g.is_alerting());

        g.check(stale_at);
        assert!(g.is_alerting(), "first stale check should raise the alert");

        // Subsequent checks stay stale but must not re-alert.
        g.check(stale_at + Duration::seconds(1));
        g.check(stale_at + Duration::seconds(2));
        assert!(g.is_alerting());
    }

    #[test]
    fn recovery_clears_the_alert() {
        let mut g = guard(60);
        g.record_update(base());

        g.check(base() + Duration::seconds(120));
        assert!(g.is_alerting());

        let recovery = base() + Duration::seconds(180);
        g.record_update(recovery);

        assert_eq!(g.check(recovery), OracleStatus::Fresh { age_seconds: 0 });
        assert!(!g.is_alerting());
    }

    #[test]
    fn a_second_outage_alerts_again() {
        let mut g = guard(60);

        g.record_update(base());
        g.check(base() + Duration::seconds(120));
        assert!(g.is_alerting());

        let recovery = base() + Duration::seconds(180);
        g.record_update(recovery);
        g.check(recovery);
        assert!(!g.is_alerting());

        g.check(recovery + Duration::seconds(120));
        assert!(g.is_alerting(), "a fresh outage must alert again");
    }

    #[test]
    fn no_data_alerts_too() {
        let mut g = guard(60);

        assert_eq!(g.check(base()), OracleStatus::NoData);
        assert!(g.is_alerting());
    }

    #[test]
    fn check_and_status_agree() {
        let mut g = guard(60);
        g.record_update(base());

        for offset in [0, 30, 60, 61, 600] {
            let now = base() + Duration::seconds(offset);
            let read = g.status(now);

            assert_eq!(g.check(now), read);
        }
    }

    #[test]
    fn the_configured_threshold_is_reported() {
        assert_eq!(guard(42).max_staleness_seconds(), 42);
    }

    // `from_env` mutates process-global state, so these share the crate-wide
    // env lock with the config and errors tests.

    #[test]
    fn from_env_reads_the_configured_threshold() {
        let _env = crate::errors::env_guard();

        unsafe { std::env::set_var(MAX_STALENESS_ENV, "45") };
        assert_eq!(OracleGuard::from_env().max_staleness_seconds(), 45);
        unsafe { std::env::remove_var(MAX_STALENESS_ENV) };
    }

    #[test]
    fn from_env_falls_back_when_unset() {
        let _env = crate::errors::env_guard();

        unsafe { std::env::remove_var(MAX_STALENESS_ENV) };
        assert_eq!(
            OracleGuard::from_env().max_staleness_seconds(),
            DEFAULT_MAX_STALENESS_SECONDS
        );
    }

    /// A misconfigured threshold must not take the service down, and zero
    /// would mark every price stale the instant it arrived.
    #[test]
    fn from_env_falls_back_on_nonsense() {
        let _env = crate::errors::env_guard();

        for value in ["0", "-5", "not-a-number", ""] {
            unsafe { std::env::set_var(MAX_STALENESS_ENV, value) };

            assert_eq!(
                OracleGuard::from_env().max_staleness_seconds(),
                DEFAULT_MAX_STALENESS_SECONDS,
                "{value:?} should fall back to the default"
            );
        }

        unsafe { std::env::remove_var(MAX_STALENESS_ENV) };
    }

    #[test]
    fn a_custom_threshold_is_honoured() {
        let mut g = guard(10);
        g.record_update(base());

        assert!(g.allows_execution(base() + Duration::seconds(10)));
        assert!(!g.allows_execution(base() + Duration::seconds(11)));
    }
}
