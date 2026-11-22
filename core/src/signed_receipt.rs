use std::collections::HashMap;
use std::env;
use std::sync::Arc;

use base64::{engine::general_purpose::URL_SAFE_NO_PAD as BASE64_URL, Engine};
use hmac::{Hmac, Mac};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;
use uuid::Uuid;

type HmacSha256 = Hmac<Sha256>;

pub const RECEIPT_HEADER: &str = "x-perigee-receipt";
pub const RECEIPT_ID_HEADER: &str = "x-perigee-receipt-id";

#[derive(Debug, Error)]
pub enum ReceiptError {
    #[error("receipt secret must not be empty")]
    EmptySecret,
    #[error("PERIGEE_RECEIPT_SECRET must be configured in production")]
    MissingSecret,
    #[error("receipt signature is invalid")]
    InvalidSignature,
    #[error("receipt key is not registered")]
    MissingKey,
    #[error("receipt payload could not be serialized: {0}")]
    Serialization(#[from] serde_json::Error),
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SignedReceipt {
    pub payment_id: String,
    pub amount: u64,
    pub receiver_agent: String,
    pub sender_agent: String,
    pub signature: Vec<u8>,
    pub signer_public_key: Vec<u8>,
}

impl SignedReceipt {
    pub fn new(payment_id: String, amount: u64, receiver: String, sender: String) -> Self {
        Self {
            payment_id,
            amount,
            receiver_agent: receiver,
            sender_agent: sender,
            signature: Vec::new(),
            signer_public_key: Vec::new(),
        }
    }

    pub fn verify_signature(&self) -> bool {
        self.signature.len() == 32 && self.signer_public_key.len() == 32
    }

    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        append_field(&mut bytes, b"perigee-receipt-v1");
        append_field(&mut bytes, self.payment_id.as_bytes());
        bytes.extend_from_slice(&self.amount.to_be_bytes());
        append_field(&mut bytes, self.receiver_agent.as_bytes());
        append_field(&mut bytes, self.sender_agent.as_bytes());
        bytes
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.to_canonical_bytes()
    }

    pub fn sign_with_secret(&self, secret: &[u8]) -> Result<Self, ReceiptError> {
        ReceiptSigner::new(secret)?.sign(self)
    }

    pub fn verify_with_secret(&self, secret: &[u8]) -> bool {
        if !self.verify_signature() {
            return false;
        }
        let fingerprint = key_fingerprint(secret);
        if self.signer_public_key.as_slice() != fingerprint.as_slice() {
            return false;
        }
        verify_mac(secret, &self.to_canonical_bytes(), &self.signature)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ApiReceipt {
    pub receipt_id: String,
    pub operation: String,
    pub resource_id: String,
    pub actor: String,
    pub method: String,
    pub path: String,
    pub status: u16,
    pub response_digest: String,
    pub issued_at: i64,
    pub key_id: String,
    pub signature: String,
}

pub type WriteReceipt = ApiReceipt;

impl ApiReceipt {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation: impl Into<String>,
        resource_id: impl Into<String>,
        actor: impl Into<String>,
        method: impl Into<String>,
        path: impl Into<String>,
        status: u16,
        response_digest: impl Into<String>,
        issued_at: i64,
    ) -> Self {
        Self {
            receipt_id: Uuid::new_v4().to_string(),
            operation: operation.into(),
            resource_id: resource_id.into(),
            actor: actor.into(),
            method: method.into(),
            path: path.into(),
            status,
            response_digest: response_digest.into(),
            issued_at,
            key_id: String::new(),
            signature: String::new(),
        }
    }

    pub fn to_canonical_bytes(&self) -> Vec<u8> {
        let mut bytes = Vec::new();
        append_field(&mut bytes, b"perigee-api-receipt-v1");
        append_field(&mut bytes, self.receipt_id.as_bytes());
        append_field(&mut bytes, self.operation.as_bytes());
        append_field(&mut bytes, self.resource_id.as_bytes());
        append_field(&mut bytes, self.actor.as_bytes());
        append_field(&mut bytes, self.method.as_bytes());
        append_field(&mut bytes, self.path.as_bytes());
        bytes.extend_from_slice(&self.status.to_be_bytes());
        append_field(&mut bytes, self.response_digest.as_bytes());
        bytes.extend_from_slice(&self.issued_at.to_be_bytes());
        append_field(&mut bytes, self.key_id.as_bytes());
        bytes
    }

    pub fn canonical_bytes(&self) -> Vec<u8> {
        self.to_canonical_bytes()
    }

    pub fn verify_signature(&self, secret: &[u8]) -> bool {
        if self.signature.len() > 128 {
            return false;
        }
        let Ok(signature) = BASE64_URL.decode(self.signature.as_bytes()) else {
            return false;
        };
        verify_mac(secret, &self.to_canonical_bytes(), &signature)
    }
}

#[derive(Clone)]
pub struct ReceiptSigner {
    secret: Arc<Vec<u8>>,
    key_id: String,
}

impl ReceiptSigner {
    pub fn new(secret: impl AsRef<[u8]>) -> Result<Self, ReceiptError> {
        let secret = secret.as_ref();
        if secret.is_empty() || secret.iter().all(|byte| byte.is_ascii_whitespace()) {
            return Err(ReceiptError::EmptySecret);
        }
        Ok(Self {
            secret: Arc::new(secret.to_vec()),
            key_id: hex::encode(key_fingerprint(secret)),
        })
    }

    pub fn from_env() -> Result<Self, ReceiptError> {
        Self::from_env_with_app_env(None)
    }

    pub fn from_env_with_app_env(app_env: Option<&str>) -> Result<Self, ReceiptError> {
        let secret = env::var("PERIGEE_RECEIPT_SECRET")
            .or_else(|_| env::var("RECEIPT_HMAC_SECRET"))
            .unwrap_or_default();
        if secret.trim().is_empty() {
            let production = app_env
                .filter(|value| !value.trim().is_empty())
                .map(|value| value.trim().eq_ignore_ascii_case("production"))
                .unwrap_or_else(|| {
                    env::var("APP_ENV")
                        .map(|value| value.trim().eq_ignore_ascii_case("production"))
                        .unwrap_or(false)
                });
            if production {
                return Err(ReceiptError::MissingSecret);
            }
            let mut generated = [0u8; 32];
            rand::thread_rng().fill_bytes(&mut generated);
            tracing::warn!("PERIGEE_RECEIPT_SECRET is not set; using an ephemeral receipt key");
            return Self::new(generated);
        }
        Self::new(secret.as_bytes())
    }

    pub fn from_secret(secret: impl AsRef<[u8]>) -> Result<Self, ReceiptError> {
        Self::new(secret)
    }

    pub fn key_id(&self) -> &str {
        &self.key_id
    }

    pub fn sign(&self, receipt: &SignedReceipt) -> Result<SignedReceipt, ReceiptError> {
        let mut signed = receipt.clone();
        signed.signature = calculate_mac(self.secret.as_slice(), &receipt.to_canonical_bytes())?;
        signed.signer_public_key = key_fingerprint(self.secret.as_slice()).to_vec();
        Ok(signed)
    }

    pub fn sign_receipt(&self, receipt: &SignedReceipt) -> Result<SignedReceipt, ReceiptError> {
        self.sign(receipt)
    }

    pub fn sign_api(&self, receipt: &ApiReceipt) -> Result<ApiReceipt, ReceiptError> {
        let mut signed = receipt.clone();
        signed.key_id = self.key_id.clone();
        let signature = calculate_mac(self.secret.as_slice(), &signed.to_canonical_bytes())?;
        signed.signature = BASE64_URL.encode(signature);
        Ok(signed)
    }

    pub fn sign_api_receipt(&self, receipt: ApiReceipt) -> Result<ApiReceipt, ReceiptError> {
        self.sign_api(&receipt)
    }
}

#[derive(Default)]
pub struct ReceiptVerifier {
    trusted_keys: HashMap<String, Vec<u8>>,
}

impl ReceiptVerifier {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_secret(secret: impl AsRef<[u8]>) -> Result<Self, ReceiptError> {
        let signer = ReceiptSigner::new(secret.as_ref())?;
        let mut verifier = Self::new();
        verifier.add_key(signer.key_id, secret.as_ref());
        Ok(verifier)
    }

    pub fn add_trusted_key(&mut self, secret: Vec<u8>) {
        self.add_key(hex::encode(key_fingerprint(&secret)), secret);
    }

    pub fn add_key(&mut self, key_id: impl Into<String>, secret: impl AsRef<[u8]>) {
        self.trusted_keys
            .insert(key_id.into(), secret.as_ref().to_vec());
    }

    pub fn verify(&self, receipt: &SignedReceipt) -> bool {
        if !receipt.verify_signature() {
            return false;
        }
        let key_id = hex::encode(&receipt.signer_public_key);
        self.trusted_keys
            .get(&key_id)
            .is_some_and(|secret| receipt.verify_with_secret(secret))
    }

    pub fn from_secret(secret: impl AsRef<[u8]>) -> Result<Self, ReceiptError> {
        Self::with_secret(secret)
    }

    pub fn verify_receipt(&self, receipt: &SignedReceipt) -> bool {
        self.verify(receipt)
    }

    pub fn verify_api(&self, receipt: &ApiReceipt) -> bool {
        self.trusted_keys
            .get(&receipt.key_id)
            .is_some_and(|secret| receipt.verify_signature(secret))
    }

    pub fn verify_api_at(
        &self,
        receipt: &ApiReceipt,
        now: i64,
        max_age_secs: i64,
    ) -> bool {
        if max_age_secs < 0 || receipt.issued_at > now {
            return false;
        }
        if now.saturating_sub(receipt.issued_at) > max_age_secs {
            return false;
        }
        self.verify_api(receipt)
    }
}

fn calculate_mac(secret: &[u8], payload: &[u8]) -> Result<Vec<u8>, ReceiptError> {
    let mut mac = HmacSha256::new_from_slice(secret).map_err(|_| ReceiptError::EmptySecret)?;
    mac.update(payload);
    Ok(mac.finalize().into_bytes().to_vec())
}

fn verify_mac(secret: &[u8], payload: &[u8], signature: &[u8]) -> bool {
    if secret.is_empty() || signature.len() != 32 {
        return false;
    }
    let Ok(mut mac) = HmacSha256::new_from_slice(secret) else {
        return false;
    };
    mac.update(payload);
    mac.verify_slice(signature).is_ok()
}

fn key_fingerprint(secret: &[u8]) -> [u8; 32] {
    let mut input = Vec::with_capacity(secret.len() + 20);
    input.extend_from_slice(b"perigee-receipt-key\0");
    input.extend_from_slice(secret);
    Sha256::digest(input).into()
}

fn append_field(bytes: &mut Vec<u8>, field: &[u8]) {
    bytes.extend_from_slice(&(field.len() as u64).to_be_bytes());
    bytes.extend_from_slice(field);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signed_receipt_round_trip_and_tamper_detection() {
        let signer = ReceiptSigner::new(b"test-secret").unwrap();
        let receipt = SignedReceipt::new("pay-1".into(), 1000, "receiver".into(), "sender".into());
        let signed = signer.sign(&receipt).unwrap();
        let mut verifier = ReceiptVerifier::new();
        verifier.add_trusted_key(b"test-secret".to_vec());
        assert!(verifier.verify(&signed));
        let mut tampered = signed;
        tampered.amount += 1;
        assert!(!verifier.verify(&tampered));
    }

    #[test]
    fn api_receipt_round_trip() {
        let signer = ReceiptSigner::new(b"test-secret").unwrap();
        let receipt = ApiReceipt::new(
            "vault.update",
            "vault-1",
            "manager-1",
            "PATCH",
            "/v1/vaults/vault-1",
            200,
            "abc123",
            1_700_000_000,
        );
        let signed = signer.sign_api(&receipt).unwrap();
        let mut verifier = ReceiptVerifier::new();
        verifier.add_key(signer.key_id(), b"test-secret");
        assert!(verifier.verify_api(&signed));
        let mut tampered = signed;
        tampered.status = 500;
        assert!(!verifier.verify_api(&tampered));
    }
}
