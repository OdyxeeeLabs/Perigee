//! Graceful shutdown coordinator — Issue #594.
//!
//! Handles SIGTERM / SIGINT for zero-downtime deployments.
//!
//! # Shutdown sequence
//! 1. Mark the service as "shutting down" (stop accepting new requests).
//! 2. Wait for all in-flight requests to complete (configurable timeout,
//!    default 30 s).
//! 3. Close database connection pools.
//! 4. Flush pending log / metrics buffers.
//! 5. Close message-queue consumers.
//! 6. Exit with status 0.

use std::sync::{
    atomic::{AtomicBool, AtomicUsize, Ordering},
    Arc,
};
use std::time::Duration;
use tokio::sync::broadcast;
use tokio::time::timeout;

// ---------------------------------------------------------------------------
// ShutdownCoordinator
// ---------------------------------------------------------------------------

/// Configuration for the graceful shutdown process.
#[derive(Debug, Clone)]
pub struct ShutdownConfig {
    /// Maximum time to wait for in-flight requests to drain.
    /// After this deadline the process exits regardless.
    pub drain_timeout: Duration,
    /// When `true`, the coordinator logs each lifecycle step.
    pub verbose: bool,
}

impl Default for ShutdownConfig {
    fn default() -> Self {
        Self {
            drain_timeout: Duration::from_secs(30),
            verbose: true,
        }
    }
}

/// Shared state that all request handlers should hold a reference to.
#[derive(Debug, Default)]
pub struct ShutdownState {
    /// Set to `true` once a shutdown signal is received. New requests should
    /// check this flag and return 503 immediately.
    pub shutting_down: AtomicBool,
    /// Running count of in-flight requests. Incremented on accept, decremented
    /// on completion.
    pub in_flight: AtomicUsize,
}

impl ShutdownState {
    /// Returns `true` if the service is currently accepting new requests.
    pub fn is_accepting(&self) -> bool {
        !self.shutting_down.load(Ordering::SeqCst)
    }

    /// Record that a new request has started.
    pub fn request_started(&self) {
        self.in_flight.fetch_add(1, Ordering::SeqCst);
    }

    /// Record that a request has completed.
    pub fn request_finished(&self) {
        self.in_flight.fetch_sub(1, Ordering::SeqCst);
    }

    /// Current number of in-flight requests.
    pub fn in_flight_count(&self) -> usize {
        self.in_flight.load(Ordering::SeqCst)
    }
}

// ---------------------------------------------------------------------------
// GracefulShutdown
// ---------------------------------------------------------------------------

/// Coordinates a multi-step graceful shutdown.
pub struct GracefulShutdown {
    config: ShutdownConfig,
    state: Arc<ShutdownState>,
    /// Broadcast sender — recipients are per-component shutdown watchers.
    signal_tx: broadcast::Sender<()>,
}

impl GracefulShutdown {
    /// Create a new coordinator with the given configuration.
    pub fn new(config: ShutdownConfig) -> (Self, Arc<ShutdownState>) {
        let state = Arc::new(ShutdownState::default());
        let (signal_tx, _) = broadcast::channel(1);
        let coord = Self {
            config,
            state: Arc::clone(&state),
            signal_tx,
        };
        (coord, state)
    }

    /// Returns a receiver that fires when shutdown is initiated.
    pub fn subscribe(&self) -> broadcast::Receiver<()> {
        self.signal_tx.subscribe()
    }

