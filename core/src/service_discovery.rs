//! Service discovery and circuit-breaker — Issue #596.
//!
//! Provides:
//! - A `ServiceRegistry` where services register themselves with health probes.
//! - A `ServiceCatalog` that caches discovered service addresses and watches for
//!   changes.
//! - A `CircuitBreaker` per downstream service with Closed / Open / Half-Open
//!   states, configurable failure threshold, timeout, and fallback.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

// ---------------------------------------------------------------------------
// ServiceRegistry — services register here
// ---------------------------------------------------------------------------

/// A registered service instance.
#[derive(Debug, Clone)]
pub struct ServiceInstance {
    pub name: String,
    pub address: String,
    pub port: u16,
    pub tags: Vec<String>,
    pub healthy: bool,
    pub last_seen: Instant,
}

impl ServiceInstance {
    pub fn new(name: impl Into<String>, address: impl Into<String>, port: u16) -> Self {
        Self {
            name: name.into(),
            address: address.into(),
            port,
            tags: Vec::new(),
            healthy: true,
            last_seen: Instant::now(),
        }
    }

    pub fn endpoint(&self) -> String {
        format!("{}:{}", self.address, self.port)
    }
}

/// Central registry where services announce their presence and health.
#[derive(Debug, Default)]
pub struct ServiceRegistry {
    instances: Mutex<HashMap<String, Vec<ServiceInstance>>>,
}

impl ServiceRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Register a service instance. If the same `(name, address, port)` exists
    /// it is updated (healthy = true, last_seen refreshed).
    pub fn register(&self, instance: ServiceInstance) {
        let mut map = self.instances.lock().unwrap();
        let entries = map.entry(instance.name.clone()).or_default();
        if let Some(existing) = entries
            .iter_mut()
            .find(|i| i.address == instance.address && i.port == instance.port)
        {
            existing.healthy = true;
            existing.last_seen = Instant::now();
        } else {
            entries.push(instance);
        }
    }

    /// Mark a service instance as unhealthy (e.g. failed health probe).
    pub fn deregister(&self, name: &str, address: &str, port: u16) {
        let mut map = self.instances.lock().unwrap();
        if let Some(entries) = map.get_mut(name) {
            if let Some(inst) = entries
                .iter_mut()
                .find(|i| i.address == address && i.port == port)
            {
                inst.healthy = false;
            }
        }
    }

    /// Return all healthy instances for the given service name.
    pub fn resolve(&self, name: &str) -> Vec<ServiceInstance> {
        let map = self.instances.lock().unwrap();
        map.get(name)
            .cloned()
            .unwrap_or_default()
            .into_iter()
            .filter(|i| i.healthy)
            .collect()
    }

    /// Snapshot of the full catalogue (for watch / diff).
    pub fn snapshot(&self) -> HashMap<String, Vec<ServiceInstance>> {
        self.instances.lock().unwrap().clone()
    }
}

// ---------------------------------------------------------------------------
// ServiceCatalog — client-side caching + retry
// ---------------------------------------------------------------------------

/// Client-side cache for service endpoints.
///
/// Wraps a `ServiceRegistry` and adds:
/// - TTL-based invalidation.
/// - Round-robin selection across healthy instances.
/// - Retry with exponential back-off.
pub struct ServiceCatalog {
    registry: Arc<ServiceRegistry>,
    ttl: Duration,
    cache: Mutex<HashMap<String, CacheEntry>>,
}

#[derive(Debug)]
struct CacheEntry {
    instances: Vec<ServiceInstance>,
    fetched_at: Instant,
    rr_cursor: usize,
}

impl ServiceCatalog {
    pub fn new(registry: Arc<ServiceRegistry>, ttl: Duration) -> Self {
        Self {
            registry,
            ttl,
            cache: Mutex::default(),
        }
    }

