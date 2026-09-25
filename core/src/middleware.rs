//! Axum middleware helpers.
//!
//! ## Middleware included
//!
//! ### `correlation_id_middleware`
//! Propagates (or generates) an `x-correlation-id` / `x-request-id` pair and
//! emits a structured tracing span per request.
//!
//! ### `metrics_layer` / `MetricsMiddleware`
//! Tower [`Layer`] that auto-instruments every HTTP request with three
//! Prometheus metrics from [`crate::metrics::Metrics`]:
//!
//! - `http_requests_total{method, route, status}` — counter
//! - `http_request_duration_seconds{method, route}` — histogram
//! - `http_requests_in_flight{method, route}` — gauge (RAII-tracked)
//!
//! Wire it into the Router **before** `TraceLayer` so it captures the
//! matched route pattern rather than the raw URI:
//!
//! ```ignore
//! let app = Router::new()
//!     ...
//!     .layer(MetricsLayer::new(Arc::clone(&metrics)))
//!     .layer(TraceLayer::new_for_http());
//! ```

use std::{
    collections::HashMap,
    future::Future,
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
    time::Instant,
};

use axum::{
    body::{Body, Bytes},
    extract::{Extension, Request},
    http::{header, HeaderName, HeaderValue, Method},
    middleware::Next,
    response::Response,
};
use tower::{Layer, Service};
use tower_http::cors::{AllowHeaders, AllowMethods, AllowOrigin, CorsLayer};
use tracing::{info, Instrument};
use uuid::Uuid;

use crate::error_codes::{ErrorCode, ErrorResponse};
use sha2::{Digest, Sha256};

use crate::metrics::Metrics;
use crate::runner::RequestCancellation;
use crate::signed_receipt::{ApiReceipt, ReceiptSigner, RECEIPT_HEADER, RECEIPT_ID_HEADER};

const CORRELATION_ID_HEADER: &str = "x-correlation-id";
const REQUEST_ID_HEADER: &str = "x-request-id";

#[derive(Clone, Debug)]
pub struct RequestContext {
    pub request_id: String,
    pub correlation_id: String,
}

fn normalized_id(value: Option<&HeaderValue>) -> Option<String> {
    let value = value?.to_str().ok()?.trim();
    if value.is_empty()
        || value.len() > 128
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"-_.:".contains(&byte))
    {
        return None;
    }
    Some(value.to_string())
}

fn response_header(value: &str) -> HeaderValue {
    HeaderValue::from_str(value).unwrap_or_else(|_| HeaderValue::from_static("unknown"))
}
pub async fn receipt_middleware(
    Extension(signer): Extension<ReceiptSigner>,
    request: Request,
    next: Next,
) -> Response {
    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let request_id = request
        .headers()
        .get(REQUEST_ID_HEADER)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty() && value.len() <= 128)
        .map(str::to_string)
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let (mut parts, body) = next.run(request).await.into_parts();
    let write_request = matches!(method.as_str(), "POST" | "PUT" | "PATCH" | "DELETE");
    if !write_request {
        return Response::from_parts(parts, body);
    }

    let bytes = match axum::body::to_bytes(body, 2 * 1024 * 1024).await {
        Ok(bytes) => bytes,
        Err(_) => {
            parts.status = axum::http::StatusCode::INTERNAL_SERVER_ERROR;
            for header in [
                axum::http::header::CONTENT_ENCODING,
                axum::http::header::CONTENT_RANGE,
                axum::http::header::ETAG,
                axum::http::header::CONTENT_DISPOSITION,
                axum::http::header::CONTENT_LANGUAGE,
                axum::http::header::CACHE_CONTROL,
                axum::http::header::EXPIRES,
                axum::http::header::LAST_MODIFIED,
                axum::http::header::ACCEPT_RANGES,
                axum::http::header::LOCATION,
                axum::http::header::SET_COOKIE,
                axum::http::header::WWW_AUTHENTICATE,
                axum::http::header::ALLOW,
                axum::http::header::LINK,
            ] {
                parts.headers.remove(header);
            }
            parts.headers.insert(
                axum::http::header::CONTENT_TYPE,
                HeaderValue::from_static("application/json"),
            );
            Bytes::from(
                serde_json::json!({
                    "error": "RECEIPT_BODY_TOO_LARGE",
                    "message": "The response could not be buffered for receipt signing"
                })
                .to_string(),
            )
        }
    };
    parts.headers.remove(axum::http::header::CONTENT_LENGTH);
    parts.headers.remove(axum::http::header::TRANSFER_ENCODING);
    parts.headers.remove(axum::http::header::TRAILER);
    let response_digest = hex::encode(Sha256::digest(bytes.as_ref()));
    let receipt = ApiReceipt::new(
        format!("{} {}", method.as_str(), path),
        path.clone(),
        request_id,
        method.as_str(),
        path,
        parts.status.as_u16(),
        response_digest,
        chrono::Utc::now().timestamp(),
    );

    let mut response = Response::from_parts(parts, Body::from(bytes));
    if !attach_receipt_headers(&mut response, &signer, &receipt) {
        tracing::error!(
            method = %method,
            path = %receipt.path,
            "Failed to attach API receipt"
        );
    }
    response
}

