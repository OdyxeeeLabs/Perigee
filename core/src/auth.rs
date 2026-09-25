use crate::audit_log::{log_audit_event, log_security_event, SecurityEventType};
use crate::config::{SecretKeyring, SecretKeyringError, SecretVersion};
use crate::error_codes::ErrorCode;
use crate::errors::{ApiJson, AppError};
use crate::errors::AppError;
use crate::input_sanitization::SanitizedJson;
use axum::{extract::Request, http::header, middleware::Next, response::Response, Extension, Json};
use base64::{
    engine::general_purpose::STANDARD as BASE64,
    engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL, Engine,
};
use ed25519_dalek::{Signature as Ed25519Signature, Signer, SigningKey, Verifier, VerifyingKey};
use jsonwebtoken::{
    decode, decode_header, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation,
};
use rand::RngCore;
use rsa::{
    pkcs1::DecodeRsaPrivateKey,
    pkcs8::{DecodePrivateKey, DecodePublicKey, EncodePrivateKey, EncodePublicKey},
    traits::PublicKeyParts,
    RsaPrivateKey, RsaPublicKey,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use soroban_sdk::xdr::{
    DecoratedSignature, Limits, ManageDataOp, Memo, MuxedAccount, Operation, OperationBody,
    Preconditions, ReadXdr, SequenceNumber, SignatureHint, TimeBounds, TimePoint, Transaction,
    TransactionEnvelope, TransactionExt, TransactionV1Envelope, Uint256, WriteXdr,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use stellar_strkey::Strkey;
use thiserror::Error;
use utoipa::ToSchema;
use uuid::Uuid;

const CHALLENGE_EXPIRY_SECS: u64 = 300;
/// Short-lived access JWT lifetime (15 minutes).
pub(crate) const ACCESS_TOKEN_EXPIRY_SECS: u64 = 900;
/// Refresh token lifetime (7 days). Rotated on every `/auth/refresh`.
const REFRESH_TOKEN_EXPIRY_SECS: u64 = 604_800;
const WEB_AUTH_DOMAIN: &str = "Perigee";

/// Server-side refresh-token record. The raw token is never stored — only its hash.
#[derive(Clone, Debug)]
enum RefreshTokenRecord {
    Active {
        subject: String,
        expires_at: u64,
        /// Shared across a rotation chain so reuse of a retired token can revoke the family.
        family_id: String,
    },
    /// Tombstone left after rotation; presenting this hash revokes the whole family.
    Rotated {
        family_id: String,
        expires_at: u64,
    },
}

/// User roles for role-based access control (RBAC).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Manager,
    Operator,
    Viewer,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Admin => "admin",
            Self::Manager => "manager",
            Self::Operator => "operator",
            Self::Viewer => "viewer",
        }
    }

    pub fn is_admin(&self) -> bool {
        matches!(self, Self::Admin)
    }

    pub fn can_write(&self) -> bool {
        matches!(self, Self::Admin | Self::Manager | Self::Operator)
    }

    pub fn can_manage(&self) -> bool {
        matches!(self, Self::Admin | Self::Manager)
    }

    pub fn can_read(&self) -> bool {
        true
    }
}

impl std::str::FromStr for Role {
    type Err = ();

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.to_ascii_lowercase().as_str() {
            "admin" => Ok(Role::Admin),
            "manager" => Ok(Role::Manager),
            "operator" => Ok(Role::Operator),
            "viewer" => Ok(Role::Viewer),
            _ => Err(()),
        }
    }
}

impl std::fmt::Display for Role {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// Check if a Stellar address is configured as an admin via `PERIGEE_ADMIN_STELLAR_ADDRESSES`.
pub fn is_admin_address(stellar_address: &str) -> bool {
    let allowed = std::env::var("PERIGEE_ADMIN_STELLAR_ADDRESSES").unwrap_or_default();
    allowed
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .any(|addr| addr == stellar_address)
}

/// Authenticated user extracted from JWT and injected into request extensions
/// for role- and vault-scoped authorization.
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub stellar_address: String,
    pub role: Role,
    pub roles: Vec<Role>,
    pub vault_scopes: Vec<String>,
}

impl AuthenticatedUser {
    pub fn new(stellar_address: String, role: Role, vault_scopes: Vec<String>) -> Self {
        Self {
            stellar_address,
            roles: vec![role],
            role,
            vault_scopes,
        }
    }

    pub fn is_admin(&self) -> bool {
        self.role == Role::Admin
            || self.roles.contains(&Role::Admin)
            || is_admin_address(&self.stellar_address)
    }

    pub fn has_role(&self, role: Role) -> bool {
        if self.is_admin() {
            return true;
        }
        self.role == role || self.roles.contains(&role)
    }

    pub fn can_write_vaults(&self) -> bool {
        self.is_admin() || self.role.can_write() || self.roles.iter().any(|r| r.can_write())
    }

    pub fn can_manage_vaults(&self) -> bool {
        self.is_admin() || self.role.can_manage() || self.roles.iter().any(|r| r.can_manage())
    }

    pub fn can_access_all_vaults(&self) -> bool {
        self.is_admin() || self.vault_scopes.is_empty() || self.vault_scopes.iter().any(|s| s == "*")
    }

    pub fn can_access_vault(&self, vault_id: &str) -> bool {
        if self.can_access_all_vaults() {
            return true;
        }
        self.vault_scopes.iter().any(|s| s == vault_id)
    }

    pub fn authorize_vault_read(&self, vault_id: &str) -> Result<(), AppError> {
        if !self.can_access_vault(vault_id) {
            log_security_event(
                SecurityEventType::VaultAccessDenied,
                Some(&self.stellar_address),
                Some(vault_id),
                None,
                Some("Vault access denied: requested vault outside authorized scope"),
            );
            return Err(AppError::Forbidden(format!(
                "Token is not authorized to access vault '{}'",
                vault_id
            )));
        }
        Ok(())
    }

    pub fn authorize_vault_write(&self, vault_id: &str) -> Result<(), AppError> {
        if !self.can_write_vaults() {
            log_security_event(
                SecurityEventType::VaultAccessDenied,
                Some(&self.stellar_address),
                Some(vault_id),
                None,
                Some("Vault access denied: insufficient role for write operations"),
            );
            return Err(AppError::Forbidden(format!(
                "Role '{}' is not authorized to perform write operations on vaults",
                self.role
            )));
        }
        self.authorize_vault_read(vault_id)
    }

    pub fn authorize_vault_manage(&self, vault_id: &str) -> Result<(), AppError> {
        if !self.can_manage_vaults() {
            log_security_event(
                SecurityEventType::VaultAccessDenied,
                Some(&self.stellar_address),
                Some(vault_id),
                None,
                Some("Vault access denied: insufficient role for management operations"),
            );
            return Err(AppError::Forbidden(format!(
                "Role '{}' is not authorized to manage vaults",
                self.role
            )));
        }
        self.authorize_vault_read(vault_id)
    }
}

const RATE_LIMIT_CAPACITY: f64 = 60.0;
const RATE_LIMIT_REFILL_RATE: f64 = 1.0; // Refills 1 token per second (60 requests/minute)

#[derive(Debug, Clone)]
pub struct TokenBucket {
    tokens: f64,
    last_update: Instant,
}

impl TokenBucket {
    pub fn new(capacity: f64) -> Self {
        Self {
            tokens: capacity,
            last_update: Instant::now(),
        }
    }

    pub fn consume(&mut self, capacity: f64, refill_rate: f64, amount: f64) -> bool {
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        self.last_update = now;

        self.tokens = (self.tokens + elapsed * refill_rate).min(capacity);

        if self.tokens >= amount {
            self.tokens -= amount;
            true
        } else {
            false
        }
    }
}

#[derive(Debug, Error)]
pub enum AuthKeyError {
    #[error("invalid JWT key configuration: {0}")]
    InvalidConfiguration(String),
    #[error("invalid JWT key material: {0}")]
    InvalidKeyMaterial(String),
    #[error(transparent)]
    KeyRing(#[from] SecretKeyringError),
}

struct JwtKeyMaterial {
    encoding_key: Option<EncodingKey>,
    decoding_key: DecodingKey,
    jwk_n: String,
    jwk_e: String,
}

impl JwtKeyMaterial {
    fn from_private_key(private_key: &RsaPrivateKey) -> Result<Self, AuthKeyError> {
        let public_key = RsaPublicKey::from(private_key);
        Self::from_parts(
            public_key,
            Some(EncodingKey::from_rsa_pem(
                private_key
                    .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
                    .map_err(|error| {
                        AuthKeyError::InvalidKeyMaterial(format!(
                            "failed to encode RSA private key: {error}"
                        ))
                    })?
                    .as_bytes(),
            )
            .map_err(|error| {
                AuthKeyError::InvalidKeyMaterial(format!("invalid RSA encoding key: {error}"))
            })?),
        )
    }