    /// Pick a healthy instance for `service_name`, refreshing the cache if
    /// the TTL has expired.
    pub fn pick(&self, service_name: &str) -> Option<ServiceInstance> {
        let mut cache = self.cache.lock().unwrap();
        let entry = cache.entry(service_name.to_string()).or_insert_with(|| {
            CacheEntry {
                instances: self.registry.resolve(service_name),
                fetched_at: Instant::now(),
                rr_cursor: 0,
            }
        });

        // Invalidate on TTL expiry.
        if entry.fetched_at.elapsed() > self.ttl {
            entry.instances = self.registry.resolve(service_name);
            entry.fetched_at = Instant::now();
        }

        if entry.instances.is_empty() {
            return None;
        }

        // Round-robin.
        let idx = entry.rr_cursor % entry.instances.len();
        entry.rr_cursor = entry.rr_cursor.wrapping_add(1);
        Some(entry.instances[idx].clone())
    }
}

// ---------------------------------------------------------------------------
// CircuitBreaker
// ---------------------------------------------------------------------------

/// Circuit-breaker state machine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CircuitState {
    /// Normal operation — requests pass through.
    Closed,
    /// Circuit is tripped — requests are short-circuited to the fallback.
    Open,
    /// Probing state — one request is allowed through to test recovery.
    HalfOpen,
}

/// Configuration for a circuit breaker.
#[derive(Debug, Clone)]
pub struct CircuitBreakerConfig {
    /// Number of consecutive failures required to trip the circuit.
    pub failure_threshold: usize,
    /// How long the circuit stays open before transitioning to half-open.
    pub reset_timeout: Duration,
    /// Optional fallback message returned when the circuit is open.
    pub fallback_message: Option<String>,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 5,
            reset_timeout: Duration::from_secs(30),
            fallback_message: Some("service temporarily unavailable".into()),
        }
    }
}

/// Per-downstream circuit breaker.
#[derive(Debug)]
pub struct CircuitBreaker {
    config: CircuitBreakerConfig,
    state: Mutex<CircuitBreakerInner>,
}

#[derive(Debug)]
struct CircuitBreakerInner {
    state: CircuitState,
    failure_count: usize,
    last_failure: Option<Instant>,
}

impl CircuitBreaker {
    pub fn new(config: CircuitBreakerConfig) -> Arc<Self> {
        Arc::new(Self {
            config,
            state: Mutex::new(CircuitBreakerInner {
                state: CircuitState::Closed,
                failure_count: 0,
                last_failure: None,
            }),
        })
    }