fn attach_receipt_headers(
    response: &mut Response,
    signer: &ReceiptSigner,
    receipt: &ApiReceipt,
) -> bool {
    let Ok(signed) = signer.sign_api(receipt) else {
        return false;
    };
    let Ok(serialized) = serde_json::to_vec(&signed) else {
        return false;
    };
    let Ok(value) = HeaderValue::from_bytes(&serialized) else {
        return false;
    };
    let Ok(receipt_id) = HeaderValue::from_str(&signed.receipt_id) else {
        return false;
    };
    response.headers_mut().insert(HeaderName::from_static(RECEIPT_HEADER), value);
    response
        .headers_mut()
        .insert(HeaderName::from_static(RECEIPT_ID_HEADER), receipt_id);
    true
}

pub async fn request_cancellation_middleware(mut request: Request, next: Next) -> Response {
    let cancellation = RequestCancellation::new();
    request.extensions_mut().insert(cancellation.clone());
    let guard = cancellation.guard();
    let mut response = next.run(request).await;
    response.extensions_mut().insert(Arc::new(guard));
    response
}

pub async fn correlation_id_middleware(request: Request, next: Next) -> Response {
    let correlation_id = request
        .headers()
        .get(CORRELATION_ID_HEADER)
        .and_then(|h| h.to_str().ok())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

pub async fn correlation_id_middleware(mut request: Request, next: Next) -> Response {
    let incoming_request_id = normalized_id(request.headers().get(REQUEST_ID_HEADER));
    let incoming_correlation_id = normalized_id(request.headers().get(CORRELATION_ID_HEADER));
    let request_id = incoming_request_id
        .clone()
        .or_else(|| incoming_correlation_id.clone())
        .unwrap_or_else(|| Uuid::new_v4().to_string());
    let correlation_id = incoming_correlation_id.unwrap_or_else(|| request_id.clone());
    let method = request.method().clone();
    let path = request.uri().path().to_owned();

    let span = tracing::info_span!(
        "http_request",
        correlation_id = %correlation_id,
        request_id = %request_id,
        method = %method,
        path = %path,
    );

    request.headers_mut().insert(
        HeaderName::from_static(CORRELATION_ID_HEADER),
        response_header(&correlation_id),
    );
    request.headers_mut().insert(
        HeaderName::from_static(REQUEST_ID_HEADER),
        response_header(&request_id),
        HeaderValue::from_str(&correlation_id)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid-correlation-id")),
    );
    request.headers_mut().insert(
        HeaderName::from_static(REQUEST_ID_HEADER),
        HeaderValue::from_str(&request_id)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid-request-id")),
    );
    request.extensions_mut().insert(RequestContext {
        request_id: request_id.clone(),
        correlation_id: correlation_id.clone(),
    });

    let start = Instant::now();
    let mut response = next.run(request).instrument(span.clone()).await;
    let latency = start.elapsed();
    let status = response.status();

    info!(
        parent: &span,
        correlation_id = %correlation_id,
        request_id = %request_id,
        method = %method,
        path = %path,
        status = %status,
        latency_ms = latency.as_millis(),
        "Request completed"
    );

    response.headers_mut().insert(
        HeaderName::from_static(CORRELATION_ID_HEADER),
        response_header(&correlation_id),
    );
    response.headers_mut().insert(
        HeaderName::from_static(REQUEST_ID_HEADER),
        response_header(&request_id),
        HeaderValue::from_str(&correlation_id)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid-correlation-id")),
    );
    response.headers_mut().insert(
        HeaderName::from_static(REQUEST_ID_HEADER),
        HeaderValue::from_str(&request_id)
            .unwrap_or_else(|_| HeaderValue::from_static("invalid-request-id")),
    );

    response
}

// ── Prometheus HTTP metrics layer ────────────────────────────────────────────

/// Tower [`Layer`] that records HTTP metrics for every request.
///
/// Clone is cheap — it only clones an `Arc`.
#[derive(Clone)]
pub struct MetricsLayer {
    metrics: Arc<Metrics>,
}

impl MetricsLayer {
    pub fn new(metrics: Arc<Metrics>) -> Self {
        Self { metrics }
    }
}

impl<S> Layer<S> for MetricsLayer {
    type Service = MetricsMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        MetricsMiddleware {
            inner,
            metrics: Arc::clone(&self.metrics),
        }
    }
}