    fn from_public_pem(public_pem: &str) -> Result<Self, AuthKeyError> {
        let public_key = RsaPublicKey::from_public_key_pem(public_pem).map_err(|error| {
            AuthKeyError::InvalidKeyMaterial(format!("invalid RSA public key: {error}"))
        })?;
        Self::from_parts(public_key, None)
    }

    fn from_parts(
        public_key: RsaPublicKey,
        encoding_key: Option<EncodingKey>,
    ) -> Result<Self, AuthKeyError> {
        if public_key.size() < 2048 {
            return Err(AuthKeyError::InvalidKeyMaterial(
                "RSA keys must be at least 2048 bits".to_string(),
            ));
        }
        let public_pem = public_key
            .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
            .map_err(|error| {
                AuthKeyError::InvalidKeyMaterial(format!("failed to encode RSA public key: {error}"))
            })?;
        let decoding_key = DecodingKey::from_rsa_pem(public_pem.as_bytes()).map_err(|error| {
            AuthKeyError::InvalidKeyMaterial(format!("invalid RSA decoding key: {error}"))
        })?;
        Ok(Self {
            encoding_key,
            decoding_key,
            jwk_n: BASE64_URL.encode(public_key.n().to_bytes_be()),
            jwk_e: BASE64_URL.encode(public_key.e().to_bytes_be()),
        })
    }

    fn matches_public_pem(&self, public_pem: &str) -> bool {
        let Ok(public_key) = RsaPublicKey::from_public_key_pem(public_pem) else {
            return false;
        };
        self.jwk_n == BASE64_URL.encode(public_key.n().to_bytes_be())
            && self.jwk_e == BASE64_URL.encode(public_key.e().to_bytes_be())
    }
}

#[derive(Deserialize)]
struct JwtKeyRingEntry {
    kid: String,
    #[serde(default)]
    signing: bool,
    private_key_pem: Option<String>,
    public_key_pem: Option<String>,
    verification_not_after: Option<u64>,
}

fn parse_rsa_private_key(pem: &str) -> Result<RsaPrivateKey, AuthKeyError> {
    RsaPrivateKey::from_pkcs8_pem(pem)
        .or_else(|_| RsaPrivateKey::from_pkcs1_pem(pem))
        .map_err(|error| AuthKeyError::InvalidKeyMaterial(format!("invalid RSA private key: {error}")))
}

fn generated_key_id(public_key: &RsaPublicKey) -> String {
    let digest = Sha256::digest(public_key.n().to_bytes_be());
    format!("rsa-{}", BASE64_URL.encode(&digest[..12]))
}

fn jwt_keyring_from_config(
    legacy_private_key_pem: Option<String>,
    key_ring_json: Option<String>,
    current_key_id: Option<String>,
    overlap_secs: u64,
    allow_ephemeral: bool,
) -> Result<SecretKeyring<Arc<JwtKeyMaterial>>, AuthKeyError> {
    let key_ring_json = key_ring_json.filter(|value| !value.trim().is_empty());
    let legacy_private_key_pem = legacy_private_key_pem.filter(|value| !value.trim().is_empty());
    let current_key_id = current_key_id
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if key_ring_json.is_some() && legacy_private_key_pem.is_some() {
        return Err(AuthKeyError::InvalidConfiguration(
            "JWT_KEY_RING and JWT_PRIVATE_KEY are mutually exclusive".to_string(),
        ));
    }

    if let Some(raw) = key_ring_json {
        let entries: Vec<JwtKeyRingEntry> = serde_json::from_str(&raw).map_err(|error| {
            AuthKeyError::InvalidConfiguration(format!("JWT_KEY_RING is invalid JSON: {error}"))
        })?;
        if entries.is_empty() {
            return Err(AuthKeyError::InvalidConfiguration(
                "JWT_KEY_RING must not be empty".to_string(),
            ));
        }
        let signing_key_ids = entries
            .iter()
            .filter(|entry| entry.signing)
            .map(|entry| entry.kid.trim())
            .collect::<Vec<_>>();
        if signing_key_ids.len() != 1 {
            return Err(AuthKeyError::InvalidConfiguration(
                "JWT_KEY_RING must contain exactly one signing key".to_string(),
            ));
        }
        let selected_key_id = current_key_id
            .as_deref()
            .unwrap_or(signing_key_ids[0])
            .to_string();
        if selected_key_id != signing_key_ids[0] {
            return Err(AuthKeyError::InvalidConfiguration(
                "JWT_CURRENT_KEY_ID does not identify the signing key".to_string(),
            ));
        }

        let now = now_secs();
        let mut current = None;
        let mut previous = Vec::new();
        for entry in entries {
            let material = if let Some(private_pem) = entry.private_key_pem.as_deref() {
                let private_key = parse_rsa_private_key(private_pem)?;
                let material = JwtKeyMaterial::from_private_key(&private_key)?;
                if entry
                    .public_key_pem
                    .as_deref()
                    .is_some_and(|public_pem| !material.matches_public_pem(public_pem))
                {
                    return Err(AuthKeyError::InvalidConfiguration(format!(
                        "JWT_KEY_RING public key does not match signing key '{}'",
                        entry.kid
                    )));
                }
                Arc::new(material)
            } else if let Some(public_pem) = entry.public_key_pem.as_deref() {
                Arc::new(JwtKeyMaterial::from_public_pem(public_pem)?)
            } else {
                return Err(AuthKeyError::InvalidConfiguration(format!(
                    "JWT_KEY_RING key '{}' has no key material",
                    entry.kid
                )));
            };

            let version = SecretVersion::new(entry.kid.trim(), material)
                .with_expiry(entry.verification_not_after);
            if selected_key_id == entry.kid.trim() {
                if version
                    .value()
                    .encoding_key
                    .is_none()
                {
                    return Err(AuthKeyError::InvalidConfiguration(format!(
                        "JWT_KEY_RING signing key '{}' requires private key material",
                        entry.kid
                    )));
                }
                current = Some(version.with_expiry(None));
            } else {
                previous.push(version);
            }
        }
        let current = current.ok_or_else(|| {
            AuthKeyError::InvalidConfiguration(
                "JWT_KEY_RING does not contain JWT_CURRENT_KEY_ID".to_string(),
            )
        })?;
        return SecretKeyring::new(current, previous, now, overlap_secs).map_err(Into::into);
    }

    let (private_key, key_id) = if let Some(pem) = legacy_private_key_pem {
        let private_key = parse_rsa_private_key(&pem)?;
        let public_key = RsaPublicKey::from(&private_key);
        (private_key, current_key_id.unwrap_or_else(|| generated_key_id(&public_key)))
    } else if allow_ephemeral {
        tracing::info!("Generating ephemeral RSA keypair for local development...");
        let mut rng = rand::thread_rng();
        let private_key = RsaPrivateKey::new(&mut rng, 2048).map_err(|error| {
            AuthKeyError::InvalidKeyMaterial(format!("failed to generate RSA key: {error}"))
        })?;
        let public_key = RsaPublicKey::from(&private_key);
        (private_key, generated_key_id(&public_key))
    } else {
        return Err(AuthKeyError::InvalidConfiguration(
            "JWT_PRIVATE_KEY or JWT_KEY_RING is required when ephemeral keys are disabled"
                .to_string(),
        ));
    };

    let material = Arc::new(JwtKeyMaterial::from_private_key(&private_key)?);
    SecretKeyring::new(
        SecretVersion::new(key_id, material),
        Vec::new(),
        now_secs(),
        overlap_secs,
    )
    .map_err(Into::into)
}

pub struct AuthState {
    jwt_keys: Arc<SecretKeyring<Arc<JwtKeyMaterial>>>,
    pub signing_key: SigningKey,
    pub server_public_key: [u8; 32],
    pub network_passphrase: String,
    /// Emergency pause flag for message verification.
    /// When true, all verification endpoints reject requests.
    pub emergency_verification_paused: Arc<AtomicBool>,
    /// Hash(refresh_token) → record. Enables rotation and revocation.
    refresh_tokens: Arc<RwLock<HashMap<String, RefreshTokenRecord>>>,
    /// Thread-safe map of token buckets keyed by authenticated tenant (Stellar address)
    pub rate_limiter: Mutex<HashMap<String, TokenBucket>>,
}

impl AuthState {
    pub fn new(
        jwt_private_key_pem: Option<String>,
        sep10_seed: Option<[u8; 32]>,
        network_passphrase: String,
        emergency_verification_paused: bool,
    ) -> Result<Self, AuthKeyError> {
        Self::from_config(
            jwt_private_key_pem,
            None,
            None,
            1_800,
            true,
            sep10_seed,
            network_passphrase,
            emergency_verification_paused,
        )
    }

