//! WebSocket streaming for simulation progress (Issue #105).
//!
//! # Design
//!
//! A lightweight pub/sub bus ([`SimulationBus`]) wraps a Tokio
//! [`broadcast`] channel.  Any part of the application that holds an
//! [`Arc<SimulationBus>`] can publish [`SimulationEvent`]s.  The
//! [`JobWorker`](crate::jobs::JobWorker) is the primary publisher; the
//! WebSocket upgrade handler ([`ws_handler`]) is the primary consumer.
//!
//! ## Client protocol
//!
//! Connect with:
//! ```
//! GET /ws/jobs/<job_id>
//! Upgrade: websocket
//! ```
//!
//! The server streams newline-delimited JSON frames until the job
//! reaches a terminal state (`completed` / `failed` / `cancelled`), at
//! which point it sends the final event and closes the connection.
//!
//! ### Event shape
//! ```json
//! {
//!   "event": "progress",
//!   "job_id": "550e8400-e29b-41d4-a716-446655440000",
//!   "data": { "percent": 30, "message": "Running simulation" },
//!   "timestamp": "2026-04-25T12:00:00Z"
//! }
//! ```
//!
//! `event` can be: `progress` | `provider_failover` | `consensus_check`
//! | `completed` | `failed`.

use axum::{
    extract::{
        ws::{Message, WebSocket, WebSocketUpgrade},
        State,
    },
    response::IntoResponse,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;

use crate::backpressure::{
    BackpressurePolicy, BackpressureStats, BoundedEventBus, BoundedSubscription, PublishOutcome,
};
use crate::input_sanitization::SanitizedPath;
use crate::jobs::JobId;

// ── Channel capacity ─────────────────────────────────────────────────────────

/// Number of events that can be buffered per broadcast channel slot before
/// slow consumers are forced to drop events via `RecvError::Lagged`.
const BUS_CAPACITY: usize = 256;
const BUS_PUBLISH_TIMEOUT: Duration = Duration::from_millis(100);

// ── Heartbeat & reconnection (CORE-25) ───────────────────────────────────────

/// Interval between server-sent `Ping` frames on an idle connection.
pub const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);

/// If no `Pong` (or any inbound frame) arrives within this window the
/// connection is considered dead: the server closes the socket so the client
/// can reconnect, rather than leaving a zombie stream open.
pub const PONG_TIMEOUT: Duration = Duration::from_secs(30);

/// Base delay of the exponential reconnect backoff.
pub const RECONNECT_BASE_DELAY: Duration = Duration::from_millis(500);

/// Upper bound on the reconnect backoff delay.
pub const RECONNECT_MAX_DELAY: Duration = Duration::from_secs(30);

/// Growth factor between successive reconnect attempts.
pub const RECONNECT_MULTIPLIER: f64 = 2.0;

/// Jitter ratio (0–1) mixed into the backoff so reconnecting clients do not
/// stampede the server in lockstep.
pub const RECONNECT_JITTER: f64 = 0.2;

// ── Event types ──────────────────────────────────────────────────────────────

/// Progress update emitted at each stage of job execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProgressPayload {
    /// Completion percentage (0–100).
    pub percent: i32,
    /// Human-readable status message.
    pub message: String,
}

/// Emitted when the engine fails over to a different RPC provider.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderFailoverPayload {
    /// Provider that failed / was tripped.
    pub from_provider: String,
    /// Provider now being used.
    pub to_provider: String,
    /// Reason for the failover (e.g. "timeout", "http_error").
    pub reason: String,
}

/// Emitted once per consensus quorum check (only in `consensus` mode).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConsensusCheckPayload {
    /// Whether all sampled providers agreed on resources + ledger changes.
    pub agreement: bool,
    /// Providers that were queried.
    pub providers: Vec<String>,
    /// Optional human-readable mismatch summary.
    pub detail: Option<String>,
}

/// Emitted when a job finishes successfully — carries the full resource report.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompletedPayload {
    pub cpu_instructions: u64,
    pub ram_bytes: u64,
    pub ledger_read_bytes: u64,
    pub ledger_write_bytes: u64,
    pub transaction_size_bytes: u64,
    pub cost_stroops: u64,
}

