# Transactional outbox

`core/src/outbox.rs` and migration `008_create_outbox.sql` make event publishing reliable.

## Pattern

1. **Write.** In the same database transaction as the business change, insert the event into
   `outbox_events` (`sql::INSERT_EVENT`). If the transaction rolls back, the event never
   existed; if it commits, the event is guaranteed to be published eventually.
2. **Relay.** A separate process (`Relay::poll_once`) reads pending events oldest first,
   publishes them to the broker (Kafka/RabbitMQ behind `Broker`) and marks them published.
   Delivery is at least once.
3. **Consume.** Consumers record each `idempotency_key` they have handled
   (`processed_messages`, `IdempotentConsumer`), so a redelivered event has no second
   effect. The key is recorded after the handler succeeds, so failures are retried.

## Guarantees

* **Duplicate detection.** `idempotency_key` is unique: re-running a handler that appends the
  same logical event adds nothing.
* **Ordering per aggregate.** Each event gets a per-aggregate `sequence`. Messages for one
  aggregate go to one broker partition (`Message::partition_key`), and while an event is
  failing, later events of that aggregate are held back. Other aggregates are not delayed.
* **Poison messages.** After `max_attempts` (default 5) an event is quarantined
  (`status = 'QUARANTINED'`) so it stops blocking its aggregate. Consumers see the gap in
  `sequence`. Fix the cause, then `requeue` it.

## Monitoring

`OutboxStats` reports `pending`, `quarantined`, `published` and the lag (age of the oldest
pending event). `to_prometheus()` renders `outbox_pending`, `outbox_quarantined` and
`outbox_lag_seconds`; alert when `is_lagging(threshold)` or `quarantined > 0`.

## Housekeeping

Prune published events after your retention window with `sql::PRUNE_PUBLISHED`.