/// Tower [`Service`] wrapper that wraps each call with Prometheus bookkeeping.
#[derive(Clone)]
pub struct MetricsMiddleware<S> {
    inner: S,
    metrics: Arc<Metrics>,
}

impl<S> Service<Request<Body>> for MetricsMiddleware<S>
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

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        let metrics = Arc::clone(&self.metrics);
        let method = req.method().to_string();

        // Prefer the matched-route pattern (`/analyze/:id`) over the raw URI so
        // cardinality stays bounded.  Axum injects this via the
        // `axum::extract::MatchedPath` extension — fall back to the path
        // component of the URI when the extension is absent (e.g. 404 paths).
        let route = req
            .extensions()
            .get::<axum::extract::MatchedPath>()
            .map(|mp| mp.as_str().to_string())
            .unwrap_or_else(|| {
                let p = req.uri().path().to_string();
                // Truncate long unknown paths so we don't blow up cardinality.
                if p.len() > 64 { "unknown".to_string() } else { p }
            });

        // Increment the in-flight gauge; decrement it when the future drops.
        metrics
            .http_requests_in_flight
            .with_label_values(&[&method, &route])
            .inc();

        let start = Instant::now();
        let mut inner = self.inner.clone();

        Box::pin(async move {
            // Always decrement in-flight when done (success or error).
            let _guard = InFlightGuard {
                metrics: Arc::clone(&metrics),
                method: method.clone(),
                route: route.clone(),
            };

            let result = inner.call(req).await;

            let elapsed = start.elapsed().as_secs_f64();
            let status = match &result {
                Ok(resp) => resp.status().as_u16().to_string(),
                Err(_) => "500".to_string(),
            };

            metrics
                .http_requests_total
                .with_label_values(&[&method, &route, &status])
                .inc();

            metrics
                .http_request_duration_seconds
                .with_label_values(&[&method, &route])
                .observe(elapsed);

            result
        })
    }
}

/// RAII guard that decrements `http_requests_in_flight` when dropped.
struct InFlightGuard {
    metrics: Arc<Metrics>,
    method: String,
    route: String,
}

impl Drop for InFlightGuard {
    fn drop(&mut self) {
        self.metrics
            .http_requests_in_flight
            .with_label_values(&[&self.method, &self.route])
            .dec();
    }
}

// ── Method-Not-Allowed normaliser ────────────────────────────────────────────

/// Intercept Axum's automatic `405 Method Not Allowed` responses and rewrite
/// them into the standard `{ "error": "METHOD_NOT_ALLOWED", "message": "…" }`
/// envelope so every error code — 404, 405, and application errors — looks
/// identical to the client.
///
/// Axum generates 405 internally (before reaching any handler) when the path
/// matches a route but the HTTP method does not.  This middleware runs
/// *after* the response is produced and normalises those bare 405 bodies.
pub async fn method_not_allowed_middleware(request: Request, next: Next) -> Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;

    let method = request.method().clone();
    let path = request.uri().path().to_owned();
    let response = next.run(request).await;

    if response.status() == StatusCode::METHOD_NOT_ALLOWED {
        tracing::debug!(
            method = %method,
            path = %path,
            "Method not allowed"
        );
        let body = Json(serde_json::json!({
            "error": "METHOD_NOT_ALLOWED",
            "message": format!("Method {} is not allowed for {}", method, path)
        }));
        return (StatusCode::METHOD_NOT_ALLOWED, body).into_response();
        let body = Json(ErrorResponse::from_error_code(
            ErrorCode::MethodNotAllowed,
            format!("Method {} is not allowed for {}", method, uri.path()),
        ));
        let mut normalized = (StatusCode::METHOD_NOT_ALLOWED, body).into_response();
        for (name, value) in response.headers() {
            if !normalized.headers().contains_key(name) {
                normalized.headers_mut().insert(name.clone(), value.clone());
            }
        }
        return normalized;
    }

    response
}

// ── Request body size limits (CORE-20) ──────────────────────────────────────

/// Enforced size limit when nothing is configured.
const DEFAULT_MAX_BODY_BYTES: usize = 1024 * 1024;

/// Per-route maximum POST/PUT request body size (CORE-20).
///
/// Configured from the environment:
/// - `PERIGEE_MAX_BODY_BYTES`           — global default (1 MiB).
/// - `PERIGEE_MAX_BODY_BYTES_PER_ROUTE` — comma-separated `route=bytes` pairs,
///   e.g. `/analyze=4194304,/vaults=1048576`.
///
/// Requests whose `Content-Length` exceeds the matched route's limit are
/// rejected with `413 Payload Too Large` before reaching the handler, closing
/// the oversized-body denial-of-service vector without per-route code changes.
#[derive(Debug, Clone)]
pub struct BodySizePolicy {
    default_max: usize,
    per_route: HashMap<String, usize>,
}

