//! Configurable per-manager API quotas (API-37).
//!
//! The public limiter in [`crate::rate_limiter`] throttles by client address,
//! which penalises shared networks and cannot tell a small integration from a
//! high-volume partner. This module adds a second, administrator-controlled
//! quota keyed to the **authenticated manager** and a bounded
//! [`OperationClass`].
//!
//! * Each `(manager, class)` pair has its own fixed-window counter, so one
//!   manager exhausting a quota never affects another.
//! * Defaults per class ([`QuotaDefaults`]) apply until an administrator sets
//!   an override with `PUT /admin/quotas/:manager`.
//! * Overrides are validated and swapped atomically; a change applies to the
//!   very next request without resetting usage already counted in the current
//!   window (see `docs/MANAGER_QUOTAS.md`).
//! * Every authenticated response carries `X-Quota-*` headers and a `429`
//!   carries `Retry-After`, so clients can back off precisely.
//! * Quota identifiers are a truncated SHA-256 of the manager's public
//!   address. Bearer tokens, API keys and other credentials are never part of
//!   a key, identifier, header or log line.

use crate::auth::AuthenticatedUser;
use crate::rest_api::{ProblemDetails, PROBLEM_CONTENT_TYPE};
use axum::{
    extract::{Path, Request},
    http::{header, HeaderMap, HeaderName, HeaderValue, Method, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard};

pub const HEADER_QUOTA_LIMIT: &str = "x-quota-limit";
pub const HEADER_QUOTA_REMAINING: &str = "x-quota-remaining";
/// Unix epoch seconds at which the current window ends.
pub const HEADER_QUOTA_RESET: &str = "x-quota-reset";
pub const HEADER_QUOTA_CLASS: &str = "x-quota-class";
/// Opaque, credential-free identifier of the quota bucket.
pub const HEADER_QUOTA_ID: &str = "x-quota-id";

/// Bounds an administrator may configure.
pub const MAX_QUOTA_LIMIT: u32 = 1_000_000;
pub const MAX_QUOTA_WINDOW_SECS: u32 = 86_400;
/// Upper bound on tracked `(manager, class)` windows before expired ones are pruned.
pub const MAX_TRACKED_WINDOWS: usize = 100_000;
const MAX_MANAGER_ID_LEN: usize = 128;

/// The bounded set of operation classes a quota can target.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum OperationClass {
    /// `GET`/`HEAD` requests.
    Read,
    /// Mutating requests (`POST`, `PUT`, `PATCH`, `DELETE`).
    Write,
    /// Simulation and analysis requests (`POST /analyze*`, `*/simulate*`).
    Simulation,
}

impl OperationClass {
    pub const ALL: [OperationClass; 3] = [Self::Read, Self::Write, Self::Simulation];

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Simulation => "simulation",
        }
    }

    /// Classify a request by method and path.
    pub fn classify(method: &Method, path: &str) -> Self {
        let path = path.strip_prefix("/v1").unwrap_or(path);
        if *method == Method::GET || *method == Method::HEAD || *method == Method::OPTIONS {
            return Self::Read;
        }
        if path.starts_with("/analyze") || path.contains("/simulate") {
            return Self::Simulation;
        }
        Self::Write
    }
}

/// `limit` requests per `window_secs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct QuotaPolicy {
    pub limit: u32,
    pub window_secs: u32,
}

impl QuotaPolicy {
    pub fn new(limit: u32, window_secs: u32) -> Result<Self, QuotaError> {
        let policy = Self { limit, window_secs };
        policy.validate()?;
        Ok(policy)
    }

    pub fn validate(&self) -> Result<(), QuotaError> {
        if self.limit == 0 || self.limit > MAX_QUOTA_LIMIT {
            return Err(QuotaError::InvalidPolicy(format!(
                "limit must be between 1 and {MAX_QUOTA_LIMIT}"
            )));
        }
        if self.window_secs == 0 || self.window_secs > MAX_QUOTA_WINDOW_SECS {
            return Err(QuotaError::InvalidPolicy(format!(
                "window_secs must be between 1 and {MAX_QUOTA_WINDOW_SECS}"
            )));
        }
        Ok(())
    }