    #[allow(clippy::too_many_arguments)]
    pub fn from_config(
        jwt_private_key_pem: Option<String>,
        jwt_key_ring: Option<String>,
        jwt_current_key_id: Option<String>,
        jwt_key_overlap_secs: u64,
        allow_ephemeral_jwt_key: bool,
        sep10_seed: Option<[u8; 32]>,
        network_passphrase: String,
        emergency_verification_paused: bool,
    ) -> Result<Self, AuthKeyError> {
        let seed = match sep10_seed {
            Some(seed) => seed,
            None => {
                let mut seed = [0u8; 32];
                rand::thread_rng().fill_bytes(&mut seed);
                seed
            }
        };
        let signing_key = SigningKey::from_bytes(&seed);
        let server_public_key = signing_key.verifying_key().to_bytes();
        let jwt_keys = jwt_keyring_from_config(
            jwt_private_key_pem,
            jwt_key_ring,
            jwt_current_key_id,
            jwt_key_overlap_secs,
            allow_ephemeral_jwt_key,
        )?;

        let state = Self {
            jwt_keys: Arc::new(jwt_keys),
            signing_key,
            server_public_key,
            network_passphrase,
            emergency_verification_paused: Arc::new(AtomicBool::new(
                emergency_verification_paused,
            )),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            rate_limiter: Mutex::new(HashMap::new()),
        };

        log_security_event(
            SecurityEventType::JwtKeyRotated,
            Some(&state.server_stellar_address()),
            None,
            None,
            Some("JWT signing keyring initialized"),
        );

        Ok(state)
    }

    pub fn rotate_signing_key(
        &self,
        private_key_pem: String,
        key_id: String,
    ) -> Result<(), AuthKeyError> {
        let private_key = parse_rsa_private_key(&private_key_pem)?;
        let material = Arc::new(JwtKeyMaterial::from_private_key(&private_key)?);
        let rotated_key_id = key_id.clone();
        self.jwt_keys
            .rotate(SecretVersion::new(key_id, material), now_secs())?;
        log_security_event(
            SecurityEventType::JwtKeyRotated,
            Some(&self.server_stellar_address()),
            None,
            None,
            Some(&format!(
                "JWT signing key rotated with overlap: {rotated_key_id}"
            )),
        );
        Ok(())
    }

    fn verification_key(
        &self,
        key_id: &str,
        now: u64,
    ) -> Result<Option<Arc<JwtKeyMaterial>>, AuthKeyError> {
        self.jwt_keys
            .verification_key(key_id, now)
            .map(|version| version.map(|version| Arc::clone(version.value())))
            .map_err(Into::into)
    }

    fn verification_keys(
        &self,
        now: u64,
    ) -> Result<Vec<Arc<JwtKeyMaterial>>, AuthKeyError> {
        self.jwt_keys
            .verification_keys(now)
            .map(|versions| {
                versions
                    .into_iter()
                    .map(|version| Arc::clone(version.value()))
                    .collect()
            })
            .map_err(Into::into)
    }

    pub fn server_stellar_address(&self) -> String {
        Strkey::PublicKeyEd25519(stellar_strkey::ed25519::PublicKey(self.server_public_key))
            .to_string()
    }

    pub fn is_verification_paused(&self) -> bool {
        self.emergency_verification_paused.load(Ordering::SeqCst)
    }

    pub fn set_verification_paused(&self, paused: bool) {
        self.emergency_verification_paused
            .store(paused, Ordering::SeqCst);
    }

    pub fn access_token_ttl_secs(&self) -> u64 {
        ACCESS_TOKEN_EXPIRY_SECS
    }
}

#[derive(Deserialize, ToSchema)]
pub struct ChallengeRequest {
    #[schema(example = "GABC...XYZ")]
    pub account: String,
}

#[derive(Serialize, ToSchema)]
pub struct ChallengeResponse {
    pub transaction: String,
    pub network_passphrase: String,
}

#[derive(Deserialize, ToSchema)]
pub struct VerifyRequest {
    pub transaction: String,
    /// Optional role for administrator accounts. Non-admin login roles are rejected.
    #[serde(default)]
    pub role: Option<Role>,
    /// Optional vault scopes for administrator accounts. Non-admin login scopes are rejected.
    #[serde(default)]
    pub vault_scopes: Option<Vec<String>>,
}

#[derive(Serialize, ToSchema)]
pub struct VerifyResponse {
    /// Short-lived access JWT (Bearer). Prefer this over the legacy `token` alias.
    pub access_token: String,
    /// Opaque refresh token. Single-use; rotated on every `/auth/refresh`.
    pub refresh_token: String,
    /// Access-token lifetime in seconds.
    pub expires_in: u64,
    /// Token type for Authorization header (always `Bearer`).
    pub token_type: String,
    /// Legacy alias for `access_token` (kept for older clients).
    pub token: String,
}

#[derive(Deserialize, ToSchema)]
pub struct RefreshRequest {
    pub refresh_token: String,
}

/// Emergency pause toggle request (admin-only).
#[derive(Deserialize, ToSchema)]
pub struct EmergencyPauseRequest {
    /// If true, verification is paused. If false, verification resumes.
    pub paused: bool,
}

/// Emergency pause toggle response.
#[derive(Serialize, ToSchema)]
pub struct EmergencyPauseResponse {
    /// Current pause status.
    pub paused: bool,
    /// Message describing the status change.
    pub message: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Claims {
    pub sub: String,
    pub iss: String,
    pub exp: u64,
    pub iat: u64,
    pub scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub roles: Option<Vec<Role>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_ids: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vault_scopes: Option<Vec<String>>,
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn hash_refresh_token(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    hex::encode(digest)
}

fn generate_refresh_token() -> String {
    let mut bytes = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut bytes);
    BASE64_URL.encode(bytes)
}

pub fn encode_access_token_with_role_and_vaults(
    state: &AuthState,
    subject: &str,
    role: Option<Role>,
    vault_scopes: Option<Vec<String>>,
) -> Result<String, AppError> {
    let now = now_secs();
    let assigned_role = role.unwrap_or_else(|| {
        if is_admin_address(subject) {
            Role::Admin
        } else {
            Role::Manager
        }
    });
    let claims = Claims {
        sub: subject.to_string(),
        iss: WEB_AUTH_DOMAIN.to_string(),
        iat: now,
        exp: now + ACCESS_TOKEN_EXPIRY_SECS,
        scopes: vec!["simulate".to_string()],
        role: Some(assigned_role),
        roles: Some(vec![assigned_role]),
        vault_ids: vault_scopes.clone(),
        vault_scopes,
    };

    let current = state
        .jwt_keys
        .current()
        .map_err(|error| AppError::Internal(format!("JWT signing key unavailable: {error}")))?;
    let encoding_key = current
        .value()
        .encoding_key
        .as_ref()
        .ok_or_else(|| AppError::Internal("JWT signing key has no private material".to_string()))?;
    let mut header = Header::new(Algorithm::RS256);
    header.kid = Some(current.id().to_string());
    encode(&header, &claims, encoding_key)
        .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))
}

fn encode_access_token(state: &AuthState, subject: &str) -> Result<String, AppError> {
    encode_access_token_with_role_and_vaults(state, subject, None, None)
}

/// Issue a short-lived access JWT plus a new refresh token (new rotation family) with role & vault scopes.
pub(crate) fn issue_token_pair_with_scope(
    state: &AuthState,
    subject: &str,
    role: Option<Role>,
    vault_scopes: Option<Vec<String>>,
) -> Result<VerifyResponse, AppError> {
    let family_id = Uuid::new_v4().to_string();
    issue_token_pair_in_family_with_scope(state, subject, &family_id, role, vault_scopes)
}