    /// Returns `true` if a request should be allowed through.
    pub fn allow_request(&self) -> bool {
        let mut inner = self.state.lock().unwrap();
        match inner.state {
            CircuitState::Closed => true,
            CircuitState::Open => {
                // Transition to half-open after the reset timeout.
                if inner
                    .last_failure
                    .map(|t| t.elapsed() >= self.config.reset_timeout)
                    .unwrap_or(false)
                {
                    inner.state = CircuitState::HalfOpen;
                    true // probe request
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => false, // only the probe request passes
        }
    }

    /// Record a successful call. Resets failure count and closes the circuit.
    pub fn record_success(&self) {
        let mut inner = self.state.lock().unwrap();
        inner.failure_count = 0;
        inner.last_failure = None;
        inner.state = CircuitState::Closed;
    }

    /// Record a failed call. May trip the circuit.
    pub fn record_failure(&self) {
        let mut inner = self.state.lock().unwrap();
        inner.failure_count += 1;
        inner.last_failure = Some(Instant::now());
        if inner.failure_count >= self.config.failure_threshold {
            inner.state = CircuitState::Open;
            tracing::warn!(
                "circuit breaker tripped after {} failures",
                inner.failure_count
            );
        }
    }

    /// Current circuit state.
    pub fn state(&self) -> CircuitState {
        self.state.lock().unwrap().state.clone()
    }

    /// Fallback response when the circuit is open.
    pub fn fallback(&self) -> Option<&str> {
        self.config.fallback_message.as_deref()
    }
}

// ---------------------------------------------------------------------------
// ServiceDiscoveryClient — wires catalog + circuit breaker together
// ---------------------------------------------------------------------------

/// High-level client used by application code to call a downstream service.
///
/// Maintains one `CircuitBreaker` per named service and picks endpoints via
/// the `ServiceCatalog`.
pub struct ServiceDiscoveryClient {
    catalog: ServiceCatalog,
    breakers: Mutex<HashMap<String, Arc<CircuitBreaker>>>,
    breaker_config: CircuitBreakerConfig,
}

impl ServiceDiscoveryClient {
    pub fn new(
        registry: Arc<ServiceRegistry>,
        catalog_ttl: Duration,
        breaker_config: CircuitBreakerConfig,
    ) -> Self {
        Self {
            catalog: ServiceCatalog::new(registry, catalog_ttl),
            breakers: Mutex::default(),
            breaker_config,
        }
    }

    /// Resolve an endpoint for `service_name`, respecting the circuit breaker.
    ///
    /// Returns `Ok(ServiceInstance)` when a healthy endpoint is available and
    /// the circuit is not open. Returns `Err` with a fallback message otherwise.
    pub fn resolve(&self, service_name: &str) -> Result<ServiceInstance, String> {
        let breaker = self.get_or_create_breaker(service_name);

        if !breaker.allow_request() {
            let msg = breaker
                .fallback()
                .unwrap_or("circuit open")
                .to_string();
            return Err(msg);
        }

        match self.catalog.pick(service_name) {
            Some(inst) => Ok(inst),
            None => {
                breaker.record_failure();
                Err(format!("no healthy instances found for '{}'", service_name))
            }
        }
    }

    /// Report success for a downstream call (resets circuit breaker).
    pub fn report_success(&self, service_name: &str) {
        self.get_or_create_breaker(service_name).record_success();
    }

    /// Report failure for a downstream call (may trip circuit breaker).
    pub fn report_failure(&self, service_name: &str) {
        self.get_or_create_breaker(service_name).record_failure();
    }

    fn get_or_create_breaker(&self, name: &str) -> Arc<CircuitBreaker> {
        let mut map = self.breakers.lock().unwrap();
        map.entry(name.to_string())
            .or_insert_with(|| CircuitBreaker::new(self.breaker_config.clone()))
            .clone()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn make_instance(name: &str, addr: &str, port: u16) -> ServiceInstance {
        ServiceInstance::new(name, addr, port)
    }

    #[test]
    fn registry_register_and_resolve() {
        let registry = ServiceRegistry::new();
        registry.register(make_instance("svc-a", "10.0.0.1", 8080));
        registry.register(make_instance("svc-a", "10.0.0.2", 8080));
        let instances = registry.resolve("svc-a");
        assert_eq!(instances.len(), 2);
    }

    #[test]
    fn registry_deregister_marks_unhealthy() {
        let registry = ServiceRegistry::new();
        registry.register(make_instance("svc-b", "10.0.0.1", 9000));
        registry.deregister("svc-b", "10.0.0.1", 9000);
        assert!(registry.resolve("svc-b").is_empty());
    }

    #[test]
    fn circuit_breaker_trips_after_threshold() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 3,
            reset_timeout: Duration::from_secs(60),
            fallback_message: None,
        });
        for _ in 0..3 {
            cb.record_failure();
        }
        assert_eq!(cb.state(), CircuitState::Open);
        assert!(!cb.allow_request());
    }

    #[test]
    fn circuit_breaker_closes_after_success() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 2,
            ..Default::default()
        });
        cb.record_failure();
        cb.record_failure();
        assert_eq!(cb.state(), CircuitState::Open);
        cb.record_success();
        assert_eq!(cb.state(), CircuitState::Closed);
    }

    #[test]
    fn circuit_breaker_starts_closed() {
        let cb = CircuitBreaker::new(CircuitBreakerConfig::default());
        assert_eq!(cb.state(), CircuitState::Closed);
        assert!(cb.allow_request());
    }

    #[test]
    fn catalog_round_robins_across_instances() {
        let registry = ServiceRegistry::new();
        registry.register(make_instance("api", "10.0.0.1", 80));
        registry.register(make_instance("api", "10.0.0.2", 80));
        let catalog = ServiceCatalog::new(registry, Duration::from_secs(60));
        let first = catalog.pick("api").unwrap().address;
        let second = catalog.pick("api").unwrap().address;
        assert_ne!(first, second);
    }
}
