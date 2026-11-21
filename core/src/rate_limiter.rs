//! Rate limiter — Issue #597
//!
//! Implements a token-bucket rate limiter per client identity with:
//! - Tiered policies (Free / Pro / Enterprise)
//! - Redis-backed sliding window for multi-instance deployments
//! - Per-endpoint limit overrides
//! - Standard `X-RateLimit-*` response headers
//! - Admin API for dynamic rule updates

use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;

// ── Header name constants ────────────────────────────────────────────────────

pub const HEADER_RATELIMIT_LIMIT: &str = "X-RateLimit-Limit";
pub const HEADER_RATELIMIT_REMAINING: &str = "X-RateLimit-Remaining";
pub const HEADER_RATELIMIT_RESET: &str = "X-RateLimit-Reset";
pub const HEADER_RATELIMIT_RETRY_AFTER: &str = "Retry-After";

// ── Tier definitions ─────────────────────────────────────────────────────────

/// Service tier that determines which default policy applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RateLimitTier {
    Free,
    Pro,
    Enterprise,
}

impl Default for RateLimitTier {
    fn default() -> Self {
        Self::Free
    }
}

// ── Policy ───────────────────────────────────────────────────────────────────

/// Rate-limit policy for a client tier or a specific endpoint override.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RateLimitPolicy {
    /// Maximum requests allowed per window.
    pub requests_per_window: u64,
    /// Window length in seconds.
    pub window_secs: u64,
    /// Additional burst tokens above `requests_per_window`.
    pub burst_allowance: u64,
}

impl RateLimitPolicy {
    pub fn new(requests_per_window: u64, window_secs: u64, burst_allowance: u64) -> Self {
        Self {
            requests_per_window,
            window_secs,
            burst_allowance,
        }
    }

    /// Total capacity including burst.
    pub fn capacity(&self) -> u64 {
        self.requests_per_window + self.burst_allowance
    }
}

impl Default for RateLimitPolicy {
    fn default() -> Self {
        Self::new(60, 60, 0)
    }
}

/// Built-in default policies per tier.
pub fn default_policy(tier: RateLimitTier) -> RateLimitPolicy {
    match tier {
        RateLimitTier::Free => RateLimitPolicy::new(60, 60, 10),
        RateLimitTier::Pro => RateLimitPolicy::new(1_000, 60, 100),
        RateLimitTier::Enterprise => RateLimitPolicy::new(10_000, 60, 1_000),
    }
}

// ── Client key ───────────────────────────────────────────────────────────────

/// The identity of the client making a request.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "type", content = "value")]
pub enum RateLimitKey {
    ApiKey(String),
    IpAddress(String),
    UserId(String),
}

impl RateLimitKey {
    pub fn as_str(&self) -> &str {
        match self {
            Self::ApiKey(s) | Self::IpAddress(s) | Self::UserId(s) => s.as_str(),
        }
    }

    pub fn cache_key(&self) -> String {
        match self {
            Self::ApiKey(k) => format!("rl:api:{}", k),
            Self::IpAddress(ip) => format!("rl:ip:{}", ip),
            Self::UserId(uid) => format!("rl:user:{}", uid),
        }
    }
}

// ── Result ───────────────────────────────────────────────────────────────────

/// Outcome of a rate-limit check.
#[derive(Debug, Clone)]
pub struct RateLimitResult {
    pub allowed: bool,
    /// Total capacity for this client/policy.
    pub limit: u64,
    /// Tokens remaining after this request.
    pub remaining: u64,
    /// Unix timestamp (seconds) when the window resets.
    pub reset_at: u64,
}

impl RateLimitResult {
    /// Convert to HTTP response headers as `(name, value)` pairs.
    pub fn to_headers(&self) -> Vec<(String, String)> {
        vec![
            (HEADER_RATELIMIT_LIMIT.to_string(), self.limit.to_string()),
            (
                HEADER_RATELIMIT_REMAINING.to_string(),
                self.remaining.to_string(),
            ),
            (HEADER_RATELIMIT_RESET.to_string(), self.reset_at.to_string()),
        ]
    }
}

// ── Trait ────────────────────────────────────────────────────────────────────

/// Core rate-limiter interface.
pub trait RateLimiter: Send + Sync {
    /// Check (and consume) one token for `key` using `policy`.
    /// Returns a [`RateLimitResult`] indicating whether the request is allowed.
    fn check(&self, key: &RateLimitKey, policy: &RateLimitPolicy) -> RateLimitResult;

    /// Dynamically replace the policy for a given tier.
    fn update_rule(&self, tier: RateLimitTier, policy: RateLimitPolicy);
}

// ── In-memory token-bucket implementation ───────────────────────────────────