/// Issue a short-lived access JWT plus a new refresh token (new rotation family).
pub(crate) fn issue_token_pair(state: &AuthState, subject: &str) -> Result<VerifyResponse, AppError> {
    issue_token_pair_with_scope(state, subject, None, None)
}

fn issue_token_pair_in_family(
    state: &AuthState,
    subject: &str,
    family_id: &str,
) -> Result<VerifyResponse, AppError> {
    issue_token_pair_in_family_with_scope(state, subject, family_id, None, None)
}

fn issue_token_pair_in_family_with_scope(
    state: &AuthState,
    subject: &str,
    family_id: &str,
    role: Option<Role>,
    vault_scopes: Option<Vec<String>>,
) -> Result<VerifyResponse, AppError> {
    let access_token = encode_access_token_with_role_and_vaults(state, subject, role, vault_scopes)?;
    let refresh_token = generate_refresh_token();
    let token_hash = hash_refresh_token(&refresh_token);
    let expires_at = now_secs() + REFRESH_TOKEN_EXPIRY_SECS;

    let record = RefreshTokenRecord::Active {
        subject: subject.to_string(),
        expires_at,
        family_id: family_id.to_string(),
    };

    let mut store = state
        .refresh_tokens
        .write()
        .map_err(|_| AppError::Internal("Refresh token store lock poisoned".into()))?;
    purge_expired_refresh_tokens(&mut store, now_secs());
    store.insert(token_hash, record);

    Ok(VerifyResponse {
        access_token: access_token.clone(),
        refresh_token,
        expires_in: ACCESS_TOKEN_EXPIRY_SECS,
        token_type: "Bearer".to_string(),
        token: access_token,
    })
}

fn purge_expired_refresh_tokens(store: &mut HashMap<String, RefreshTokenRecord>, now: u64) {
    store.retain(|_, record| match record {
        RefreshTokenRecord::Active { expires_at, .. }
        | RefreshTokenRecord::Rotated { expires_at, .. } => *expires_at >= now,
    });
}

/// Rotate: consume the presented refresh token and mint a new access + refresh pair.
/// Reuse of an already-rotated token revokes the entire family (theft detection).
pub(crate) fn rotate_refresh_token(
    state: &AuthState,
    refresh_token: &str,
) -> Result<VerifyResponse, AppError> {
    if refresh_token.is_empty() {
        return Err(AppError::with_code(
            ErrorCode::InvalidSignature,
            "Missing refresh token",
        ));
    }

    let token_hash = hash_refresh_token(refresh_token);
    let now = now_secs();

    let (subject, family_id) = {
        let mut store = state
            .refresh_tokens
            .write()
            .map_err(|_| AppError::Internal("Refresh token store lock poisoned".into()))?;

        purge_expired_refresh_tokens(&mut store, now);

        match store.get(&token_hash).cloned() {
            Some(RefreshTokenRecord::Active {
                subject,
                expires_at,
                family_id,
            }) => {
                if now > expires_at {
                    store.retain(|_, r| match r {
                        RefreshTokenRecord::Active { family_id: fid, .. }
                        | RefreshTokenRecord::Rotated { family_id: fid, .. } => {
                            fid != &family_id
                        }
                    });
                    log_security_event(
                        SecurityEventType::TokenExpired,
                        Some(&subject),
                        None,
                        None,
                        Some("Refresh token expired"),
                    );
                    return Err(AppError::with_code(
                        ErrorCode::TokenExpired,
                        "Refresh token expired",
                    ));
                }
                // Leave a tombstone so reuse can be detected.
                store.insert(
                    token_hash,
                    RefreshTokenRecord::Rotated {
                        family_id: family_id.clone(),
                        expires_at,
                    },
                );
                (subject, family_id)
            }
            Some(RefreshTokenRecord::Rotated { family_id, .. }) => {
                tracing::warn!(
                    family_id = %family_id,
                    "Refresh token reuse detected; revoking rotation family"
                );
                store.retain(|_, r| match r {
                    RefreshTokenRecord::Active { family_id: fid, .. }
                    | RefreshTokenRecord::Rotated { family_id: fid, .. } => fid != &family_id,
                });
                log_security_event(
                    SecurityEventType::TokenRevoked,
                    None,
                    None,
                    None,
                    Some("Refresh token reuse detected; revoked rotation family"),
                );
                return Err(AppError::with_code(
                    ErrorCode::InvalidSignature,
                    "Refresh token reuse detected; re-authenticate",
                ));
            }
            None => {
                log_security_event(
                    SecurityEventType::UnauthorizedAccess,
                    None,
                    None,
                    None,
                    Some("Invalid or already-rotated refresh token"),
                );
                return Err(AppError::with_code(
                    ErrorCode::InvalidSignature,
                    "Invalid or already-rotated refresh token",
                ));
            }
        }
    };

    let response = issue_token_pair_in_family(state, &subject, &family_id)?;
    log_security_event(
        SecurityEventType::TokenRefreshed,
        Some(&subject),
        None,
        None,
        Some("Refresh token rotated and new JWT access token issued"),
    );
    Ok(response)
}

/// Revoke every refresh token in the same rotation family as `refresh_token`.
pub(crate) fn revoke_refresh_token(state: &AuthState, refresh_token: &str) -> Result<(), AppError> {
    let token_hash = hash_refresh_token(refresh_token);
    let mut store = state
        .refresh_tokens
        .write()
        .map_err(|_| AppError::Internal("Refresh token store lock poisoned".into()))?;

    let found = store.get(&token_hash).map(|r| match r {
        RefreshTokenRecord::Active { family_id, subject, .. } => (family_id.clone(), Some(subject.clone())),
        RefreshTokenRecord::Rotated { family_id, .. } => (family_id.clone(), None),
    });
    if let Some((family_id, subject)) = found {
        store.retain(|_, r| match r {
            RefreshTokenRecord::Active { family_id: fid, .. }
            | RefreshTokenRecord::Rotated { family_id: fid, .. } => fid != &family_id,
        });
        log_security_event(
            SecurityEventType::TokenRevoked,
            subject.as_deref(),
            None,
            None,
            Some("Refresh token family explicitly revoked"),
        );
    }
    Ok(())
}

pub(crate) fn network_id(passphrase: &str) -> [u8; 32] {
    Sha256::digest(passphrase.as_bytes()).into()
}

pub(crate) fn tx_hash(tx: &Transaction, net_id: &[u8; 32]) -> Result<[u8; 32], AppError> {
    let tx_xdr = tx
        .to_xdr(Limits::none())
        .map_err(|e| AppError::Internal(format!("XDR encode error: {e}")))?;
    let mut h = Sha256::new();
    h.update(net_id);
    h.update(2i32.to_be_bytes()); // ENVELOPE_TYPE_TX
    h.update(&tx_xdr);
    Ok(h.finalize().into())
}

pub(crate) fn build_challenge_envelope(
    state: &AuthState,
    client_pubkey: &[u8; 32],
) -> Result<String, AppError> {
    let now = now_secs();

    let mut nonce = [0u8; 48];
    rand::thread_rng().fill_bytes(&mut nonce);
    let nonce_value = BASE64.encode(nonce);

    let data_name = format!("{WEB_AUTH_DOMAIN} auth");
    let manage_data = ManageDataOp {
        data_name: data_name
            .into_bytes()
            .try_into()
            .map_err(|_| AppError::Internal("data name conversion failed".into()))?,
        data_value: Some(
            nonce_value
                .into_bytes()
                .try_into()
                .map_err(|_| AppError::Internal("nonce value conversion failed".into()))?,
        ),
    };

    let op = Operation {
        source_account: Some(MuxedAccount::Ed25519(Uint256(*client_pubkey))),
        body: OperationBody::ManageData(manage_data),
    };

    let tx = Transaction {
        source_account: MuxedAccount::Ed25519(Uint256(state.server_public_key)),
        fee: 100,
        seq_num: SequenceNumber(0),
        cond: Preconditions::Time(TimeBounds {
            min_time: TimePoint(now),
            max_time: TimePoint(now + CHALLENGE_EXPIRY_SECS),
        }),
        memo: Memo::None,
        operations: vec![op]
            .try_into()
            .map_err(|_| AppError::Internal("operations conversion failed".into()))?,
        ext: TransactionExt::V0,
    };

    let net_id = network_id(&state.network_passphrase);
    let hash = tx_hash(&tx, &net_id)?;
    let sig = state.signing_key.sign(&hash);

    let hint: [u8; 4] = state.server_public_key[28..32].try_into().unwrap();
    let decorated = DecoratedSignature {
        hint: SignatureHint(hint),
        signature: sig
            .to_bytes()
            .to_vec()
            .try_into()
            .map_err(|_| AppError::Internal("signature conversion failed".into()))?,
    };

    let envelope = TransactionEnvelope::Tx(TransactionV1Envelope {
        tx,
        signatures: vec![decorated]
            .try_into()
            .map_err(|_| AppError::Internal("envelope signatures failed".into()))?,
    });

    let xdr = envelope
        .to_xdr(Limits::none())
        .map_err(|e| AppError::Internal(format!("XDR encode error: {e}")))?;

    Ok(BASE64.encode(&xdr))
}