    /// Parse `"<limit>/<window_secs>"`, e.g. `"600/60"`.
    pub fn parse(raw: &str) -> Result<Self, QuotaError> {
        let (limit, window) = raw.trim().split_once('/').ok_or_else(|| {
            QuotaError::InvalidPolicy(format!("`{raw}` is not <limit>/<window_secs>"))
        })?;
        let limit = limit
            .trim()
            .parse()
            .map_err(|_| QuotaError::InvalidPolicy(format!("`{raw}` has a non-numeric limit")))?;
        let window_secs = window
            .trim()
            .parse()
            .map_err(|_| QuotaError::InvalidPolicy(format!("`{raw}` has a non-numeric window")))?;
        Self::new(limit, window_secs)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum QuotaError {
    InvalidPolicy(String),
    InvalidManager(String),
}

impl std::fmt::Display for QuotaError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidPolicy(msg) | Self::InvalidManager(msg) => f.write_str(msg),
        }
    }
}

impl std::error::Error for QuotaError {}

/// Default policy for each class, used when a manager has no override.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct QuotaDefaults {
    pub read: QuotaPolicy,
    pub write: QuotaPolicy,
    pub simulation: QuotaPolicy,
}

impl Default for QuotaDefaults {
    fn default() -> Self {
        Self {
            read: QuotaPolicy {
                limit: 600,
                window_secs: 60,
            },
            write: QuotaPolicy {
                limit: 120,
                window_secs: 60,
            },
            simulation: QuotaPolicy {
                limit: 30,
                window_secs: 60,
            },
        }
    }
}

impl QuotaDefaults {
    pub fn for_class(&self, class: OperationClass) -> QuotaPolicy {
        match class {
            OperationClass::Read => self.read,
            OperationClass::Write => self.write,
            OperationClass::Simulation => self.simulation,
        }
    }

    /// Read `PERIGEE_QUOTA_DEFAULT_{READ,WRITE,SIMULATION}` (`"<limit>/<window_secs>"`).
    /// Invalid values are logged and the built-in default is kept.
    pub fn from_env() -> Self {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup(lookup: impl Fn(&str) -> Option<String>) -> Self {
        let mut defaults = Self::default();
        for (key, slot) in [
            ("PERIGEE_QUOTA_DEFAULT_READ", &mut defaults.read),
            ("PERIGEE_QUOTA_DEFAULT_WRITE", &mut defaults.write),
            ("PERIGEE_QUOTA_DEFAULT_SIMULATION", &mut defaults.simulation),
        ] {
            if let Some(raw) = lookup(key) {
                match QuotaPolicy::parse(&raw) {
                    Ok(policy) => *slot = policy,
                    Err(error) => {
                        tracing::warn!(variable = key, error = %error, "ignoring invalid quota default")
                    }
                }
            }
        }
        defaults
    }
}

/// Outcome of one quota check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuotaDecision {
    pub allowed: bool,
    pub class: OperationClass,
    pub quota_id: String,
    pub limit: u32,
    pub remaining: u32,
    /// Unix epoch seconds at which the current window ends.
    pub reset_at: u64,
    /// Seconds until a retry can succeed; `0` when allowed.
    pub retry_after_secs: u64,
}

impl QuotaDecision {
    /// Headers that let a client pace itself.
    pub fn headers(&self) -> Vec<(&'static str, String)> {
        let mut headers = vec![
            (HEADER_QUOTA_LIMIT, self.limit.to_string()),
            (HEADER_QUOTA_REMAINING, self.remaining.to_string()),
            (HEADER_QUOTA_RESET, self.reset_at.to_string()),
            (HEADER_QUOTA_CLASS, self.class.as_str().to_string()),
            (HEADER_QUOTA_ID, self.quota_id.clone()),
        ];
        if !self.allowed {
            headers.push(("retry-after", self.retry_after_secs.max(1).to_string()));
        }
        headers
    }