struct BucketState {
    tokens: f64,
    last_refill: Instant,
    window_start: u64, // Unix epoch seconds
use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use axum::{
    body::Body,
    extract::{MatchedPath, Request},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use redis::Commands;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tower::{Layer, Service};

use crate::error_codes::{ErrorCode, ErrorResponse};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,
    last_refill: f64,
}

/// In-process token-bucket rate limiter (single-instance safe).
///
/// For multi-instance deployments, replace with [`RedisRateLimiter`].
pub struct TokenBucketRateLimiter {
    /// Per-key bucket state.
    buckets: Arc<RwLock<HashMap<String, BucketState>>>,
    /// Per-tier policy overrides (admin-writable).
    tier_policies: Arc<RwLock<HashMap<RateLimitTier, RateLimitPolicy>>>,
}

impl TokenBucketRateLimiter {
    pub fn new() -> Self {
        let mut policies = HashMap::new();
        for tier in [
            RateLimitTier::Free,
            RateLimitTier::Pro,
            RateLimitTier::Enterprise,
        ] {
            policies.insert(tier, default_policy(tier));
        }
        Self {
            buckets: Arc::new(RwLock::new(HashMap::new())),
            tier_policies: Arc::new(RwLock::new(policies)),
        }
    }

    fn now_secs() -> u64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs()
    }
}

impl Default for TokenBucketRateLimiter {
    fn default() -> Self {
        Self::new()
    }
}

impl RateLimiter for TokenBucketRateLimiter {
    fn check(&self, key: &RateLimitKey, policy: &RateLimitPolicy) -> RateLimitResult {
        // Use blocking read on the async RwLock via try_read; fall back to a
        // new bucket when lock is contended (simple non-blocking path).
        let cache_key = key.cache_key();
        let capacity = policy.capacity() as f64;
        let refill_rate = policy.requests_per_window as f64 / policy.window_secs as f64;

        let mut buckets = self.buckets.blocking_write();
        let now_secs = Self::now_secs();

        let bucket = buckets.entry(cache_key).or_insert_with(|| BucketState {
            tokens: capacity,
            last_refill: Instant::now(),
            window_start: now_secs,
        });

        // Refill tokens based on elapsed time.
        let elapsed = bucket.last_refill.elapsed().as_secs_f64();
        bucket.tokens = (bucket.tokens + elapsed * refill_rate).min(capacity);
        bucket.last_refill = Instant::now();

        let reset_at = bucket.window_start + policy.window_secs;

        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            let remaining = bucket.tokens.floor() as u64;
            RateLimitResult {
                allowed: true,
                limit: policy.capacity(),
                remaining,
                reset_at,
            }
            tokens: max_tokens,
            max_tokens,
            refill_rate,
            last_refill: Self::now(),
        }
    }

    fn now() -> f64 {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs_f64()
    }

    pub fn try_consume(&mut self, tokens: f64) -> bool {
        self.refill();
        if self.tokens >= tokens {
            self.tokens -= tokens;
            true
        } else {
            RateLimitResult {
                allowed: false,
                limit: policy.capacity(),
                remaining: 0,
                reset_at,
            }
        }
    }

    fn update_rule(&self, tier: RateLimitTier, policy: RateLimitPolicy) {
        let mut policies = self.tier_policies.blocking_write();
        policies.insert(tier, policy);
    }
}

// ── Per-endpoint policy registry ─────────────────────────────────────────────

/// Registry mapping `(endpoint_pattern, tier)` to an override policy.
/// Falls back to the tier's default policy when no override is present.
#[derive(Default)]
pub struct EndpointPolicyRegistry {
    /// key: (endpoint_pattern, tier)
    overrides: RwLock<HashMap<(String, RateLimitTier), RateLimitPolicy>>,
    tier_defaults: RwLock<HashMap<RateLimitTier, RateLimitPolicy>>,
}

impl EndpointPolicyRegistry {
    pub fn new() -> Self {
        let mut defaults = HashMap::new();
        for tier in [
            RateLimitTier::Free,
            RateLimitTier::Pro,
            RateLimitTier::Enterprise,
        ] {
            defaults.insert(tier, default_policy(tier));
        }
        Self {
            overrides: RwLock::new(HashMap::new()),
            tier_defaults: RwLock::new(defaults),
        }
    }

    /// Resolve the effective policy for a request.
    pub fn resolve(&self, endpoint: &str, tier: RateLimitTier) -> RateLimitPolicy {
        let overrides = self.overrides.blocking_read();
        if let Some(p) = overrides.get(&(endpoint.to_string(), tier)) {
            return p.clone();
        }
        let defaults = self.tier_defaults.blocking_read();
        defaults
            .get(&tier)
            .cloned()
            .unwrap_or_else(|| default_policy(tier))
    }

    /// Admin: set a per-endpoint override for a tier.
    pub fn set_endpoint_policy(
        &self,
        endpoint: impl Into<String>,
        tier: RateLimitTier,
        policy: RateLimitPolicy,
    ) {
        let mut overrides = self.overrides.blocking_write();
        overrides.insert((endpoint.into(), tier), policy);
    }

    /// Admin: update the tier-wide default.
    pub fn set_tier_policy(&self, tier: RateLimitTier, policy: RateLimitPolicy) {
        let mut defaults = self.tier_defaults.blocking_write();
        defaults.insert(tier, policy);
    }
}

// ── Admin API DTOs ────────────────────────────────────────────────────────────