    /// Begin the shutdown sequence. This method drives all steps to
    /// completion before returning.
    ///
    /// # Shutdown steps
    /// 1. Mark `shutting_down = true` so new requests are rejected.
    /// 2. Broadcast the shutdown signal to all subscribers.
    /// 3. Drain in-flight requests (up to `drain_timeout`).
    /// 4. (Caller-supplied) close databases, flush logs, close queues.
    /// 5. Return `Ok(())`.
    pub async fn shutdown(&self) -> Result<(), ShutdownError> {
        // Step 1: stop accepting new requests.
        self.state.shutting_down.store(true, Ordering::SeqCst);
        if self.config.verbose {
            tracing::info!("graceful_shutdown: stopped accepting new requests");
        }

        // Step 2: notify all subscribers.
        let _ = self.signal_tx.send(());

        // Step 3: drain in-flight requests.
        let drain = self.drain_in_flight();
        match timeout(self.config.drain_timeout, drain).await {
            Ok(()) => {
                if self.config.verbose {
                    tracing::info!("graceful_shutdown: all in-flight requests drained");
                }
            }
            Err(_) => {
                let remaining = self.state.in_flight_count();
                tracing::warn!(
                    "graceful_shutdown: drain timeout exceeded; {} request(s) still in-flight",
                    remaining
                );
                return Err(ShutdownError::DrainTimeout { remaining });
            }
        }

        // Steps 4–5 are caller-supplied; this function signals completion.
        if self.config.verbose {
            tracing::info!("graceful_shutdown: shutdown complete");
        }
        Ok(())
    }

    /// Spin until `in_flight` reaches zero.
    async fn drain_in_flight(&self) {
        loop {
            if self.state.in_flight_count() == 0 {
                return;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    }
}

// ---------------------------------------------------------------------------
// Signal handler
// ---------------------------------------------------------------------------

/// Listen for SIGTERM or SIGINT and initiate graceful shutdown.
///
/// ```ignore
/// let (coord, state) = GracefulShutdown::new(ShutdownConfig::default());
/// tokio::spawn(handle_signals(coord));
/// ```
#[cfg(unix)]
pub async fn handle_signals(coord: GracefulShutdown) {
    use tokio::signal::unix::{signal, SignalKind};

    let mut sigterm = signal(SignalKind::terminate())
        .expect("failed to register SIGTERM handler");
    let mut sigint = signal(SignalKind::interrupt())
        .expect("failed to register SIGINT handler");

    tokio::select! {
        _ = sigterm.recv() => {
            tracing::info!("received SIGTERM — initiating graceful shutdown");
        }
        _ = sigint.recv() => {
            tracing::info!("received SIGINT — initiating graceful shutdown");
        }
    }

    if let Err(e) = coord.shutdown().await {
        tracing::error!("graceful shutdown error: {:?}", e);
        std::process::exit(1);
    }
    std::process::exit(0);
}

/// Windows / non-Unix fallback: listen for Ctrl-C only.
#[cfg(not(unix))]
pub async fn handle_signals(coord: GracefulShutdown) {
    tokio::signal::ctrl_c()
        .await
        .expect("failed to register Ctrl-C handler");
    tracing::info!("received Ctrl-C — initiating graceful shutdown");
    if let Err(e) = coord.shutdown().await {
        tracing::error!("graceful shutdown error: {:?}", e);
        std::process::exit(1);
    }
    std::process::exit(0);
}

// ---------------------------------------------------------------------------
// Errors
// ---------------------------------------------------------------------------

/// Errors that can occur during shutdown.
#[derive(Debug)]
pub enum ShutdownError {
    /// The drain timeout elapsed before all in-flight requests completed.
    DrainTimeout { remaining: usize },
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn state_starts_accepting() {
        let state = ShutdownState::default();
        assert!(state.is_accepting());
    }

    #[tokio::test]
    async fn in_flight_counter_increments_and_decrements() {
        let state = ShutdownState::default();
        state.request_started();
        state.request_started();
        assert_eq!(state.in_flight_count(), 2);
        state.request_finished();
        assert_eq!(state.in_flight_count(), 1);
    }

    #[tokio::test]
    async fn shutdown_drains_zero_in_flight_immediately() {
        let (coord, _state) = GracefulShutdown::new(ShutdownConfig {
            drain_timeout: Duration::from_secs(5),
            verbose: false,
        });
        let result = coord.shutdown().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn shutdown_times_out_when_requests_never_finish() {
        let (coord, state) = GracefulShutdown::new(ShutdownConfig {
            drain_timeout: Duration::from_millis(100),
            verbose: false,
        });
        // Simulate a hung request that never calls request_finished.
        state.request_started();
        let result = coord.shutdown().await;
        assert!(matches!(result, Err(ShutdownError::DrainTimeout { .. })));
    }
}