/// Emitted when a job fails.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedPayload {
    pub error: String,
    pub error_type: String,
}

/// All events that can be published on the [`SimulationBus`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum SimulationEvent {
    Progress {
        job_id: String,
        data: ProgressPayload,
        timestamp: DateTime<Utc>,
    },
    ProviderFailover {
        job_id: String,
        data: ProviderFailoverPayload,
        timestamp: DateTime<Utc>,
    },
    ConsensusCheck {
        job_id: String,
        data: ConsensusCheckPayload,
        timestamp: DateTime<Utc>,
    },
    Completed {
        job_id: String,
        data: CompletedPayload,
        timestamp: DateTime<Utc>,
    },
    Failed {
        job_id: String,
        data: FailedPayload,
        timestamp: DateTime<Utc>,
    },
}

impl SimulationEvent {
    /// Returns the `job_id` string embedded in every event variant.
    pub fn job_id(&self) -> &str {
        match self {
            Self::Progress { job_id, .. } => job_id,
            Self::ProviderFailover { job_id, .. } => job_id,
            Self::ConsensusCheck { job_id, .. } => job_id,
            Self::Completed { job_id, .. } => job_id,
            Self::Failed { job_id, .. } => job_id,
        }
    }

    /// Returns `true` if this event signals the end of a job.
    pub fn is_terminal(&self) -> bool {
        matches!(self, Self::Completed { .. } | Self::Failed { .. })
    }
}

// ── Bus ──────────────────────────────────────────────────────────────────────

/// Application-wide pub/sub bus for simulation events.
///
/// Clone the bus cheaply via [`Arc`]; call [`SimulationBus::publish`] from any
/// async context and [`SimulationBus::subscribe`] to get a receiver.
#[derive(Clone)]
pub struct SimulationBus {
    sender: broadcast::Sender<SimulationEvent>,
    bounded: BoundedEventBus<SimulationEvent>,
}

impl SimulationBus {
    /// Create a new bus with the default channel capacity.
    pub fn new() -> Arc<Self> {
        let (sender, _) = broadcast::channel(BUS_CAPACITY);
        Arc::new(Self {
            sender,
            bounded: BoundedEventBus::new(BUS_CAPACITY, BackpressurePolicy::Wait),
        })
    }

    pub fn publish(&self, event: SimulationEvent) -> usize {
        if self.bounded.is_closed() {
            return 0;
        }
        if event.is_terminal() {
            self.bounded
                .publish_with_policy(event.clone(), BackpressurePolicy::DropOldest);
        } else {
            self.bounded.publish(event.clone());
        }
        self.sender.send(event).unwrap_or(0)
    }

    pub async fn publish_async(&self, event: SimulationEvent) -> PublishOutcome {
        if self.bounded.is_closed() {
            return PublishOutcome::Closed;
        }
        let outcome = if event.is_terminal() {
            self.bounded
                .publish_with_policy(event.clone(), BackpressurePolicy::DropOldest)
        } else {
            self.bounded
                .publish_async(event.clone(), Some(BUS_PUBLISH_TIMEOUT))
                .await
        };
        let _ = self.sender.send(event);
        outcome
    }

    pub fn subscribe(&self) -> broadcast::Receiver<SimulationEvent> {
        self.sender.subscribe()
    }

    pub fn subscribe_bounded(&self) -> BoundedSubscription<SimulationEvent> {
        self.bounded.subscribe()
    }

    pub fn backpressure_stats(&self) -> BackpressureStats {
        self.bounded.stats()
    }

    pub fn close(&self) {
        self.bounded.close();
    }

    // ── Convenience constructors ─────────────────────────────────────────

    pub fn progress(job_id: &JobId, percent: i32, message: impl Into<String>) -> SimulationEvent {
        SimulationEvent::Progress {
            job_id: job_id.to_string(),
            data: ProgressPayload {
                percent,
                message: message.into(),
            },
            timestamp: Utc::now(),
        }
    }

    pub fn provider_failover(
        job_id: &JobId,
        from_provider: impl Into<String>,
        to_provider: impl Into<String>,
        reason: impl Into<String>,
    ) -> SimulationEvent {
        SimulationEvent::ProviderFailover {
            job_id: job_id.to_string(),
            data: ProviderFailoverPayload {
                from_provider: from_provider.into(),
                to_provider: to_provider.into(),
                reason: reason.into(),
            },
            timestamp: Utc::now(),
        }
    }