impl BodySizePolicy {
    /// Create a policy with a global default and no per-route overrides.
    pub fn new(default_max: usize) -> Self {
        Self {
            default_max,
            per_route: HashMap::new(),
        }
    }

    /// Build the policy from `PERIGEE_MAX_BODY_BYTES` and
    /// `PERIGEE_MAX_BODY_BYTES_PER_ROUTE` (falling back to sane defaults).
    pub fn from_env() -> Self {
        let default_max = std::env::var("PERIGEE_MAX_BODY_BYTES")
            .ok()
            .and_then(|v| v.trim().parse::<usize>().ok())
            .filter(|n| *n > 0)
            .unwrap_or(DEFAULT_MAX_BODY_BYTES);

        let mut policy = Self::new(default_max);
        if let Ok(overrides) = std::env::var("PERIGEE_MAX_BODY_BYTES_PER_ROUTE") {
            for pair in overrides.split(',') {
                if let Some((route, size)) = parse_size_pair(pair) {
                    policy.per_route.insert(route, size);
                }
            }
        }
        policy
    }

    /// The global default body-size limit, in bytes.
    pub fn default_max(&self) -> usize {
        self.default_max
    }

    /// The byte limit that applies to `route` (per-route override, if any).
    pub fn limit_for(&self, route: &str) -> usize {
        self.per_route
            .get(route)
            .copied()
            .unwrap_or(self.default_max)
    }
}

/// Parse a single `route=bytes` override; malformed pairs are skipped.
fn parse_size_pair(pair: &str) -> Option<(String, usize)> {
    let (route, size) = pair.split_once('=')?;
    let size = size.trim().parse::<usize>().ok()?;
    if size == 0 {
        return None;
    }
    Some((route.trim().to_string(), size))
}

/// Tower [`Layer`] enforcing [`BodySizePolicy`] per matched route.
#[derive(Clone)]
pub struct BodySizeLimitLayer {
    policy: Arc<BodySizePolicy>,
}

impl BodySizeLimitLayer {
    pub fn new(policy: Arc<BodySizePolicy>) -> Self {
        Self { policy }
    }
}

impl<S> Layer<S> for BodySizeLimitLayer {
    type Service = BodySizeLimitMiddleware<S>;

    fn layer(&self, inner: S) -> Self::Service {
        BodySizeLimitMiddleware {
            inner,
            policy: Arc::clone(&self.policy),
        }
    }
}

/// Tower [`Service`] that short-circuits oversized POST/PUT bodies with `413`.
#[derive(Clone)]
pub struct BodySizeLimitMiddleware<S> {
    inner: S,
    policy: Arc<BodySizePolicy>,
}

impl<S> Service<Request<Body>> for BodySizeLimitMiddleware<S>
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

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        // Only POST/PUT endpoints carry the request bodies CORE-20 polices.
        if matches!(req.method().as_str(), "POST" | "PUT") {
            let route = matched_route(&req);
            let limit = self.policy.limit_for(&route);

            if let Some(len) = content_length(&req) {
                if len > limit {
                    let response = payload_too_large(limit);
                    return Box::pin(async move { Ok(response) });
                }
            }
        }

        let mut inner = self.inner.clone();
        Box::pin(async move { inner.call(req).await })
    }
}

/// The matched-route pattern (`/analyze/:id`) or the raw path when unmatched.
fn matched_route(req: &Request<Body>) -> String {
    req.extensions()
        .get::<axum::extract::MatchedPath>()
        .map(|mp| mp.as_str().to_string())
        .unwrap_or_else(|| req.uri().path().to_string())
}

/// The declared `Content-Length`, when the request carries one.
fn content_length(req: &Request<Body>) -> Option<usize> {
    req.headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<usize>().ok())
}

/// Standard `413 Payload Too Large` JSON envelope, mirroring the repository's
/// `{ "error", "message" }` error shape.
fn payload_too_large(limit: usize) -> Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;

    let body = Json(serde_json::json!({
        "code": ErrorCode::PayloadTooLarge.as_str(),
        "error": "PAYLOAD_TOO_LARGE",
        "message": format!(
            "Request body exceeds the {limit} byte limit for this route"
        ),
        "limit": limit,
    }));
    (StatusCode::PAYLOAD_TOO_LARGE, body).into_response()
}

// ── CORS configuration module (CORE-21) ─────────────────────────────────────