    pub fn apply_headers(&self, target: &mut HeaderMap) {
        for (name, value) in self.headers() {
            if let Ok(value) = HeaderValue::from_str(&value) {
                target.insert(HeaderName::from_static(name), value);
            }
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Window {
    start: u64,
    used: u32,
}

#[derive(Default)]
struct Inner {
    overrides: HashMap<(String, OperationClass), QuotaPolicy>,
    windows: HashMap<(String, OperationClass), Window>,
}

/// Effective quota for one class of a manager, as reported to administrators.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct EffectiveQuota {
    pub class: OperationClass,
    pub limit: u32,
    pub window_secs: u32,
    /// `true` when an administrator override is in force.
    pub overridden: bool,
}

/// Process-wide per-manager quota state.
pub struct ManagerQuotaRegistry {
    defaults: QuotaDefaults,
    inner: Mutex<Inner>,
}

impl Default for ManagerQuotaRegistry {
    fn default() -> Self {
        Self::new(QuotaDefaults::default())
    }
}

/// Stable, credential-free key for a manager's public address.
pub fn manager_key(manager_id: &str) -> String {
    hex::encode(Sha256::digest(manager_id.trim().as_bytes()))[..16].to_string()
}

fn validate_manager_id(manager_id: &str) -> Result<&str, QuotaError> {
    let trimmed = manager_id.trim();
    if trimmed.is_empty() || trimmed.len() > MAX_MANAGER_ID_LEN {
        return Err(QuotaError::InvalidManager(format!(
            "manager id must be 1-{MAX_MANAGER_ID_LEN} characters"
        )));
    }
    Ok(trimmed)
}

impl ManagerQuotaRegistry {
    pub fn new(defaults: QuotaDefaults) -> Self {
        Self {
            defaults,
            inner: Mutex::new(Inner::default()),
        }
    }

    pub fn from_env() -> Self {
        Self::new(QuotaDefaults::from_env())
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn policy_in(&self, inner: &Inner, key: &str, class: OperationClass) -> (QuotaPolicy, bool) {
        match inner.overrides.get(&(key.to_string(), class)) {
            Some(policy) => (*policy, true),
            None => (self.defaults.for_class(class), false),
        }
    }

    /// Effective policies for every class of `manager_id`.
    pub fn effective(&self, manager_id: &str) -> Result<Vec<EffectiveQuota>, QuotaError> {
        let key = manager_key(validate_manager_id(manager_id)?);
        let inner = self.lock();
        Ok(OperationClass::ALL
            .iter()
            .map(|class| {
                let (policy, overridden) = self.policy_in(&inner, &key, *class);
                EffectiveQuota {
                    class: *class,
                    limit: policy.limit,
                    window_secs: policy.window_secs,
                    overridden,
                }
            })
            .collect())
    }

    /// Install or replace an override. Invalid policies leave the old one in place.
    pub fn set_override(
        &self,
        manager_id: &str,
        class: OperationClass,
        policy: QuotaPolicy,
    ) -> Result<(), QuotaError> {
        policy.validate()?;
        let key = manager_key(validate_manager_id(manager_id)?);
        self.lock().overrides.insert((key, class), policy);
        Ok(())
    }

    /// Remove an override so the class falls back to its default.
    pub fn clear_override(
        &self,
        manager_id: &str,
        class: OperationClass,
    ) -> Result<bool, QuotaError> {
        let key = manager_key(validate_manager_id(manager_id)?);
        Ok(self.lock().overrides.remove(&(key, class)).is_some())
    }

    /// Count one request against the manager's quota for `class`.
    pub fn check(&self, manager_id: &str, class: OperationClass) -> QuotaDecision {
        let now = u64::try_from(Utc::now().timestamp()).unwrap_or(0);
        self.check_at(manager_id, class, now)
    }

    /// [`Self::check`] with an explicit clock (epoch seconds), for tests.
    pub fn check_at(&self, manager_id: &str, class: OperationClass, now: u64) -> QuotaDecision {
        let key = manager_key(manager_id);
        let quota_id = format!("q_{key}_{}", class.as_str());
        let mut inner = self.lock();
        let (policy, _) = self.policy_in(&inner, &key, class);
        let window_len = u64::from(policy.window_secs);

        if inner.windows.len() >= MAX_TRACKED_WINDOWS
            && !inner.windows.contains_key(&(key.clone(), class))
        {
            // Drop windows that have certainly expired under the longest allowed window.
            let horizon = u64::from(MAX_QUOTA_WINDOW_SECS);
            inner
                .windows
                .retain(|_, w| w.start.saturating_add(horizon) > now);
        }

        let window = inner.windows.entry((key, class)).or_insert(Window {
            start: now,
            used: 0,
        });
        // The window length always comes from the *current* policy, so a
        // shortened window takes effect immediately and a lengthened one
        // extends the current window rather than granting a fresh one.
        if now >= window.start.saturating_add(window_len) {
            *window = Window {
                start: now,
                used: 0,
            };
        }
        let reset_at = window.start.saturating_add(window_len);

        if window.used < policy.limit {
            window.used += 1;
            QuotaDecision {
                allowed: true,
                class,
                quota_id,
                limit: policy.limit,
                remaining: policy.limit - window.used,
                reset_at,
                retry_after_secs: 0,
            }
        } else {
            QuotaDecision {
                allowed: false,
                class,
                quota_id,
                limit: policy.limit,
                remaining: 0,
                reset_at,
                retry_after_secs: reset_at.saturating_sub(now).max(1),
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn problem(status: StatusCode, detail: &str, instance: &str) -> Response {
    let body = ProblemDetails::from_status(status.as_u16(), detail)
        .with_instance(instance)
        .to_json();
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
    );
    response
}

/// Enforces the caller's quota. Must run *inside* the auth middleware so the
/// [`AuthenticatedUser`] extension is present; unauthenticated requests pass
/// through untouched (the public per-IP limiter covers them). Administrators
/// are exempt so they can always adjust quotas.
pub async fn manager_quota_middleware(
    Extension(registry): Extension<Arc<ManagerQuotaRegistry>>,
    request: Request,
    next: Next,
) -> Response {
    let Some(user) = request.extensions().get::<AuthenticatedUser>().cloned() else {
        return next.run(request).await;
    };
    if user.is_admin() {
        return next.run(request).await;
    }

    let class = OperationClass::classify(request.method(), request.uri().path());
    let decision = registry.check(&user.stellar_address, class);
    if !decision.allowed {
        tracing::info!(
            quota_id = %decision.quota_id,
            class = class.as_str(),
            retry_after_secs = decision.retry_after_secs,
            "manager quota exhausted"
        );
        let path = request.uri().path().to_string();
        let mut response = problem(
            StatusCode::TOO_MANY_REQUESTS,
            &format!(
                "The {} quota for this manager is exhausted; retry after {} seconds",
                class.as_str(),
                decision.retry_after_secs
            ),
            &path,
        );
        decision.apply_headers(response.headers_mut());
        return response;
    }

    let mut response = next.run(request).await;
    decision.apply_headers(response.headers_mut());
    response
}

/// Body of `PUT /admin/quotas/:manager`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SetQuotaRequest {
    pub class: OperationClass,
    pub limit: u32,
    pub window_secs: u32,
}

#[derive(Debug, Serialize)]
pub struct QuotaListResponse {
    /// Credential-free identifier prefix shared by this manager's buckets.
    pub quota_key: String,
    pub quotas: Vec<EffectiveQuota>,
}

/// `Some(403 response)` when the caller is not an administrator.
fn deny_non_admin(user: &AuthenticatedUser, instance: &str) -> Option<Response> {
    (!user.is_admin()).then(|| {
        problem(
            StatusCode::FORBIDDEN,
            "Managing quotas requires the administrator permission",
            instance,
        )
    })
}

fn list_response(registry: &ManagerQuotaRegistry, manager: &str) -> Response {
    match registry.effective(manager) {
        Ok(quotas) => Json(QuotaListResponse {
            quota_key: format!("q_{}", manager_key(manager)),
            quotas,
        })
        .into_response(),
        Err(error) => problem(StatusCode::BAD_REQUEST, &error.to_string(), "/admin/quotas"),
    }
}

/// `GET /admin/quotas/:manager` – effective policies for a manager.
pub async fn get_manager_quotas_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(registry): Extension<Arc<ManagerQuotaRegistry>>,
    Path(manager): Path<String>,
) -> Response {
    if let Some(response) = deny_non_admin(&user, "/admin/quotas") {
        return response;
    }
    list_response(&registry, &manager)
}

/// `PUT /admin/quotas/:manager` – set one class override.
pub async fn set_manager_quota_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(registry): Extension<Arc<ManagerQuotaRegistry>>,
    Path(manager): Path<String>,
    Json(body): Json<SetQuotaRequest>,
) -> Response {
    if let Some(response) = deny_non_admin(&user, "/admin/quotas") {
        return response;
    }
    let policy = QuotaPolicy {
        limit: body.limit,
        window_secs: body.window_secs,
    };
    if let Err(error) = registry.set_override(&manager, body.class, policy) {
        return problem(
            StatusCode::UNPROCESSABLE_ENTITY,
            &error.to_string(),
            "/admin/quotas",
        );
    }
    crate::audit_log::log_audit_event(
        &manager,
        &format!(
            "quota.set class={} limit={} window_secs={}",
            body.class.as_str(),
            body.limit,
            body.window_secs
        ),
        &user.stellar_address,
    );
    list_response(&registry, &manager)
}

/// `DELETE /admin/quotas/:manager/:class` – revert one class to its default.
pub async fn clear_manager_quota_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(registry): Extension<Arc<ManagerQuotaRegistry>>,
    Path((manager, class)): Path<(String, OperationClass)>,
) -> Response {
    if let Some(response) = deny_non_admin(&user, "/admin/quotas") {
        return response;
    }
    if let Err(error) = registry.clear_override(&manager, class) {
        return problem(StatusCode::BAD_REQUEST, &error.to_string(), "/admin/quotas");
    }
    crate::audit_log::log_audit_event(
        &manager,
        &format!("quota.cleared class={}", class.as_str()),
        &user.stellar_address,
    );
    list_response(&registry, &manager)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use axum::{body::Body, http::Request as HttpRequest, routing::get, Router};
    use tower::ServiceExt;

    const ALICE: &str = "GALICEMANAGERADDRESS";
    const BOB: &str = "GBOBMANAGERADDRESS";

    fn registry(read: u32, write: u32) -> ManagerQuotaRegistry {
        ManagerQuotaRegistry::new(QuotaDefaults {
            read: QuotaPolicy {
                limit: read,
                window_secs: 60,
            },
            write: QuotaPolicy {
                limit: write,
                window_secs: 60,
            },
            simulation: QuotaPolicy {
                limit: 1,
                window_secs: 60,
            },
        })
    }

    #[test]
    fn requests_are_classified_into_bounded_classes() {
        assert_eq!(
            OperationClass::classify(&Method::GET, "/vaults"),
            OperationClass::Read
        );
        assert_eq!(
            OperationClass::classify(&Method::POST, "/v1/analyze/wasm"),
            OperationClass::Simulation
        );
        assert_eq!(
            OperationClass::classify(&Method::POST, "/vaults/v1/simulate"),
            OperationClass::Simulation
        );
        assert_eq!(
            OperationClass::classify(&Method::PATCH, "/vaults/v1"),
            OperationClass::Write
        );
        assert_eq!(
            OperationClass::classify(&Method::DELETE, "/vaults/v1"),
            OperationClass::Write
        );
    }

    #[test]
    fn manager_quotas_are_isolated() {
        let quotas = registry(2, 2);
        assert!(quotas.check_at(ALICE, OperationClass::Read, 100).allowed);
        assert!(quotas.check_at(ALICE, OperationClass::Read, 100).allowed);
        assert!(!quotas.check_at(ALICE, OperationClass::Read, 100).allowed);

        // Bob and Alice's other classes are unaffected.
        let bob = quotas.check_at(BOB, OperationClass::Read, 100);
        assert!(bob.allowed);
        assert_eq!(bob.remaining, 1);
        assert!(quotas.check_at(ALICE, OperationClass::Write, 100).allowed);
    }

    #[test]
    fn overrides_apply_per_manager() {
        let quotas = registry(1, 1);
        quotas
            .set_override(BOB, OperationClass::Read, QuotaPolicy::new(5, 60).unwrap())
            .unwrap();
        assert_eq!(quotas.check_at(BOB, OperationClass::Read, 0).limit, 5);
        assert_eq!(quotas.check_at(ALICE, OperationClass::Read, 0).limit, 1);
    }

    #[test]
    fn denied_requests_carry_correct_retry_metadata() {
        let quotas = registry(1, 1);
        let first = quotas.check_at(ALICE, OperationClass::Read, 1_000);
        assert_eq!(
            (first.allowed, first.remaining, first.reset_at),
            (true, 0, 1_060)
        );

        let denied = quotas.check_at(ALICE, OperationClass::Read, 1_045);
        assert!(!denied.allowed);
        assert_eq!(denied.reset_at, 1_060);
        assert_eq!(denied.retry_after_secs, 15);
        let headers: HashMap<_, _> = denied.headers().into_iter().collect();
        assert_eq!(headers["retry-after"], "15");
        assert_eq!(headers[HEADER_QUOTA_LIMIT], "1");
        assert_eq!(headers[HEADER_QUOTA_REMAINING], "0");
        assert_eq!(headers[HEADER_QUOTA_RESET], "1060");
        assert_eq!(headers[HEADER_QUOTA_CLASS], "read");

        // Retrying at the advertised time succeeds.
        assert!(quotas.check_at(ALICE, OperationClass::Read, 1_060).allowed);
        // Allowed responses never advertise Retry-After.
        assert!(!first
            .headers()
            .iter()
            .any(|(name, _)| *name == "retry-after"));
    }

    #[test]
    fn lowering_a_quota_mid_window_takes_effect_without_resetting_usage() {
        let quotas = registry(10, 10);
        for _ in 0..3 {
            assert!(quotas.check_at(ALICE, OperationClass::Write, 0).allowed);
        }
        quotas
            .set_override(
                ALICE,
                OperationClass::Write,
                QuotaPolicy::new(3, 60).unwrap(),
            )
            .unwrap();
        let denied = quotas.check_at(ALICE, OperationClass::Write, 10);
        assert!(
            !denied.allowed,
            "usage already counted must not be forgiven"
        );
        assert_eq!(denied.retry_after_secs, 50);

        // Raising it again admits the next request immediately.
        quotas
            .set_override(
                ALICE,
                OperationClass::Write,
                QuotaPolicy::new(4, 60).unwrap(),
            )
            .unwrap();
        assert!(quotas.check_at(ALICE, OperationClass::Write, 11).allowed);
    }

    #[test]
    fn shortening_a_window_applies_to_the_current_window() {
        let quotas = registry(1, 1);
        assert!(quotas.check_at(ALICE, OperationClass::Read, 0).allowed);
        quotas
            .set_override(
                ALICE,
                OperationClass::Read,
                QuotaPolicy::new(1, 10).unwrap(),
            )
            .unwrap();
        let denied = quotas.check_at(ALICE, OperationClass::Read, 5);
        assert_eq!(denied.retry_after_secs, 5);
        assert!(quotas.check_at(ALICE, OperationClass::Read, 10).allowed);
    }

    #[test]
    fn invalid_changes_are_rejected_and_keep_the_previous_policy() {
        let quotas = registry(1, 1);
        quotas
            .set_override(
                ALICE,
                OperationClass::Read,
                QuotaPolicy::new(7, 60).unwrap(),
            )
            .unwrap();
        for bad in [
            QuotaPolicy {
                limit: 0,
                window_secs: 60,
            },
            QuotaPolicy {
                limit: 5,
                window_secs: 0,
            },
            QuotaPolicy {
                limit: MAX_QUOTA_LIMIT + 1,
                window_secs: 60,
            },
            QuotaPolicy {
                limit: 5,
                window_secs: MAX_QUOTA_WINDOW_SECS + 1,
            },
        ] {
            assert!(quotas
                .set_override(ALICE, OperationClass::Read, bad)
                .is_err());
        }
        assert!(quotas
            .set_override("", OperationClass::Read, QuotaPolicy::new(1, 1).unwrap())
            .is_err());
        let read = &quotas.effective(ALICE).unwrap()[0];
        assert_eq!((read.limit, read.overridden), (7, true));

        assert!(quotas.clear_override(ALICE, OperationClass::Read).unwrap());
        let read = &quotas.effective(ALICE).unwrap()[0];
        assert_eq!((read.limit, read.overridden), (1, false));
    }

    #[test]
    fn defaults_parse_from_env_style_values() {
        let defaults = QuotaDefaults::from_lookup(|key| match key {
            "PERIGEE_QUOTA_DEFAULT_READ" => Some("1000/30".to_string()),
            "PERIGEE_QUOTA_DEFAULT_WRITE" => Some("nonsense".to_string()),
            _ => None,
        });
        assert_eq!(
            defaults.read,
            QuotaPolicy {
                limit: 1000,
                window_secs: 30
            }
        );
        assert_eq!(defaults.write, QuotaDefaults::default().write);
        assert_eq!(defaults.simulation, QuotaDefaults::default().simulation);
    }

    #[test]
    fn quota_identifiers_never_expose_credentials() {
        let quotas = registry(1, 1);
        let decision = quotas.check_at(ALICE, OperationClass::Read, 0);
        assert!(decision.quota_id.starts_with("q_"));
        assert!(!decision.quota_id.contains(ALICE));
        assert_eq!(decision.quota_id, format!("q_{}_read", manager_key(ALICE)));
        assert_eq!(manager_key(ALICE).len(), 16);
    }

    fn app(registry: Arc<ManagerQuotaRegistry>, user: AuthenticatedUser) -> Router {
        Router::new()
            .route(
                "/vaults",
                get(|| async { "ok" }).post(|| async { "created" }),
            )
            .route(
                "/admin/quotas/:manager",
                get(get_manager_quotas_handler).put(set_manager_quota_handler),
            )
            .route_layer(axum::middleware::from_fn(manager_quota_middleware))
            .layer(Extension(user))
            .layer(Extension(registry))
    }

    async fn send(router: Router, request: HttpRequest<Body>) -> Response {
        router.oneshot(request).await.unwrap()
    }

    #[tokio::test]
    async fn middleware_sets_headers_and_returns_429_with_retry_after() {
        let quotas = Arc::new(registry(1, 1));
        let alice = AuthenticatedUser::new(ALICE.into(), Role::Manager, vec![]);
        let bearer = "Bearer eyJhbGciOiJFZERTQSJ9.secret.token";

        let ok = send(
            app(quotas.clone(), alice.clone()),
            HttpRequest::get("/vaults")
                .header(header::AUTHORIZATION, bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(ok.status(), StatusCode::OK);
        assert_eq!(ok.headers()[HEADER_QUOTA_REMAINING], "0");
        assert!(ok.headers().get("retry-after").is_none());

        let limited = send(
            app(quotas.clone(), alice),
            HttpRequest::get("/vaults")
                .header(header::AUTHORIZATION, bearer)
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(limited.status(), StatusCode::TOO_MANY_REQUESTS);
        assert_eq!(
            limited.headers()[header::CONTENT_TYPE],
            PROBLEM_CONTENT_TYPE
        );
        let retry: u64 = limited.headers()["retry-after"]
            .to_str()
            .unwrap()
            .parse()
            .unwrap();
        assert!((1..=60).contains(&retry));
        for (_, value) in limited.headers() {
            assert!(
                !value.to_str().unwrap_or("").contains("secret"),
                "credential leaked in headers"
            );
        }

        // A different manager is not affected.
        let bob = AuthenticatedUser::new(BOB.into(), Role::Manager, vec![]);
        let other = send(
            app(quotas, bob),
            HttpRequest::get("/vaults").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(other.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn only_admins_can_change_quotas_and_changes_apply_to_the_next_request() {
        let quotas = Arc::new(registry(1, 1));
        let manager = AuthenticatedUser::new(ALICE.into(), Role::Manager, vec![]);
        let admin = AuthenticatedUser::new("GADMIN".into(), Role::Admin, vec![]);
        let body = r#"{"class":"read","limit":3,"window_secs":60}"#;
        let put_request = || {
            HttpRequest::put(format!("/admin/quotas/{ALICE}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(body))
                .unwrap()
        };

        let denied = send(app(quotas.clone(), manager.clone()), put_request()).await;
        assert_eq!(denied.status(), StatusCode::FORBIDDEN);

        let accepted = send(app(quotas.clone(), admin.clone()), put_request()).await;
        assert_eq!(accepted.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(accepted.into_body(), 1 << 16)
            .await
            .unwrap();
        let json: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(json["quotas"][0]["limit"], 3);
        assert_eq!(json["quotas"][0]["overridden"], true);

        let ok = send(
            app(quotas.clone(), manager),
            HttpRequest::get("/vaults").body(Body::empty()).unwrap(),
        )
        .await;
        assert_eq!(ok.headers()[HEADER_QUOTA_LIMIT], "3");

        let invalid = send(
            app(quotas, admin),
            HttpRequest::put(format!("/admin/quotas/{ALICE}"))
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"class":"read","limit":0,"window_secs":60}"#))
                .unwrap(),
        )
        .await;
        assert_eq!(invalid.status(), StatusCode::UNPROCESSABLE_ENTITY);
    }

    #[test]
    fn unbounded_classes_are_rejected_by_the_admin_api() {
        let parsed: Result<SetQuotaRequest, _> =
            serde_json::from_str(r#"{"class":"everything","limit":1,"window_secs":1}"#);
        assert!(parsed.is_err());
    }
}
