use chrono::{DateTime, Utc};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI64, AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

use crate::metrics::Metrics;

const CIRCUIT_BREAKER_THRESHOLD: u64 = 3;
const CIRCUIT_BREAKER_COOLDOWN: Duration = Duration::from_secs(5 * 60);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    Closed,
    Open,
    HalfOpen,
}

impl CircuitState {
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Closed => "closed",
            Self::Open => "open",
            Self::HalfOpen => "half_open",
        }
    }
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.as_str())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CircuitBreakerConfig {
    pub failure_threshold: u32,
    pub recovery_timeout: Duration,
    pub success_threshold: u32,
    pub half_open_max_calls: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: CIRCUIT_BREAKER_THRESHOLD as u32,
            recovery_timeout: CIRCUIT_BREAKER_COOLDOWN,
            success_threshold: 1,
            half_open_max_calls: 1,
        }
    }
}

impl CircuitBreakerConfig {
    pub fn new(failure_threshold: u32, recovery_timeout: Duration) -> Self {
        Self {
            failure_threshold: failure_threshold.max(1),
            recovery_timeout,
            ..Self::default()
        }
    }

    pub fn with_failure_threshold(mut self, failure_threshold: u32) -> Self {
        self.failure_threshold = failure_threshold.max(1);
        self
    }

    pub fn with_recovery_timeout(mut self, recovery_timeout: Duration) -> Self {
        self.recovery_timeout = recovery_timeout;
        self
    }

    pub fn with_success_threshold(mut self, success_threshold: u32) -> Self {
        self.success_threshold = success_threshold.max(1);
        self
    }

    pub fn with_half_open_max_calls(mut self, half_open_max_calls: u32) -> Self {
        self.half_open_max_calls = half_open_max_calls.max(1);
        self
    }

    pub fn from_env() -> Self {
        let failure_threshold = read_env_u32(&[
            "RPC_CIRCUIT_BREAKER_FAILURE_THRESHOLD",
            "CIRCUIT_BREAKER_FAILURE_THRESHOLD",
        ])
        .unwrap_or(CIRCUIT_BREAKER_THRESHOLD as u32)
        .max(1);
        let recovery_ms = read_env_u64(&[
            "RPC_CIRCUIT_BREAKER_RECOVERY_TIMEOUT_MS",
            "CIRCUIT_BREAKER_RECOVERY_TIMEOUT_MS",
        ]);
        let recovery_secs = read_env_u64(&[
            "RPC_CIRCUIT_BREAKER_RECOVERY_TIMEOUT_SECS",
            "RPC_CIRCUIT_BREAKER_RECOVERY_TIMEOUT_SECONDS",
            "RPC_CIRCUIT_BREAKER_COOLDOWN_SECS",
            "RPC_CIRCUIT_BREAKER_TIMEOUT_SECS",
            "CIRCUIT_BREAKER_RECOVERY_TIMEOUT_SECS",
        ])
        .unwrap_or(CIRCUIT_BREAKER_COOLDOWN.as_secs());
        let success_threshold = read_env_u32(&[
            "RPC_CIRCUIT_BREAKER_SUCCESS_THRESHOLD",
            "CIRCUIT_BREAKER_SUCCESS_THRESHOLD",
        ])
        .unwrap_or(1)
        .max(1);
        let half_open_max_calls = read_env_u32(&[
            "RPC_CIRCUIT_BREAKER_HALF_OPEN_MAX_CALLS",
            "CIRCUIT_BREAKER_HALF_OPEN_MAX_CALLS",
        ])
        .unwrap_or(1)
        .max(1);

        Self {
            failure_threshold,
            recovery_timeout: recovery_ms
                .map(Duration::from_millis)
                .unwrap_or_else(|| Duration::from_secs(recovery_secs)),
            success_threshold,
            half_open_max_calls,
        }
    }
}

fn read_env_u32(names: &[&str]) -> Option<u32> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.trim().parse::<u32>().ok())
    })
}

fn read_env_u64(names: &[&str]) -> Option<u64> {
    names.iter().find_map(|name| {
        std::env::var(name)
            .ok()
            .and_then(|value| value.trim().parse::<u64>().ok())
    })
}
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_secs(10);
const REMOTE_OBSERVATION_TTL: Duration = Duration::from_secs(5 * 60);
const PEER_STALE_AFTER: Duration = Duration::from_secs(10 * 60);
const MAX_HEALTH_SCORE: i64 = 100;
const MIN_HEALTH_SCORE: i64 = 0;
const LOCAL_PROVIDER_STARTING_SCORE: i64 = 70;
const DISCOVERED_PROVIDER_STARTING_SCORE: i64 = 55;
const PEER_STARTING_SCORE: i64 = 60;
const LOCAL_SUCCESS_BONUS: i64 = 12;
const LOCAL_FAILURE_PENALTY: i64 = 25;
const PEER_SUCCESS_BONUS: i64 = 8;
const PEER_FAILURE_PENALTY: i64 = 20;
const MIN_PROVIDER_SCORE: i64 = 25;
const MAX_GOSSIP_PROVIDERS: usize = 64;
const MAX_GOSSIP_PEERS: usize = 64;
pub const MIN_SAMPLES_FOR_EMA: u64 = 5;

#[derive(Debug)]
pub struct ProviderStats {
    ema_rtt_us: AtomicU64,
    sample_count: AtomicU64,
    window: u64,
}

impl ProviderStats {
    pub fn new(window: u64) -> Self {
        Self {
            ema_rtt_us: AtomicU64::new(0),
            sample_count: AtomicU64::new(0),
            window: window.max(1),
        }
    }

    pub fn record(&self, rtt_us: u64) {
        let count = self.sample_count.fetch_add(1, Ordering::Relaxed);
        if count == 0 {
            self.ema_rtt_us.store(rtt_us, Ordering::Relaxed);
            return;
        }

        let previous = self.ema_rtt_us.load(Ordering::Relaxed);
        let alpha_num = 2u128;
        let alpha_den = (self.window + 1) as u128;
        let next = ((rtt_us as u128 * alpha_num) + (previous as u128 * (alpha_den - alpha_num)))
            / alpha_den;
        self.ema_rtt_us.store(next as u64, Ordering::Relaxed);
    }