/// Request body for `POST /admin/rate-limits/rules`.
#[derive(Debug, Deserialize, Serialize)]
pub struct UpdateRuleRequest {
    pub tier: RateLimitTier,
    /// Optional endpoint pattern (e.g. `/v1/analyze`).
    /// When `None`, the update applies to the tier-wide default.
    pub endpoint: Option<String>,
    pub policy: RateLimitPolicy,
}

/// Response body for rule-update operations.
#[derive(Debug, Serialize)]
pub struct UpdateRuleResponse {
    pub success: bool,
    pub message: String,
}

/// Axum handler for `POST /admin/rate-limits/rules`.
///
/// # Example
///
/// ```ignore
/// Router::new()
///     .route("/admin/rate-limits/rules", post(admin_update_rate_limit_rule))
///     .with_state(Arc::new(registry))
/// ```
pub async fn admin_update_rate_limit_rule(
    axum::extract::State(registry): axum::extract::State<Arc<EndpointPolicyRegistry>>,
    axum::Json(body): axum::Json<UpdateRuleRequest>,
) -> axum::Json<UpdateRuleResponse> {
    match body.endpoint {
        Some(endpoint) => {
            registry.set_endpoint_policy(&endpoint, body.tier, body.policy);
            axum::Json(UpdateRuleResponse {
                success: true,
                message: format!(
                    "Updated endpoint policy for {:?} on {}",
                    body.tier, endpoint
                ),
            })
        }
        None => {
            registry.set_tier_policy(body.tier, body.policy);
            axum::Json(UpdateRuleResponse {
                success: true,
                message: format!("Updated tier-default policy for {:?}", body.tier),
            })
    fn is_stale(&self, max_age: Duration) -> bool {
        Self::now() - self.last_refill >= max_age.as_secs_f64()
    }

    fn refill(&mut self) {
        let now = Self::now();
        let elapsed = now - self.last_refill;
        if elapsed > 0.0 {
            self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
            self.last_refill = now;
        }
    }
}

const TOKEN_BUCKET_LUA: &str = r#"
local max_req = tonumber(ARGV[1])
local refill = tonumber(ARGV[2])
local consume = tonumber(ARGV[3])
local now = tonumber(ARGV[4])
local ttl = tonumber(ARGV[5])

local tokens = max_req
local last_refill = now

local data = redis.call('GET', KEYS[1])
if data then
    local ok, b = pcall(cjson.decode, data)
    if ok and type(b) == "table" then
        tokens = tonumber(b.tokens) or max_req
        last_refill = tonumber(b.last_refill) or now
    end
end

local elapsed = math.max(0, now - last_refill)
tokens = math.min(max_req, tokens + (elapsed * refill))

local allowed = tokens >= consume
if allowed then
    tokens = tokens - consume
end

local next_bucket = {
    tokens = tokens,
    max_tokens = max_req,
    refill_rate = refill,
    last_refill = now
}

redis.call('SETEX', KEYS[1], math.max(1, ttl), cjson.encode(next_bucket))

if allowed then
    return {1, tostring(tokens)}
else
    return {0, tostring(tokens)}
end
"#;

pub struct AgentRateLimiter {
    redis_client: Option<redis::Client>,
    buckets: HashMap<String, TokenBucket>,
    default_max: f64,
    default_refill_rate: f64,
}

impl AgentRateLimiter {
    pub fn new(default_max: f64, default_refill_rate: f64) -> Self {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let redis_client = redis::Client::open(url).ok();
        Self {
            redis_client,
            buckets: HashMap::new(),
            default_max,
            default_refill_rate,
        }
    }
}

// ── Redis-backed sliding-window rate limiter ─────────────────────────────────
//
// Uses a sorted-set per key where each member is a request timestamp.
// Members older than the window are pruned on every check (sliding window).

/// Redis-backed rate limiter for multi-instance deployments.
///
/// Requires a `redis::aio::ConnectionManager` (async multiplexed connection).
pub struct RedisRateLimiter {
    conn: redis::aio::ConnectionManager,
}

impl RedisRateLimiter {
    pub fn new(conn: redis::aio::ConnectionManager) -> Self {
        Self { conn }
    }

    /// Async check — preferred over the blocking `RateLimiter` trait method.
    pub async fn check_async(
        &self,
        key: &RateLimitKey,
        policy: &RateLimitPolicy,
    ) -> RateLimitResult {
        use redis::AsyncCommands;

        let cache_key = key.cache_key();
        let now_micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_micros() as i64;
        let window_micros = (policy.window_secs as i64) * 1_000_000;
        let cutoff = now_micros - window_micros;
        let capacity = policy.capacity();
        let reset_at = (now_micros / 1_000_000) as u64 + policy.window_secs;

        let mut conn = self.conn.clone();

        // Remove expired entries.
        let _: () = conn
            .zrembyscore(&cache_key, "-inf", cutoff)
            .await
            .unwrap_or(());

        // Count remaining entries in window.
        let count: u64 = conn.zcard(&cache_key).await.unwrap_or(capacity);

        if count < capacity {
            // Add this request.
            let member = format!("{}-{}", now_micros, uuid::Uuid::new_v4());
            let _: () = conn
                .zadd(&cache_key, &member, now_micros)
                .await
                .unwrap_or(());
            // Set TTL so stale keys are cleaned up automatically.
            let _: () = conn
                .expire(&cache_key, policy.window_secs as i64 * 2)
                .await
                .unwrap_or(());

            RateLimitResult {
                allowed: true,
                limit: capacity,
                remaining: capacity.saturating_sub(count + 1),
                reset_at,
            }
        } else {
            RateLimitResult {
                allowed: false,
                limit: capacity,
                remaining: 0,
                reset_at,
            }
        }
    }
}

// ── Blanket blocking impl for RedisRateLimiter (wraps async in block_in_place)

impl RateLimiter for RedisRateLimiter {
    fn check(&self, key: &RateLimitKey, policy: &RateLimitPolicy) -> RateLimitResult {
        tokio::task::block_in_place(|| {
            tokio::runtime::Handle::current()
                .block_on(self.check_async(key, policy))
        })
    }

    fn update_rule(&self, _tier: RateLimitTier, _policy: RateLimitPolicy) {
        // Redis implementation: policies are stored externally (e.g. config
        // service or database); this method is a no-op for the Redis backend.
    }
}

// ── Tests ────────────────────────────────────────────────────────────────────
    pub fn try_acquire(&mut self, agent_id: &str, tokens: f64) -> bool {
        let default_max = self.default_max;
        let default_refill_rate = self.default_refill_rate;

        if let Some(client) = &self.redis_client {
            if let Ok(mut con) = client.get_connection() {
                let key = format!("ratelimit:agent:{}", agent_id);
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
                let ttl = if default_refill_rate > 0.0 {
                    (default_max / default_refill_rate).ceil() as usize * 2
                } else {
                    3600
                };
                
                let script = redis::Script::new(TOKEN_BUCKET_LUA);
                let result: redis::RedisResult<(i32, String)> = script
                    .key(&key)
                    .arg(default_max)
                    .arg(default_refill_rate)
                    .arg(tokens)
                    .arg(now)
                    .arg(ttl)
                    .invoke(&mut con);

                if let Ok((allowed_int, _)) = result {
                    return allowed_int == 1;
                }
            }
        }

        let bucket = self
            .buckets
            .entry(agent_id.to_string())
            .or_insert_with(|| TokenBucket::new(default_max, default_refill_rate));
        bucket.try_consume(tokens)
    }
}

pub const DEFAULT_PUBLIC_RATE_LIMIT_REQUESTS: u64 = 60;
pub const DEFAULT_PUBLIC_RATE_LIMIT_WINDOW_SECS: u64 = 60;
const MAX_RATE_LIMIT_BUCKETS: usize = 100_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EndpointRateLimit {
    pub max_requests: u64,
    pub window: Duration,
}

impl Default for EndpointRateLimit {
    fn default() -> Self {
        Self::new(
            DEFAULT_PUBLIC_RATE_LIMIT_REQUESTS,
            Duration::from_secs(DEFAULT_PUBLIC_RATE_LIMIT_WINDOW_SECS),
        )
    }
}

impl EndpointRateLimit {
    pub fn new(max_requests: u64, window: Duration) -> Self {
        Self {
            max_requests: max_requests.max(1),
            window: if window.is_zero() {
                Duration::from_secs(1)
            } else {
                window
            },
        }
    }
}

#[derive(Debug, Clone)]
pub struct RateLimitConfig {
    pub enabled: bool,
    pub default: EndpointRateLimit,
    pub endpoints: HashMap<String, EndpointRateLimit>,
}

impl Default for RateLimitConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            default: EndpointRateLimit::new(
                DEFAULT_PUBLIC_RATE_LIMIT_REQUESTS,
                Duration::from_secs(DEFAULT_PUBLIC_RATE_LIMIT_WINDOW_SECS),
            ),
            endpoints: HashMap::new(),
        }
    }
}

