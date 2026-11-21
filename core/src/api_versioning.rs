//! API versioning strategy — Issue #599.
//!
//! Implements:
//! - URL-prefix versioning: `/v1/…`, `/v2/…`
//! - Header fallback: `Accept-Version: v2`
//! - Deprecation lifecycle: `Sunset` header + migration guide URL
//! - Backward-compatibility contract within a major version
//! - `CompatibilityTest` trait for automated regression suites

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// ApiVersion
// ---------------------------------------------------------------------------

/// Supported API versions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum ApiVersion {
    V1,
    V2,
}

impl ApiVersion {
    /// Parse from a string like `"v1"`, `"V1"`, or `"1"`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().trim_start_matches('v') {
            "1" => Some(Self::V1),
            "2" => Some(Self::V2),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Self::V1 => "v1",
            Self::V2 => "v2",
        }
    }

    /// Returns `true` if this version has been deprecated.
    pub fn is_deprecated(self) -> bool {
        matches!(self, Self::V1)
    }
}

impl std::fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// VersionNegotiator
// ---------------------------------------------------------------------------

/// Extracts the requested API version from an incoming request.
///
/// Priority (highest → lowest):
/// 1. URL path prefix — `/v2/endpoints`
/// 2. `Accept-Version: v2` header
/// 3. Default version (`V1`)
pub struct VersionNegotiator {
    default_version: ApiVersion,
}

impl Default for VersionNegotiator {
    fn default() -> Self {
        Self { default_version: ApiVersion::V1 }
    }
}

impl VersionNegotiator {
    pub fn new(default_version: ApiVersion) -> Self {
        Self { default_version }
    }

    /// Extract the API version from the URL path.
    ///
    /// ```
    /// # use perigee_core::api_versioning::{VersionNegotiator, ApiVersion};
    /// let n = VersionNegotiator::default();
    /// assert_eq!(n.from_path("/v2/vaults/123"), Some(ApiVersion::V2));
    /// assert_eq!(n.from_path("/v1/health"), Some(ApiVersion::V1));
    /// assert_eq!(n.from_path("/vaults"), None);
    /// ```
    pub fn from_path(&self, path: &str) -> Option<ApiVersion> {
        let segment = path.trim_start_matches('/').split('/').next()?;
        ApiVersion::parse(segment)
    }

    /// Extract the API version from an `Accept-Version` header value.
    pub fn from_header(&self, header_value: &str) -> Option<ApiVersion> {
        ApiVersion::parse(header_value.trim())
    }

    /// Negotiate the version using path, then header, then default.
    pub fn negotiate(&self, path: &str, accept_version_header: Option<&str>) -> ApiVersion {
        if let Some(v) = self.from_path(path) {
            return v;
        }
        if let Some(h) = accept_version_header {
            if let Some(v) = self.from_header(h) {
                return v;
            }
        }
        self.default_version
    }

    /// Strip the version prefix from a path so handlers receive a clean path.
    ///
    /// `/v2/vaults/123` → `/vaults/123`
    pub fn strip_prefix<'a>(&self, path: &'a str) -> &'a str {
        let trimmed = path.trim_start_matches('/');
        let segment = trimmed.split('/').next().unwrap_or("");
        if ApiVersion::parse(segment).is_some() {
            let rest = &trimmed[segment.len()..];
            if rest.is_empty() { "/" } else { rest }
        } else {
            path
        }
    }
}

// ---------------------------------------------------------------------------
// DeprecationPolicy
// ---------------------------------------------------------------------------

/// Describes the deprecation lifecycle for a specific API version.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecationPolicy {
    /// The version being deprecated.
    pub version: String,
    /// Date after which the version may be removed.
    /// Exposed via the HTTP `Sunset` response header.
    pub sunset_date: DateTime<Utc>,
    /// URL pointing to the migration guide for consumers.
    pub migration_guide_url: String,
    /// Human-readable deprecation notice included in the `Deprecation` header.
    pub notice: String,
}

impl DeprecationPolicy {
    /// Build the value of the `Sunset` header (RFC 8594).
    pub fn sunset_header_value(&self) -> String {
        self.sunset_date.to_rfc2822()
    }

    /// Build the value of the `Deprecation` header.
    pub fn deprecation_header_value(&self) -> String {
        self.sunset_date.to_rfc3339()
    }
}

/// Registry of deprecation policies keyed by API version string.
#[derive(Debug, Default)]
pub struct DeprecationRegistry {
    policies: HashMap<String, DeprecationPolicy>,
}

impl DeprecationRegistry {
    pub fn register(&mut self, policy: DeprecationPolicy) {
        self.policies.insert(policy.version.clone(), policy);
    }

    pub fn get(&self, version: &ApiVersion) -> Option<&DeprecationPolicy> {
        self.policies.get(version.as_str())
    }
}

// ---------------------------------------------------------------------------
// VersionRouter
// ---------------------------------------------------------------------------

/// A handler function type: takes a request body bytes and returns a response.
pub type HandlerFn = Box<dyn Fn(&[u8]) -> Vec<u8> + Send + Sync>;

/// Routes requests to the appropriate handler based on their API version.
pub struct VersionRouter {
    negotiator: VersionNegotiator,
    handlers: HashMap<ApiVersion, HandlerFn>,
    deprecation_registry: DeprecationRegistry,
}

impl VersionRouter {
    pub fn new() -> Self {
        Self {
            negotiator: VersionNegotiator::default(),
            handlers: HashMap::new(),
            deprecation_registry: DeprecationRegistry::default(),
        }
    }

