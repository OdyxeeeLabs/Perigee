//! # Outbox relay health (API-09)
//!
//! Makes relay recovery observable without touching the relay itself. Feed it
//! each [`RelayReport`] after `poll_once` and it keeps counters, logs
//! structured events, and exposes Prometheus text in the same style as
//! [`OutboxStats::to_prometheus`]. No labels are used, so no manager or vault
//! ids can leak into metrics.
//!
//! * `recovered`: events published after at least one failed attempt.
//! * `retrying`: pending events that have already failed at least once.
//! * `stuck`: the oldest pending event is older than the threshold and nothing
//!   was published in this pass. An idle worker (nothing pending) is healthy.

use crate::outbox::{EventStatus, InMemoryOutbox, OutboxError, RelayReport};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct RelayHealth {
    pub published_total: u64,
    pub failed_total: u64,
    pub quarantined_total: u64,
    pub recovered_total: u64,
    pub retrying: u64,
    pub stuck: bool,
    pub last_success_at: Option<u64>,
    seen_recovered: u64,
}

impl RelayHealth {
    pub fn observe(
        &mut self,
        outbox: &InMemoryOutbox,
        report: &RelayReport,
        now: u64,
        stuck_after_secs: u64,
    ) -> Result<(), OutboxError> {
        self.published_total += report.published as u64;
        self.failed_total += report.failed as u64;
        self.quarantined_total += report.quarantined as u64;

        let events = outbox.events()?;
        let recovered = events.iter().filter(|e| e.status == EventStatus::Published && e.attempts > 0).count() as u64;
        let newly_recovered = recovered.saturating_sub(self.seen_recovered);
        self.seen_recovered = recovered;
        self.recovered_total += newly_recovered;
        self.retrying = events.iter().filter(|e| e.status == EventStatus::Pending && e.attempts > 0).count() as u64;

        if report.published > 0 {
            self.last_success_at = Some(now);
        }
        let oldest_age = outbox.stats(now)?.oldest_pending_age_secs;
        let was_stuck = self.stuck;
        self.stuck = report.published == 0 && oldest_age.is_some_and(|age| age > stuck_after_secs);

        if newly_recovered > 0 {
            tracing::info!(recovered = newly_recovered, "outbox relay recovered events after retries");
        }
        if report.quarantined > 0 {
            tracing::warn!(quarantined = report.quarantined, "outbox relay quarantined poison events");
        }
        if self.stuck && !was_stuck {
            tracing::warn!(oldest_pending_age_secs = oldest_age.unwrap_or(0), retrying = self.retrying, "outbox relay appears stuck");
        }
        Ok(())
    }

    /// Prometheus text exposition, appended to the outbox metrics.
    pub fn to_prometheus(&self) -> String {
        format!(
            "# TYPE outbox_published_total counter\noutbox_published_total {}\n\
             # TYPE outbox_failed_total counter\noutbox_failed_total {}\n\
             # TYPE outbox_quarantined_total counter\noutbox_quarantined_total {}\n\
             # TYPE outbox_recovered_total counter\noutbox_recovered_total {}\n\
             # TYPE outbox_retrying gauge\noutbox_retrying {}\n\
             # TYPE outbox_worker_stuck gauge\noutbox_worker_stuck {}\n\
             # TYPE outbox_last_success_timestamp_seconds gauge\noutbox_last_success_timestamp_seconds {}\n",
            self.published_total,
            self.failed_total,
            self.quarantined_total,
            self.recovered_total,
            self.retrying,
            self.stuck as u8,
            self.last_success_at.unwrap_or(0)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::outbox::{Broker, NewEvent, Relay};
    use serde_json::json;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Flaky(Arc<Mutex<bool>>);
    impl Broker for Flaky {
        fn publish(&self, _m: &crate::outbox::Message) -> Result<(), String> {
            if *self.0.lock().unwrap() { Err("down".into()) } else { Ok(()) }
        }
    }

    #[test]
    fn failure_marks_stuck_and_recovery_is_counted() {
        let outbox = InMemoryOutbox::new();
        let mut tx = outbox.begin(0);
        tx.append(NewEvent::new("vault", "v1", "created", json!({}), "k1"));
        tx.commit().unwrap();
        let broker = Flaky::default();
        *broker.0.lock().unwrap() = true;
        let relay = Relay::new(outbox.clone(), broker.clone());
        let mut health = RelayHealth::default();

        let report = relay.poll_once().unwrap();
        health.observe(&outbox, &report, 100, 60).unwrap();
        assert!(health.stuck && health.retrying == 1 && health.failed_total == 1);

        *broker.0.lock().unwrap() = false;
        let report = relay.poll_once().unwrap();
        health.observe(&outbox, &report, 110, 60).unwrap();
        assert!(!health.stuck);
        assert_eq!((health.recovered_total, health.published_total, health.last_success_at), (1, 1, Some(110)));
        assert!(health.to_prometheus().contains("outbox_recovered_total 1"));
    }
}