impl RateLimitConfig {
    pub fn new(default: EndpointRateLimit) -> Self {
        Self {
            enabled: true,
            default,
            endpoints: HashMap::new(),
        }
    }

    pub fn from_env() -> Self {
        let enabled = match read_env(&[
            "PUBLIC_API_RATE_LIMIT_ENABLED",
            "PERIGEE_RATE_LIMIT_ENABLED",
            "RATE_LIMIT_ENABLED",
        ]) {
            Some(value) => match value.trim().to_ascii_lowercase().as_str() {
                "0" | "false" | "no" | "off" => false,
                _ => true,
            },
            None => true,
        };

        let max_requests = read_env(&[
            "PUBLIC_API_RATE_LIMIT_REQUESTS",
            "PUBLIC_API_RATE_LIMIT_MAX_REQUESTS",
            "PUBLIC_RATE_LIMIT_REQUESTS",
            "API_RATE_LIMIT_REQUESTS",
            "PERIGEE_RATE_LIMIT_REQUESTS",
            "RATE_LIMIT_REQUESTS",
        ])
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PUBLIC_RATE_LIMIT_REQUESTS);

        let window_secs = read_env(&[
            "PUBLIC_API_RATE_LIMIT_WINDOW_SECS",
            "PUBLIC_API_RATE_LIMIT_WINDOW_SECONDS",
            "PUBLIC_RATE_LIMIT_WINDOW_SECONDS",
            "API_RATE_LIMIT_WINDOW_SECONDS",
            "PERIGEE_RATE_LIMIT_WINDOW_SECS",
            "RATE_LIMIT_WINDOW_SECS",
        ])
        .and_then(|value| value.trim().parse::<u64>().ok())
        .filter(|value| *value > 0)
        .unwrap_or(DEFAULT_PUBLIC_RATE_LIMIT_WINDOW_SECS);

