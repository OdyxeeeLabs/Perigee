//! API-key lifecycle management for server-to-server integrations (API-38).
//!
//! Integrations that cannot hold an interactive wallet session get a scoped,
//! revocable credential instead:
//!
//! * **Creation** returns the secret exactly once. Only a non-reversible
//!   verifier (SHA-256 over the key id and the 256-bit random secret) is
//!   stored, so neither listings nor the store itself can disclose it.
//! * **Least privilege.** A key carries an explicit, non-empty set of
//!   [`ApiKeyScope`]s and can never exceed its creator's role: read-only
//!   callers can only mint read scopes, and no scope grants key management or
//!   administrator access.
//! * **Rotation** replaces the secret in place; the previous secret stops
//!   working immediately. **Revocation** is permanent.
//! * **Tenant separation.** Keys belong to the manager who created them. Other
//!   managers get `404` for them, exactly as for a key that does not exist.
//! * **Audit.** Creation, rotation, revocation and rejected authentications
//!   are recorded in [`ApiKeyStore::audit_events`] and the global audit log.
//!
//! Keys are presented in the `X-API-Key` header as `pgk_<key id>.<secret>`.
//! Managing keys requires a wallet session (JWT); an API key cannot create,
//! rotate or revoke keys. See `docs/API_KEYS.md`.

use crate::auth::AuthenticatedUser;
use crate::rest_api::{ProblemDetails, PROBLEM_CONTENT_TYPE};
use crate::timestamps::{rfc3339_utc, rfc3339_utc_option};
use axum::{
    extract::{Path, Request},
    http::{header, HeaderValue, StatusCode},
    middleware::Next,
    response::{IntoResponse, Response},
    Extension, Json,
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use chrono::{DateTime, Duration, Utc};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeSet, HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard};
use subtle::ConstantTimeEq;

/// Header that carries an API key.
pub const API_KEY_HEADER: &str = "x-api-key";
/// Prefix of every issued key, so leaked keys are easy to recognise and scan for.
pub const KEY_PREFIX: &str = "pgk_";
pub const MAX_KEYS_PER_OWNER: usize = 25;
pub const DEFAULT_EXPIRY_DAYS: u32 = 90;
pub const MAX_EXPIRY_DAYS: u32 = 365;
const MAX_NAME_LEN: usize = 64;
const MAX_AUDIT_EVENTS: usize = 1_000;
const SECRET_BYTES: usize = 32;
const KEY_ID_BYTES: usize = 8;

/// Permissions an API key can carry. Deliberately narrow: there is no scope
/// for key management, manager approval or administration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum ApiKeyScope {
    #[serde(rename = "vaults:read")]
    VaultsRead,
    #[serde(rename = "vaults:write")]
    VaultsWrite,
    #[serde(rename = "simulations:run")]
    SimulationsRun,
    #[serde(rename = "reports:read")]
    ReportsRead,
}

impl ApiKeyScope {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::VaultsRead => "vaults:read",
            Self::VaultsWrite => "vaults:write",
            Self::SimulationsRun => "simulations:run",
            Self::ReportsRead => "reports:read",
        }
    }

    /// Whether a creator with the given write capability may grant this scope.
    fn grantable_by(&self, creator_can_write: bool) -> bool {
        match self {
            Self::VaultsWrite => creator_can_write,
            Self::VaultsRead | Self::SimulationsRun | Self::ReportsRead => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiKeyError {
    Invalid(String),
    Forbidden(String),
    NotFound,
    LimitReached,
    /// The presented credential is missing, malformed, unknown, revoked,
    /// expired or wrong. Deliberately not more specific.
    Unauthenticated,
    MissingScope(ApiKeyScope),
}

impl std::fmt::Display for ApiKeyError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Invalid(msg) | Self::Forbidden(msg) => f.write_str(msg),
            Self::NotFound => f.write_str("API key not found"),
            Self::LimitReached => write!(
                f,
                "at most {MAX_KEYS_PER_OWNER} active API keys are allowed"
            ),
            Self::Unauthenticated => f.write_str("invalid API key"),
            Self::MissingScope(scope) => write!(f, "API key lacks the `{}` scope", scope.as_str()),
        }
    }
}

impl std::error::Error for ApiKeyError {}