    pub fn consensus_check(
        job_id: &JobId,
        agreement: bool,
        providers: Vec<String>,
        detail: Option<String>,
    ) -> SimulationEvent {
        SimulationEvent::ConsensusCheck {
            job_id: job_id.to_string(),
            data: ConsensusCheckPayload {
                agreement,
                providers,
                detail,
            },
            timestamp: Utc::now(),
        }
    }

    pub fn completed(
        job_id: &JobId,
        resources: &crate::simulation::SorobanResources,
        cost_stroops: u64,
    ) -> SimulationEvent {
        SimulationEvent::Completed {
            job_id: job_id.to_string(),
            data: CompletedPayload {
                cpu_instructions: resources.cpu_instructions,
                ram_bytes: resources.ram_bytes,
                ledger_read_bytes: resources.ledger_read_bytes,
                ledger_write_bytes: resources.ledger_write_bytes,
                transaction_size_bytes: resources.transaction_size_bytes,
                cost_stroops,
            },
            timestamp: Utc::now(),
        }
    }

    pub fn failed(
        job_id: &JobId,
        error: impl Into<String>,
        error_type: impl Into<String>,
    ) -> SimulationEvent {
        SimulationEvent::Failed {
            job_id: job_id.to_string(),
            data: FailedPayload {
                error: error.into(),
                error_type: error_type.into(),
            },
            timestamp: Utc::now(),
        }
    }
}

impl Default for SimulationBus {
    fn default() -> Self {
        let (sender, _) = broadcast::channel(BUS_CAPACITY);
        Self {
            sender,
            bounded: BoundedEventBus::new(BUS_CAPACITY, BackpressurePolicy::Wait),
        }
    }
}

// ── Connection lifecycle (CORE-25) ───────────────────────────────────────────

/// Lifecycle state of a WebSocket session.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectionState {
    /// The wire is up and the server is streaming events and heartbeats.
    Connected,
    /// The wire dropped; the client is waiting [`reconnect_backoff`] before
    /// dialling again.
    Reconnecting { attempt: u32 },
    /// Terminal — the session will not be resumed (e.g. job finished).
    Closed,
}

impl ConnectionState {
    /// Returns `true` while a client may still dial back in.
    pub fn is_resumable(&self) -> bool {
        !matches!(self, Self::Closed)
    }
}

/// Exponential backoff delay (with jitter) before the `attempt`-th reconnect,
/// where `0` is the first attempt after a drop.
///
/// ```text
/// delay = min(max_delay, base * multiplier^min(attempt, cap))
///       × (1 - jitter + 2·jitter·rand)
/// ```
///
/// Bounded by [`RECONNECT_MAX_DELAY`]; mixed with [`RECONNECT_JITTER`] so a
/// field of dropped clients reconnects over a spread of delays.
pub fn reconnect_backoff(attempt: u32) -> Duration {
    let exponent = attempt.min(20) as i32;
    let raw = RECONNECT_BASE_DELAY.as_millis() as f64 * RECONNECT_MULTIPLIER.powi(exponent);
    let capped = raw.min(RECONNECT_MAX_DELAY.as_millis() as f64);
    let rnd = rand::random::<f64>();
    let jittered = capped * (1.0 - RECONNECT_JITTER + (2.0 * RECONNECT_JITTER * rnd));
    Duration::from_millis(jittered.max(1.0) as u64)
}

// ── Simulation memoization (CORE-24) ─────────────────────────────────────────

/// Upper bound on the number of terminal results retained in the memo.
const MEMO_MAX_ENTRIES: usize = 1024;

/// Content-addressable cache of simulation results, keyed by a hash of the
/// request inputs (contract + function + serialised inputs).
///
/// When a request matches one already completed — e.g. a WebSocket client
/// reconnecting after its stream dropped, or a frequently-run analysis — the
/// cached terminal result is replayed instead of re-running the full
/// simulation (CORE-24).
///
/// This module is deliberately small and self-contained: wiring it into the
/// simulation runner and the WebSocket handler is left to the contributors who
/// own those modules.
pub struct SimulationMemo {
    entries: Mutex<HashMap<String, SimulationEvent>>,
}