        let mut config = Self::new(EndpointRateLimit::new(
            max_requests,
            Duration::from_secs(window_secs),
        ));
        config.enabled = enabled;

        let raw_endpoints = read_env(&[
            "PUBLIC_API_RATE_LIMIT_ENDPOINTS",
            "PUBLIC_RATE_LIMIT_ENDPOINTS",
            "API_RATE_LIMIT_ENDPOINTS",
            "PER_ENDPOINT_RATE_LIMITS",
            "RATE_LIMIT_ENDPOINTS",
        ]);
        if let Some(raw_endpoints) = raw_endpoints {
            for (route, limit) in parse_endpoint_limits(&raw_endpoints, config.default) {
                config.endpoints.insert(route, limit);
            }
        }

        config
    }

    pub fn with_endpoint(mut self, route: impl Into<String>, limit: EndpointRateLimit) -> Self {
        self.endpoints.insert(normalize_endpoint(&route.into()), limit);
        self
    }

    pub fn policy_for(&self, endpoint: &str) -> EndpointRateLimit {
        let normalized = normalize_endpoint(endpoint);
        if let Some(limit) = self.endpoints.get(&normalized) {
            return *limit;
        }

        self.endpoints
            .iter()
            .filter_map(|(route, limit)| {
                let Some(prefix) = route.strip_suffix("/*") else {
                    return None;
                };
                normalized
                    .strip_prefix(prefix)
                    .filter(|remainder| remainder.is_empty() || remainder.starts_with('/'))
                    .map(|_| (prefix.len(), *limit))
            })
            .max_by_key(|(prefix_len, _)| *prefix_len)
            .map(|(_, limit)| limit)
            .unwrap_or(self.default)
    }

    pub fn limit_for(&self, endpoint: &str) -> EndpointRateLimit {
        self.policy_for(endpoint)
    }

    pub fn for_endpoint(&self, endpoint: &str) -> EndpointRateLimit {
        self.policy_for(endpoint)
    }
}

fn read_env(names: &[&str]) -> Option<String> {
    names.iter().find_map(|name| std::env::var(name).ok())
}

fn parse_endpoint_limits(
    raw: &str,
    default: EndpointRateLimit,
) -> Vec<(String, EndpointRateLimit)> {
    raw.split(',')
        .filter_map(|pair| {
            let (route, specification) = pair.split_once('=')?;
            let route = normalize_endpoint(route);
            if route.is_empty() {
                return None;
            }

            let (requests, window) = if let Some((requests, window)) = specification
                .split_once('/')
                .or_else(|| specification.split_once(':'))
            {
                (
                    requests.trim().parse::<u64>().ok()?,
                    window.trim().parse::<u64>().ok()?,
                )
            } else {
                (
                    specification.trim().parse::<u64>().ok()?,
                    default.window.as_secs(),
                )
            };

            Some((route, EndpointRateLimit::new(requests, Duration::from_secs(window))))
        })
        .collect()
}

pub fn normalize_endpoint(endpoint: &str) -> String {
    let trimmed = endpoint.trim();
    let without_version = trimmed
        .strip_prefix("/v1")
        .filter(|path| path.is_empty() || path.starts_with('/'))
        .map(|path| {
            if path.is_empty() {
                "/".to_string()
            } else {
                path.to_string()
            }
        })
        .unwrap_or_else(|| trimmed.to_string());
    let with_root = if without_version.starts_with('/') {
        without_version
    } else {
        format!("/{}", without_version)
    };
    let normalized = with_root.trim_end_matches('/');
    if normalized.is_empty() {
        "/".to_string()
    } else {
        normalized.to_string()
    }
}

pub fn identify_client(headers: &HeaderMap) -> String {
    if let Some(value) = headers
        .get("x-api-key")
        .and_then(|value| value.to_str().ok())
    {
        let value = value.trim();
        if !value.is_empty() {
            return value.to_string();
        }
    }

    if let Some(value) = headers
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
    {
        if let Some((scheme, token)) = value.split_once(' ') {
            let token = token.trim();
            if !token.is_empty()
                && (scheme.eq_ignore_ascii_case("apikey")
                    || scheme.eq_ignore_ascii_case("bearer"))
            {
                return token.to_string();
            }
        }
    }

    if let Some(value) = headers
        .get("x-forwarded-for")
        .and_then(|value| value.to_str().ok())
    {
        if let Some(address) = value.split(',').next().map(str::trim) {
            if !address.is_empty() {
                return address.to_string();
            }
        }
    }

    headers
        .get("x-real-ip")
        .and_then(|value| value.to_str().ok())
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("anonymous")
        .to_string()
}