/// Stored record. Holds the verifier, never the secret.
#[derive(Debug, Clone)]
struct ApiKeyRecord {
    key_id: String,
    owner: String,
    name: String,
    scopes: BTreeSet<ApiKeyScope>,
    verifier: [u8; 32],
    created_at: DateTime<Utc>,
    rotated_at: Option<DateTime<Utc>>,
    last_used_at: Option<DateTime<Utc>>,
    expires_at: DateTime<Utc>,
    revoked_at: Option<DateTime<Utc>>,
}

/// What listings return: metadata only.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiKeyMetadata {
    pub key_id: String,
    pub name: String,
    pub scopes: Vec<ApiKeyScope>,
    /// First characters of the presented key, safe to display.
    pub display_prefix: String,
    pub status: ApiKeyStatus,
    #[serde(with = "rfc3339_utc")]
    pub created_at: DateTime<Utc>,
    #[serde(with = "rfc3339_utc_option")]
    pub rotated_at: Option<DateTime<Utc>>,
    #[serde(with = "rfc3339_utc_option")]
    pub last_used_at: Option<DateTime<Utc>>,
    #[serde(with = "rfc3339_utc")]
    pub expires_at: DateTime<Utc>,
    #[serde(with = "rfc3339_utc_option")]
    pub revoked_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ApiKeyStatus {
    Active,
    Expired,
    Revoked,
}

/// Returned by create and rotate only. The secret is not retrievable later.
#[derive(Debug, Clone, Serialize)]
pub struct IssuedApiKey {
    pub key: ApiKeyMetadata,
    /// The full credential. Shown once; store it in a secret manager.
    pub secret: String,
}

/// The identity of a request authenticated with an API key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiKeyPrincipal {
    pub key_id: String,
    /// The manager (tenant) that owns the key.
    pub owner: String,
    pub scopes: BTreeSet<ApiKeyScope>,
}

impl ApiKeyPrincipal {
    pub fn require(&self, scope: ApiKeyScope) -> Result<(), ApiKeyError> {
        if self.scopes.contains(&scope) {
            Ok(())
        } else {
            Err(ApiKeyError::MissingScope(scope))
        }
    }
}

/// Lifecycle actions recorded for audit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ApiKeyAuditEvent {
    pub action: &'static str,
    pub key_id: Option<String>,
    pub owner: Option<String>,
    pub actor: String,
    #[serde(with = "rfc3339_utc")]
    pub at: DateTime<Utc>,
}

/// Body of `POST /api-keys`.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateApiKeyRequest {
    pub name: String,
    pub scopes: Vec<ApiKeyScope>,
    #[serde(default)]
    pub expires_in_days: Option<u32>,
}

#[derive(Default)]
struct Inner {
    keys: HashMap<String, ApiKeyRecord>,
    audit: VecDeque<ApiKeyAuditEvent>,
}

/// In-process API-key store.
#[derive(Default)]
pub struct ApiKeyStore {
    inner: Mutex<Inner>,
}

fn verifier(key_id: &str, secret: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"perigee-api-key-v1\0");
    hasher.update(key_id.as_bytes());
    hasher.update(b"\0");
    hasher.update(secret.as_bytes());
    hasher.finalize().into()
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}

fn new_secret() -> String {
    URL_SAFE_NO_PAD.encode(random_bytes::<SECRET_BYTES>())
}

fn render_key(key_id: &str, secret: &str) -> String {
    format!("{KEY_PREFIX}{key_id}.{secret}")
}

/// Split `pgk_<key id>.<secret>` into its parts.
fn parse_key(presented: &str) -> Option<(&str, &str)> {
    let rest = presented.trim().strip_prefix(KEY_PREFIX)?;
    let (key_id, secret) = rest.split_once('.')?;
    let valid_id =
        key_id.len() == KEY_ID_BYTES * 2 && key_id.bytes().all(|b| b.is_ascii_hexdigit());
    (valid_id && !secret.is_empty() && secret.len() <= 128).then_some((key_id, secret))
}

impl ApiKeyRecord {
    fn status(&self, now: DateTime<Utc>) -> ApiKeyStatus {
        if self.revoked_at.is_some() {
            ApiKeyStatus::Revoked
        } else if now >= self.expires_at {
            ApiKeyStatus::Expired
        } else {
            ApiKeyStatus::Active
        }
    }