/// Per-origin CORS allowlist with per-route overrides (CORE-21).
///
/// Configured from the environment:
/// - `CORS_ALLOWED_ORIGINS`   — comma-separated exact origins, e.g.
///   `https://app.example.com,http://localhost:5173`.
/// - `CORS_ALLOWED_METHODS`   — comma-separated methods
///   (default `GET,HEAD,POST,PUT,PATCH,DELETE,OPTIONS`).
/// - `CORS_PER_ROUTE_ORIGINS` — comma-separated `route=origins` pairs where the
///   origins value is a pipe-separated sub-list, e.g.
///   `/analyze=https://a.example.com|https://b.example.com`.
/// - `CORS_ALLOW_CREDENTIALS` — `true`/`1` enables credential-bearing requests.
///
/// Unlike the repository's single global [`CorsLayer`], this module supports
/// distinct per-route policies, so public endpoints can stay permissive while
/// sensitive routes allow only their own frontend.
#[derive(Debug, Clone)]
pub struct CorsConfig {
    pub allowed_origins: Vec<String>,
    pub allowed_methods: Vec<Method>,
    pub allow_credentials: bool,
    pub per_route_origins: HashMap<String, Vec<String>>,
}

impl CorsConfig {
    /// Create a config from an explicit allowlist.
    pub fn new(allowed_origins: Vec<String>, allow_credentials: bool) -> Self {
        Self {
            allowed_origins,
            allowed_methods: vec![
                Method::GET,
                Method::HEAD,
                Method::POST,
                Method::PUT,
                Method::PATCH,
                Method::DELETE,
                Method::OPTIONS,
            ],
            allow_credentials,
            per_route_origins: HashMap::new(),
        }
    }

    /// Build the config from the `CORS_*` environment variables.
    pub fn from_env() -> Self {
        let allowed_origins = split_csv(&std::env::var("CORS_ALLOWED_ORIGINS").unwrap_or_default());

        let allowed_methods: Vec<Method> = {
            let raw = std::env::var("CORS_ALLOWED_METHODS").unwrap_or_default();
            let parsed = split_csv(&raw)
                .into_iter()
                .filter_map(|m| Method::from_bytes(m.as_bytes()).ok())
                .collect::<Vec<_>>();
            if parsed.is_empty() {
                Self::new(Vec::new(), false).allowed_methods
            } else {
                parsed
            }
        };

        let allow_credentials = matches!(
            std::env::var("CORS_ALLOW_CREDENTIALS")
                .unwrap_or_default()
                .trim()
                .to_lowercase()
                .as_str(),
            "true" | "1"
        );

        let mut per_route_origins = HashMap::new();
        if let Ok(pairs) = std::env::var("CORS_PER_ROUTE_ORIGINS") {
            for pair in pairs.split(',') {
                if let Some((route, origins)) = pair.split_once('=') {
                    let list = split_csv(&origins.replace('|', ","));
                    if !list.is_empty() {
                        per_route_origins.insert(route.trim().to_string(), list);
                    }
                }
            }
        }

        Self {
            allowed_origins,
            allowed_methods,
            allow_credentials,
            per_route_origins,
        }
    }

    /// The origin allowlist that applies to `route`.
    pub fn origins_for(&self, route: &str) -> &[String] {
        self.per_route_origins
            .get(route)
            .map(|v| v.as_slice())
            .unwrap_or_else(|| self.allowed_origins.as_slice())
    }

    /// Whether exact `origin` is allowed for `route`.
    pub fn allows_origin(&self, origin: &str, route: &str) -> bool {
        self.origins_for(route).iter().any(|o| o == origin)
    }

    /// Build a [`CorsLayer`] for `route`, honouring any per-route allowlist.
    pub fn to_cors_layer(&self, route: &str) -> CorsLayer {
        let origin_values: Vec<HeaderValue> = self
            .origins_for(route)
            .iter()
            .filter_map(|o| HeaderValue::from_str(o).ok())
            .collect();

        let layer = if origin_values.is_empty() {
            CorsLayer::new().allow_origin(AllowOrigin::any())
        } else {
            CorsLayer::new().allow_origin(AllowOrigin::list(origin_values))
        };

        layer
            .allow_methods(AllowMethods::list(self.allowed_methods.clone()))
            .allow_headers(AllowHeaders::list(vec![
                header::CONTENT_TYPE,
                header::AUTHORIZATION,
                header::HeaderName::from_static("x-request-id"),
                header::HeaderName::from_static("x-correlation-id"),
                header::HeaderName::from_static("x-api-key"),
            ]))
            .expose_headers([
                header::HeaderName::from_static(RECEIPT_HEADER),
                header::HeaderName::from_static(RECEIPT_ID_HEADER),
            ])
            .allow_credentials(self.allow_credentials)
    }
}

