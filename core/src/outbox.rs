//! # Transactional outbox
//!
//! Guarantees that a domain change and the event announcing it are either both
//! saved or both lost, without a distributed transaction:
//!
//! 1. **Write**: the handler appends events to the `outbox_events` table *in the
//!    same database transaction* as the business change ([`OutboxTx`] models
//!    this; the SQL for a real database is in [`sql`]). Roll the transaction
//!    back and the events vanish with it.
//! 2. **Relay**: a separate process polls the table and publishes to the broker
//!    (Kafka/RabbitMQ behind the [`Broker`] trait), then marks events published.
//!    Delivery is therefore *at least once*.
//! 3. **Consume**: consumers record each `idempotency_key` they have handled
//!    ([`IdempotentConsumer`]), which turns at-least-once delivery into exactly
//!    one effect.
//!
//! Ordering is per aggregate: events carry a per-aggregate `sequence`, are
//! published in that order, and a failed event holds back the later events of
//! the same aggregate (other aggregates keep flowing). An event that keeps
//! failing is a *poison message*: after `max_attempts` it is quarantined for
//! manual review so it stops blocking its aggregate; consumers can see the gap
//! in `sequence`. [`OutboxStats`] reports the lag to monitor. See
//! `docs/OUTBOX.md`.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard};
use thiserror::Error;

pub const DEFAULT_MAX_ATTEMPTS: u32 = 5;
pub const DEFAULT_BATCH_SIZE: usize = 100;

/// SQL for the real store (PostgreSQL or SQLite). `?` placeholders are positional.
pub mod sql {
    /// Appends an event with the next per-aggregate sequence number. Run inside
    /// the business transaction. A repeated `idempotency_key` is ignored.
    pub const INSERT_EVENT: &str = "\
INSERT INTO outbox_events \
 (id, aggregate_type, aggregate_id, event_type, payload, idempotency_key, sequence) \
SELECT ?, ?, ?, ?, ?, ?, COALESCE(MAX(sequence), 0) + 1 \
FROM outbox_events WHERE aggregate_type = ? AND aggregate_id = ? \
ON CONFLICT (idempotency_key) DO NOTHING";

    /// Oldest unpublished events first.
    pub const SELECT_PENDING: &str = "\
SELECT id, aggregate_type, aggregate_id, event_type, payload, idempotency_key, sequence, attempts \
FROM outbox_events WHERE status = 'PENDING' \
ORDER BY created_at, aggregate_type, aggregate_id, sequence LIMIT ?";

    pub const MARK_PUBLISHED: &str = "\
UPDATE outbox_events SET status = 'PUBLISHED', published_at = CURRENT_TIMESTAMP WHERE id = ?";

    pub const RECORD_FAILURE: &str = "\
UPDATE outbox_events SET attempts = attempts + 1, last_error = ? WHERE id = ?";

    pub const QUARANTINE: &str = "\
UPDATE outbox_events SET status = 'QUARANTINED', quarantined_at = CURRENT_TIMESTAMP, last_error = ? WHERE id = ?";

    /// Consumer-side deduplication: zero rows inserted means "already handled".
    pub const CLAIM_MESSAGE: &str = "\
INSERT INTO processed_messages (consumer, idempotency_key) VALUES (?, ?) ON CONFLICT DO NOTHING";

    /// Housekeeping: published events older than the retention window.
    pub const PRUNE_PUBLISHED: &str = "\
DELETE FROM outbox_events WHERE status = 'PUBLISHED' AND published_at < ?";
}