    fn metadata(&self, now: DateTime<Utc>) -> ApiKeyMetadata {
        ApiKeyMetadata {
            key_id: self.key_id.clone(),
            name: self.name.clone(),
            scopes: self.scopes.iter().copied().collect(),
            display_prefix: format!("{KEY_PREFIX}{}", self.key_id),
            status: self.status(now),
            created_at: self.created_at,
            rotated_at: self.rotated_at,
            last_used_at: self.last_used_at,
            expires_at: self.expires_at,
            revoked_at: self.revoked_at,
        }
    }
}

impl ApiKeyStore {
    pub fn new() -> Self {
        Self::default()
    }

    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn record(
        inner: &mut Inner,
        action: &'static str,
        key_id: Option<&str>,
        owner: Option<&str>,
        actor: &str,
    ) {
        if inner.audit.len() >= MAX_AUDIT_EVENTS {
            inner.audit.pop_front();
        }
        inner.audit.push_back(ApiKeyAuditEvent {
            action,
            key_id: key_id.map(str::to_string),
            owner: owner.map(str::to_string),
            actor: actor.to_string(),
            at: Utc::now(),
        });
        let detail = match key_id {
            Some(id) => format!("{action} key_id={id}"),
            None => action.to_string(),
        };
        crate::audit_log::log_audit_event(owner.unwrap_or("-"), &detail, actor);
    }

    /// Most recent lifecycle events, oldest first.
    pub fn audit_events(&self) -> Vec<ApiKeyAuditEvent> {
        self.lock().audit.iter().cloned().collect()
    }

    /// Create a key for `owner`. `owner_can_write` caps the grantable scopes.
    pub fn create(
        &self,
        owner: &str,
        owner_can_write: bool,
        request: &CreateApiKeyRequest,
    ) -> Result<IssuedApiKey, ApiKeyError> {
        let name = request.name.trim();
        if name.is_empty()
            || name.chars().count() > MAX_NAME_LEN
            || name.chars().any(char::is_control)
        {
            return Err(ApiKeyError::Invalid(format!(
                "name must be 1-{MAX_NAME_LEN} printable characters"
            )));
        }
        let scopes: BTreeSet<ApiKeyScope> = request.scopes.iter().copied().collect();
        if scopes.is_empty() {
            return Err(ApiKeyError::Invalid(
                "at least one scope is required".into(),
            ));
        }
        if let Some(scope) = scopes.iter().find(|s| !s.grantable_by(owner_can_write)) {
            return Err(ApiKeyError::Forbidden(format!(
                "your role cannot grant the `{}` scope",
                scope.as_str()
            )));
        }
        let days = request.expires_in_days.unwrap_or(DEFAULT_EXPIRY_DAYS);
        if days == 0 || days > MAX_EXPIRY_DAYS {
            return Err(ApiKeyError::Invalid(format!(
                "expires_in_days must be between 1 and {MAX_EXPIRY_DAYS}"
            )));
        }

        let now = Utc::now();
        let mut inner = self.lock();
        let active = inner
            .keys
            .values()
            .filter(|k| k.owner == owner && k.status(now) == ApiKeyStatus::Active)
            .count();
        if active >= MAX_KEYS_PER_OWNER {
            return Err(ApiKeyError::LimitReached);
        }

        let key_id = hex::encode(random_bytes::<KEY_ID_BYTES>());
        let secret = new_secret();
        let record = ApiKeyRecord {
            key_id: key_id.clone(),
            owner: owner.to_string(),
            name: name.to_string(),
            scopes,
            verifier: verifier(&key_id, &secret),
            created_at: now,
            rotated_at: None,
            last_used_at: None,
            expires_at: now + Duration::days(i64::from(days)),
            revoked_at: None,
        };
        let metadata = record.metadata(now);
        inner.keys.insert(key_id.clone(), record);
        Self::record(
            &mut inner,
            "api_key.created",
            Some(&key_id),
            Some(owner),
            owner,
        );
        Ok(IssuedApiKey {
            key: metadata,
            secret: render_key(&key_id, &secret),
        })
    }