/// Split a comma-separated value into trimmed, non-empty items.
fn split_csv(value: &str) -> Vec<String> {
    value
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect()
}

// ── API Versioning Middleware ──────────────────────────────────────────────────

pub const DEFAULT_API_VERSION: &str = "v1";
pub const SUPPORTED_API_VERSIONS: &[&str] = &["v1", "1"];
pub const API_VERSION_HEADER: &str = "x-api-version";
pub const ACCEPT_VERSION_HEADER: &str = "accept-version";
pub const ALT_API_VERSION_HEADER: &str = "api-version";
pub const VENDOR_MEDIA_TYPE: &str = "application/vnd.perigee.v1+json";

#[derive(Debug, Clone, PartialEq, Eq)]
struct AcceptEntry {
    media_type: String,
    version: Option<String>,
    quality: u16,
    vendor: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AcceptNegotiation {
    vendor: bool,
}

fn normalize_version(value: &str) -> Option<String> {
    let value = value
        .trim()
        .trim_matches('"')
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    if value.is_empty() {
        return None;
    }
    if value == "1" {
        return Some("v1".to_string());
    }
    if let Some(number) = value.strip_prefix('v') {
        if !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit()) {
            return Some(value);
        }
    }
    Some(value)
}

fn is_supported_version(value: &str) -> bool {
    normalize_version(value).as_deref() == Some("v1")
}

fn path_version(path: &str) -> Option<String> {
    let first = path.split('/').find(|segment| !segment.is_empty())?;
    let normalized = first.to_ascii_lowercase();
    let suffix = normalized.strip_prefix('v')?;
    if suffix.is_empty() || !suffix.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    Some(normalized)
}

fn header_value(headers: &axum::http::HeaderMap, name: &str) -> Option<String> {
    let values: Vec<String> = headers
        .get_all(name)
        .iter()
        .filter_map(|value| value.to_str().ok())
        .map(str::to_string)
        .collect();
    (!values.is_empty()).then(|| values.join(","))
}

fn explicit_version(headers: &axum::http::HeaderMap) -> Option<String> {
    header_value(headers, API_VERSION_HEADER)
        .or_else(|| header_value(headers, ACCEPT_VERSION_HEADER))
        .or_else(|| header_value(headers, ALT_API_VERSION_HEADER))
        .map(|value| value.split(',').next().unwrap_or_default().to_string())
}

fn parse_quality(value: &str) -> u16 {
    let value = value.trim().trim_matches('"');
    match value.parse::<f64>().ok() {
        Some(parsed) if parsed.is_finite() && (0.0..=1.0).contains(&parsed) => {
            parsed.mul_add(1_000.0, 0.0) as u16
        }
        _ => 0,
    }
}

fn vendor_version(media_type: &str) -> Option<String> {
    let suffix = media_type.strip_prefix("application/vnd.perigee.")?;
    let token = suffix.split('+').next()?.trim();
    normalize_version(token)
}

fn parse_accept(value: &str) -> Vec<AcceptEntry> {
    value
        .split(',')
        .filter_map(|raw_entry| {
            let mut parts = raw_entry.split(';');
            let media_type = parts.next()?.trim().to_ascii_lowercase();
            if media_type.is_empty() {
                return None;
            }
            let mut version = None;
            let mut quality = 1_000u16;
            for parameter in parts {
                let Some((name, parameter_value)) = parameter.split_once('=') else {
                    continue;
                };
                match name.trim().to_ascii_lowercase().as_str() {
                    "q" => quality = parse_quality(parameter_value),
                    "version" => version = normalize_version(parameter_value),
                    _ => {}
                }
            }
            let vendor = media_type.starts_with("application/vnd.perigee");
            if let Some(media_version) = vendor_version(&media_type) {
                version = Some(match version {
                    Some(parameter_version) if parameter_version != media_version => {
                        "__conflict__".to_string()
                    }
                    _ => media_version,
                });
            } else if media_type == "application/vnd.perigee+json" && version.is_none() {
                version = Some("v1".to_string());
            }
            Some(AcceptEntry {
                media_type,
                version,
                quality,
                vendor,
            })
        })
        .collect()
}

fn is_json_media_type(media_type: &str) -> bool {
    matches!(
        media_type,
        "application/json"
            | "text/json"
            | "application/*"
            | "application/*+json"
            | "*/*"
            | "*"
    )
}