fn client_bucket_id(client_id: &str) -> String {
    hex::encode(Sha256::digest(client_id.as_bytes()))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RateLimitDecision {
    pub allowed: bool,
    pub limit: u64,
    pub remaining: u64,
    pub retry_after: Duration,
}

#[derive(Clone)]
pub struct ApiRateLimiter {
    redis_client: Option<redis::Client>,
    buckets: Arc<Mutex<HashMap<(String, String), TokenBucket>>>,
    config: RateLimitConfig,
}

impl Default for ApiRateLimiter {
    fn default() -> Self {
        Self::new(RateLimitConfig::default())
    }
}

impl ApiRateLimiter {
    pub fn new(config: RateLimitConfig) -> Self {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        let redis_client = redis::Client::open(url).ok();
        Self {
            redis_client,
            buckets: Arc::new(Mutex::new(HashMap::new())),
            config,
        }
    }

    pub fn from_env() -> Self {
        Self::new(RateLimitConfig::from_env())
    }

    pub fn config(&self) -> &RateLimitConfig {
        &self.config
    }

    pub fn try_acquire(&self, client_id: &str, endpoint: &str) -> bool {
        self.check(client_id, endpoint).allowed
    }

    pub fn check(&self, client_id: &str, endpoint: &str) -> RateLimitDecision {
        if !self.config.enabled {
            return RateLimitDecision {
                allowed: true,
                limit: 0,
                remaining: 0,
                retry_after: Duration::ZERO,
            };
        }

        let policy = self.config.policy_for(endpoint);
        let policy = EndpointRateLimit::new(policy.max_requests, policy.window);
        let bucket_id = client_bucket_id(client_id);
        let normalized_endpoint = normalize_endpoint(endpoint);

        let max_req = policy.max_requests as f64;
        let refill = policy.max_requests as f64 / policy.window.as_secs_f64();

        let mut allowed = false;
        let mut remaining = 0;
        let mut retry_after = Duration::ZERO;
        let mut bucket_refill_rate = 0.0;
        let mut bucket_available = 0.0;

        let mut used_redis = false;

        if let Some(client) = &self.redis_client {
            if let Ok(mut con) = client.get_connection() {
                used_redis = true;
                let key = format!("ratelimit:api:{}:{}", bucket_id, normalized_endpoint);
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_secs_f64();
                let ttl = policy.window.as_secs() * 2;
                
                let script = redis::Script::new(TOKEN_BUCKET_LUA);
                let result: redis::RedisResult<(i32, String)> = script
                    .key(&key)
                    .arg(max_req)
                    .arg(refill)
                    .arg(1.0)
                    .arg(now)
                    .arg(ttl)
                    .invoke(&mut con);

                if let Ok((allowed_int, tokens_str)) = result {
                    allowed = allowed_int == 1;
                    let tokens_f64 = tokens_str.parse::<f64>().unwrap_or(0.0);
                    remaining = tokens_f64.floor().max(0.0) as u64;
                    bucket_refill_rate = refill;
                    bucket_available = tokens_f64;
                } else {
                    used_redis = false;
                }
            }
        }

        if !used_redis && crate::rate_limit_failure_policy::fail_closed() {
            tracing::warn!("rate-limit shared store unavailable; failing closed");
            return RateLimitDecision {
                allowed: false,
                limit: policy.max_requests,
                remaining: 0,
                retry_after: policy.window,
            };
        }

        if !used_redis {
            let key = (bucket_id, normalized_endpoint);
            let mut buckets = self
                .buckets
                .lock()
                .unwrap_or_else(|poisoned| poisoned.into_inner());
            if !buckets.contains_key(&key) && buckets.len() >= MAX_RATE_LIMIT_BUCKETS {
                let stale_after = policy.window.saturating_mul(2);
                buckets.retain(|_, bucket| !bucket.is_stale(stale_after));
                if buckets.len() >= MAX_RATE_LIMIT_BUCKETS {
                    return RateLimitDecision {
                        allowed: false,
                        limit: policy.max_requests,
                        remaining: 0,
                        retry_after: policy.window,
                    };
                }
            }
            let bucket = buckets.entry(key).or_insert_with(|| {
                TokenBucket::new(max_req, refill)
            });
            allowed = bucket.try_consume(1.0);
            remaining = bucket.available().floor().max(0.0) as u64;
            bucket_refill_rate = bucket.refill_rate;
            bucket_available = bucket.available();
        }

        if !allowed {
            retry_after = if bucket_refill_rate > 0.0 {
                Duration::from_secs_f64((1.0 - bucket_available).max(0.0) / bucket_refill_rate)
            } else {
                policy.window
            };
        }

        RateLimitDecision {
            allowed,
            limit: policy.max_requests,
            remaining: remaining.min(policy.max_requests),
            retry_after,
        }
    }

    pub fn clear(&self) {
        self.buckets
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clear();
    }
}

#[derive(Clone)]
pub struct RateLimitLayer {
    limiter: ApiRateLimiter,
}

impl Default for RateLimitLayer {
    fn default() -> Self {
        Self::new(ApiRateLimiter::default())
    }
}

pub fn public_api_rate_limit_layer_from_env() -> RateLimitLayer {
    RateLimitLayer::from_env()
}

impl RateLimitLayer {
    pub fn new(limiter: ApiRateLimiter) -> Self {
        Self { limiter }
    }

    pub fn from_env() -> Self {
        Self::new(ApiRateLimiter::from_env())
    }
}

impl<S> Layer<S> for RateLimitLayer {
    type Service = RateLimitMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        RateLimitMiddleware {
            inner,
            limiter: self.limiter.clone(),
        }
    }
}