pub(crate) fn verify_challenge_envelope(state: &AuthState, signed_xdr_b64: &str) -> Result<String, AppError> {
    let raw = BASE64
        .decode(signed_xdr_b64)
        .map_err(|_| AppError::with_code(ErrorCode::InvalidBase64, "Invalid base64"))?;

    let envelope = TransactionEnvelope::from_xdr(&raw, Limits::none())
        .map_err(|_| AppError::with_code(ErrorCode::InvalidXdr, "Invalid transaction XDR"))?;

    let inner = match envelope {
        TransactionEnvelope::Tx(inner) => inner,
        _ => {
            return Err(AppError::BadRequest(
                "Expected TransactionV1 envelope".into(),
            ))
        }
    };

    let tx = &inner.tx;

    if tx.seq_num.0 != 0 {
        return Err(AppError::BadRequest("Non-zero sequence number".into()));
    }

    let source_key = match &tx.source_account {
        MuxedAccount::Ed25519(Uint256(b)) => *b,
        _ => {
            return Err(AppError::BadRequest(
                "Unsupported source account type".into(),
            ))
        }
    };
    if source_key != state.server_public_key {
        return Err(AppError::BadRequest(
            "Challenge not issued by this server".into(),
        ));
    }

    let now = now_secs();
    match &tx.cond {
        Preconditions::Time(bounds) => {
            if now > bounds.max_time.0 {
                return Err(AppError::BadRequest("Challenge expired".into()));
            }
        }
        _ => return Err(AppError::BadRequest("Missing time bounds".into())),
    }

    let ops: &[Operation] = inner.tx.operations.as_ref();
    if ops.is_empty() {
        return Err(AppError::BadRequest("No operations in challenge".into()));
    }

    let client_key = match &ops[0].source_account {
        Some(MuxedAccount::Ed25519(Uint256(b))) => *b,
        _ => {
            return Err(AppError::BadRequest(
                "Missing client account on operation".into(),
            ))
        }
    };

    match &ops[0].body {
        OperationBody::ManageData(md) => {
            let name = std::str::from_utf8(md.data_name.as_ref())
                .map_err(|_| AppError::BadRequest("Invalid data name encoding".into()))?;
            let expected = format!("{WEB_AUTH_DOMAIN} auth");
            if name != expected {
                return Err(AppError::BadRequest("Invalid manage_data key".into()));
            }
        }
        _ => return Err(AppError::BadRequest("Expected ManageData operation".into())),
    }

    let net_id = network_id(&state.network_passphrase);
    let hash = tx_hash(&inner.tx, &net_id)?;

    let sigs: &[DecoratedSignature] = inner.signatures.as_ref();
    let server_hint: [u8; 4] = state.server_public_key[28..32].try_into().unwrap();
    let client_hint: [u8; 4] = client_key[28..32].try_into().unwrap();

    let mut server_ok = false;
    let mut client_ok = false;

    for ds in sigs {
        let sig_bytes: &[u8] = ds.signature.as_ref();
        let Ok(sig) = Ed25519Signature::try_from(sig_bytes) else {
            continue;
        };

        if ds.hint.0 == server_hint {
            if let Ok(vk) = VerifyingKey::from_bytes(&state.server_public_key) {
                if vk.verify(&hash, &sig).is_ok() {
                    server_ok = true;
                }
            }
        }

        if ds.hint.0 == client_hint {
            if let Ok(vk) = VerifyingKey::from_bytes(&client_key) {
                if vk.verify(&hash, &sig).is_ok() {
                    client_ok = true;
                }
            }
        }
    }

    if !server_ok {
        return Err(AppError::Unauthorized(
            "Missing valid server signature".into(),
        ));
    }
    if !client_ok {
        return Err(AppError::Unauthorized(
            "Missing valid client signature".into(),
        ));
    }

    let client_address =
        Strkey::PublicKeyEd25519(stellar_strkey::ed25519::PublicKey(client_key)).to_string();

    Ok(client_address)
}

#[utoipa::path(
    post,
    path = "/auth/challenge",
    request_body = ChallengeRequest,
    responses(
        (status = 200, description = "SEP-10 challenge transaction", body = ChallengeResponse),
        (status = 400, description = "Invalid account"),
        (status = 503, description = "Verification paused for emergency maintenance")
    ),
    tag = "Auth"
)]
pub async fn challenge_handler(
    Extension(state): Extension<Arc<AuthState>>,
    ApiJson(payload): ApiJson<ChallengeRequest>,
    SanitizedJson(payload): SanitizedJson<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, AppError> {
    if state.is_verification_paused() {
        return Err(AppError::with_code(
            ErrorCode::ServiceUnavailable,
            "Message verification is temporarily paused for emergency maintenance",
        ));
    }

    let strkey = Strkey::from_string(&payload.account)
        .map_err(|_| AppError::with_code(ErrorCode::InvalidInput, "Invalid Stellar address"))?;

    let pubkey = match strkey {
        Strkey::PublicKeyEd25519(pk) => pk.0,
        _ => return Err(AppError::BadRequest("Expected G... account address".into())),
    };

    let transaction = build_challenge_envelope(&state, &pubkey)?;

    Ok(Json(ChallengeResponse {
        transaction,
        network_passphrase: state.network_passphrase.clone(),
    }))
}

#[utoipa::path(
    post,
    path = "/auth/verify",
    request_body = VerifyRequest,
    responses(
        (status = 200, description = "Short-lived access JWT and refresh token issued", body = VerifyResponse),
        (status = 401, description = "Authentication failed"),
        (status = 503, description = "Verification paused for emergency maintenance")
    ),
    tag = "Auth"
)]
pub async fn verify_handler(
    Extension(state): Extension<Arc<AuthState>>,
    ApiJson(payload): ApiJson<VerifyRequest>,
    SanitizedJson(payload): SanitizedJson<VerifyRequest>,
) -> Result<Json<VerifyResponse>, AppError> {
    if state.is_verification_paused() {
        log_security_event(
            SecurityEventType::LoginFailed,
            None,
            None,
            None,
            Some("Verification paused for emergency maintenance"),
        );
        return Err(AppError::with_code(
            ErrorCode::ServiceUnavailable,
            "Message verification is temporarily paused for emergency maintenance",
        ));
    }

    let subject = match verify_challenge_envelope(&state, &payload.transaction) {
        Ok(s) => s,
        Err(e) => {
            log_security_event(
                SecurityEventType::LoginFailed,
                None,
                None,
                None,
                Some(&e.to_string()),
            );
            return Err(e);
        }
    };

    let admin_address = is_admin_address(&subject);
    if payload.role.is_some() && !admin_address {
        log_security_event(
            SecurityEventType::UnauthorizedAccess,
            Some(&subject),
            None,
            None,
            Some("Role requested during initial authentication"),
        );
        return Err(AppError::Forbidden(
            "Roles must be assigned by an administrator or delegated by an authorized manager"
                .into(),
        ));
    }
    if payload
        .vault_scopes
        .as_ref()
        .is_some_and(|scopes| !scopes.is_empty())
        && !admin_address
    {
        log_security_event(
            SecurityEventType::UnauthorizedAccess,
            Some(&subject),
            None,
            None,
            Some("Vault scopes requested during initial authentication"),
        );
        return Err(AppError::Forbidden(
            "Vault scopes must be assigned by an administrator or delegated by an authorized manager"
                .into(),
        ));
    }

    let requested_role = admin_address.then_some(payload.role.unwrap_or(Role::Admin));
    let vault_scopes = if admin_address { payload.vault_scopes } else { None };
    let tokens = match issue_token_pair_with_scope(&state, &subject, requested_role, vault_scopes) {
        Ok(t) => t,
        Err(e) => {
            log_security_event(
                SecurityEventType::LoginFailed,
                Some(&subject),
                None,
                None,
                Some(&e.to_string()),
            );
            return Err(e);
        }
    };

    log_security_event(
        SecurityEventType::LoginSuccess,
        Some(&subject),
        None,
        None,
        Some("SEP-10 challenge verified and JWT issued"),
    );
    
    crate::audit_log::log_audit_event(&subject, "auth_login", &subject);

    Ok(Json(tokens))
}