    /// Register a handler for the given version.
    pub fn on_version(&mut self, version: ApiVersion, handler: HandlerFn) {
        self.handlers.insert(version, handler);
    }

    /// Register a deprecation policy for a version.
    pub fn deprecate(&mut self, policy: DeprecationPolicy) {
        self.deprecation_registry.register(policy);
    }

    /// Dispatch a request, returning the handler response and any deprecation
    /// headers that should be added to the HTTP response.
    pub fn dispatch(
        &self,
        path: &str,
        accept_version_header: Option<&str>,
        body: &[u8],
    ) -> DispatchResult {
        let version = self.negotiator.negotiate(path, accept_version_header);
        let deprecation_headers = self
            .deprecation_registry
            .get(&version)
            .map(|p| DeprecationHeaders {
                sunset: p.sunset_header_value(),
                deprecation: p.deprecation_header_value(),
                link: format!("<{}>; rel=\"successor-version\"", p.migration_guide_url),
            });

        let response = self
            .handlers
            .get(&version)
            .map(|h| h(body))
            .unwrap_or_else(|| b"version not found".to_vec());

        DispatchResult {
            version,
            response,
            deprecation_headers,
        }
    }
}

impl Default for VersionRouter {
    fn default() -> Self {
        Self::new()
    }
}

/// Result of routing a request through the `VersionRouter`.
pub struct DispatchResult {
    pub version: ApiVersion,
    pub response: Vec<u8>,
    /// Set when the negotiated version has a registered deprecation policy.
    pub deprecation_headers: Option<DeprecationHeaders>,
}

/// HTTP response headers to inject when a deprecated version is used.
#[derive(Debug, Clone)]
pub struct DeprecationHeaders {
    pub sunset: String,
    pub deprecation: String,
    pub link: String,
}

// ---------------------------------------------------------------------------
// CompatibilityTest — automated backward-compat testing
// ---------------------------------------------------------------------------

/// Contract for an automated backward-compatibility test suite.
///
/// Implementors define a set of named test cases. The test runner calls each
/// one against both the old and new version; the test passes if the new
/// version's output is compatible with the old version's contract.
pub trait CompatibilityTest {
    /// Human-readable name for this test suite.
    fn name(&self) -> &str;

    /// Run one named test case and return `Ok(())` if the new version's
    /// response is backward-compatible with the old version.
    fn run_case(&self, case_name: &str) -> Result<(), String>;

    /// All test case names in this suite.
    fn case_names(&self) -> Vec<&str>;

    /// Run all cases and collect failures.
    fn run_all(&self) -> Vec<(String, String)> {
        self.case_names()
            .iter()
            .filter_map(|name| {
                self.run_case(name)
                    .err()
                    .map(|e| (name.to_string(), e))
            })
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_version_from_path() {
        let n = VersionNegotiator::default();
        assert_eq!(n.from_path("/v1/health"), Some(ApiVersion::V1));
        assert_eq!(n.from_path("/v2/vaults/abc"), Some(ApiVersion::V2));
        assert_eq!(n.from_path("/health"), None);
    }

    #[test]
    fn negotiate_falls_back_to_header_when_no_prefix() {
        let n = VersionNegotiator::default();
        assert_eq!(
            n.negotiate("/vaults", Some("v2")),
            ApiVersion::V2
        );
    }

    #[test]
    fn negotiate_falls_back_to_default_when_no_signal() {
        let n = VersionNegotiator::new(ApiVersion::V2);
        assert_eq!(n.negotiate("/vaults", None), ApiVersion::V2);
    }

    #[test]
    fn strip_prefix_removes_version_segment() {
        let n = VersionNegotiator::default();
        assert_eq!(n.strip_prefix("/v2/vaults/123"), "/vaults/123");
        assert_eq!(n.strip_prefix("/v1/health"), "/health");
        assert_eq!(n.strip_prefix("/health"), "/health");
    }

    #[test]
    fn v1_is_deprecated_v2_is_not() {
        assert!(ApiVersion::V1.is_deprecated());
        assert!(!ApiVersion::V2.is_deprecated());
    }

    #[test]
    fn version_router_dispatches_to_correct_handler() {
        let mut router = VersionRouter::new();
        router.on_version(ApiVersion::V1, Box::new(|_| b"v1-response".to_vec()));
        router.on_version(ApiVersion::V2, Box::new(|_| b"v2-response".to_vec()));

        let r1 = router.dispatch("/v1/ping", None, b"");
        assert_eq!(r1.response, b"v1-response");
        assert_eq!(r1.version, ApiVersion::V1);

        let r2 = router.dispatch("/v2/ping", None, b"");
        assert_eq!(r2.response, b"v2-response");
    }

    #[test]
    fn sunset_header_injected_for_deprecated_version() {
        use chrono::TimeZone;
        let mut router = VersionRouter::new();
        router.on_version(ApiVersion::V1, Box::new(|_| b"old".to_vec()));
        router.deprecate(DeprecationPolicy {
            version: "v1".into(),
            sunset_date: Utc.with_ymd_and_hms(2027, 1, 1, 0, 0, 0).unwrap(),
            migration_guide_url: "https://docs.perigee.io/migrate/v2".into(),
            notice: "v1 is deprecated".into(),
        });
        let result = router.dispatch("/v1/ping", None, b"");
        assert!(result.deprecation_headers.is_some());
        let headers = result.deprecation_headers.unwrap();
        assert!(headers.sunset.contains("2027"));
        assert!(headers.link.contains("migrate/v2"));
    }
}
