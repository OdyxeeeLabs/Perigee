//! Rate limiter using token bucket algorithm.
//!
//! **Single-instance deployment constraint:** This module uses in-memory state
//! (`HashMap`) for rate limit counters. It is NOT shared across multiple
//! backend instances. In a horizontally-scaled deployment, each instance has
//! its own independent rate limit counters, making rate limiting ineffective
//! at the fleet level.
//!
//! To use across multiple instances, the token bucket state must be backed by
//! a shared store such as Redis or the database. Until then, this module is
//! intended for single-instance deployments only.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::{
    body::Body,
    extract::{MatchedPath, Request},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    response::{IntoResponse, Response},
    Json,
};
use sha2::{Digest, Sha256};
use tower::{Layer, Service};

use crate::error_codes::{ErrorCode, ErrorResponse};

pub struct TokenBucket {
    tokens: f64,
    max_tokens: f64,
    refill_rate: f64,
    last_refill: Instant,
}

impl TokenBucket {
    pub fn new(max_tokens: f64, refill_rate: f64) -> Self {
        Self {
            tokens: max_tokens,
            max_tokens,
            refill_rate,
            last_refill: Instant::now(),
        }
    }

    pub fn try_consume(&mut self, tokens: f64) -> bool {
        self.refill();
        if self.tokens >= tokens {
            self.tokens -= tokens;
            true
        } else {
            false
        }
    }

    pub fn available(&self) -> f64 {
        self.tokens
    }

    fn is_stale(&self, max_age: Duration) -> bool {
        self.last_refill.elapsed() >= max_age
    }

    fn refill(&mut self) {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_refill).as_secs_f64();
        self.tokens = (self.tokens + elapsed * self.refill_rate).min(self.max_tokens);
        self.last_refill = now;
    }
}

pub struct AgentRateLimiter {
    buckets: HashMap<String, TokenBucket>,
    default_max: f64,
    default_refill_rate: f64,
}

impl AgentRateLimiter {
    pub fn new(default_max: f64, default_refill_rate: f64) -> Self {
        Self {
            buckets: HashMap::new(),
            default_max,
            default_refill_rate,
        }
    }

    pub fn register_agent(&mut self, agent_id: &str) {
        self.buckets
            .entry(agent_id.to_string())
            .or_insert_with(|| TokenBucket::new(self.default_max, self.default_refill_rate));
    }

    pub fn try_acquire(&mut self, agent_id: &str, tokens: f64) -> bool {
        let default_max = self.default_max;
        let default_refill_rate = self.default_refill_rate;
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
        Self {
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
        let key = (
            client_bucket_id(client_id),
            normalize_endpoint(endpoint),
        );
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
            TokenBucket::new(
                policy.max_requests as f64,
                policy.max_requests as f64 / policy.window.as_secs_f64(),
            )
        });
        let allowed = bucket.try_consume(1.0);
        let remaining = bucket.available().floor().max(0.0) as u64;
        let retry_after = if allowed {
            Duration::ZERO
        } else if bucket.refill_rate > 0.0 {
            Duration::from_secs_f64((1.0 - bucket.available()).max(0.0) / bucket.refill_rate)
        } else {
            policy.window
        };

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

    #[test]
    fn test_token_bucket_basic() {
        let mut bucket = TokenBucket::new(3.0, 1.0);
        assert!(bucket.try_consume(1.0));
        assert!(bucket.try_consume(1.0));
        assert!(bucket.try_consume(1.0));
        assert!(!bucket.try_consume(1.0));
    }

    #[test]
    fn test_token_bucket_available() {
        let mut bucket = TokenBucket::new(5.0, 1.0);
        assert_eq!(bucket.available(), 5.0);
        bucket.try_consume(2.0);
        assert_eq!(bucket.available(), 3.0);
    }

    #[test]
    fn test_agent_rate_limiter() {
        let mut limiter = AgentRateLimiter::new(2.0, 0.0);
        assert!(limiter.try_acquire("a", 1.0));
        assert!(limiter.try_acquire("a", 1.0));
        assert!(!limiter.try_acquire("a", 1.0));
    }

    #[test]
    fn test_agent_rate_limiter_auto_register() {
        let mut limiter = AgentRateLimiter::new(1.0, 0.0);
        assert!(limiter.try_acquire("new-agent", 1.0));
        assert!(!limiter.try_acquire("new-agent", 1.0));
    }
}