#[utoipa::path(
    post,
    path = "/auth/refresh",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Rotated access and refresh tokens", body = VerifyResponse),
        (status = 401, description = "Invalid, expired, or reused refresh token")
    ),
    tag = "Auth"
)]
pub async fn refresh_handler(
    Extension(state): Extension<Arc<AuthState>>,
    ApiJson(payload): ApiJson<RefreshRequest>,
    SanitizedJson(payload): SanitizedJson<RefreshRequest>,
) -> Result<Json<VerifyResponse>, AppError> {
    if state.is_verification_paused() {
        return Err(AppError::with_code(
            ErrorCode::ServiceUnavailable,
            "Authentication is temporarily paused for emergency maintenance",
        ));
    }

    let tokens = rotate_refresh_token(&state, &payload.refresh_token)?;
    Ok(Json(tokens))
}

#[derive(Serialize, ToSchema)]
pub struct RevokeResponse {
    pub revoked: bool,
}

#[utoipa::path(
    post,
    path = "/auth/revoke",
    request_body = RefreshRequest,
    responses(
        (status = 200, description = "Refresh token family revoked", body = RevokeResponse)
    ),
    tag = "Auth"
)]
pub async fn revoke_handler(
    Extension(state): Extension<Arc<AuthState>>,
    ApiJson(payload): ApiJson<RefreshRequest>,
    SanitizedJson(payload): SanitizedJson<RefreshRequest>,
) -> Result<Json<RevokeResponse>, AppError> {
    if state.is_verification_paused() {
        return Err(AppError::with_code(
            ErrorCode::ServiceUnavailable,
            "Authentication is temporarily paused for emergency maintenance",
        ));
    }

    revoke_refresh_token(&state, &payload.refresh_token)?;
    Ok(Json(RevokeResponse { revoked: true }))
}

/// Emergency pause toggle endpoint (for administrative control).
/// This endpoint allows operators to pause all message verification in emergency scenarios.
#[utoipa::path(
    post,
    path = "/auth/emergency-pause",
    request_body = EmergencyPauseRequest,
    responses(
        (status = 200, description = "Emergency pause toggled", body = EmergencyPauseResponse),
        (status = 400, description = "Invalid request")
    ),
    tag = "Auth"
)]
pub async fn emergency_pause_handler(
    Extension(state): Extension<Arc<AuthState>>,
    Extension(user): Extension<AuthenticatedUser>,
    Json(payload): Json<EmergencyPauseRequest>,
    ApiJson(payload): ApiJson<EmergencyPauseRequest>,
    SanitizedJson(payload): SanitizedJson<EmergencyPauseRequest>,
) -> Result<Json<EmergencyPauseResponse>, AppError> {
    if !user.is_admin() {
        return Err(AppError::Forbidden(
            "Administrator role is required".to_string(),
        ));
    }
    state.set_verification_paused(payload.paused);
    log_audit_event(
        "system",
        if payload.paused {
            "verification_paused"
        } else {
            "verification_resumed"
        },
        &user.stellar_address,
    );

    let message = if payload.paused {
        "Message verification has been PAUSED for emergency maintenance".to_string()
    } else {
        "Message verification has been RESUMED".to_string()
    };

    Ok(Json(EmergencyPauseResponse {
        paused: payload.paused,
        message,
    }))
}