    pub fn ema_rtt_us(&self) -> u64 {
        self.ema_rtt_us.load(Ordering::Relaxed)
    }

    pub fn sample_count(&self) -> u64 {
        self.sample_count.load(Ordering::Relaxed)
    }

    pub fn is_warmed(&self) -> bool {
        self.sample_count() >= MIN_SAMPLES_FOR_EMA
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderStatsSnapshot {
    pub name: String,
    pub url: String,
    pub ema_rtt_us: u64,
    pub sample_count: u64,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq, Eq)]
pub struct RpcProvider {
    pub name: String,
    pub url: String,
    #[serde(default)]
    pub auth_header: Option<String>,
    #[serde(default)]
    pub auth_value: Option<String>,
    /// Controls whether this provider can be advertised to peer nodes.
    ///
    /// When omitted, providers with credentials are kept local-only.
    #[serde(default)]
    pub advertise: Option<bool>,
}

impl RpcProvider {
    fn should_advertise(&self) -> bool {
        self.advertise.unwrap_or(self.auth_value.is_none())
    }

    fn public_provider(&self) -> PublicRpcProvider {
        PublicRpcProvider {
            name: self.name.clone(),
            url: self.url.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct PublicRpcProvider {
    pub name: String,
    pub url: String,
}

#[derive(Debug, Clone)]
pub struct RegistryConfig {
    pub instance_id: String,
    pub public_base_url: Option<String>,
    pub seed_peers: Vec<String>,
}

impl Default for RegistryConfig {
    fn default() -> Self {
        Self {
            instance_id: "local".to_string(),
            public_base_url: None,
            seed_peers: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PeerAdvertisement {
    pub instance_id: Option<String>,
    pub base_url: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GossipProviderSnapshot {
    pub provider: PublicRpcProvider,
    pub score: i64,
    pub latest_ledger: Option<u64>,
    pub consecutive_failures: u64,
    pub healthy: bool,
    pub observed_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RegistrySnapshot {
    pub instance_id: String,
    pub base_url: Option<String>,
    pub generated_at: DateTime<Utc>,
    pub peers: Vec<PeerAdvertisement>,
    pub providers: Vec<GossipProviderSnapshot>,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProviderHealthReport {
    pub name: String,
    pub url: String,
    pub effective_score: i64,
    pub local_score: i64,
    pub peer_score: i64,
    pub latest_ledger: u64,
    pub consecutive_failures: u64,
    pub healthy: bool,
    pub source: String,
    pub observation_count: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct PeerHealthReport {
    pub base_url: String,
    pub instance_id: Option<String>,
    pub score: i64,
    pub consecutive_failures: u64,
    pub healthy: bool,
    pub last_seen_at: Option<DateTime<Utc>>,
    pub discovered_from: Vec<String>,
    pub last_error: Option<String>,
}

#[derive(Debug, Clone)]
struct RemoteProviderObservation {
    score: i64,
    latest_ledger: u64,
    consecutive_failures: u64,
    healthy: bool,
    observed_at: DateTime<Utc>,
}

#[derive(Debug)]
struct ProviderCircuitState {
    state: CircuitState,
    opened_at: Option<Instant>,
    half_open_started_at: Option<Instant>,
    closed_in_flight: u32,
    closed_in_flight_since: Option<Instant>,
    half_open_in_flight: u32,
    half_open_successes: u32,
}

impl Default for ProviderCircuitState {
    fn default() -> Self {
        Self {
            state: CircuitState::Closed,
            opened_at: None,
            half_open_started_at: None,
            closed_in_flight: 0,
            closed_in_flight_since: None,
            half_open_in_flight: 0,
            half_open_successes: 0,
        }
    }
}

struct ProviderState {
    provider: RwLock<RpcProvider>,
    source: &'static str,
    stats: ProviderStats,
    local_score: AtomicI64,
    consecutive_failures: AtomicU64,
    circuit: RwLock<ProviderCircuitState>,
    latest_ledger: AtomicU64,
    last_local_observed_at: RwLock<Option<DateTime<Utc>>>,
    remote_observations: RwLock<HashMap<String, RemoteProviderObservation>>,
}

impl ProviderState {
    fn new(provider: RpcProvider, source: &'static str, local_score: i64) -> Self {
        Self {
            provider: RwLock::new(provider),
            source,
            stats: ProviderStats::new(20),
            local_score: AtomicI64::new(local_score),
            consecutive_failures: AtomicU64::new(0),
            circuit: RwLock::new(ProviderCircuitState::default()),
            latest_ledger: AtomicU64::new(0),
            last_local_observed_at: RwLock::new(None),
            remote_observations: RwLock::new(HashMap::new()),
        }
    }
}

#[derive(Debug)]
struct PeerState {
    base_url: String,
    instance_id: RwLock<Option<String>>,
    score: AtomicI64,
    consecutive_failures: AtomicU64,
    last_seen_at: RwLock<Option<DateTime<Utc>>>,
    discovered_from: RwLock<HashSet<String>>,
    last_error: RwLock<Option<String>>,
}

impl PeerState {
    fn new(base_url: String, instance_id: Option<String>) -> Self {
        Self {
            base_url,
            instance_id: RwLock::new(instance_id),
            score: AtomicI64::new(PEER_STARTING_SCORE),
            consecutive_failures: AtomicU64::new(0),
            last_seen_at: RwLock::new(None),
            discovered_from: RwLock::new(HashSet::new()),
            last_error: RwLock::new(None),
        }
    }
}

pub struct ProviderRegistry {
    states: RwLock<HashMap<String, Arc<ProviderState>>>,
    peers: RwLock<HashMap<String, Arc<PeerState>>>,
    client: Client,
    instance_id: String,
    public_base_url: Option<String>,
    latency_cursor: AtomicUsize,
    circuit_breaker_config: CircuitBreakerConfig,
    /// Optional Prometheus metrics handle; absent in tests / CLI paths.
    metrics: Option<Arc<Metrics>>,
}

impl ProviderRegistry {
    pub fn new(providers: Vec<RpcProvider>) -> Arc<Self> {
        Self::new_with_config(providers, RegistryConfig::default())
    }

    pub fn new_from_env(providers: Vec<RpcProvider>) -> Arc<Self> {
        Self::new_with_config_and_circuit_breaker(
            providers,
            RegistryConfig::default(),
            CircuitBreakerConfig::from_env(),
        )
    }

    pub fn new_with_config(providers: Vec<RpcProvider>, config: RegistryConfig) -> Arc<Self> {
        Self::new_with_config_and_circuit_breaker(
            providers,
            config,
            CircuitBreakerConfig::default(),
        )
    }

    pub fn new_with_config_and_breaker(
        providers: Vec<RpcProvider>,
        config: RegistryConfig,
        circuit_breaker_config: CircuitBreakerConfig,
    ) -> Arc<Self> {
        Self::new_with_config_and_circuit_breaker(providers, config, circuit_breaker_config)
    }

    pub fn new_with_config_and_circuit_breaker(
        providers: Vec<RpcProvider>,
        config: RegistryConfig,
        circuit_breaker_config: CircuitBreakerConfig,
    ) -> Arc<Self> {
        let mut states = HashMap::new();

        for provider in providers {
            states.insert(
                provider.url.clone(),
                Arc::new(ProviderState::new(
                    provider,
                    "seed",
                    LOCAL_PROVIDER_STARTING_SCORE,
                )),
            );
        }

        let mut peers = HashMap::new();
        for peer in config
            .seed_peers
            .into_iter()
            .map(|peer| normalize_base_url(&peer))
            .filter(|peer| !peer.is_empty())
        {
            if config.public_base_url.as_deref() == Some(peer.as_str()) {
                continue;
            }

            peers.insert(peer.clone(), Arc::new(PeerState::new(peer, None)));
        }

        let circuit_breaker_config = CircuitBreakerConfig {
            failure_threshold: circuit_breaker_config.failure_threshold.max(1),
            recovery_timeout: circuit_breaker_config.recovery_timeout,
            success_threshold: circuit_breaker_config.success_threshold.max(1),
            half_open_max_calls: circuit_breaker_config.half_open_max_calls.max(1),
        };

        Arc::new(Self {
            states: RwLock::new(states),
            peers: RwLock::new(peers),
            client: Client::builder()
                .pool_max_idle_per_host(10)
                .build()
                .unwrap_or_else(|_| Client::new()),
            instance_id: config.instance_id,
            public_base_url: config.public_base_url.map(|url| normalize_base_url(&url)),
            latency_cursor: AtomicUsize::new(0),
            circuit_breaker_config,
            // Documented as absent on CLI/test paths; the HTTP server attaches
            // its own handle. The field was added to the struct without being
            // threaded through this constructor.
            metrics: None,
        })
    }

    pub fn instance_id(&self) -> &str {
        &self.instance_id
    }

    pub fn public_base_url(&self) -> Option<&str> {
        self.public_base_url.as_deref()
    }

    pub fn circuit_breaker_config(&self) -> CircuitBreakerConfig {
        self.circuit_breaker_config
    }

    pub async fn provider_circuit_state(&self, url: &str) -> Option<CircuitState> {
        let state = self.find_by_url(url).await?;
        Some(self.refresh_circuit_state(&state).await)
    }

    async fn refresh_circuit_state(&self, state: &ProviderState) -> CircuitState {
        let mut circuit = state.circuit.write().await;
        let ready = circuit
            .opened_at
            .map(|opened_at| opened_at.elapsed() >= self.circuit_breaker_config.recovery_timeout)
            .unwrap_or(false);
        let stale_closed = circuit
            .closed_in_flight_since
            .map(|started_at| started_at.elapsed() >= self.circuit_breaker_config.recovery_timeout)
            .unwrap_or(false);
        if circuit.state == CircuitState::Open && stale_closed {
            circuit.closed_in_flight = 0;
            circuit.closed_in_flight_since = None;
        }
        if circuit.state == CircuitState::Open
            && ready
            && circuit.closed_in_flight == 0
        {
            circuit.state = CircuitState::HalfOpen;
            circuit.opened_at = None;
            circuit.closed_in_flight_since = None;
            circuit.half_open_started_at = Some(Instant::now());
            circuit.half_open_in_flight = 0;
            circuit.half_open_successes = 0;
        }
        circuit.state
    }

    pub async fn try_acquire_provider(&self, url: &str) -> bool {
        let Some(state) = self.find_by_url(url).await else {
            return false;
        };

        let mut circuit = state.circuit.write().await;
        match circuit.state {
            CircuitState::Closed => {
                if circuit.closed_in_flight == 0 {
                    circuit.closed_in_flight_since = Some(Instant::now());
                }
                circuit.closed_in_flight = circuit.closed_in_flight.saturating_add(1);
                true
            }
            CircuitState::Open => {
                let ready = circuit
                    .opened_at
                    .map(|opened_at| {
                        opened_at.elapsed() >= self.circuit_breaker_config.recovery_timeout
                    })
                    .unwrap_or(true);
                let stale_closed = circuit
                    .closed_in_flight_since
                    .map(|started_at| {
                        started_at.elapsed() >= self.circuit_breaker_config.recovery_timeout
                    })
                    .unwrap_or(false);
                if stale_closed {
                    circuit.closed_in_flight = 0;
                    circuit.closed_in_flight_since = None;
                }
                if ready && circuit.closed_in_flight == 0 {
                    circuit.state = CircuitState::HalfOpen;
                    circuit.opened_at = None;
                    circuit.half_open_started_at = Some(Instant::now());
                    circuit.half_open_in_flight = 1;
                    circuit.half_open_successes = 0;
                    true
                } else {
                    false
                }
            }
            CircuitState::HalfOpen => {
                let permit_stale = circuit
                    .half_open_started_at
                    .map(|started_at| {
                        started_at.elapsed() >= self.circuit_breaker_config.recovery_timeout
                    })
                    .unwrap_or(true);
                if permit_stale {
                    circuit.half_open_started_at = Some(Instant::now());
                    circuit.half_open_in_flight = 0;
                    circuit.half_open_successes = 0;
                }
                if circuit.half_open_in_flight < self.circuit_breaker_config.half_open_max_calls {
                    circuit.half_open_in_flight += 1;
                    true
                } else {
                    false
                }
            }
        }
    }

    pub async fn try_acquire(&self, url: &str) -> bool {
        self.try_acquire_provider(url).await
    }

    pub(crate) async fn release_provider(&self, url: &str) {
        let Some(state) = self.find_by_url(url).await else {
            return;
        };
        let mut circuit = state.circuit.write().await;
        if circuit.closed_in_flight > 0 {
            circuit.closed_in_flight = circuit.closed_in_flight.saturating_sub(1);
            if circuit.closed_in_flight == 0 {
                circuit.closed_in_flight_since = None;
            }
        } else if circuit.half_open_in_flight > 0 {
            circuit.half_open_in_flight = circuit.half_open_in_flight.saturating_sub(1);
        }
    }

    /// Return the list of providers that are currently available for requests,
    /// in priority order (skipping tripped providers whose cooldown hasn't elapsed).
    pub async fn healthy_providers(&self) -> Vec<RpcProvider> {
        let mut available = Vec::new();
        let states = self.states.read().await;
        let provider_states = states.values().cloned().collect::<Vec<_>>();
        drop(states);

        for state in provider_states {
            if self.is_available(state.clone()).await {
                available.push(state.provider.read().await.clone());
            }
        }
        available
    }

    pub async fn providers_by_latency(&self) -> Vec<RpcProvider> {
        let states = self.states.read().await;
        let provider_states = states.values().cloned().collect::<Vec<_>>();
        drop(states);

        let mut available = Vec::new();
        for state in provider_states {
            if self.is_available(state.clone()).await {
                available.push(state);
            }
        }

        if available.is_empty() {
            return Vec::new();
        }

        if available.iter().all(|state| state.stats.is_warmed()) {
            available.sort_by_key(|state| state.stats.ema_rtt_us());
        } else {
            let offset = self.latency_cursor.fetch_add(1, Ordering::Relaxed) % available.len();
            available.rotate_left(offset);
        }

        let mut providers = Vec::with_capacity(available.len());
        for state in available {
            providers.push(state.provider.read().await.clone());
        }
        providers
    }

    pub fn record_rtt(&self, url: &str, rtt_us: u64) {
        if let Ok(states) = self.states.try_read() {
            if let Some(state) = states.get(url) {
                state.stats.record(rtt_us);
            }
        }
    }

    pub fn stats_snapshot(&self) -> Vec<ProviderStatsSnapshot> {
        let Ok(states) = self.states.try_read() else {
            return Vec::new();
        };

        states
            .values()
            .filter_map(|state| {
                state
                    .provider
                    .try_read()
                    .ok()
                    .map(|provider| ProviderStatsSnapshot {
                        name: provider.name.clone(),
                        url: provider.url.clone(),
                        ema_rtt_us: state.stats.ema_rtt_us(),
                        sample_count: state.stats.sample_count(),
                    })
            })
            .collect()
    }

    pub async fn provider_reports(&self) -> Vec<ProviderHealthReport> {
        self.collect_provider_reports()
            .await
            .into_iter()
            .map(|(_, report)| report)
            .collect()
    }

    pub async fn peer_reports(&self) -> Vec<PeerHealthReport> {
        let peers = self.peers.read().await;
        let peer_states = peers.values().cloned().collect::<Vec<_>>();
        drop(peers);

        let mut reports = Vec::with_capacity(peer_states.len());
        for peer in peer_states {
            reports.push(self.build_peer_report(peer).await);
        }
        reports
    }

    pub async fn report_success(&self, url: &str) {
        let Some(state) = self.find_by_url(url).await else {
            return;
        };

        let should_update = {
            let mut circuit = state.circuit.write().await;
            let had_closed_permit = circuit.closed_in_flight > 0;
            let had_half_open_permit = circuit.half_open_in_flight > 0;
            if had_closed_permit {
                circuit.closed_in_flight = circuit.closed_in_flight.saturating_sub(1);
                if circuit.closed_in_flight == 0 {
                    circuit.closed_in_flight_since = None;
                }
            } else if had_half_open_permit {
                circuit.half_open_in_flight = circuit.half_open_in_flight.saturating_sub(1);
            }

            match circuit.state {
                CircuitState::HalfOpen if had_closed_permit => false,
                CircuitState::HalfOpen => {
                    circuit.half_open_successes = circuit.half_open_successes.saturating_add(1);
                    if circuit.half_open_successes >= self.circuit_breaker_config.success_threshold
                        && circuit.half_open_in_flight == 0
                    {
                        circuit.state = CircuitState::Closed;
                        circuit.opened_at = None;
                        circuit.half_open_started_at = None;
                        circuit.closed_in_flight_since = None;
                        circuit.half_open_in_flight = 0;
                        circuit.half_open_successes = 0;
                    }
                    true
                }
                CircuitState::Open if had_closed_permit || had_half_open_permit => false,
                CircuitState::Open => {
                    circuit.state = CircuitState::Closed;
                    circuit.opened_at = None;
                    circuit.half_open_started_at = None;
                    circuit.closed_in_flight_since = None;
                    circuit.half_open_in_flight = 0;
                    circuit.half_open_successes = 0;
                    true
                }
                CircuitState::Closed => true,
            }
        };

        if !should_update {
            return;
        }

        state.consecutive_failures.store(0, Ordering::Relaxed);
        let recovered_score = clamp_score(
            state
                .local_score
                .load(Ordering::Relaxed)
                .max(DISCOVERED_PROVIDER_STARTING_SCORE)
                + LOCAL_SUCCESS_BONUS,
        );
        state.local_score.store(recovered_score, Ordering::Relaxed);
        state
            .last_local_observed_at
            .write()
            .await
            .replace(Utc::now());
    }

    pub async fn report_failure(&self, url: &str) {
        if let Some(state) = self.find_by_url(url).await {
            let prev = state.consecutive_failures.fetch_add(1, Ordering::Relaxed);
            let failures = prev.saturating_add(1);
            state.local_score.store(
                adjust_score(
                    state.local_score.load(Ordering::Relaxed),
                    -LOCAL_FAILURE_PENALTY,
                ),
                Ordering::Relaxed,
            );
            state
                .last_local_observed_at
                .write()
                .await
                .replace(Utc::now());

            let mut circuit = state.circuit.write().await;
            let had_closed_permit = circuit.closed_in_flight > 0;
            let had_half_open_permit = circuit.half_open_in_flight > 0;
            if had_closed_permit {
                circuit.closed_in_flight = circuit.closed_in_flight.saturating_sub(1);
                if circuit.closed_in_flight == 0 {
                    circuit.closed_in_flight_since = None;
                }
            } else if had_half_open_permit {
                circuit.half_open_in_flight = circuit.half_open_in_flight.saturating_sub(1);
            }

            let was_open = circuit.state == CircuitState::Open;
            let should_open = circuit.state == CircuitState::HalfOpen
                || (!was_open && failures >= self.circuit_breaker_config.failure_threshold as u64);
            if should_open {
                circuit.state = CircuitState::Open;
                if !was_open {
                    circuit.opened_at = Some(Instant::now());
                }
                circuit.half_open_started_at = None;
                circuit.half_open_successes = 0;
                if !was_open {
                    let provider = state.provider.read().await;
                    tracing::warn!(
                        provider = %provider.name,
                        url = %provider.url,
                        failures,
                        "Provider circuit breaker tripped"
                    );
                }
            }
        }
    }

    pub fn is_retryable_status(status: u16) -> bool {
        status == 408 || status == 429 || status >= 500
    }

    pub fn spawn_health_checker(
        self: &Arc<Self>,
        interval: Duration,
    ) -> tokio::task::JoinHandle<()> {
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                registry.run_health_checks().await;
            }
        })
    }

    pub fn spawn_gossip_task(self: &Arc<Self>, interval: Duration) -> tokio::task::JoinHandle<()> {
        let registry = Arc::clone(self);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(interval);
            loop {
                ticker.tick().await;
                registry.run_gossip_round().await;
            }
        })
    }

    async fn collect_provider_reports(&self) -> Vec<(RpcProvider, ProviderHealthReport)> {
        let states = self.states.read().await;
        let provider_states = states.values().cloned().collect::<Vec<_>>();
        drop(states);

        let mut reports = Vec::with_capacity(provider_states.len());
        for state in provider_states {
            let provider = state.provider.read().await.clone();
            let report = self.build_provider_report(&provider, &state).await;
            reports.push((provider, report));
        }

        reports
    }

    async fn build_provider_report(
        &self,
        provider: &RpcProvider,
        state: &ProviderState,
    ) -> ProviderHealthReport {
        let local_score = state.local_score.load(Ordering::Relaxed);
        let latest_ledger = state.latest_ledger.load(Ordering::Relaxed);
        let consecutive_failures = state.consecutive_failures.load(Ordering::Relaxed);
        let circuit_state = self.refresh_circuit_state(state).await;
        let tripped = circuit_state == CircuitState::Open;

        let remote_observations = state.remote_observations.read().await;
        let fresh_observations = remote_observations
            .values()
            .filter(|observation| !is_observation_stale(observation.observed_at))
            .cloned()
            .collect::<Vec<_>>();
        drop(remote_observations);

        let peer_score = if fresh_observations.is_empty() {
            0
        } else {
            fresh_observations.iter().map(|o| o.score).sum::<i64>()
                / fresh_observations.len() as i64
        };

        let remote_healthy = fresh_observations
            .iter()
            .any(|observation| observation.healthy);
        let best_remote_ledger = fresh_observations
            .iter()
            .map(|observation| observation.latest_ledger)
            .max()
            .unwrap_or(0);
        let remote_failure_floor = fresh_observations
            .iter()
            .map(|observation| observation.consecutive_failures)
            .min()
            .unwrap_or(consecutive_failures);

        let effective_score = clamp_score((local_score * 2 + peer_score) / 3);
        let healthy = circuit_state == CircuitState::Closed
            && !tripped
            && (effective_score >= MIN_PROVIDER_SCORE || remote_healthy)
            && (consecutive_failures < self.circuit_breaker_config.failure_threshold as u64
                || remote_healthy);

        ProviderHealthReport {
            name: provider.name.clone(),
            url: provider.url.clone(),
            effective_score,
            local_score,
            peer_score,
            latest_ledger: latest_ledger.max(best_remote_ledger),
            consecutive_failures: consecutive_failures.min(remote_failure_floor),
            healthy,
            source: state.source.to_string(),
            observation_count: fresh_observations.len(),
        }
    }

    async fn build_peer_report(&self, peer: Arc<PeerState>) -> PeerHealthReport {
        let last_seen_at = *peer.last_seen_at.read().await;
        let discovered_from = peer
            .discovered_from
            .read()
            .await
            .iter()
            .cloned()
            .collect::<Vec<_>>();
        let last_error = peer.last_error.read().await.clone();
        let instance_id = peer.instance_id.read().await.clone();
        let score = peer.score.load(Ordering::Relaxed);
        let consecutive_failures = peer.consecutive_failures.load(Ordering::Relaxed);
        let healthy = score >= (PEER_STARTING_SCORE / 2)
            && last_seen_at
                .map(|seen_at| {
                    Utc::now()
                        .signed_duration_since(seen_at)
                        .to_std()
                        .unwrap_or_default()
                        < PEER_STALE_AFTER
                })
                .unwrap_or(true);

        PeerHealthReport {
            base_url: peer.base_url.clone(),
            instance_id,
            score,
            consecutive_failures,
            healthy,
            last_seen_at,
            discovered_from,
            last_error,
        }
    }

    async fn run_health_checks(&self) {
        let states = self.states.read().await;
        let provider_states = states.values().cloned().collect::<Vec<_>>();
        drop(states);

        for state in provider_states {
            let provider = state.provider.read().await.clone();
            if !self.try_acquire_provider(&provider.url).await {
                continue;
            }

            let result = self.probe_provider(&state).await;
            match result {
                Ok(ledger) => {
                    state.latest_ledger.store(ledger, Ordering::Relaxed);
                    self.report_success(&provider.url).await;
                }
                Err(error) => {
                    let consecutive_failures = state
                        .consecutive_failures
                        .load(Ordering::Relaxed)
                        .saturating_add(1);
                    tracing::warn!(
                        provider = %provider.name,
                        url = %provider.url,
                        consecutive_failures,
                        error = %error,
                        "Provider health check failed"
                    );
                    self.report_failure(&provider.url).await;
                }
            }
        }
    }

    async fn run_gossip_round(&self) {
        let peers = self.peers.read().await;
        let peer_states = peers.values().cloned().collect::<Vec<_>>();
        drop(peers);

        if peer_states.is_empty() {
            return;
        }

        let local_snapshot = self.registry_snapshot().await;

        for peer in peer_states {
            let endpoint = format!("{}/registry/gossip", peer.base_url);
            match self
                .client
                .post(&endpoint)
                .json(&local_snapshot)
                .send()
                .await
            {
                Ok(response) if response.status().is_success() => {
                    match response.json::<RegistrySnapshot>().await {
                        Ok(snapshot) => {
                            self.merge_snapshot(snapshot).await;
                            self.report_peer_success(&peer.base_url).await;
                        }
                        Err(error) => {
                            self.report_peer_failure(
                                &peer.base_url,
                                format!("invalid gossip payload: {error}"),
                            )
                            .await;
                        }
                    }
                }
                Ok(response) => {
                    self.report_peer_failure(
                        &peer.base_url,
                        format!("HTTP {}", response.status().as_u16()),
                    )
                    .await;
                }
                Err(error) => {
                    self.report_peer_failure(&peer.base_url, error.to_string())
                        .await;
                }
            }
        }
    }

    async fn probe_provider(&self, state: &ProviderState) -> Result<u64, String> {
        let provider = state.provider.read().await.clone();
        let body = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "getLatestLedger",
            "params": null
        });

        let mut req = self.client.post(&provider.url).json(&body);
        if let (Some(header), Some(value)) = (&provider.auth_header, &provider.auth_value) {
            req = req.header(header.as_str(), value.as_str());
        }

        let response = tokio::time::timeout(HEALTH_CHECK_TIMEOUT, req.send())
            .await
            .map_err(|_| "timeout".to_string())?
            .map_err(|error| format!("request error: {error}"))?;

        if !response.status().is_success() {
            return Err(format!("HTTP {}", response.status().as_u16()));
        }

        let json: serde_json::Value = response
            .json()
            .await
            .map_err(|error| format!("parse error: {error}"))?;

        json["result"]["sequence"]
            .as_u64()
            .ok_or_else(|| "missing sequence in response".to_string())
    }

    async fn find_by_url(&self, url: &str) -> Option<Arc<ProviderState>> {
        let states = self.states.read().await;
        states.get(url).cloned()
    }

    async fn get_or_insert_provider(
        &self,
        provider: RpcProvider,
        source: &'static str,
        starting_score: i64,
    ) -> Arc<ProviderState> {
        if let Some(existing) = self.find_by_url(&provider.url).await {
            return existing;
        }

        let mut states = self.states.write().await;
        if let Some(existing) = states.get(&provider.url) {
            return existing.clone();
        }

        let state = Arc::new(ProviderState::new(provider.clone(), source, starting_score));
        states.insert(provider.url.clone(), Arc::clone(&state));
        state
    }

    async fn register_peer(
        &self,
        base_url: &str,
        instance_id: Option<String>,
        discovered_from: Option<&str>,
    ) {
        let normalized = normalize_base_url(base_url);
        if normalized.is_empty() || self.public_base_url.as_deref() == Some(normalized.as_str()) {
            return;
        }

        let peer = {
            let mut peers = self.peers.write().await;
            peers
                .entry(normalized.clone())
                .or_insert_with(|| {
                    Arc::new(PeerState::new(normalized.clone(), instance_id.clone()))
                })
                .clone()
        };

        if let Some(instance_id) = instance_id {
            *peer.instance_id.write().await = Some(instance_id);
        }

        if let Some(discovered_from) = discovered_from {
            peer.discovered_from
                .write()
                .await
                .insert(discovered_from.to_string());
        }
    }

    async fn report_peer_success(&self, base_url: &str) {
        let normalized = normalize_base_url(base_url);
        self.register_peer(&normalized, None, None).await;

        if let Some(peer) = self.peers.read().await.get(&normalized).cloned() {
            peer.consecutive_failures.store(0, Ordering::Relaxed);
            peer.score.store(
                adjust_score(peer.score.load(Ordering::Relaxed), PEER_SUCCESS_BONUS),
                Ordering::Relaxed,
            );
            peer.last_seen_at.write().await.replace(Utc::now());
            *peer.last_error.write().await = None;
        }
    }

    async fn report_peer_failure(&self, base_url: &str, error: String) {
        let normalized = normalize_base_url(base_url);
        self.register_peer(&normalized, None, None).await;

        if let Some(peer) = self.peers.read().await.get(&normalized).cloned() {
            peer.consecutive_failures.fetch_add(1, Ordering::Relaxed);
            peer.score.store(
                adjust_score(peer.score.load(Ordering::Relaxed), -PEER_FAILURE_PENALTY),
                Ordering::Relaxed,
            );
            *peer.last_error.write().await = Some(error);
        }
    }

    async fn is_available(&self, state: Arc<ProviderState>) -> bool {
        match self.refresh_circuit_state(&state).await {
            CircuitState::Open => false,
            CircuitState::HalfOpen => {
                let circuit = state.circuit.read().await;
                let permit_stale = circuit
                    .half_open_started_at
                    .map(|started_at| {
                        started_at.elapsed() >= self.circuit_breaker_config.recovery_timeout
                    })
                    .unwrap_or(true);
                permit_stale
                    || circuit.half_open_in_flight
                        < self.circuit_breaker_config.half_open_max_calls
            }
            CircuitState::Closed => {
                state.local_score.load(Ordering::Relaxed) >= MIN_PROVIDER_SCORE
                    || state.consecutive_failures.load(Ordering::Relaxed)
                        < self.circuit_breaker_config.failure_threshold as u64
            }
        }
    }

    pub async fn registry_snapshot(&self) -> RegistrySnapshot {
        let reports = self.collect_provider_reports().await;
        let providers = reports
            .into_iter()
            .filter(|(provider, report)| provider.should_advertise() && report.healthy)
            .take(MAX_GOSSIP_PROVIDERS)
            .map(|(provider, report)| GossipProviderSnapshot {
                provider: provider.public_provider(),
                score: report.effective_score,
                latest_ledger: Some(report.latest_ledger),
                consecutive_failures: report.consecutive_failures,
                healthy: report.healthy,
                observed_at: Utc::now(),
            })
            .collect();

        let peers = self
            .peer_reports()
            .await
            .into_iter()
            .filter(|peer| peer.healthy)
            .take(MAX_GOSSIP_PEERS)
            .map(|peer| PeerAdvertisement {
                instance_id: peer.instance_id,
                base_url: peer.base_url,
            })
            .collect();

        RegistrySnapshot {
            instance_id: self.instance_id.clone(),
            base_url: self.public_base_url.clone(),
            generated_at: Utc::now(),
            peers,
            providers,
        }
    }

    pub async fn merge_snapshot(&self, snapshot: RegistrySnapshot) {
        if let Some(base_url) = snapshot.base_url.as_deref() {
            self.register_peer(base_url, Some(snapshot.instance_id.clone()), None)
                .await;
        }

        for peer in snapshot.peers {
            self.register_peer(
                &peer.base_url,
                peer.instance_id,
                snapshot.base_url.as_deref(),
            )
            .await;
        }

        for snapshot_provider in snapshot.providers {
            let provider = RpcProvider {
                name: snapshot_provider.provider.name,
                url: snapshot_provider.provider.url,
                auth_header: None,
                auth_value: None,
                advertise: Some(true),
            };
            let state = self
                .get_or_insert_provider(provider, "gossip", DISCOVERED_PROVIDER_STARTING_SCORE)
                .await;
            let source = snapshot
                .base_url
                .clone()
                .unwrap_or_else(|| snapshot.instance_id.clone());
            state.remote_observations.write().await.insert(
                source,
                RemoteProviderObservation {
                    score: snapshot_provider.score,
                    latest_ledger: snapshot_provider.latest_ledger.unwrap_or(0),
                    consecutive_failures: snapshot_provider.consecutive_failures,
                    healthy: snapshot_provider.healthy,
                    observed_at: snapshot_provider.observed_at,
                },
            );
        }
    }

}

fn normalize_base_url(url: &str) -> String {
    url.trim().trim_end_matches('/').to_string()
}

fn clamp_score(score: i64) -> i64 {
    score.clamp(MIN_HEALTH_SCORE, MAX_HEALTH_SCORE)
}

fn adjust_score(current: i64, delta: i64) -> i64 {
    clamp_score(current + delta)
}

fn is_observation_stale(observed_at: DateTime<Utc>) -> bool {
    Utc::now()
        .signed_duration_since(observed_at)
        .to_std()
        .unwrap_or_default()
        >= REMOTE_OBSERVATION_TTL
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_provider(name: &str, url: &str) -> RpcProvider {
        RpcProvider {
            name: name.to_string(),
            url: url.to_string(),
            auth_header: None,
            auth_value: None,
            advertise: None,
        }
    }

    #[tokio::test]
    async fn test_all_seed_providers_are_healthy_initially() {
        let registry = ProviderRegistry::new(vec![
            make_provider("a", "http://a.test"),
            make_provider("b", "http://b.test"),
        ]);

        let providers = registry.healthy_providers().await;
        let urls = providers
            .iter()
            .map(|provider| provider.url.as_str())
            .collect::<HashSet<_>>();

        assert_eq!(providers.len(), 2);
        assert!(urls.contains("http://a.test"));
        assert!(urls.contains("http://b.test"));
    }

    #[tokio::test]
    async fn test_circuit_breaker_trips_after_threshold() {
        let registry = ProviderRegistry::new(vec![make_provider("a", "http://a.test")]);

        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            registry.report_failure("http://a.test").await;
        }

        assert!(registry.healthy_providers().await.is_empty());
    }

    #[tokio::test]
    async fn test_success_clears_tripped_provider() {
        let registry = ProviderRegistry::new(vec![make_provider("a", "http://a.test")]);

        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            registry.report_failure("http://a.test").await;
        }
        assert!(registry.healthy_providers().await.is_empty());

        registry.report_success("http://a.test").await;
        assert_eq!(registry.healthy_providers().await.len(), 1);
    }

    #[tokio::test]
    async fn test_gossip_discovers_provider_and_peer() {
        let registry = ProviderRegistry::new_with_config(
            vec![make_provider("seed", "http://seed.test")],
            RegistryConfig {
                instance_id: "node-a".to_string(),
                public_base_url: Some("http://node-a.test".to_string()),
                seed_peers: Vec::new(),
            },
        );

        registry
            .merge_snapshot(RegistrySnapshot {
                instance_id: "node-b".to_string(),
                base_url: Some("http://node-b.test".to_string()),
                generated_at: Utc::now(),
                peers: vec![PeerAdvertisement {
                    instance_id: Some("node-c".to_string()),
                    base_url: "http://node-c.test".to_string(),
                }],
                providers: vec![GossipProviderSnapshot {
                    provider: PublicRpcProvider {
                        name: "shared".to_string(),
                        url: "http://shared.test".to_string(),
                    },
                    score: 90,
                    latest_ledger: Some(123),
                    consecutive_failures: 0,
                    healthy: true,
                    observed_at: Utc::now(),
                }],
            })
            .await;

        let providers = registry.healthy_providers().await;
        assert!(providers
            .iter()
            .any(|provider| provider.url == "http://shared.test"));

        let peers = registry.peer_reports().await;
        assert!(peers
            .iter()
            .any(|peer| peer.base_url == "http://node-b.test"));
        assert!(peers
            .iter()
            .any(|peer| peer.base_url == "http://node-c.test"));
    }

    #[tokio::test]
    async fn test_snapshot_omits_private_provider_credentials() {
        let registry = ProviderRegistry::new(vec![
            make_provider("public", "http://public.test"),
            RpcProvider {
                name: "private".to_string(),
                url: "http://private.test".to_string(),
                auth_header: Some("Authorization".to_string()),
                auth_value: Some("secret".to_string()),
                advertise: None,
            },
        ]);

        let snapshot = registry.registry_snapshot().await;
        assert!(snapshot
            .providers
            .iter()
            .any(|provider| provider.provider.url == "http://public.test"));
        assert!(!snapshot
            .providers
            .iter()
            .any(|provider| provider.provider.url == "http://private.test"));
    }

    #[tokio::test]
    async fn test_peer_failures_lower_peer_score() {
        let registry = ProviderRegistry::new_with_config(
            vec![make_provider("seed", "http://seed.test")],
            RegistryConfig {
                instance_id: "node-a".to_string(),
                public_base_url: Some("http://node-a.test".to_string()),
                seed_peers: vec!["http://node-b.test".to_string()],
            },
        );

        registry
            .report_peer_failure("http://node-b.test", "timeout".to_string())
            .await;

        let report = registry
            .peer_reports()
            .await
            .into_iter()
            .find(|peer| peer.base_url == "http://node-b.test")
            .unwrap();

        assert!(report.score < PEER_STARTING_SCORE);
        assert_eq!(report.last_error.as_deref(), Some("timeout"));
    }

    // ── ProviderStats / latency routing tests ─────────────────────────────

    /// Helper: record `count` identical samples to warm the EMA past the
    /// routing threshold and onto a stable value.
    fn warm_stats(stats: &ProviderStats, rtt_us: u64, count: u64) {
        for _ in 0..count {
            stats.record(rtt_us);
        }
    }

    #[test]
    fn ema_converges_toward_true_value() {
        let stats = ProviderStats::new(20);
        warm_stats(&stats, 1000, 100);
        // With α ≈ 0.095 and 100 identical samples, the EMA sits right
        // on the true value — anything further than 5% off would be a
        // real bug.
        let ema = stats.ema_rtt_us();
        let drift = ema.abs_diff(1000);
        assert!(drift <= 50, "EMA drifted {drift}µs from 1000µs");
        assert_eq!(stats.sample_count(), 100);
    }

    #[test]
    fn ema_first_sample_seeds_exact_value() {
        // The series starts at the first sample instead of climbing from
        // zero — otherwise early routing decisions would penalise brand
        // new providers.
        let stats = ProviderStats::new(20);
        stats.record(777);
        assert_eq!(stats.ema_rtt_us(), 777);
    }

    #[test]
    fn ema_zero_window_is_clamped_safely() {
        // A caller-supplied zero window would have divided by zero; the
        // constructor clamps it to a 1-sample minimum.
        let stats = ProviderStats::new(0);
        stats.record(500);
        stats.record(1000);
        // Just ensure no panic and the EMA moved.
        assert!(stats.ema_rtt_us() > 0);
        assert_eq!(stats.sample_count(), 2);
    }

    #[test]
    fn is_warmed_reflects_sample_count_threshold() {
        let stats = ProviderStats::new(20);
        assert!(!stats.is_warmed());
        for _ in 0..MIN_SAMPLES_FOR_EMA - 1 {
            stats.record(100);
        }
        assert!(
            !stats.is_warmed(),
            "{} samples is below threshold",
            stats.sample_count()
        );
        stats.record(100);
        assert!(
            stats.is_warmed(),
            "{} samples should be warmed",
            stats.sample_count()
        );
    }

    #[tokio::test]
    async fn providers_by_latency_picks_fastest_once_warm() {
        let registry = ProviderRegistry::new(vec![
            make_provider("slow", "http://slow.test"),
            make_provider("fast", "http://fast.test"),
        ]);
        // Warm both with very different latencies.
        for _ in 0..MIN_SAMPLES_FOR_EMA {
            registry.record_rtt("http://slow.test", 500_000);
            registry.record_rtt("http://fast.test", 50_000);
        }
        let ordered = registry.providers_by_latency().await;
        assert_eq!(ordered.len(), 2);
        assert_eq!(ordered[0].name, "fast");
        assert_eq!(ordered[1].name, "slow");
    }

    #[tokio::test]
    async fn providers_by_latency_round_robins_before_warmup() {
        let registry = ProviderRegistry::new(vec![
            make_provider("a", "http://a.test"),
            make_provider("b", "http://b.test"),
        ]);
        // Neither provider has any samples → round-robin fallback.
        let first = registry.providers_by_latency().await;
        let second = registry.providers_by_latency().await;
        assert_eq!(first.len(), 2);
        assert_eq!(second.len(), 2);
        // The cursor rotates by one each call, so the head differs.
        assert_ne!(first[0].name, second[0].name);
    }

    #[tokio::test]
    async fn providers_by_latency_excludes_unhealthy_providers() {
        let registry = ProviderRegistry::new(vec![
            make_provider("fast-but-down", "http://fast-down.test"),
            make_provider("slow-but-up", "http://slow-up.test"),
        ]);
        // Fast provider has the best EMA but we trip its breaker.
        for _ in 0..MIN_SAMPLES_FOR_EMA {
            registry.record_rtt("http://fast-down.test", 10_000);
            registry.record_rtt("http://slow-up.test", 500_000);
        }
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            registry.report_failure("http://fast-down.test").await;
        }
        let ordered = registry.providers_by_latency().await;
        assert_eq!(ordered.len(), 1);
        assert_eq!(ordered[0].name, "slow-but-up");
    }