#[derive(Clone)]
pub struct RateLimitMiddleware<S> {
    inner: S,
    limiter: ApiRateLimiter,
}

impl<S> Service<Request<Body>> for RateLimitMiddleware<S>
where
    S: Service<Request<Body>, Response = Response> + Send + Clone + 'static,
    S::Future: Send + 'static,
    S::Error: std::fmt::Display,
{
    type Response = Response;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, request: Request<Body>) -> Self::Future {
        if request.method() == Method::OPTIONS {
            let mut inner = self.inner.clone();
            return Box::pin(async move { inner.call(request).await });
        }

        let endpoint = request
            .extensions()
            .get::<MatchedPath>()
            .map(|path| path.as_str().to_string())
            .unwrap_or_else(|| request.uri().path().to_string());
        let client_id = identify_client(request.headers());
        let decision = self.limiter.check(&client_id, &endpoint);

        if !decision.allowed {
            return Box::pin(async move { Ok(rate_limit_response(decision)) });
        }

        let mut inner = self.inner.clone();
        Box::pin(async move {
            let mut response = inner.call(request).await?;
            add_rate_limit_headers(&mut response, decision);
            Ok(response)
        })
    }
}

fn add_rate_limit_headers(response: &mut Response, decision: RateLimitDecision) {
    if decision.limit == 0 {
        return;
    }
    insert_header(response, "x-ratelimit-limit", decision.limit.to_string());
    insert_header(
        response,
        "x-ratelimit-remaining",
        decision.remaining.to_string(),
    );
    if !decision.allowed {
        insert_header(
            response,
            "retry-after",
            decision.retry_after.as_secs().max(1).to_string(),
        );
    }
}

fn rate_limit_response(decision: RateLimitDecision) -> Response {
    let body = Json(ErrorResponse::from_error_code_with_legacy(
        ErrorCode::RateLimitExceeded,
        ErrorCode::TooManyRequests,
        "Rate limit exceeded for this client and endpoint",
    ));
    let mut response = (StatusCode::TOO_MANY_REQUESTS, body).into_response();
    add_rate_limit_headers(&mut response, decision);
    response
}