impl SimulationMemo {
    /// Create an empty memo.
    pub fn new() -> Self {
        Self {
            entries: Mutex::new(HashMap::new()),
        }
    }

    /// Content-addressable request hash over length-prefixed input segments.
    ///
    /// Length-prefixes each segment (mirroring `audit_log`) so no two different
    /// splits of the same bytes can collide.
    pub fn request_hash(segments: &[&[u8]]) -> String {
        let mut hasher = Sha256::new();
        for segment in segments {
            hasher.update((segment.len() as u64).to_be_bytes());
            hasher.update(segment);
        }
        hex::encode(hasher.finalize())
    }

    /// Number of cached results.
    pub fn len(&self) -> usize {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .len()
    }

    /// Whether the memo holds no results.
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Store `event` under `request_hash` if it is terminal, evicting an
    /// arbitrary entry once the memo reaches [`MEMO_MAX_ENTRIES`].
    pub fn store(&self, request_hash: &str, event: &SimulationEvent) {
        if !event.is_terminal() {
            return;
        }

        let mut guard = self
            .entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let entries = &mut *guard;

        if entries.len() >= MEMO_MAX_ENTRIES && !entries.contains_key(request_hash) {
            if let Some(stale) = entries.keys().next().cloned() {
                entries.remove(&stale);
            }
        }

        entries.insert(request_hash.to_string(), event.clone());
    }

    /// Look up a previously stored terminal result, if any.
    pub fn get(&self, request_hash: &str) -> Option<SimulationEvent> {
        self.entries
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .get(request_hash)
            .cloned()
    }
}

impl Default for SimulationMemo {
    fn default() -> Self {
        Self::new()
    }
}

/// Process-wide memo backing [`memoize_result`] and [`memoized_result`].
fn global_memo() -> &'static SimulationMemo {
    static MEMO: OnceLock<SimulationMemo> = OnceLock::new();
    MEMO.get_or_init(SimulationMemo::new)
}

/// Record a terminal simulation result under its request hash (CORE-24).
pub fn memoize_result(request_hash: &str, event: &SimulationEvent) {
    global_memo().store(request_hash, event);
}

/// Replay a previously memoised terminal result, if the request has been seen.
pub fn memoized_result(request_hash: &str) -> Option<SimulationEvent> {
    global_memo().get(request_hash)
}

// ── Axum extractor alias ─────────────────────────────────────────────────────

/// Shared state slice required by the WebSocket handler.
/// The handler accesses the bus through the main [`AppState`](crate::AppState).
pub struct WsState {
    pub bus: Arc<SimulationBus>,
}

// ── WebSocket handler ─────────────────────────────────────────────────────────

/// Upgrade handler for `GET /ws/jobs/:job_id`.
///
/// Clients connect with a standard WebSocket handshake; the server streams
/// JSON-serialised [`SimulationEvent`]s until the job reaches a terminal state
/// or the client disconnects.
#[utoipa::path(
    get,
    path = "/ws/jobs/{job_id}",
    params(
        ("job_id" = String, Path, description = "Job ID")
    ),
    responses(
        (status = 101, description = "Switching Protocols to WebSocket")
    ),
    tag = "Streaming"
)]
pub async fn ws_handler(
    ws: WebSocketUpgrade,
    SanitizedPath(job_id): SanitizedPath<String>,
    State(state): State<Arc<crate::AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| handle_socket(socket, job_id, state))
}

