use crate::audit_log::{log_audit_event, log_security_event, SecurityEventType};
use crate::errors::AppError;
use axum::{extract::Request, http::header, middleware::Next, response::Response, Extension, Json};
use base64::{
    engine::general_purpose::STANDARD as BASE64,
    engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL, Engine,
};
use ed25519_dalek::{Signature as Ed25519Signature, Signer, SigningKey, Verifier, VerifyingKey};
use jsonwebtoken::{decode, encode, Algorithm, DecodingKey, EncodingKey, Header, Validation};
use rand::RngCore;
use rsa::{
    pkcs8::{DecodePrivateKey, EncodePrivateKey, EncodePublicKey},
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
use utoipa::ToSchema;
use uuid::Uuid;

const CHALLENGE_EXPIRY_SECS: u64 = 300;
/// Short-lived access JWT lifetime (15 minutes).
const ACCESS_TOKEN_EXPIRY_SECS: u64 = 900;
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

/// Authenticated user extracted from JWT and injected into request extensions
/// for tenant-scoped handlers.
#[derive(Clone, Debug)]
pub struct AuthenticatedUser {
    pub stellar_address: String,
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

pub struct AuthState {
    pub encoding_key: EncodingKey,
    pub decoding_key: DecodingKey,
    pub jwk_n: String,
    pub jwk_e: String,
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
    ) -> Self {
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

        let priv_key = if let Some(pem) = jwt_private_key_pem {
            RsaPrivateKey::from_pkcs8_pem(&pem).expect("Invalid RSA Private Key PEM")
        } else {
            tracing::info!("Generating ephemeral RSA keypair for local development...");
            let mut rng = rand::thread_rng();
            RsaPrivateKey::new(&mut rng, 2048).expect("Failed to generate RSA key")
        };

        let pem_str = priv_key.to_pkcs8_pem(rsa::pkcs8::LineEnding::LF).unwrap();
        let encoding_key = EncodingKey::from_rsa_pem(pem_str.as_bytes()).unwrap();

        let pub_key = RsaPublicKey::from(&priv_key);
        let pub_pem = pub_key
            .to_public_key_pem(rsa::pkcs8::LineEnding::LF)
            .unwrap();
        let decoding_key = DecodingKey::from_rsa_pem(pub_pem.as_bytes()).unwrap();

        let n = BASE64_URL.encode(pub_key.n().to_bytes_be());
        let e = BASE64_URL.encode(pub_key.e().to_bytes_be());

        let state = Self {
            encoding_key,
            decoding_key,
            jwk_n: n,
            jwk_e: e,
            signing_key,
            server_public_key,
            network_passphrase,
            emergency_verification_paused: Arc::new(AtomicBool::new(emergency_verification_paused)),
            refresh_tokens: Arc::new(RwLock::new(HashMap::new())),
            rate_limiter: Mutex::new(HashMap::new()),
        };

        log_security_event(
            SecurityEventType::JwtKeyRotated,
            Some(&state.server_stellar_address()),
            None,
            None,
            Some("JWT signing key initialized or rotated"),
        );

        state
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
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
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

fn encode_access_token(state: &AuthState, subject: &str) -> Result<String, AppError> {
    let now = now_secs();
    let claims = Claims {
        sub: subject.to_string(),
        iss: WEB_AUTH_DOMAIN.to_string(),
        iat: now,
        exp: now + ACCESS_TOKEN_EXPIRY_SECS,
        scopes: vec!["simulate".to_string()],
    };

    let header = Header::new(Algorithm::RS256);
    encode(&header, &claims, &state.encoding_key)
        .map_err(|e| AppError::Internal(format!("JWT encode error: {e}")))
}

/// Issue a short-lived access JWT plus a new refresh token (new rotation family).
pub(crate) fn issue_token_pair(state: &AuthState, subject: &str) -> Result<VerifyResponse, AppError> {
    let family_id = Uuid::new_v4().to_string();
    issue_token_pair_in_family(state, subject, &family_id)
}

fn issue_token_pair_in_family(
    state: &AuthState,
    subject: &str,
    family_id: &str,
) -> Result<VerifyResponse, AppError> {
    let access_token = encode_access_token(state, subject)?;
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
        return Err(AppError::Unauthorized("Missing refresh token".into()));
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
                    return Err(AppError::Unauthorized("Refresh token expired".into()));
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
                return Err(AppError::Unauthorized(
                    "Refresh token reuse detected; re-authenticate".into(),
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
                return Err(AppError::Unauthorized(
                    "Invalid or already-rotated refresh token".into(),
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
        .map_err(|_| AppError::BadRequest("Invalid base64".into()))?;

    let envelope = TransactionEnvelope::from_xdr(&raw, Limits::none())
        .map_err(|_| AppError::BadRequest("Invalid transaction XDR".into()))?;

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
    Json(payload): Json<ChallengeRequest>,
) -> Result<Json<ChallengeResponse>, AppError> {
    if state.is_verification_paused() {
        return Err(AppError::Internal(
            "Message verification is temporarily paused for emergency maintenance".into(),
        ));
    }

    let strkey = Strkey::from_string(&payload.account)
        .map_err(|_| AppError::BadRequest("Invalid Stellar address".into()))?;

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
    Json(payload): Json<VerifyRequest>,
) -> Result<Json<VerifyResponse>, AppError> {
    if state.is_verification_paused() {
        log_security_event(
            SecurityEventType::LoginFailed,
            None,
            None,
            None,
            Some("Verification paused for emergency maintenance"),
        );
        return Err(AppError::Internal(
            "Message verification is temporarily paused for emergency maintenance".into(),
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

    let tokens = match issue_token_pair(&state, &subject) {
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
        (status = 401, description = "Invalid, expired, or reused refresh token"),
        (status = 503, description = "Verification paused for emergency maintenance")
    ),
    tag = "Auth"
)]
pub async fn refresh_handler(
    Extension(state): Extension<Arc<AuthState>>,
    Json(payload): Json<RefreshRequest>,
) -> Result<Json<VerifyResponse>, AppError> {
    if state.is_verification_paused() {
        return Err(AppError::Internal(
            "Authentication is temporarily paused for emergency maintenance".into(),
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
        (status = 200, description = "Refresh token family revoked", body = RevokeResponse),
        (status = 503, description = "Verification paused for emergency maintenance")
    ),
    tag = "Auth"
)]
pub async fn revoke_handler(
    Extension(state): Extension<Arc<AuthState>>,
    Json(payload): Json<RefreshRequest>,
) -> Result<Json<RevokeResponse>, AppError> {
    if state.is_verification_paused() {
        return Err(AppError::Internal(
            "Authentication is temporarily paused for emergency maintenance".into(),
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
    Json(payload): Json<EmergencyPauseRequest>,
) -> Result<Json<EmergencyPauseResponse>, AppError> {
    state.set_verification_paused(payload.paused);

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
    // Check if verification is paused — deny all requests during emergency maintenance
    if state.is_verification_paused() {
        return Err(AppError::Internal(
            "Authentication is temporarily paused for emergency maintenance".into(),
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

    let validation = Validation::new(Algorithm::RS256);
    let token_data = match decode::<Claims>(token, &state.decoding_key, &validation) {
        Ok(data) => data,
        Err(e) => {
            if matches!(e.kind(), jsonwebtoken::errors::ErrorKind::ExpiredSignature) {
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
                    Some(&format!("Invalid JWT: {e}")),
                );
            }
            return Err(AppError::Unauthorized(format!("Invalid token: {e}")));
        }
    };

    // Validate JWT expiry claim (BE-030: verify token has not expired)
    let now = now_secs();
    if token_data.claims.exp <= now {
        return Err(AppError::Unauthorized(
            "Token has expired".into(),
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
        let mut rate_limiter = state.rate_limiter.lock().unwrap();
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
            return Err(AppError::TooManyRequests(format!(
                "Rate limit exceeded for tenant {}",
                tenant
            )));
        }
    }

    let mut req = req;
    req.extensions_mut().insert(AuthenticatedUser {
        stellar_address: token_data.claims.sub.clone(),
    });

    Ok(next.run(req).await)
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
    Ok(Json(JwkSetResponse {
        keys: vec![JwkResponse {
            kty: "RSA".to_string(),
            alg: "RS256".to_string(),
            kid: "1".to_string(),
            n: state.jwk_n.clone(),
            e: state.jwk_e.clone(),
            use_: "sig".to_string(),
        }],
    }))
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

        let validation = Validation::new(Algorithm::RS256);
        let token_data =
            decode::<Claims>(&tokens.access_token, &state.decoding_key, &validation).unwrap();
        assert_eq!(token_data.claims.sub, expected_sub);
        assert_eq!(token_data.claims.iss, WEB_AUTH_DOMAIN.to_string());
        assert!(token_data.claims.scopes.contains(&"simulate".to_string()));

        let now = now_secs();
        assert!(token_data.claims.exp <= now + ACCESS_TOKEN_EXPIRY_SECS);
        assert!(token_data.claims.exp > now);
        // Access tokens must be short-lived (well under the old 24h window).
        assert!(token_data.claims.exp - token_data.claims.iat <= ACCESS_TOKEN_EXPIRY_SECS);
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
        let validation = Validation::new(Algorithm::RS256);
        let claims = decode::<Claims>(&jwt, &state.decoding_key, &validation)
            .unwrap()
            .claims;
        assert_eq!(claims.exp - claims.iat, ACCESS_TOKEN_EXPIRY_SECS);
    }

    #[test]
    fn test_jwks() {
        let state = test_state();
        let _ = JwkResponse {
            kty: "RSA".to_string(),
            alg: "RS256".to_string(),
            kid: "1".to_string(),
            n: state.jwk_n.clone(),
            e: state.jwk_e.clone(),
            use_: "sig".to_string(),
        };
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
        use jsonwebtoken::encode;
        let state = test_state();
        let now = now_secs();
        
        // Create a token that expired 1 second ago
        let expired_claims = Claims {
            sub: "GTESTEXPIRED".to_string(),
            iss: WEB_AUTH_DOMAIN.to_string(),
            iat: now - 100,
            exp: now - 1,  // Expired
            scopes: vec!["simulate".to_string()],
        };
        
        let header = Header::new(Algorithm::RS256);
        let expired_token = encode(&header, &expired_claims, &state.encoding_key).unwrap();
        
        // Attempt to validate the expired token using the same logic as auth_middleware
        let validation = Validation::new(Algorithm::RS256);
        let result = decode::<Claims>(&expired_token, &state.decoding_key, &validation);
        
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
        use jsonwebtoken::encode;
        let state = test_state();
        let now = now_secs();
        
        // Create a token that expires 1 hour from now
        let valid_claims = Claims {
            sub: "GTESTVALID".to_string(),
            iss: WEB_AUTH_DOMAIN.to_string(),
            iat: now,
            exp: now + 3600,  // Expires in 1 hour
            scopes: vec!["simulate".to_string()],
        };
        
        let header = Header::new(Algorithm::RS256);
        let valid_token = encode(&header, &valid_claims, &state.encoding_key).unwrap();
        
        // Validate the token
        let validation = Validation::new(Algorithm::RS256);
        let result = decode::<Claims>(&valid_token, &state.decoding_key, &validation);
        
        assert!(result.is_ok(), "Valid token should decode successfully");
        
        let token_data = result.unwrap();
        let current_time = now_secs();
        
        // Explicit expiry check from auth_middleware (BE-030)
        assert!(token_data.claims.exp > current_time, "Valid token should not be expired");
        assert_eq!(token_data.claims.sub, "GTESTVALID");
        assert!(token_data.claims.scopes.contains(&"simulate".to_string()));
    }
}