fn insert_header(response: &mut Response, name: &'static str, value: String) {
    if let Ok(value) = HeaderValue::from_str(&value) {
        response
            .headers_mut()
            .insert(HeaderName::from_static(name), value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn free_policy() -> RateLimitPolicy {
        RateLimitPolicy::new(3, 60, 1) // capacity = 4
    }

    #[test]
    fn test_token_bucket_allows_within_limit() {
        let limiter = TokenBucketRateLimiter::new();
        let key = RateLimitKey::ApiKey("test-key".to_string());
        let policy = free_policy();

        let r1 = limiter.check(&key, &policy);
        assert!(r1.allowed);
        assert_eq!(r1.limit, 4);

        let r2 = limiter.check(&key, &policy);
        assert!(r2.allowed);
        assert!(r2.remaining < r1.remaining);
    }

    #[test]
    fn test_token_bucket_exhausts_capacity() {
        let limiter = TokenBucketRateLimiter::new();
        let key = RateLimitKey::IpAddress("127.0.0.1".to_string());
        let policy = RateLimitPolicy::new(2, 60, 0); // capacity = 2

        assert!(limiter.check(&key, &policy).allowed);
        assert!(limiter.check(&key, &policy).allowed);
        assert!(!limiter.check(&key, &policy).allowed);
    }

    #[test]
    fn test_different_keys_independent() {
        let limiter = TokenBucketRateLimiter::new();
        let policy = RateLimitPolicy::new(1, 60, 0);

        let k1 = RateLimitKey::UserId("user-1".to_string());
        let k2 = RateLimitKey::UserId("user-2".to_string());

        assert!(limiter.check(&k1, &policy).allowed);
        assert!(!limiter.check(&k1, &policy).allowed);
        // k2 is unaffected by k1
        assert!(limiter.check(&k2, &policy).allowed);
    }

    #[test]
    fn test_update_rule() {
        let limiter = TokenBucketRateLimiter::new();
        let new_policy = RateLimitPolicy::new(500, 60, 50);
        limiter.update_rule(RateLimitTier::Free, new_policy.clone());

        let policies = limiter.tier_policies.blocking_read();
        let stored = policies.get(&RateLimitTier::Free).unwrap();
        assert_eq!(stored.requests_per_window, 500);
    }

    #[test]
    fn test_result_to_headers() {
        let result = RateLimitResult {
            allowed: true,
            limit: 100,
            remaining: 42,
            reset_at: 9999,
        };
        let headers = result.to_headers();
        assert_eq!(headers.len(), 3);
        assert!(headers.iter().any(|(k, v)| k == HEADER_RATELIMIT_LIMIT && v == "100"));
        assert!(headers.iter().any(|(k, v)| k == HEADER_RATELIMIT_REMAINING && v == "42"));
        assert!(headers.iter().any(|(k, v)| k == HEADER_RATELIMIT_RESET && v == "9999"));
    }

    #[test]
    fn test_endpoint_registry_fallback() {
        let registry = EndpointPolicyRegistry::new();
        let policy = registry.resolve("/v1/simulate", RateLimitTier::Free);
        assert_eq!(policy.requests_per_window, 60);
    }

    #[test]
    fn test_endpoint_registry_override() {
        let registry = EndpointPolicyRegistry::new();
        let custom = RateLimitPolicy::new(5, 10, 2);
        registry.set_endpoint_policy("/v1/admin", RateLimitTier::Free, custom);

        let resolved = registry.resolve("/v1/admin", RateLimitTier::Free);
        assert_eq!(resolved.requests_per_window, 5);
        // Other endpoints still fall back to default
        let fallback = registry.resolve("/v1/simulate", RateLimitTier::Free);
        assert_eq!(fallback.requests_per_window, 60);
    }

    #[test]
    fn test_cache_key_prefixes() {
        assert!(RateLimitKey::ApiKey("x".into()).cache_key().starts_with("rl:api:"));
        assert!(RateLimitKey::IpAddress("x".into()).cache_key().starts_with("rl:ip:"));
        assert!(RateLimitKey::UserId("x".into()).cache_key().starts_with("rl:user:"));
    }

    #[test]
    fn test_default_tier_policies() {
        let free = default_policy(RateLimitTier::Free);
        let pro = default_policy(RateLimitTier::Pro);
        let ent = default_policy(RateLimitTier::Enterprise);

        assert!(pro.requests_per_window > free.requests_per_window);
        assert!(ent.requests_per_window > pro.requests_per_window);
        assert!(ent.burst_allowance > pro.burst_allowance);
    }

    #[test]
    fn test_redis_persistence_agent() {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        if redis::Client::open(&url).and_then(|c| c.get_connection()).is_err() {
            return;
        }

        let agent_id = format!("test-agent-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros());
        let mut limiter1 = AgentRateLimiter::new(10.0, 1.0);
        let allowed = limiter1.try_acquire(&agent_id, 10.0);
        assert!(allowed);
        let allowed = limiter1.try_acquire(&agent_id, 1.0);
        assert!(!allowed);

        // Simulated restart
        let mut limiter2 = AgentRateLimiter::new(10.0, 1.0);
        let allowed = limiter2.try_acquire(&agent_id, 1.0);
        assert!(!allowed);
    }

    #[test]
    fn test_redis_persistence_api() {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        if redis::Client::open(&url).and_then(|c| c.get_connection()).is_err() {
            return;
        }

        let client_id = format!("test-client-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros());
        let endpoint = "/test/endpoint";
        
        let config = RateLimitConfig::new(EndpointRateLimit::new(10, Duration::from_secs(10)));
        let limiter1 = ApiRateLimiter::new(config.clone());
        
        for _ in 0..10 {
            assert!(limiter1.check(&client_id, endpoint).allowed);
        }
        assert!(!limiter1.check(&client_id, endpoint).allowed);

        // Simulated restart
        let limiter2 = ApiRateLimiter::new(config);
        assert!(!limiter2.check(&client_id, endpoint).allowed);
    }
    
    #[test]
    fn test_concurrent_api_requests() {
        let url = std::env::var("REDIS_URL").unwrap_or_else(|_| "redis://127.0.0.1:6379".to_string());
        if redis::Client::open(&url).and_then(|c| c.get_connection()).is_err() {
            return;
        }

        let client_id = format!("concurrent-client-{}", std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_micros());
        let endpoint = "/test/concurrent";
        
        let config = RateLimitConfig::new(EndpointRateLimit::new(100, Duration::from_secs(60)));
        
        let mut threads = vec![];
        for _ in 0..10 {
            let config = config.clone();
            let client_id = client_id.clone();
            threads.push(std::thread::spawn(move || {
                let limiter = ApiRateLimiter::new(config);
                let mut allowed_count = 0;
                for _ in 0..20 {
                    if limiter.check(&client_id, endpoint).allowed {
                        allowed_count += 1;
                    }
                }
                allowed_count
            }));
        }
        
        let total_allowed: i32 = threads.into_iter().map(|t| t.join().unwrap()).sum();
        assert_eq!(total_allowed, 100);
    }
}