fn negotiate_accept(
    value: &str,
    required_version: Option<&str>,
) -> Result<AcceptNegotiation, ()> {
    let entries = parse_accept(value);
    let mut best: Option<(u16, bool)> = None;
    for entry in entries {
        if entry.quality == 0 {
            continue;
        }
        let compatible = if let Some(version) = entry.version.as_deref() {
            is_supported_version(version)
                && required_version
                    .map(|required| normalize_version(required).as_deref() == Some(version))
                    .unwrap_or(true)
        } else {
            !entry.vendor && is_json_media_type(&entry.media_type)
        };
        if !compatible {
            continue;
        }
        let candidate = (entry.quality, entry.vendor);
        if best.map(|current| candidate > current).unwrap_or(true) {
            best = Some(candidate);
        }
    }
    best.map(|(_, vendor)| AcceptNegotiation { vendor }).ok_or(())
}

fn version_error_response(
    status: axum::http::StatusCode,
    error: &str,
    message: String,
) -> Response {
    let body = axum::Json(serde_json::json!({
        "code": error,
        "error": error,
        "message": message,
    }));
    let mut response = (status, body).into_response();
    response.headers_mut().insert(
        HeaderName::from_static(API_VERSION_HEADER),
        HeaderValue::from_static("v1"),
    );
    response.headers_mut().append(
        header::VARY,
        HeaderValue::from_static("Accept"),
    );
    response.headers_mut().append(
        header::VARY,
        HeaderValue::from_static(API_VERSION_HEADER),
    );
    response.headers_mut().append(
        header::VARY,
        HeaderValue::from_static(ACCEPT_VERSION_HEADER),
    );
    response.headers_mut().append(
        header::VARY,
        HeaderValue::from_static(ALT_API_VERSION_HEADER),
    );
    response
}

pub async fn api_version_middleware(request: Request, next: Next) -> Response {
    use axum::response::IntoResponse;

    let path = request.uri().path().to_string();
    let requested_from_path = path_version(&path);
    let requested_version = requested_from_path.clone().or_else(|| explicit_version(request.headers()));
    let version = requested_version
        .clone()
        .unwrap_or_else(|| DEFAULT_API_VERSION.to_string());

    if !is_supported_version(&version) {
    if !is_supported {
        let safe_version = version.chars().take(64).collect::<String>();
        tracing::warn!(
            version = %safe_version,
            path = %path,
            "Unsupported API version requested"
        );
        return version_error_response(
            axum::http::StatusCode::NOT_ACCEPTABLE,
            "UNSUPPORTED_API_VERSION",
            format!(

        let body = Json(serde_json::json!({
            "code": ErrorCode::UnsupportedApiVersion.as_str(),
            "error": ErrorCode::UnsupportedApiVersion.as_str(),
            "message": format!(
                "API version '{}' is not supported. Supported versions: {}",
                version,
                SUPPORTED_API_VERSIONS.join(", ")
            ),
        );
    }

    let accept = header_value(request.headers(), header::ACCEPT.as_str());
    let required_version = requested_version.as_deref();
    let negotiation = match accept.as_deref() {
        Some(value) => match negotiate_accept(value, required_version) {
            Ok(negotiation) => Some(negotiation),
            Err(()) => {
                return version_error_response(
                    axum::http::StatusCode::NOT_ACCEPTABLE,
                    "NOT_ACCEPTABLE",
                    "The requested API representation is not supported".to_string(),
                );
            }
        },
        None => None,
    };

    let mut response = next.run(request).await;
    response.headers_mut().insert(
        HeaderName::from_static(API_VERSION_HEADER),
        HeaderValue::from_static("v1"),
    );
    response.headers_mut().append(header::VARY, HeaderValue::from_static("Accept"));
    response.headers_mut().append(
        header::VARY,
        HeaderValue::from_static(API_VERSION_HEADER),
    );
    response.headers_mut().append(
        header::VARY,
        HeaderValue::from_static(ACCEPT_VERSION_HEADER),
    );
    response.headers_mut().append(
        header::VARY,
        HeaderValue::from_static(ALT_API_VERSION_HEADER),
    );
    if negotiation.map(|value| value.vendor).unwrap_or(false) {
        response.headers_mut().insert(
            header::CONTENT_TYPE,
            HeaderValue::from_static(VENDOR_MEDIA_TYPE),
        );
    }
    response
}

#[cfg(test)]
mod version_middleware_tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode},
        routing::get,
        Router,
    };
    use tower::ServiceExt;

    async fn test_app() -> Router {
        Router::new()
            .route("/health", get(|| async { "ok" }))
            .nest("/v1", Router::new().route("/health", get(|| async { "ok" })))
            .layer(axum::middleware::from_fn(api_version_middleware))
    }

    #[tokio::test]
    async fn test_default_version_header() {
        let app = test_app().await;
        let req = Request::builder().uri("/health").body(Body::empty()).unwrap();
        let res = app.oneshot(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get("x-api-version").unwrap().to_str().unwrap(),
            "v1"
        );
    }

    #[tokio::test]
    async fn test_uri_v1_version() {
        let app = test_app().await;
        let req = Request::builder().uri("/v1/health").body(Body::empty()).unwrap();
        let res = app.oneshot(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get("x-api-version").unwrap().to_str().unwrap(),
            "v1"
        );
    }

    #[tokio::test]
    async fn test_header_v1_version() {
        let app = test_app().await;
        let req = Request::builder()
            .uri("/health")
            .header("x-api-version", "v1")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get("x-api-version").unwrap().to_str().unwrap(),
            "v1"
        );
    }

    #[tokio::test]
    async fn test_unsupported_uri_version() {
        let app = test_app().await;
        let req = Request::builder().uri("/v2/health").body(Body::empty()).unwrap();
        let res = app.oneshot(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::NOT_ACCEPTABLE);
        assert_eq!(
            res.headers().get("x-api-version").unwrap().to_str().unwrap(),
            "v1"
        );
    }

    #[tokio::test]
    async fn test_unsupported_header_version() {
        let app = test_app().await;
        let req = Request::builder()
            .uri("/health")
            .header("x-api-version", "v2")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::NOT_ACCEPTABLE);
        assert_eq!(
            res.headers().get("x-api-version").unwrap().to_str().unwrap(),
            "v1"
        );
    }

    #[tokio::test]
    async fn test_vendor_accept_is_negotiated() {
        let app = test_app().await;
        let req = Request::builder()
            .uri("/health")
            .header("accept", "application/vnd.perigee.v1+json")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::OK);
        assert_eq!(
            res.headers().get("content-type").unwrap().to_str().unwrap(),
            VENDOR_MEDIA_TYPE
        );
    }

    #[tokio::test]
    async fn test_unsupported_vendor_accept_returns_406() {
        let app = test_app().await;
        let req = Request::builder()
            .uri("/health")
            .header("accept", "application/vnd.perigee.v2+json")
            .body(Body::empty())
            .unwrap();
        let res = app.oneshot(req).await.unwrap();

        assert_eq!(res.status(), StatusCode::NOT_ACCEPTABLE);
    }
}