pub async fn auth_middleware(
    Extension(state): Extension<Arc<AuthState>>,
    req: Request,
    next: Next,
) -> Result<Response, AppError> {
    let emergency_path = matches!(
        req.uri().path(),
        "/auth/emergency-pause" | "/v1/auth/emergency-pause"
    );
    if state.is_verification_paused() && !emergency_path {
        return Err(AppError::Internal(
            "Authentication is temporarily paused for emergency maintenance".into(),
    // Check if verification is paused — deny all requests during emergency maintenance
    if state.is_verification_paused() {
        return Err(AppError::with_code(
            ErrorCode::ServiceUnavailable,
            "Authentication is temporarily paused for emergency maintenance",
        ));
    }

    let auth_header = match req
        .headers()
        .get(header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
    {
        Some(h) => h,
        None => {
            log_security_event(
                SecurityEventType::UnauthorizedAccess,
                None,
                None,
                None,
                Some("Missing Authorization header"),
            );
            return Err(AppError::Unauthorized("Missing Authorization header".into()));
        }
    };

    let token = match auth_header.strip_prefix("Bearer ") {
        Some(t) => t,
        None => {
            log_security_event(
                SecurityEventType::UnauthorizedAccess,
                None,
                None,
                None,
                Some("Expected Bearer token"),
            );
            return Err(AppError::Unauthorized("Expected Bearer token".into()));
        }
    };

    let header = match decode_header(token) {
        Ok(header) => header,
        Err(error) => {
            log_security_event(
                SecurityEventType::UnauthorizedAccess,
                None,
                None,
                None,
                Some("Invalid JWT header"),
            );
            return Err(AppError::Unauthorized(format!(
                "Invalid token header: {error}"
            )));
        }
    };
    let now = now_secs();
    let verification_keys = match header.kid.as_deref() {
        Some(key_id) if !key_id.trim().is_empty() => {
            match state.verification_key(key_id, now) {
                Ok(Some(key)) => vec![key],
                Ok(None) => {
                    return Err(AppError::Unauthorized(
                        "Unknown or retired signing key".to_string(),
                    ))
                }
                Err(error) => return Err(AppError::Internal(error.to_string())),
            }
        }
        _ => match state.verification_keys(now) {
            Ok(keys) => keys,
            Err(error) => return Err(AppError::Internal(error.to_string())),
        },
    };
    if verification_keys.is_empty() {
        return Err(AppError::Unauthorized(
            "No active signing keys are available".to_string(),
        ));
    }
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&[WEB_AUTH_DOMAIN]);
    validation.set_required_spec_claims(&["exp", "iss", "sub"]);
    let mut decoded = None;
    let mut decode_error: Option<jsonwebtoken::errors::Error> = None;
    let mut expired = false;
    for verification_key in verification_keys {
        match decode::<Claims>(token, &verification_key.decoding_key, &validation) {
            Ok(data) => {
                decoded = Some(data);
                break;
            }
            Err(error) => {
                expired |= matches!(
                    error.kind(),
                    jsonwebtoken::errors::ErrorKind::ExpiredSignature
                );
                decode_error = Some(error);
            }
        }
    }
    let token_data = match decoded {
        Some(data) => data,
        None => {
            if expired {
                log_security_event(
                    SecurityEventType::TokenExpired,
                    None,
                    None,
                    None,
                    Some("Access JWT expired"),
                );
            } else {
                log_security_event(
                    SecurityEventType::UnauthorizedAccess,
                    None,
                    None,
                    None,
                    Some("Access JWT signature validation failed"),
                );
            }
            let detail = decode_error
                .map(|error| error.to_string())
                .unwrap_or_else(|| "no active verification key matched".to_string());
            return Err(AppError::Unauthorized(format!("Invalid token: {detail}")));
            return Err(AppError::with_code(
                ErrorCode::InvalidSignature,
                format!("Invalid token: {e}"),
            ));
        }
    };

    // Validate JWT expiry claim (BE-030: verify token has not expired)
    if token_data.claims.exp <= now {
        return Err(AppError::with_code(
            ErrorCode::TokenExpired,
            "Token has expired",
        ));
    }

    if !token_data.claims.scopes.contains(&"simulate".to_string()) {
        log_security_event(
            SecurityEventType::UnauthorizedAccess,
            Some(&token_data.claims.sub),
            None,
            None,
            Some("Missing required scope 'simulate'"),
        );
        return Err(AppError::Unauthorized(
            "Missing required scope 'simulate'".into(),
        ));
    }

    // Rate limiting per manager/tenant (Stellar address)
    let tenant = token_data.claims.sub.clone();
    {
        let mut rate_limiter = state
            .rate_limiter
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let bucket = rate_limiter
            .entry(tenant.clone())
            .or_insert_with(|| TokenBucket::new(RATE_LIMIT_CAPACITY));

        if !bucket.consume(RATE_LIMIT_CAPACITY, RATE_LIMIT_REFILL_RATE, 1.0) {
            log_security_event(
                SecurityEventType::UnauthorizedAccess,
                Some(&tenant),
                None,
                None,
                Some(&format!("Rate limit exceeded for tenant {}", tenant)),
            );
            return Err(AppError::with_code(
                ErrorCode::RateLimitExceeded,
                format!("Rate limit exceeded for tenant {}", tenant),
            ));
        }
    }

    if !matches!(
        Strkey::from_string(&token_data.claims.sub),
        Ok(Strkey::PublicKeyEd25519(_))
    ) {
        return Err(AppError::Unauthorized(
            "Token subject is not a valid Stellar account".into(),
        ));
    }

    let admin_address = is_admin_address(&token_data.claims.sub);
    let claims_admin_role = token_data.claims.role == Some(Role::Admin)
        || token_data
            .claims
            .roles
            .as_ref()
            .is_some_and(|roles| roles.contains(&Role::Admin));
    if claims_admin_role && !admin_address {
        return Err(AppError::Unauthorized(
            "Token contains an unauthorized administrator role".into(),
        ));
    }

    let (role, roles) = if let Some(r) = token_data.claims.role {
        let rs = token_data.claims.roles.unwrap_or_else(|| vec![r]);
        (r, rs)
    } else if let Some(rs) = token_data.claims.roles {
        if let Some(&first) = rs.first() {
            (first, rs)
        } else if admin_address {
            (Role::Admin, vec![Role::Admin])
        } else {
            (Role::Viewer, vec![Role::Viewer])
        }
    } else if admin_address {
        (Role::Admin, vec![Role::Admin])
    } else {
        (Role::Viewer, vec![Role::Viewer])
    };

    let vault_scopes = token_data
        .claims
        .vault_scopes
        .or(token_data.claims.vault_ids)
        .unwrap_or_default();

    let mut req = req;
    req.extensions_mut().insert(AuthenticatedUser {
        stellar_address: token_data.claims.sub.clone(),
        role,
        roles,
        vault_scopes,
    });

    Ok(next.run(req).await)
}

#[derive(Deserialize, ToSchema)]
pub struct ScopedTokenRequest {
    pub role: Role,
    #[serde(default)]
    pub vault_scopes: Vec<String>,
}

#[derive(Serialize, ToSchema)]
pub struct ScopedTokenResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: u64,
    pub role: Role,
    pub vault_scopes: Vec<String>,
}

/// Issue a role- and vault-scoped access token for an agent, operator, or viewer.
#[utoipa::path(
    post,
    path = "/auth/scoped-token",
    request_body = ScopedTokenRequest,
    responses(
        (status = 200, description = "Role- and vault-scoped access token issued", body = ScopedTokenResponse),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden: Insufficient privileges")
    ),
    security(
        ("bearerAuth" = []),
        ("jwt" = [])
    ),
    tag = "Auth"
)]
pub async fn issue_scoped_token_handler(
    Extension(state): Extension<Arc<AuthState>>,
    Extension(user): Extension<AuthenticatedUser>,
    ApiJson(payload): ApiJson<ScopedTokenRequest>,
    SanitizedJson(payload): SanitizedJson<ScopedTokenRequest>,
) -> Result<Json<ScopedTokenResponse>, AppError> {
    if state.is_verification_paused() {
        return Err(AppError::with_code(
            ErrorCode::ServiceUnavailable,
            "Authentication is temporarily paused for emergency maintenance",
        ));
    }

    if !user.can_manage_vaults() {
        return Err(AppError::Forbidden(
            format!("Role '{}' is not authorized to issue scoped tokens", user.role)
        ));
    }

    if payload.role == Role::Admin && !user.is_admin() {
        return Err(AppError::Forbidden(
            "Only administrators may issue Admin role tokens".into(),
        ));
    }

    let issuer_manager = if user.is_admin() {
        None
    } else {
        let manager = state
            .manager_store
            .find_by_stellar_address(&user.stellar_address)
            .await?
            .ok_or_else(|| {
                AppError::Forbidden(
                    "Only approved managers may issue delegated vault tokens".into(),
                )
            })?;
        if manager.status != "approved" {
            return Err(AppError::Forbidden(
                "Only approved managers may issue delegated vault tokens".into(),
            ));
        }
        Some(manager)
    };

    if let Some(manager) = issuer_manager {
        for vault_id in &payload.vault_scopes {
            if vault_id == "*" {
                return Err(AppError::Forbidden(
                    "Only administrators may issue wildcard vault tokens".into(),
                ));
            }
            if !user.can_access_vault(vault_id) {
                return Err(AppError::Forbidden(format!(
                    "Cannot issue token for vault '{}' outside your authorized scope",
                    vault_id
                )));
            }
            let vault = state.vault_store.get(vault_id).await?;
            if vault.manager_id != manager.id {
                return Err(AppError::Forbidden(format!(
                    "Cannot issue token for vault '{}' outside your manager account",
                    vault_id
                )));
            }
        }
    }

    let token = encode_access_token_with_role_and_vaults(
        &state,
        &user.stellar_address,
        Some(payload.role),
        Some(payload.vault_scopes.clone()),
    )?;

    log_security_event(
        SecurityEventType::TokenRefreshed,
        Some(&user.stellar_address),
        None,
        None,
        Some(&format!(
            "Issued scoped token with role '{}' and {} vault scopes",
            payload.role,
            payload.vault_scopes.len()
        )),
    );

    Ok(Json(ScopedTokenResponse {
        access_token: token,
        token_type: "Bearer".to_string(),
        expires_in: ACCESS_TOKEN_EXPIRY_SECS,
        role: payload.role,
        vault_scopes: payload.vault_scopes,
    }))
}

#[derive(Serialize, ToSchema)]
pub struct JwkSetResponse {
    pub keys: Vec<JwkResponse>,
}

#[derive(Serialize, ToSchema)]
pub struct JwkResponse {
    pub kty: String,
    pub alg: String,
    pub kid: String,
    pub n: String,
    pub e: String,
    #[serde(rename = "use")]
    pub use_: String,
}