async fn handle_socket(mut socket: WebSocket, job_id: String, state: Arc<crate::AppState>) {
    tracing::info!(job_id = %job_id, "WebSocket client connected");

    let mut rx = state.simulation_bus.subscribe_bounded();

    // CORE-25: heartbeat cadence + liveness window so dead sessions are closed
    // instead of left half-open.
    let mut heartbeat = tokio::time::interval(HEARTBEAT_INTERVAL);
    heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let mut last_pong = Instant::now();
    let mut connection_state = ConnectionState::Connected;

    loop {
        tokio::select! {
            // CORE-25: send a periodic ping and watch for the pong timeout.
            _ = heartbeat.tick() => {
                if last_pong.elapsed() >= PONG_TIMEOUT {
                    tracing::warn!(
                        job_id = %job_id,
                        elapsed_ms = last_pong.elapsed().as_millis(),
                        "WebSocket heartbeat missed — closing connection for reconnection"
                    );
                    connection_state = ConnectionState::Reconnecting { attempt: 0 };
                    break;
                }

                if socket
                    .send(Message::Ping("perigee-heartbeat".into()))
                    .await
                    .is_err()
                {
                    connection_state = ConnectionState::Reconnecting { attempt: 0 };
                    break;
                }
            }

            // Receive next event from the bus
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        // Only forward events belonging to the requested job
                        if event.job_id() != job_id {
                            continue;
                        }

                        let is_terminal = event.is_terminal();

                        let json = match serde_json::to_string(&event) {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::error!(
                                    job_id = %job_id,
                                    error = %e,
                                    "Failed to serialise SimulationEvent"
                                );
                                continue;
                            }
                        };

                        if socket.send(Message::Text(json)).await.is_err() {
                            // Client disconnected
                            connection_state = ConnectionState::Reconnecting { attempt: 0 };
                            break;
                        }

                        if is_terminal {
                            // Close gracefully after the terminal event
                            connection_state = ConnectionState::Closed;
                            let _ = socket.send(Message::Close(None)).await;
                            break;
                        }
                    }
                    Err(_) => {
                        connection_state = ConnectionState::Closed;
                        break;
                    }
                }
            }

            // Echo / ping handling: consume incoming messages from the client
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Ping(payload))) => {
                        last_pong = Instant::now();
                        let _ = socket.send(Message::Pong(payload)).await;
                    }
                    Some(Ok(Message::Pong(_))) => {
                        // Pong in reply to our heartbeat — the client is alive.
                        last_pong = Instant::now();
                    }
                    Some(Ok(Message::Close(_))) | None => {
                        connection_state = ConnectionState::Closed;
                        break;
                    }
                    _ => {} // ignore text/binary frames from the client
                }
            }
        }
    }

    tracing::info!(
        job_id = %job_id,
        connection_state = ?connection_state,
        "WebSocket client disconnected"
    );
}