    #[tokio::test]
    async fn providers_by_latency_empty_when_all_unhealthy() {
        let registry = ProviderRegistry::new(vec![
            make_provider("a", "http://a.test"),
            make_provider("b", "http://b.test"),
        ]);
        for _ in 0..CIRCUIT_BREAKER_THRESHOLD {
            registry.report_failure("http://a.test").await;
            registry.report_failure("http://b.test").await;
        }
        assert!(registry.providers_by_latency().await.is_empty());
    }

    #[test]
    fn record_rtt_for_unknown_url_is_noop() {
        // No panic, no sample recorded.
        let registry = ProviderRegistry::new(vec![make_provider("a", "http://a.test")]);
        registry.record_rtt("http://unknown.test", 100);
        let snap = registry.stats_snapshot();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].sample_count, 0);
    }

    #[test]
    fn stats_snapshot_reflects_recorded_samples() {
        let registry = ProviderRegistry::new(vec![
            make_provider("a", "http://a.test"),
            make_provider("b", "http://b.test"),
        ]);
        for _ in 0..5 {
            registry.record_rtt("http://a.test", 1_000);
        }
        registry.record_rtt("http://b.test", 10_000);

        let snap = registry.stats_snapshot();
        let a = snap.iter().find(|s| s.name == "a").unwrap();
        let b = snap.iter().find(|s| s.name == "b").unwrap();
        assert_eq!(a.sample_count, 5);
        assert_eq!(b.sample_count, 1);
        // EMA for `a` converged to 1_000; `b` seeded at 10_000.
        assert_eq!(a.ema_rtt_us, 1_000);
        assert_eq!(b.ema_rtt_us, 10_000);
    }
}