    /// Metadata for `owner`'s keys, newest first. Never includes secrets.
    pub fn list(&self, owner: &str) -> Vec<ApiKeyMetadata> {
        let now = Utc::now();
        let inner = self.lock();
        let mut keys: Vec<ApiKeyMetadata> = inner
            .keys
            .values()
            .filter(|k| k.owner == owner)
            .map(|k| k.metadata(now))
            .collect();
        keys.sort_by(|a, b| {
            b.created_at
                .cmp(&a.created_at)
                .then_with(|| a.key_id.cmp(&b.key_id))
        });
        keys
    }

    /// Replace the secret of an active key. The old secret stops working at once.
    pub fn rotate(&self, owner: &str, key_id: &str) -> Result<IssuedApiKey, ApiKeyError> {
        let now = Utc::now();
        let mut inner = self.lock();
        let record = inner
            .keys
            .get_mut(key_id)
            .filter(|k| k.owner == owner)
            .ok_or(ApiKeyError::NotFound)?;
        if record.status(now) != ApiKeyStatus::Active {
            return Err(ApiKeyError::Invalid(
                "only active keys can be rotated".into(),
            ));
        }
        let secret = new_secret();
        record.verifier = verifier(key_id, &secret);
        record.rotated_at = Some(now);
        let metadata = record.metadata(now);
        Self::record(
            &mut inner,
            "api_key.rotated",
            Some(key_id),
            Some(owner),
            owner,
        );
        Ok(IssuedApiKey {
            key: metadata,
            secret: render_key(key_id, &secret),
        })
    }

    /// Permanently revoke a key. Revoking twice is a no-op that returns the record.
    pub fn revoke(&self, owner: &str, key_id: &str) -> Result<ApiKeyMetadata, ApiKeyError> {
        let now = Utc::now();
        let mut inner = self.lock();
        let record = inner
            .keys
            .get_mut(key_id)
            .filter(|k| k.owner == owner)
            .ok_or(ApiKeyError::NotFound)?;
        let newly_revoked = record.revoked_at.is_none();
        if newly_revoked {
            record.revoked_at = Some(now);
        }
        let metadata = record.metadata(now);
        if newly_revoked {
            Self::record(
                &mut inner,
                "api_key.revoked",
                Some(key_id),
                Some(owner),
                owner,
            );
        }
        Ok(metadata)
    }

    /// Verify a presented key. Every failure looks the same to the caller.
    pub fn authenticate(&self, presented: &str) -> Result<ApiKeyPrincipal, ApiKeyError> {
        let now = Utc::now();
        let mut inner = self.lock();
        let Some((key_id, secret)) = parse_key(presented) else {
            Self::record(&mut inner, "api_key.auth_rejected", None, None, "anonymous");
            return Err(ApiKeyError::Unauthenticated);
        };
        let candidate = verifier(key_id, secret);
        let accepted = match inner.keys.get_mut(key_id) {
            Some(record) => {
                let matches = bool::from(record.verifier.ct_eq(&candidate));
                if matches && record.status(now) == ApiKeyStatus::Active {
                    record.last_used_at = Some(now);
                    Some(ApiKeyPrincipal {
                        key_id: record.key_id.clone(),
                        owner: record.owner.clone(),
                        scopes: record.scopes.clone(),
                    })
                } else {
                    None
                }
            }
            None => None,
        };
        match accepted {
            Some(principal) => Ok(principal),
            None => {
                let owner = inner.keys.get(key_id).map(|k| k.owner.clone());
                Self::record(
                    &mut inner,
                    "api_key.auth_rejected",
                    Some(key_id),
                    owner.as_deref(),
                    "anonymous",
                );
                Err(ApiKeyError::Unauthenticated)
            }
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP
// ---------------------------------------------------------------------------

fn problem(status: StatusCode, detail: &str) -> Response {
    let body = ProblemDetails::from_status(status.as_u16(), detail).to_json();
    let mut response = (status, Json(body)).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_static(PROBLEM_CONTENT_TYPE),
    );
    response
}

fn error_response(error: ApiKeyError) -> Response {
    let status = match error {
        ApiKeyError::Invalid(_) => StatusCode::UNPROCESSABLE_ENTITY,
        ApiKeyError::Forbidden(_) | ApiKeyError::MissingScope(_) => StatusCode::FORBIDDEN,
        ApiKeyError::NotFound => StatusCode::NOT_FOUND,
        ApiKeyError::LimitReached => StatusCode::CONFLICT,
        ApiKeyError::Unauthenticated => StatusCode::UNAUTHORIZED,
    };
    let mut response = problem(status, &error.to_string());
    if status == StatusCode::UNAUTHORIZED {
        response
            .headers_mut()
            .insert(header::WWW_AUTHENTICATE, HeaderValue::from_static("ApiKey"));
    }
    response
}

fn no_store(mut response: Response) -> Response {
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

#[derive(Debug, Serialize)]
pub struct ApiKeyListResponse {
    pub keys: Vec<ApiKeyMetadata>,
}

/// `POST /api-keys` – create a key; the response is the only time the secret is shown.
pub async fn create_api_key_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(store): Extension<Arc<ApiKeyStore>>,
    Json(request): Json<CreateApiKeyRequest>,
) -> Response {
    match store.create(&user.stellar_address, user.can_write_vaults(), &request) {
        Ok(issued) => no_store((StatusCode::CREATED, Json(issued)).into_response()),
        Err(error) => error_response(error),
    }
}

/// `GET /api-keys` – metadata for the caller's keys.
pub async fn list_api_keys_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(store): Extension<Arc<ApiKeyStore>>,
) -> Response {
    no_store(
        Json(ApiKeyListResponse {
            keys: store.list(&user.stellar_address),
        })
        .into_response(),
    )
}

/// `POST /api-keys/:key_id/rotate` – issue a new secret, invalidating the old one.
pub async fn rotate_api_key_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(store): Extension<Arc<ApiKeyStore>>,
    Path(key_id): Path<String>,
) -> Response {
    match store.rotate(&user.stellar_address, &key_id) {
        Ok(issued) => no_store(Json(issued).into_response()),
        Err(error) => error_response(error),
    }
}