// ── Unit tests ────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn bus_publish_and_receive() {
        let bus = SimulationBus::new();
        let mut rx = bus.subscribe();

        let fake_id = JobId::new();
        let event = SimulationBus::progress(&fake_id, 42, "halfway there");
        bus.publish(event);

        let received = rx.recv().await.expect("should receive event");
        assert_eq!(received.job_id(), fake_id.to_string());
        assert!(!received.is_terminal());
    }

    #[tokio::test]
    async fn terminal_events_are_identified_correctly() {
        let fake_id = JobId::new();

        let progress = SimulationBus::progress(&fake_id, 50, "running");
        assert!(!progress.is_terminal());

        let failed = SimulationBus::failed(&fake_id, "oops", "NetworkError");
        assert!(failed.is_terminal());

        let resources = crate::simulation::SorobanResources {
            cpu_instructions: 1000,
            ram_bytes: 2048,
            ledger_read_bytes: 128,
            ledger_write_bytes: 64,
            transaction_size_bytes: 512,
        };
        let completed = SimulationBus::completed(&fake_id, &resources, 500);
        assert!(completed.is_terminal());
    }

    #[tokio::test]
    async fn event_json_round_trips() {
        let fake_id = JobId::new();
        let event =
            SimulationBus::provider_failover(&fake_id, "primary-node", "backup-node", "timeout");
        let json = serde_json::to_string(&event).expect("serialise");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["event"], "provider_failover");
        assert_eq!(parsed["data"]["from_provider"], "primary-node");
    }

    #[tokio::test]
    async fn consensus_check_event_serialises() {
        let fake_id = JobId::new();
        let event = SimulationBus::consensus_check(
            &fake_id,
            true,
            vec![
                "node-a".to_string(),
                "node-b".to_string(),
                "node-c".to_string(),
            ],
            None,
        );
        let json = serde_json::to_string(&event).expect("serialise");
        let parsed: serde_json::Value = serde_json::from_str(&json).expect("parse");
        assert_eq!(parsed["event"], "consensus_check");
        assert_eq!(parsed["data"]["agreement"], true);
    }

    #[tokio::test]
    async fn no_subscribers_does_not_panic() {
        let bus = SimulationBus::new();
        let fake_id = JobId::new();
        // publish with zero subscribers — should silently return 0
        let n = bus.publish(SimulationBus::progress(&fake_id, 10, "start"));
        assert_eq!(n, 0);
    }

    #[test]
    fn reconnect_backoff_is_bounded_and_grows() {
        let first = reconnect_backoff(0);
        let later = reconnect_backoff(4);

        // Always at least 1 ms (never a busy spin) and never beyond the cap.
        assert!(first.as_millis() >= 1);
        assert!(later.as_millis() <= RECONNECT_MAX_DELAY.as_millis());

        // Enough backoff growth that a later attempt is strictly later in
        // expectation — we assert within jitter bounds for attempt 0 vs 4.
        let raw_0 = RECONNECT_BASE_DELAY.as_millis() as f64;
        let raw_4 = raw_0 * RECONNECT_MULTIPLIER.powi(4);
        assert!(first.as_millis() as f64 <= raw_0 * 1.2);
        assert!(later.as_millis() as f64 <= raw_4.min(RECONNECT_MAX_DELAY.as_millis() as f64) * 1.2);
        assert!(later.as_millis() as f64 >= raw_4 * 0.4);
    }

    #[test]
    fn reconnect_backoff_caps_at_max_delay() {
        let capped = reconnect_backoff(100);
        let max_ms = RECONNECT_MAX_DELAY.as_millis() as f64;
        assert!(capped.as_millis() as f64 >= max_ms * 0.4);
        assert!(capped.as_millis() as f64 <= max_ms * 1.2);
    }

    #[test]
    fn connection_state_resumability() {
        assert!(ConnectionState::Connected.is_resumable());
        assert!(ConnectionState::Reconnecting { attempt: 3 }.is_resumable());
        assert!(!ConnectionState::Closed.is_resumable());
    }

    #[test]
    fn request_hash_is_content_addressable() {
        // Same inputs → same hash.
        let a = SimulationMemo::request_hash(&["contract:1".as_bytes(), "fn=mint".as_bytes()]);
        let b = SimulationMemo::request_hash(&["contract:1".as_bytes(), "fn=mint".as_bytes()]);
        assert_eq!(a, b);

        // Different inputs → different hash.
        let c = SimulationMemo::request_hash(&["contract:1".as_bytes(), "fn=burn".as_bytes()]);
        assert_ne!(a, c);

        // Length-prefix encoding keeps splits unambiguous.
        let split = SimulationMemo::request_hash(&["ab".as_bytes(), "c".as_bytes()]);
        let joined = SimulationMemo::request_hash(&["a".as_bytes(), "bc".as_bytes()]);
        assert_ne!(split, joined);
    }

    #[tokio::test]
    async fn memo_stores_and_replays_terminal_results() {
        let memo = SimulationMemo::new();
        let fake_id = JobId::new();
        let resources = crate::simulation::SorobanResources {
            cpu_instructions: 1000,
            ram_bytes: 2048,
            ledger_read_bytes: 128,
            ledger_write_bytes: 64,
            transaction_size_bytes: 512,
        };
        let completed = SimulationBus::completed(&fake_id, &resources, 500);
        let hash = SimulationMemo::request_hash(&["contract:1".as_bytes(), "fn=mint".as_bytes()]);

        assert!(memo.get(&hash).is_none());
        memo.store(&hash, &completed);

        let replayed = memo.get(&hash).expect("stored terminal result");
        assert!(replayed.is_terminal());
        assert_eq!(replayed.job_id(), fake_id.to_string());
    }

    #[test]
    fn memo_ignores_non_terminal_events() {
        let memo = SimulationMemo::new();
        let fake_id = JobId::new();
        // A progress event must never be memoised — only terminal results.
        let progress = SimulationBus::progress(&fake_id, 50, "running");
        memo.store("some-hash", &progress);
        assert!(memo.is_empty());
    }
}