#[derive(Debug, Error, PartialEq, Eq)]
pub enum OutboxError {
    #[error("unknown event `{0}`")]
    UnknownEvent(String),
    #[error("event is not quarantined")]
    NotQuarantined,
    #[error("outbox state unavailable")]
    Poisoned,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EventStatus {
    Pending,
    Published,
    Quarantined,
}

/// What a handler appends.
#[derive(Debug, Clone)]
pub struct NewEvent {
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub payload: Value,
    /// Stable per logical event; consumers deduplicate on it.
    pub idempotency_key: String,
}

impl NewEvent {
    pub fn new(aggregate_type: &str, aggregate_id: &str, event_type: &str, payload: Value, idempotency_key: &str) -> Self {
        NewEvent {
            aggregate_type: aggregate_type.to_string(),
            aggregate_id: aggregate_id.to_string(),
            event_type: event_type.to_string(),
            payload,
            idempotency_key: idempotency_key.to_string(),
        }
    }
}

#[derive(Debug, Clone)]
pub struct OutboxEvent {
    pub id: u64,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub payload: Value,
    pub idempotency_key: String,
    /// 1, 2, 3... within the aggregate.
    pub sequence: u64,
    pub status: EventStatus,
    pub attempts: u32,
    pub last_error: Option<String>,
    pub created_at: u64,
}

/// The message the relay hands to the broker.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub event_id: u64,
    pub aggregate_type: String,
    pub aggregate_id: String,
    pub event_type: String,
    pub sequence: u64,
    pub idempotency_key: String,
    pub payload: Value,
}

impl Message {
    /// Partition key: all events of an aggregate go to one partition, in order.
    pub fn partition_key(&self) -> String {
        format!("{}:{}", self.aggregate_type, self.aggregate_id)
    }
}

pub trait Broker: Send + Sync {
    fn publish(&self, message: &Message) -> Result<(), String>;
}

// ---------------------------------------------------------------------------
// In-memory store with transaction semantics
// ---------------------------------------------------------------------------

#[derive(Default)]
struct Inner {
    next_id: u64,
    events: Vec<OutboxEvent>,
}

/// Reference implementation of the outbox table. The SQL equivalents are in [`sql`].
#[derive(Clone, Default)]
pub struct InMemoryOutbox {
    inner: Arc<Mutex<Inner>>,
}

/// Events buffered by a transaction. They become visible on [`OutboxTx::commit`]
/// and disappear if the transaction is dropped (rolled back).
pub struct OutboxTx {
    outbox: InMemoryOutbox,
    pending: Vec<NewEvent>,
    now: u64,
}

impl OutboxTx {
    /// Append an event to this transaction.
    pub fn append(&mut self, event: NewEvent) {
        self.pending.push(event);
    }

    pub fn commit(self) -> Result<usize, OutboxError> {
        let mut inner = self.outbox.lock()?;
        let mut added = 0;
        for event in self.pending {
            if inner.events.iter().any(|e| e.idempotency_key == event.idempotency_key) {
                continue; // duplicate: ignored, like ON CONFLICT DO NOTHING
            }
            let sequence = inner
                .events
                .iter()
                .filter(|e| e.aggregate_type == event.aggregate_type && e.aggregate_id == event.aggregate_id)
                .map(|e| e.sequence)
                .max()
                .unwrap_or(0)
                + 1;
            inner.next_id += 1;
            let id = inner.next_id;
            inner.events.push(OutboxEvent {
                id,
                aggregate_type: event.aggregate_type,
                aggregate_id: event.aggregate_id,
                event_type: event.event_type,
                payload: event.payload,
                idempotency_key: event.idempotency_key,
                sequence,
                status: EventStatus::Pending,
                attempts: 0,
                last_error: None,
                created_at: self.now,
            });
            added += 1;
        }
        Ok(added)
    }
}