/// `DELETE /api-keys/:key_id` – revoke permanently.
pub async fn revoke_api_key_handler(
    Extension(user): Extension<AuthenticatedUser>,
    Extension(store): Extension<Arc<ApiKeyStore>>,
    Path(key_id): Path<String>,
) -> Response {
    match store.revoke(&user.stellar_address, &key_id) {
        Ok(metadata) => no_store(Json(metadata).into_response()),
        Err(error) => error_response(error),
    }
}

/// Authenticates `X-API-Key` and inserts an [`ApiKeyPrincipal`]. Requests
/// without the header are rejected with `401`.
pub async fn api_key_middleware(
    Extension(store): Extension<Arc<ApiKeyStore>>,
    mut request: Request,
    next: Next,
) -> Response {
    let presented = request
        .headers()
        .get(API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(str::to_string);
    let Some(presented) = presented else {
        return error_response(ApiKeyError::Unauthenticated);
    };
    match store.authenticate(&presented) {
        Ok(principal) => {
            request.extensions_mut().insert(principal);
            next.run(request).await
        }
        Err(error) => error_response(error),
    }
}

/// Route guard: `from_fn(|req, next| require_scope(ApiKeyScope::VaultsRead, req, next))`.
/// Must run inside [`api_key_middleware`].
pub async fn require_scope(scope: ApiKeyScope, request: Request, next: Next) -> Response {
    let check = match request.extensions().get::<ApiKeyPrincipal>() {
        Some(principal) => principal.require(scope),
        None => Err(ApiKeyError::Unauthenticated),
    };
    match check {
        Ok(()) => next.run(request).await,
        Err(error) => error_response(error),
    }
}

/// Body of `GET /integrations/api-key`.
#[derive(Debug, Serialize)]
pub struct ApiKeyIntrospection {
    pub key_id: String,
    pub scopes: Vec<ApiKeyScope>,
}

/// `GET /integrations/api-key` – lets an integration confirm which key and
/// scopes it is using, without revealing anything else.
pub async fn introspect_api_key_handler(
    Extension(principal): Extension<ApiKeyPrincipal>,
) -> Response {
    Json(ApiKeyIntrospection {
        key_id: principal.key_id,
        scopes: principal.scopes.into_iter().collect(),
    })
    .into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auth::Role;
    use axum::{body::Body, http::Request as HttpRequest, routing::get, routing::post, Router};
    use tower::ServiceExt;

    const ALICE: &str = "GALICEMANAGER";
    const BOB: &str = "GBOBMANAGER";

    fn request(name: &str, scopes: &[ApiKeyScope]) -> CreateApiKeyRequest {
        CreateApiKeyRequest {
            name: name.to_string(),
            scopes: scopes.to_vec(),
            expires_in_days: None,
        }
    }

    #[test]
    fn creation_shows_the_secret_once_and_stores_only_a_verifier() {
        let store = ApiKeyStore::new();
        let issued = store
            .create(ALICE, true, &request("ci", &[ApiKeyScope::VaultsRead]))
            .unwrap();
        assert!(issued.secret.starts_with(KEY_PREFIX));
        let (_, secret_part) = parse_key(&issued.secret).unwrap();
        assert!(secret_part.len() >= 43, "256-bit secret");

        // Listing and the stored record never contain the secret.
        let listed = serde_json::to_string(&store.list(ALICE)).unwrap();
        assert!(!listed.contains(secret_part));
        assert!(!listed.contains("secret"));
        let inner = store.lock();
        let record = inner.keys.get(&issued.key.key_id).unwrap();
        assert!(!format!("{record:?}").contains(secret_part));
        assert_ne!(record.verifier.to_vec(), secret_part.as_bytes().to_vec());
    }

    #[test]
    fn scopes_are_enforced() {
        let store = ApiKeyStore::new();
        let issued = store
            .create(ALICE, true, &request("reader", &[ApiKeyScope::VaultsRead]))
            .unwrap();
        let principal = store.authenticate(&issued.secret).unwrap();
        assert!(principal.require(ApiKeyScope::VaultsRead).is_ok());
        assert_eq!(
            principal.require(ApiKeyScope::VaultsWrite),
            Err(ApiKeyError::MissingScope(ApiKeyScope::VaultsWrite))
        );
    }

    #[test]
    fn keys_cannot_exceed_the_creators_role() {
        let store = ApiKeyStore::new();
        let err = store
            .create(ALICE, false, &request("w", &[ApiKeyScope::VaultsWrite]))
            .unwrap_err();
        assert!(matches!(err, ApiKeyError::Forbidden(_)));
        assert!(store
            .create(ALICE, false, &request("r", &[ApiKeyScope::ReportsRead]))
            .is_ok());
    }

    #[test]
    fn invalid_requests_are_rejected() {
        let store = ApiKeyStore::new();
        assert!(store
            .create(ALICE, true, &request("", &[ApiKeyScope::VaultsRead]))
            .is_err());
        assert!(store
            .create(ALICE, true, &request("no scopes", &[]))
            .is_err());
        assert!(store
            .create(
                ALICE,
                true,
                &request(&"x".repeat(65), &[ApiKeyScope::VaultsRead])
            )
            .is_err());
        let mut long = request("long", &[ApiKeyScope::VaultsRead]);
        long.expires_in_days = Some(MAX_EXPIRY_DAYS + 1);
        assert!(store.create(ALICE, true, &long).is_err());
        let unknown: Result<CreateApiKeyRequest, _> =
            serde_json::from_str(r#"{"name":"x","scopes":["admin:*"]}"#);
        assert!(unknown.is_err(), "unknown scopes are not accepted");
    }

    #[test]
    fn rotation_invalidates_the_previous_secret() {
        let store = ApiKeyStore::new();
        let original = store
            .create(ALICE, true, &request("ci", &[ApiKeyScope::VaultsRead]))
            .unwrap();
        let rotated = store.rotate(ALICE, &original.key.key_id).unwrap();
        assert_eq!(rotated.key.key_id, original.key.key_id);
        assert_ne!(rotated.secret, original.secret);
        assert!(rotated.key.rotated_at.is_some());

        assert_eq!(
            store.authenticate(&original.secret),
            Err(ApiKeyError::Unauthenticated)
        );
        assert!(store.authenticate(&rotated.secret).is_ok());
    }

    #[test]
    fn revocation_is_immediate_and_permanent() {
        let store = ApiKeyStore::new();
        let issued = store
            .create(ALICE, true, &request("ci", &[ApiKeyScope::VaultsRead]))
            .unwrap();
        assert!(store.authenticate(&issued.secret).is_ok());
        let revoked = store.revoke(ALICE, &issued.key.key_id).unwrap();
        assert_eq!(revoked.status, ApiKeyStatus::Revoked);
        assert_eq!(
            store.authenticate(&issued.secret),
            Err(ApiKeyError::Unauthenticated)
        );
        assert!(
            store.rotate(ALICE, &issued.key.key_id).is_err(),
            "revoked keys cannot be revived"
        );
        // Idempotent.
        assert!(store.revoke(ALICE, &issued.key.key_id).is_ok());
    }

    #[test]
    fn tenants_are_separated() {
        let store = ApiKeyStore::new();
        let alice_key = store
            .create(ALICE, true, &request("alice", &[ApiKeyScope::VaultsRead]))
            .unwrap();
        store
            .create(BOB, true, &request("bob", &[ApiKeyScope::VaultsRead]))
            .unwrap();

        assert_eq!(store.list(ALICE).len(), 1);
        assert_eq!(store.list(BOB).len(), 1);
        assert_eq!(store.list(BOB)[0].name, "bob");
        assert_eq!(
            store.rotate(BOB, &alice_key.key.key_id).unwrap_err(),
            ApiKeyError::NotFound
        );
        assert_eq!(
            store.revoke(BOB, &alice_key.key.key_id).unwrap_err(),
            ApiKeyError::NotFound
        );
        // Alice's key still works and authenticates as Alice.
        assert_eq!(store.authenticate(&alice_key.secret).unwrap().owner, ALICE);
    }

    #[test]
    fn malformed_and_forged_keys_are_rejected_uniformly() {
        let store = ApiKeyStore::new();
        let issued = store
            .create(ALICE, true, &request("ci", &[ApiKeyScope::VaultsRead]))
            .unwrap();
        let (key_id, _) = parse_key(&issued.secret).unwrap();
        for bad in [
            String::new(),
            "Bearer abc".to_string(),
            format!("{KEY_PREFIX}{key_id}"),
            format!("{KEY_PREFIX}{key_id}.wrong-secret"),
            format!("{KEY_PREFIX}0000000000000000.whatever"),
        ] {
            assert_eq!(
                store.authenticate(&bad),
                Err(ApiKeyError::Unauthenticated),
                "{bad}"
            );
        }
    }

    #[test]
    fn lifecycle_actions_are_audited() {
        let store = ApiKeyStore::new();
        let issued = store
            .create(ALICE, true, &request("ci", &[ApiKeyScope::VaultsRead]))
            .unwrap();
        store.rotate(ALICE, &issued.key.key_id).unwrap();
        store.revoke(ALICE, &issued.key.key_id).unwrap();
        let _ = store.authenticate(&issued.secret);
        let actions: Vec<&str> = store.audit_events().iter().map(|e| e.action).collect();
        assert_eq!(
            actions,
            [
                "api_key.created",
                "api_key.rotated",
                "api_key.revoked",
                "api_key.auth_rejected"
            ]
        );
        let rendered = serde_json::to_string(&store.audit_events()).unwrap();
        assert!(!rendered.contains(&issued.secret));
    }

    #[test]
    fn active_keys_per_owner_are_bounded() {
        let store = ApiKeyStore::new();
        for i in 0..MAX_KEYS_PER_OWNER {
            store
                .create(
                    ALICE,
                    true,
                    &request(&format!("k{i}"), &[ApiKeyScope::VaultsRead]),
                )
                .unwrap();
        }
        assert_eq!(
            store
                .create(ALICE, true, &request("extra", &[ApiKeyScope::VaultsRead]))
                .unwrap_err(),
            ApiKeyError::LimitReached
        );
        assert!(store
            .create(BOB, true, &request("bob", &[ApiKeyScope::VaultsRead]))
            .is_ok());
    }

    fn management_app(store: Arc<ApiKeyStore>, user: AuthenticatedUser) -> Router {
        Router::new()
            .route(
                "/api-keys",
                get(list_api_keys_handler).post(create_api_key_handler),
            )
            .route("/api-keys/:key_id/rotate", post(rotate_api_key_handler))
            .layer(Extension(user))
            .layer(Extension(store))
    }

    fn integration_app(store: Arc<ApiKeyStore>) -> Router {
        Router::new()
            .route(
                "/vaults",
                get(|| async { "vaults" }).route_layer(axum::middleware::from_fn(|req, next| {
                    require_scope(ApiKeyScope::VaultsRead, req, next)
                })),
            )
            .route(
                "/simulate",
                post(|| async { "simulated" }).route_layer(axum::middleware::from_fn(
                    |req, next| require_scope(ApiKeyScope::SimulationsRun, req, next),
                )),
            )
            .route("/integrations/api-key", get(introspect_api_key_handler))
            .route_layer(axum::middleware::from_fn(api_key_middleware))
            .layer(Extension(store))
    }

    async fn json(response: Response) -> serde_json::Value {
        let bytes = axum::body::to_bytes(response.into_body(), 1 << 16)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn http_lifecycle_and_scope_enforcement() {
        let store = Arc::new(ApiKeyStore::new());
        let alice = AuthenticatedUser::new(ALICE.into(), Role::Manager, vec![]);

        let created = management_app(store.clone(), alice.clone())
            .oneshot(
                HttpRequest::post("/api-keys")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"name":"reporting","scopes":["vaults:read"]}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        assert_eq!(created.headers()[header::CACHE_CONTROL], "no-store");
        let body = json(created).await;
        let secret = body["secret"].as_str().unwrap().to_string();
        let key_id = body["key"]["key_id"].as_str().unwrap().to_string();
        assert!(body["key"]["created_at"].as_str().unwrap().ends_with('Z'));

        // The listing never repeats the secret.
        let listed = management_app(store.clone(), alice.clone())
            .oneshot(HttpRequest::get("/api-keys").body(Body::empty()).unwrap())
            .await
            .unwrap();
        let listed = json(listed).await;
        assert_eq!(listed["keys"].as_array().unwrap().len(), 1);
        assert!(!listed
            .to_string()
            .contains(secret.split('.').nth(1).unwrap()));

        // The key reads vaults but cannot run simulations.
        let read = integration_app(store.clone())
            .oneshot(
                HttpRequest::get("/vaults")
                    .header(API_KEY_HEADER, &secret)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(read.status(), StatusCode::OK);
        let simulate = integration_app(store.clone())
            .oneshot(
                HttpRequest::post("/simulate")
                    .header(API_KEY_HEADER, &secret)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(simulate.status(), StatusCode::FORBIDDEN);

        let introspect = integration_app(store.clone())
            .oneshot(
                HttpRequest::get("/integrations/api-key")
                    .header(API_KEY_HEADER, &secret)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let introspect = json(introspect).await;
        assert_eq!(introspect["key_id"], key_id.as_str());
        assert_eq!(introspect["scopes"], serde_json::json!(["vaults:read"]));

        // Rotation: old secret is rejected with 401, new one works.
        let rotated = management_app(store.clone(), alice)
            .oneshot(
                HttpRequest::post(format!("/api-keys/{key_id}/rotate"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let new_secret = json(rotated).await["secret"].as_str().unwrap().to_string();
        let stale = integration_app(store.clone())
            .oneshot(
                HttpRequest::get("/vaults")
                    .header(API_KEY_HEADER, &secret)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stale.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(stale.headers()[header::CONTENT_TYPE], PROBLEM_CONTENT_TYPE);
        let fresh = integration_app(store.clone())
            .oneshot(
                HttpRequest::get("/vaults")
                    .header(API_KEY_HEADER, &new_secret)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(fresh.status(), StatusCode::OK);

        // No key at all.
        let missing = integration_app(store.clone())
            .oneshot(HttpRequest::get("/vaults").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        // Another tenant cannot rotate Alice's key.
        let bob = AuthenticatedUser::new(BOB.into(), Role::Manager, vec![]);
        let foreign = management_app(store, bob)
            .oneshot(
                HttpRequest::post(format!("/api-keys/{key_id}/rotate"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(foreign.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn viewers_cannot_mint_write_keys_over_http() {
        let store = Arc::new(ApiKeyStore::new());
        let viewer = AuthenticatedUser::new("GVIEWER".into(), Role::Viewer, vec![]);
        let response = management_app(store, viewer)
            .oneshot(
                HttpRequest::post("/api-keys")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"name":"w","scopes":["vaults:write"]}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::FORBIDDEN);
    }
}