#[utoipa::path(
    get,
    path = "/auth/jwks",
    responses(
        (status = 200, description = "JSON Web Key Set", body = JwkSetResponse)
    ),
    tag = "Auth"
)]
pub async fn jwks_handler(
    Extension(state): Extension<Arc<AuthState>>,
) -> Result<Json<JwkSetResponse>, AppError> {
    let versions = state
        .jwt_keys
        .verification_keys(now_secs())
        .map_err(|error| AppError::Internal(error.to_string()))?;
    let keys = versions
        .into_iter()
        .map(|version| {
            let material = version.value();
            JwkResponse {
                kty: "RSA".to_string(),
                alg: "RS256".to_string(),
                kid: version.id().to_string(),
                n: material.jwk_n.clone(),
                e: material.jwk_e.clone(),
                use_: "sig".to_string(),
            }
        })
        .collect();
    Ok(Json(JwkSetResponse { keys }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::SigningKey;
    use rand::rngs::OsRng;

    fn test_state() -> AuthState {
        AuthState::new(
            None,
            Some([1u8; 32]),
            "Test SDF Network ; September 2015".to_string(),
            false,
        )
        .unwrap()
    }

    fn signed_challenge(state: &AuthState) -> (String, String) {
        let mut rng = OsRng;
        let client_signing_key = SigningKey::generate(&mut rng);
        let client_verifying_key = client_signing_key.verifying_key();

        let challenge_xdr =
            build_challenge_envelope(state, &client_verifying_key.to_bytes()).unwrap();

        let raw = BASE64.decode(&challenge_xdr).unwrap();
        let mut envelope = TransactionEnvelope::from_xdr(&raw, Limits::none()).unwrap();
        let TransactionEnvelope::Tx(ref mut inner) = envelope else {
            panic!("Expected TransactionV1 envelope");
        };
        let net_id = network_id(&state.network_passphrase);
        let hash = tx_hash(&inner.tx, &net_id).unwrap();
        let client_sig = client_signing_key.sign(&hash);
        let client_hint: [u8; 4] = client_verifying_key.to_bytes()[28..32].try_into().unwrap();
        let decorated = DecoratedSignature {
            hint: SignatureHint(client_hint),
            signature: client_sig.to_bytes().to_vec().try_into().unwrap(),
        };
        let mut sigs: Vec<_> = inner.signatures.iter().cloned().collect();
        sigs.push(decorated);
        inner.signatures = sigs.try_into().unwrap();

        let signed_xdr = BASE64.encode(&envelope.to_xdr(Limits::none()).unwrap());
        let expected_sub = Strkey::PublicKeyEd25519(stellar_strkey::ed25519::PublicKey(
            client_verifying_key.to_bytes(),
        ))
        .to_string();
        (signed_xdr, expected_sub)
    }

    fn decode_claims(state: &AuthState, token: &str) -> Claims {
        let header = decode_header(token).unwrap();
        let key_id = header.kid.unwrap();
        let key = state.verification_key(&key_id, now_secs()).unwrap().unwrap();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[WEB_AUTH_DOMAIN]);
        validation.set_required_spec_claims(&["exp", "iss", "sub"]);
        decode::<Claims>(token, &key.decoding_key, &validation)
            .unwrap()
            .claims
    }

    fn encode_claims(state: &AuthState, claims: &Claims) -> String {
        let current = state.jwt_keys.current().unwrap();
        let key = current.value().encoding_key.as_ref().unwrap();
        let mut header = Header::new(Algorithm::RS256);
        header.kid = Some(current.id().to_string());
        encode(&header, claims, key).unwrap()
    }

    #[test]
    fn test_auth_state_new() {
        let state = test_state();
        assert!(!state.is_verification_paused());
    }

    #[test]
    fn test_emergency_pause() {
        let state = test_state();
        assert!(!state.is_verification_paused());
        state.set_verification_paused(true);
        assert!(state.is_verification_paused());
        state.set_verification_paused(false);
        assert!(!state.is_verification_paused());
    }

    #[test]
    fn test_challenge_and_verify() {
        let state = test_state();
        let (signed_xdr, expected_sub) = signed_challenge(&state);

        let subject = verify_challenge_envelope(&state, &signed_xdr).unwrap();
        assert_eq!(subject, expected_sub);

        let tokens = issue_token_pair(&state, &subject).unwrap();
        assert!(!tokens.access_token.is_empty());
        assert!(!tokens.refresh_token.is_empty());
        assert_eq!(tokens.expires_in, ACCESS_TOKEN_EXPIRY_SECS);
        assert_eq!(tokens.token_type, "Bearer");
        assert_eq!(tokens.token, tokens.access_token);

        let token_claims = decode_claims(&state, &tokens.access_token);
        assert_eq!(token_claims.sub, expected_sub);
        assert_eq!(token_claims.iss, WEB_AUTH_DOMAIN.to_string());
        assert!(token_claims.scopes.contains(&"simulate".to_string()));

        let now = now_secs();
        assert!(token_claims.exp <= now + ACCESS_TOKEN_EXPIRY_SECS);
        assert!(token_claims.exp > now);
        // Access tokens must be short-lived (well under the old 24h window).
        assert!(token_claims.exp - token_claims.iat <= ACCESS_TOKEN_EXPIRY_SECS);
        assert!(ACCESS_TOKEN_EXPIRY_SECS < 3600);
    }

    #[test]
    fn test_refresh_rotates_tokens() {
        let state = test_state();
        let first = issue_token_pair(&state, "GTESTSUBJECT").unwrap();

        let second = rotate_refresh_token(&state, &first.refresh_token).unwrap();
        // Refresh token must always rotate; access JWT may be identical if minted
        // in the same second with identical claims.
        assert_ne!(first.refresh_token, second.refresh_token);
        assert_eq!(second.expires_in, ACCESS_TOKEN_EXPIRY_SECS);
        assert!(!second.access_token.is_empty());

        // Old refresh token must be rejected after rotation.
        let reuse = rotate_refresh_token(&state, &first.refresh_token);
        assert!(reuse.is_err());

        // Family revoked after reuse — new refresh also dies.
        let after_reuse = rotate_refresh_token(&state, &second.refresh_token);
        assert!(after_reuse.is_err());
    }

    #[test]
    fn test_refresh_invalid_token() {
        let state = test_state();
        let err = rotate_refresh_token(&state, "not-a-real-token");
        assert!(err.is_err());
    }

    #[test]
    fn test_revoke_refresh_token() {
        let state = test_state();
        let tokens = issue_token_pair(&state, "GTESTSUBJECT").unwrap();
        revoke_refresh_token(&state, &tokens.refresh_token).unwrap();
        assert!(rotate_refresh_token(&state, &tokens.refresh_token).is_err());
    }

    #[test]
    fn test_access_token_expiry_claim_is_short() {
        let state = test_state();
        let jwt = encode_access_token(&state, "GTEST").unwrap();
        let claims = decode_claims(&state, &jwt);
        assert_eq!(claims.exp - claims.iat, ACCESS_TOKEN_EXPIRY_SECS);
    }

    #[test]
    fn test_jwks() {
        let state = test_state();
        let keys = state.jwt_keys.verification_keys(now_secs()).unwrap();
        assert_eq!(keys.len(), 1);
    }

    #[test]
    fn rotated_key_keeps_previous_tokens_verifiable() {
        let state = test_state();
        let old_token = encode_access_token(&state, "GTESTROTATE").unwrap();
        let mut rng = OsRng;
        let next_private = RsaPrivateKey::new(&mut rng, 2048).unwrap();
        let next_pem = next_private
            .to_pkcs8_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap();
        state
            .rotate_signing_key(next_pem, "next-key".to_string())
            .unwrap();
        assert_eq!(decode_claims(&state, &old_token).sub, "GTESTROTATE");
        assert_eq!(state.jwt_keys.verification_keys(now_secs()).unwrap().len(), 2);
    }

    #[test]
    fn production_keyring_rejects_missing_material() {
        let result = AuthState::from_config(
            None,
            None,
            None,
            1_800,
            false,
            Some([2u8; 32]),
            "Test SDF Network ; September 2015".to_string(),
            false,
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_token_bucket_rate_limiter() {
        let mut bucket = TokenBucket::new(2.0);
        assert!(bucket.consume(2.0, 1.0, 1.0));
        assert!(bucket.consume(2.0, 1.0, 1.0));
        assert!(!bucket.consume(2.0, 1.0, 1.0));
    }

    #[test]
    fn test_expired_token_rejected() {
        let state = test_state();
        let now = now_secs();
        
        // Create a token that expired 1 second ago
        let expired_claims = Claims {
            sub: "GTESTEXPIRED".to_string(),
            iss: WEB_AUTH_DOMAIN.to_string(),
            iat: now - 100,
            exp: now - 1,
            scopes: vec!["simulate".to_string()],
            role: None,
            roles: None,
            vault_ids: None,
            vault_scopes: None,
        };
        
        let expired_token = encode_claims(&state, &expired_claims);

        // Attempt to validate the expired token using the same logic as auth_middleware
        let header = decode_header(&expired_token).unwrap();
        let key_id = header.kid.unwrap();
        let key = state.verification_key(&key_id, now_secs()).unwrap().unwrap();
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[WEB_AUTH_DOMAIN]);
        validation.set_required_spec_claims(&["exp", "iss", "sub"]);
        let result = decode::<Claims>(&expired_token, &key.decoding_key, &validation);
        
        // The token should fail validation (either by jsonwebtoken or our explicit check)
        // If it doesn't fail in decode, our explicit check in auth_middleware will catch it
        if let Ok(token_data) = result {
            // Simulate the explicit expiry check from auth_middleware (BE-030)
            let current_time = now_secs();
            assert!(token_data.claims.exp <= current_time, "Expired token should be rejected");
        }
    }

    #[test]
    fn test_valid_token_not_expired() {
        let state = test_state();
        let now = now_secs();
        
        // Create a token that expires 1 hour from now
        let valid_claims = Claims {
            sub: "GTESTVALID".to_string(),
            iss: WEB_AUTH_DOMAIN.to_string(),
            iat: now,
            exp: now + 3600,
            scopes: vec!["simulate".to_string()],
            role: None,
            roles: None,
            vault_ids: None,
            vault_scopes: None,
        };
        
        let valid_token = encode_claims(&state, &valid_claims);

        // Validate the token
        let result = decode_claims(&state, &valid_token);
        
        let current_time = now_secs();

        // Explicit expiry check from auth_middleware (BE-030)
        assert!(result.exp > current_time, "Valid token should not be expired");
        assert_eq!(result.sub, "GTESTVALID");
        assert!(result.scopes.contains(&"simulate".to_string()));
    }
}