impl InMemoryOutbox {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> Result<MutexGuard<'_, Inner>, OutboxError> {
        self.inner.lock().map_err(|_| OutboxError::Poisoned)
    }

    /// Start a transaction; commit it together with the business change.
    pub fn begin(&self, now: u64) -> OutboxTx {
        OutboxTx { outbox: self.clone(), pending: Vec::new(), now }
    }

    pub fn events(&self) -> Result<Vec<OutboxEvent>, OutboxError> {
        Ok(self.lock()?.events.clone())
    }

    fn pending(&self, limit: usize) -> Result<Vec<OutboxEvent>, OutboxError> {
        let inner = self.lock()?;
        let mut pending: Vec<OutboxEvent> = inner.events.iter().filter(|e| e.status == EventStatus::Pending).cloned().collect();
        pending.sort_by(|a, b| {
            (a.created_at, &a.aggregate_type, &a.aggregate_id, a.sequence).cmp(&(b.created_at, &b.aggregate_type, &b.aggregate_id, b.sequence))
        });
        pending.truncate(limit);
        Ok(pending)
    }

    fn update(&self, id: u64, change: impl FnOnce(&mut OutboxEvent)) -> Result<(), OutboxError> {
        let mut inner = self.lock()?;
        let event = inner.events.iter_mut().find(|e| e.id == id).ok_or_else(|| OutboxError::UnknownEvent(id.to_string()))?;
        change(event);
        Ok(())
    }

    /// Put a quarantined event back in the queue after the cause is fixed.
    pub fn requeue(&self, id: u64) -> Result<(), OutboxError> {
        let mut inner = self.lock()?;
        let event = inner.events.iter_mut().find(|e| e.id == id).ok_or_else(|| OutboxError::UnknownEvent(id.to_string()))?;
        if event.status != EventStatus::Quarantined {
            return Err(OutboxError::NotQuarantined);
        }
        event.status = EventStatus::Pending;
        event.attempts = 0;
        event.last_error = None;
        Ok(())
    }

    pub fn quarantined(&self) -> Result<Vec<OutboxEvent>, OutboxError> {
        Ok(self.lock()?.events.iter().filter(|e| e.status == EventStatus::Quarantined).cloned().collect())
    }

    /// Lag figures for dashboards and alerts.
    pub fn stats(&self, now: u64) -> Result<OutboxStats, OutboxError> {
        let inner = self.lock()?;
        let pending: Vec<&OutboxEvent> = inner.events.iter().filter(|e| e.status == EventStatus::Pending).collect();
        Ok(OutboxStats {
            pending: pending.len(),
            quarantined: inner.events.iter().filter(|e| e.status == EventStatus::Quarantined).count(),
            published: inner.events.iter().filter(|e| e.status == EventStatus::Published).count(),
            oldest_pending_age_secs: pending.iter().map(|e| now.saturating_sub(e.created_at)).max(),
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct OutboxStats {
    pub pending: usize,
    pub quarantined: usize,
    pub published: usize,
    /// How long the oldest unpublished event has waited: the relay's lag.
    pub oldest_pending_age_secs: Option<u64>,
}

impl OutboxStats {
    pub fn is_lagging(&self, threshold_secs: u64) -> bool {
        self.oldest_pending_age_secs.is_some_and(|age| age > threshold_secs)
    }

    /// Prometheus text exposition for the `/metrics` endpoint.
    pub fn to_prometheus(&self) -> String {
        format!(
            "# TYPE outbox_pending gauge\noutbox_pending {}\n# TYPE outbox_quarantined gauge\noutbox_quarantined {}\n# TYPE outbox_lag_seconds gauge\noutbox_lag_seconds {}\n",
            self.pending,
            self.quarantined,
            self.oldest_pending_age_secs.unwrap_or(0)
        )
    }
}

// ---------------------------------------------------------------------------
// Relay
// ---------------------------------------------------------------------------

#[derive(Debug, Default, PartialEq, Eq)]
pub struct RelayReport {
    pub published: usize,
    pub failed: usize,
    pub quarantined: usize,
    /// Events skipped because an earlier event of their aggregate failed.
    pub held_back: usize,
}

pub struct Relay<B: Broker> {
    outbox: InMemoryOutbox,
    broker: B,
    max_attempts: u32,
    batch_size: usize,
}

impl<B: Broker> Relay<B> {
    pub fn new(outbox: InMemoryOutbox, broker: B) -> Self {
        Relay { outbox, broker, max_attempts: DEFAULT_MAX_ATTEMPTS, batch_size: DEFAULT_BATCH_SIZE }
    }

    pub fn max_attempts(mut self, attempts: u32) -> Self {
        self.max_attempts = attempts.max(1);
        self
    }

    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size.max(1);
        self
    }

    /// One polling pass.
    pub fn poll_once(&self) -> Result<RelayReport, OutboxError> {
        let mut report = RelayReport::default();
        let mut blocked: HashSet<(String, String)> = HashSet::new();

        for event in self.outbox.pending(self.batch_size)? {
            let aggregate = (event.aggregate_type.clone(), event.aggregate_id.clone());
            if blocked.contains(&aggregate) {
                report.held_back += 1;
                continue;
            }

            let message = Message {
                event_id: event.id,
                aggregate_type: event.aggregate_type,
                aggregate_id: event.aggregate_id,
                event_type: event.event_type,
                sequence: event.sequence,
                idempotency_key: event.idempotency_key,
                payload: event.payload,
            };

            match self.broker.publish(&message) {
                Ok(()) => {
                    self.outbox.update(event.id, |e| e.status = EventStatus::Published)?;
                    report.published += 1;
                }
                Err(error) => {
                    let attempts = event.attempts + 1;
                    if attempts >= self.max_attempts {
                        // Poison message: park it so the aggregate can move on.
                        self.outbox.update(event.id, |e| {
                            e.attempts = attempts;
                            e.last_error = Some(error);
                            e.status = EventStatus::Quarantined;
                        })?;
                        report.quarantined += 1;
                    } else {
                        self.outbox.update(event.id, |e| {
                            e.attempts = attempts;
                            e.last_error = Some(error);
                        })?;
                        blocked.insert(aggregate);
                        report.failed += 1;
                    }
                }
            }
        }
        Ok(report)
    }
}

// ---------------------------------------------------------------------------
// Consumer-side idempotency
// ---------------------------------------------------------------------------

/// Remembers which `(consumer, idempotency_key)` pairs were handled. Shared by
/// every worker of a consumer; the SQL form is [`sql::CLAIM_MESSAGE`].
#[derive(Clone, Default)]
pub struct IdempotentConsumer {
    processed: Arc<Mutex<HashMap<(String, String), ()>>>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum Handled {
    Processed,
    /// Seen before: the handler was not run again.
    Duplicate,
}

impl IdempotentConsumer {
    pub fn new() -> Self {
        Self::default()
    }

    /// Run `handler` for `message` unless `consumer` already handled its key.
    /// The key is recorded only after the handler succeeds, so a failure is
    /// retried on redelivery.
    pub fn handle(&self, consumer: &str, message: &Message, handler: impl FnOnce(&Message) -> Result<(), String>) -> Result<Handled, String> {
        let key = (consumer.to_string(), message.idempotency_key.clone());
        {
            let processed = self.processed.lock().map_err(|_| "consumer state unavailable".to_string())?;
            if processed.contains_key(&key) {
                return Ok(Handled::Duplicate);
            }
        }
        handler(message)?;
        self.processed
            .lock()
            .map_err(|_| "consumer state unavailable".to_string())?
            .insert(key, ());
        Ok(Handled::Processed)
    }
}

/// Group relay output by aggregate, for tests and diagnostics.
pub fn by_aggregate(messages: &[Message]) -> BTreeMap<String, Vec<u64>> {
    let mut grouped: BTreeMap<String, Vec<u64>> = BTreeMap::new();
    for message in messages {
        grouped.entry(message.partition_key()).or_default().push(message.sequence);
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[derive(Clone, Default)]
    struct FakeBroker {
        sent: Arc<Mutex<Vec<Message>>>,
        /// Event types that fail to publish.
        failing: Arc<Mutex<HashSet<String>>>,
    }

    impl FakeBroker {
        fn fail(&self, event_type: &str) {
            self.failing.lock().unwrap().insert(event_type.to_string());
        }
        fn heal(&self, event_type: &str) {
            self.failing.lock().unwrap().remove(event_type);
        }
        fn sent(&self) -> Vec<Message> {
            self.sent.lock().unwrap().clone()
        }
    }

    impl Broker for FakeBroker {
        fn publish(&self, message: &Message) -> Result<(), String> {
            if self.failing.lock().unwrap().contains(&message.event_type) {
                return Err(format!("broker rejected {}", message.event_type));
            }
            self.sent.lock().unwrap().push(message.clone());
            Ok(())
        }
    }

    fn event(aggregate: &str, id: &str, kind: &str, key: &str) -> NewEvent {
        NewEvent::new(aggregate, id, kind, json!({"k": key}), key)
    }

    #[test]
    fn events_are_invisible_until_the_transaction_commits_and_vanish_on_rollback() {
        let outbox = InMemoryOutbox::new();

        let mut rolled_back = outbox.begin(1);
        rolled_back.append(event("vault", "v1", "created", "k1"));
        drop(rolled_back); // business transaction failed
        assert!(outbox.events().unwrap().is_empty());

        let mut tx = outbox.begin(2);
        tx.append(event("vault", "v1", "created", "k2"));
        assert!(outbox.events().unwrap().is_empty(), "not visible before commit");
        assert_eq!(tx.commit().unwrap(), 1);
        assert_eq!(outbox.events().unwrap().len(), 1);
    }

    #[test]
    fn sequences_are_per_aggregate_and_duplicate_keys_are_ignored() {
        let outbox = InMemoryOutbox::new();
        let mut tx = outbox.begin(1);
        tx.append(event("vault", "v1", "created", "a"));
        tx.append(event("vault", "v2", "created", "b"));
        tx.append(event("vault", "v1", "updated", "c"));
        tx.append(event("vault", "v1", "updated", "c")); // same logical event again
        assert_eq!(tx.commit().unwrap(), 3);

        let seqs: Vec<(String, u64)> = outbox.events().unwrap().iter().map(|e| (e.aggregate_id.clone(), e.sequence)).collect();
        assert_eq!(seqs, vec![("v1".into(), 1), ("v2".into(), 1), ("v1".into(), 2)]);

        let mut again = outbox.begin(2);
        again.append(event("vault", "v1", "created", "a"));
        assert_eq!(again.commit().unwrap(), 0, "retrying the whole handler adds nothing");
    }

    #[test]
    fn the_relay_publishes_in_order_per_aggregate_and_marks_events_published() {
        let outbox = InMemoryOutbox::new();
        let broker = FakeBroker::default();
        let mut tx = outbox.begin(1);
        tx.append(event("vault", "v1", "created", "a"));
        tx.append(event("vault", "v2", "created", "b"));
        tx.append(event("vault", "v1", "updated", "c"));
        tx.commit().unwrap();

        let report = Relay::new(outbox.clone(), broker.clone()).poll_once().unwrap();

        assert_eq!(report, RelayReport { published: 3, ..Default::default() });
        let grouped = by_aggregate(&broker.sent());
        assert_eq!(grouped["vault:v1"], vec![1, 2]);
        assert_eq!(grouped["vault:v2"], vec![1]);
        assert_eq!(outbox.stats(10).unwrap().published, 3);
        assert_eq!(Relay::new(outbox, broker).poll_once().unwrap(), RelayReport::default(), "nothing is sent twice");
    }

    #[test]
    fn a_failed_event_holds_back_its_aggregate_but_not_others() {
        let outbox = InMemoryOutbox::new();
        let broker = FakeBroker::default();
        broker.fail("created");
        let mut tx = outbox.begin(1);
        tx.append(event("vault", "v1", "created", "a"));
        tx.append(event("vault", "v1", "updated", "b")); // must not overtake `created`
        tx.append(event("vault", "v2", "updated", "c"));
        tx.commit().unwrap();

        let relay = Relay::new(outbox.clone(), broker.clone());
        let report = relay.poll_once().unwrap();
        assert_eq!(report, RelayReport { published: 1, failed: 1, held_back: 1, quarantined: 0 });
        assert_eq!(broker.sent().iter().map(|m| m.aggregate_id.as_str()).collect::<Vec<_>>(), vec!["v2"]);

        broker.heal("created");
        let report = relay.poll_once().unwrap();
        assert_eq!(report.published, 2);
        assert_eq!(by_aggregate(&broker.sent())["vault:v1"], vec![1, 2], "order preserved after recovery");
    }

    #[test]
    fn a_poison_message_is_quarantined_and_stops_blocking_its_aggregate() {
        let outbox = InMemoryOutbox::new();
        let broker = FakeBroker::default();
        broker.fail("poison");
        let mut tx = outbox.begin(1);
        tx.append(event("vault", "v1", "poison", "a"));
        tx.append(event("vault", "v1", "fine", "b"));
        tx.commit().unwrap();
        let relay = Relay::new(outbox.clone(), broker.clone()).max_attempts(3);

        let first = relay.poll_once().unwrap();
        assert_eq!((first.failed, first.held_back, first.published), (1, 1, 0), "`fine` waits behind `poison`");
        relay.poll_once().unwrap();
        // Third failure: quarantined, so `fine` is released in the same pass.
        let third = relay.poll_once().unwrap();
        assert_eq!((third.quarantined, third.published), (1, 1));

        let quarantined = outbox.quarantined().unwrap();
        assert_eq!((quarantined.len(), quarantined[0].attempts), (1, 3));
        assert_eq!(quarantined[0].last_error.as_deref(), Some("broker rejected poison"));
        // Consumers can see the gap: only sequence 2 arrived.
        assert_eq!(broker.sent()[0].sequence, 2);
        assert_eq!(outbox.stats(5).unwrap().quarantined, 1);

        broker.heal("poison");
        outbox.requeue(quarantined[0].id).unwrap();
        assert_eq!(relay.poll_once().unwrap().published, 1);
        assert_eq!(outbox.requeue(quarantined[0].id), Err(OutboxError::NotQuarantined));
    }

    #[test]
    fn lag_is_the_age_of_the_oldest_pending_event() {
        let outbox = InMemoryOutbox::new();
        assert_eq!(outbox.stats(100).unwrap().oldest_pending_age_secs, None);

        let mut tx = outbox.begin(40);
        tx.append(event("vault", "v1", "created", "a"));
        tx.commit().unwrap();
        let mut tx = outbox.begin(90);
        tx.append(event("vault", "v2", "created", "b"));
        tx.commit().unwrap();

        let stats = outbox.stats(100).unwrap();
        assert_eq!((stats.pending, stats.oldest_pending_age_secs), (2, Some(60)));
        assert!(stats.is_lagging(30));
        assert!(!stats.is_lagging(60));
        let text = stats.to_prometheus();
        assert!(text.contains("outbox_pending 2") && text.contains("outbox_lag_seconds 60"));
    }

    #[test]
    fn batches_are_bounded() {
        let outbox = InMemoryOutbox::new();
        let mut tx = outbox.begin(1);
        for i in 0..5 {
            tx.append(event("vault", &format!("v{i}"), "created", &format!("k{i}")));
        }
        tx.commit().unwrap();

        let relay = Relay::new(outbox.clone(), FakeBroker::default()).batch_size(2);
        assert_eq!(relay.poll_once().unwrap().published, 2);
        assert_eq!(outbox.stats(5).unwrap().pending, 3);
    }

    #[test]
    fn consumers_apply_each_event_once_even_when_it_is_delivered_twice() {
        let consumer = IdempotentConsumer::new();
        let message = Message {
            event_id: 1,
            aggregate_type: "vault".into(),
            aggregate_id: "v1".into(),
            event_type: "created".into(),
            sequence: 1,
            idempotency_key: "k1".into(),
            payload: json!({}),
        };
        let applied = Arc::new(Mutex::new(0));

        for _ in 0..3 {
            let applied = applied.clone();
            consumer.handle("billing", &message, move |_| {
                *applied.lock().unwrap() += 1;
                Ok(())
            }).unwrap();
        }
        assert_eq!(*applied.lock().unwrap(), 1);

        // A different consumer has its own record.
        assert_eq!(consumer.handle("search", &message, |_| Ok(())).unwrap(), Handled::Processed);
    }

    #[test]
    fn a_failing_handler_is_retried_on_redelivery() {
        let consumer = IdempotentConsumer::new();
        let message = Message {
            event_id: 1,
            aggregate_type: "a".into(),
            aggregate_id: "1".into(),
            event_type: "e".into(),
            sequence: 1,
            idempotency_key: "k".into(),
            payload: json!({}),
        };
        assert!(consumer.handle("c", &message, |_| Err("db down".into())).is_err());
        assert_eq!(consumer.handle("c", &message, |_| Ok(())).unwrap(), Handled::Processed);
        assert_eq!(consumer.handle("c", &message, |_| Ok(())).unwrap(), Handled::Duplicate);
    }

    #[test]
    fn the_sql_targets_the_columns_the_migration_creates() {
        for statement in [sql::INSERT_EVENT, sql::SELECT_PENDING, sql::MARK_PUBLISHED, sql::QUARANTINE] {
            assert!(statement.contains("outbox_events"));
        }
        assert!(sql::INSERT_EVENT.contains("ON CONFLICT (idempotency_key) DO NOTHING"));
        assert!(sql::SELECT_PENDING.contains("ORDER BY created_at, aggregate_type, aggregate_id, sequence"));
        assert!(sql::CLAIM_MESSAGE.contains("processed_messages"));
    }
}