#[cfg(test)]
mod body_size_and_cors_tests {
    use super::*;

    #[test]
    fn size_pairs_parse_and_skip_malformed() {
        assert_eq!(
            parse_size_pair("/analyze=4194304"),
            Some(("/analyze".to_string(), 4_194_304))
        );
        assert_eq!(parse_size_pair("=/0"), None);
        assert_eq!(parse_size_pair("nonsense"), None);
    }

    #[test]
    fn body_size_policy_defaults_and_overrides() {
        let mut policy = BodySizePolicy::new(1_048_576);
        policy.per_route.insert("/vaults".to_string(), 512);

        assert_eq!(policy.default_max(), 1_048_576);
        assert_eq!(policy.limit_for("/analyze"), 1_048_576);
        assert_eq!(policy.limit_for("/vaults"), 512);
    }

    #[test]
    fn content_length_is_read_from_headers() {
        let req = Request::builder()
            .uri("/analyze")
            .header(header::CONTENT_LENGTH, "12345")
            .body(Body::empty())
            .unwrap();

        assert_eq!(content_length(&req), Some(12_345));

        let no_len = Request::builder()
            .uri("/analyze")
            .body(Body::empty())
            .unwrap();
        assert_eq!(content_length(&no_len), None);
    }

    #[test]
    fn payload_too_large_returns_413_envelope() {
        let res = payload_too_large(8192);
        assert_eq!(res.status(), axum::http::StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[test]
    fn cors_allowlist_honours_per_route_overrides() {
        let mut config = CorsConfig::new(vec!["https://global.example.com".to_string()], false);
        config.per_route_origins.insert(
            "/analyze".to_string(),
            vec!["https://analyze.example.com".to_string()],
        );

        assert!(config.allows_origin("https://global.example.com", "/other"));
        assert!(!config.allows_origin("https://global.example.com", "/analyze"));
        assert!(config.allows_origin("https://analyze.example.com", "/analyze"));
        assert!(!config.allows_origin("https://evil.example.com", "/analyze"));
    }

    #[test]
    fn cors_origins_for_falls_back_to_global_allowlist() {
        let config = CorsConfig::new(vec!["https://global.example.com".to_string()], false);

        assert_eq!(config.origins_for("/unconfigured")[0], "https://global.example.com");
        // Empty global allowlist falls back to nothing (layer uses Allow-Any).
        assert!(CorsConfig::new(vec![], false).origins_for("/anything").is_empty());
    }
}
