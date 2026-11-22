#![cfg(test)]

use crate::*;
use crate::chain_info::BridgeEndpoint;
use crate::payload::{EncodedPayload, PayloadBatch, PayloadRoute};
use crate::signatures::{RecoveryKey, SignatureScheme};
use crate::verification::{VerificationResult, VerificationStatus};
use crate::{ChainInfo, CrossChainError, CrossChainPayload, PayloadMetadata};
use soroban_sdk::{Bytes, BytesN, Env, String, Symbol, Vec};

#[test]
fn test_chain_info_creation() {
    let bridge_contract = BytesN::from_array(&[0u8; 32]);
    let e = Env::default();

    let bridge_contract = BytesN::from_array(&e, &[0u8; 32]);

    let chain_info = ChainInfo {
        chain_id: 1,
        chain_name: String::from_str(&e, "stellar"),
        network_version: 1,
        bridge_contract,
        consensus_round: 100,
        is_active: true,
    };

    assert_eq!(chain_info.chain_id, 1);
    assert_eq!(chain_info.consensus_round, 100);
    assert!(chain_info.is_active);
}

#[test]
fn test_bridge_endpoint_creation() {
    let bridge_contract = BytesN::from_array(&[0u8; 32]);
    let e = Env::default();
    let bridge_contract = BytesN::from_array(&e, &[0u8; 32]);

    let source_chain = ChainInfo {
        chain_id: 1,
        chain_name: String::from_str(&e, "chain-1"),
        network_version: 1,
        bridge_contract: bridge_contract.clone(),
        consensus_round: 100,
        is_active: true,
    };

    let dest_chain = ChainInfo {
        chain_id: 2,
        chain_name: String::from_str(&e, "chain-2"),
        network_version: 1,
        bridge_contract,
        consensus_round: 100,
        is_active: true,
    };

    let endpoint = BridgeEndpoint {
        source_chain,
        destination_chain: dest_chain,
        fee_percentage: 100, // 1%
        min_liquidity: 1000000,
        is_enabled: true,
    };

    assert_eq!(endpoint.fee_percentage, 100);
    assert!(endpoint.is_enabled);
}

#[test]
fn test_cross_chain_payload_creation() {
    let payload_id = BytesN::from_array(&[1u8; 32]);
    let nonce = BytesN::from_array(&[2u8; 32]);
    let payload_hash = BytesN::from_array(&[3u8; 32]);
    let e = Env::default();
    let payload_id = BytesN::from_array(&e, &[1u8; 32]);
    let nonce = BytesN::from_array(&e, &[2u8; 32]);
    let payload_hash = BytesN::from_array(&e, &[3u8; 32]);

    let metadata = PayloadMetadata {
        version: 1,
        timestamp: 1000000,
        sequence: 1,
        expiration_height: 10000,
        nonce,
    };

    let sender = Bytes::new(&soroban_sdk::Env::default());
    let recipient = Bytes::new(&soroban_sdk::Env::default());
    let data = Bytes::new(&soroban_sdk::Env::default());
    let sender = Bytes::new(&e);
    let recipient = Bytes::new(&e);
    let data = Bytes::new(&e);

    let payload = CrossChainPayload {
        payload_id,
        source_chain_id: 1,
        destination_chain_id: 2,
        sender,
        recipient,
        data,
        operation: Symbol::new(&e, "transfer"),
        metadata,
        payload_hash,
        gas_limit: 1000000,
    };

    assert_eq!(payload.source_chain_id, 1);
    assert_eq!(payload.destination_chain_id, 2);
    assert_eq!(payload.gas_limit, 1000000);
}

#[test]
fn test_payload_batch_creation() {
    let batch_id = BytesN::from_array(&[4u8; 32]);
    let merkle_root = BytesN::from_array(&[5u8; 32]);
    let e = Env::default();
    let batch_id = BytesN::from_array(&e, &[4u8; 32]);
    let merkle_root = BytesN::from_array(&e, &[5u8; 32]);

    let batch = PayloadBatch {
        batch_id,
        source_chain_id: 1,
        payload_count: 10,
        merkle_root,
        batch_timestamp: 1000000,
        batch_ttl_seconds: 3600,
    };

    assert_eq!(batch.payload_count, 10);
    assert_eq!(batch.batch_ttl_seconds, 3600);
}

#[test]
fn test_verification_result_creation() {
    let error_msg = String::from_small_str("test error");
    let e = Env::default();
    let error_msg = String::from_str(&e, "test error");

    let result = VerificationResult {
        status: VerificationStatus::Verified,
        signatures_verified: 5,
        signatures_required: 5,
        error_message: error_msg,
        verified_at_height: 12345,
        has_rejections: false,
        rejection_count: 0,
    };

    assert_eq!(result.status, VerificationStatus::Verified);
    assert_eq!(result.signatures_verified, 5);
    assert!(!result.has_rejections);
}

#[test]
fn test_signature_schemes() {
    let schemes = vec![
        SignatureScheme::Ed25519,
        SignatureScheme::Secp256k1,
        SignatureScheme::BLS12381,
        SignatureScheme::ECDSA,
    ];

    assert_eq!(schemes.len(), 4);
}

#[test]
fn test_error_codes() {
    assert_eq!(CrossChainError::InvalidPayloadHash.as_u32(), 1);
    assert_eq!(CrossChainError::InvalidSignature.as_u32(), 2);
    assert_eq!(CrossChainError::PayloadExpired.as_u32(), 5);
    assert_eq!(CrossChainError::ReplayAttack.as_u32(), 6);
    assert_eq!(CrossChainError::Unknown.as_u32(), 255);
}

#[test]
fn test_verification_status_variants() {
    let statuses = vec![
        VerificationStatus::Pending,
        VerificationStatus::Verified,
        VerificationStatus::Failed,
        VerificationStatus::Expired,
        VerificationStatus::Cancelled,
    ];

    assert_eq!(statuses.len(), 5);
    assert_eq!(statuses[1], VerificationStatus::Verified);
}

#[test]
fn test_payload_route_creation() {
    let e = Env::default();
    let route = PayloadRoute {
        from_chain: 1,
        to_chain: 3,
        route_path: Vec::new(&e),
        priority: 100,
        is_critical: true,
    };

    assert_eq!(route.from_chain, 1);
    assert_eq!(route.to_chain, 3);
    assert!(route.is_critical);
}

#[test]
fn test_encoded_payload_creation() {
    let encoded_data = Bytes::new(&soroban_sdk::Env::default());
    let encoding_scheme = String::from_small_str("borsh");
    let compression_type = String::from_small_str("gzip");
    let e = Env::default();
    let encoded_data = Bytes::new(&e);
    let encoding_scheme = String::from_str(&e, "borsh");
    let compression_type = String::from_str(&e, "gzip");

    let encoded = EncodedPayload {
        encoded_data,
        encoding_scheme,
        compression_type,
        original_size: 1000,
        compressed_size: 600,
    };

    assert_eq!(encoded.original_size, 1000);
    assert_eq!(encoded.compressed_size, 600);
}

#[test]
fn test_recovery_key_creation() {
    let compressed_key = BytesN::from_array(&[6u8; 33]);
    let e = Env::default();
    let compressed_key = BytesN::from_array(&e, &[6u8; 33]);

    let key = RecoveryKey {
        compressed_key,
        key_type: SignatureScheme::Secp256k1,
        chain_id: 1,
        is_active: true,
        activation_height: 1000,
        deactivation_height: u64::MAX,
    };

    assert!(key.is_active);
    assert_eq!(key.chain_id, 1);
}

// ── #875 — Malformed-payload boundary tests for cross-chain messages ─────────

/// An empty payload data field is distinct from a minimum-valid payload and
/// must round-trip without corruption.
#[test]
fn test_empty_payload_data_roundtrip() {
    let e = Env::default();
    let nonce = BytesN::from_array(&e, &[0u8; 32]);
    let pid = BytesN::from_array(&e, &[1u8; 32]);
    let hash = BytesN::from_array(&e, &[2u8; 32]);

    let meta = PayloadMetadata {
        version: 1,
        timestamp: 1000,
        sequence: 0,
        expiration_height: 9999,
        nonce,
    };

    let payload = CrossChainPayload {
        payload_id: pid.clone(),
        source_chain_id: 1,
        destination_chain_id: 2,
        sender: Bytes::new(&e),
        recipient: Bytes::new(&e),
        data: Bytes::new(&e),  // empty data — minimum valid
        operation: Symbol::new(&e, "transfer"),
        metadata: meta,
        payload_hash: hash,
        gas_limit: 0,
    };

    // Round-trip: all fields survive reconstruction.
    assert_eq!(payload.payload_id, pid);
    assert_eq!(payload.data.len(), 0);
    assert_eq!(payload.source_chain_id, 1);
    assert_eq!(payload.destination_chain_id, 2);
    assert_eq!(payload.metadata.version, 1);
}

/// A payload with version 0 represents an unknown / unsupported version.
/// The struct should still be constructable so callers can inspect the version
/// and reject it at the application layer.
#[test]
fn test_unknown_version_payload_is_constructable() {
    let e = Env::default();
    let nonce = BytesN::from_array(&e, &[0u8; 32]);
    let pid = BytesN::from_array(&e, &[0u8; 32]);
    let hash = BytesN::from_array(&e, &[0u8; 32]);

    let meta = PayloadMetadata {
        version: 0, // unknown version
        timestamp: 0,
        sequence: 0,
        expiration_height: 0,
        nonce,
    };

    let payload = CrossChainPayload {
        payload_id: pid,
        source_chain_id: 0,
        destination_chain_id: 0,
        sender: Bytes::new(&e),
        recipient: Bytes::new(&e),
        data: Bytes::new(&e),
        operation: Symbol::new(&e, "noop"),
        metadata: meta.clone(),
        payload_hash: hash,
        gas_limit: 0,
    };

    // Version 0 is accessible so the application layer can reject it.
    assert_eq!(payload.metadata.version, 0);
    assert_eq!(meta.version, 0);
}

/// Maximum-size fields: source/destination chain IDs at u64::MAX must be stored
/// without truncation.
#[test]
fn test_maximum_chain_id_values() {
    let e = Env::default();
    let nonce = BytesN::from_array(&e, &[0xFFu8; 32]);
    let pid = BytesN::from_array(&e, &[0xFFu8; 32]);
    let hash = BytesN::from_array(&e, &[0xFFu8; 32]);

    let meta = PayloadMetadata {
        version: u32::MAX,
        timestamp: u64::MAX,
        sequence: u64::MAX,
        expiration_height: u64::MAX,
        nonce,
    };

    let payload = CrossChainPayload {
        payload_id: pid,
        source_chain_id: u64::MAX,
        destination_chain_id: u64::MAX,
        sender: Bytes::from_slice(&e, &[0u8; 64]),
        recipient: Bytes::from_slice(&e, &[0u8; 64]),
        data: Bytes::from_slice(&e, &[0xAAu8; 128]),
        operation: Symbol::new(&e, "swap"),
        metadata: meta,
        payload_hash: hash,
        gas_limit: u64::MAX,
    };

    assert_eq!(payload.source_chain_id, u64::MAX);
    assert_eq!(payload.destination_chain_id, u64::MAX);
    assert_eq!(payload.gas_limit, u64::MAX);
    assert_eq!(payload.metadata.timestamp, u64::MAX);
}

/// Nonce values must be distinct: two payloads with different nonces are not equal.
#[test]
fn test_distinct_nonces_produce_distinct_metadata() {
    let e = Env::default();
    let nonce_a = BytesN::from_array(&e, &[1u8; 32]);
    let nonce_b = BytesN::from_array(&e, &[2u8; 32]);
    let hash = BytesN::from_array(&e, &[0u8; 32]);

    let meta_a = PayloadMetadata {
        version: 1,
        timestamp: 1000,
        sequence: 1,
        expiration_height: 9000,
        nonce: nonce_a,
    };
    let meta_b = PayloadMetadata {
        version: 1,
        timestamp: 1000,
        sequence: 1,
        expiration_height: 9000,
        nonce: nonce_b,
    };

    // Different nonces → metadata structs are not equal.
    assert_ne!(meta_a, meta_b);
    let _ = hash; // silence unused warning
}

/// PayloadBatch fields survive construction without silent truncation.
#[test]
fn test_payload_batch_field_integrity() {
    let e = Env::default();
    let batch_id = BytesN::from_array(&e, &[7u8; 32]);
    let root = BytesN::from_array(&e, &[8u8; 32]);

    let batch = PayloadBatch {
        batch_id: batch_id.clone(),
        source_chain_id: 42,
        payload_count: 100,
        merkle_root: root.clone(),
        batch_timestamp: 999_999,
        batch_ttl_seconds: 3600,
    };

    assert_eq!(batch.batch_id, batch_id);
    assert_eq!(batch.payload_count, 100);
    assert_eq!(batch.merkle_root, root);
    assert_eq!(batch.batch_ttl_seconds, 3600);
}

/// EncodedPayload: compressed_size must not exceed original_size for well-formed
/// entries — the struct stores these for the caller to validate.
#[test]
fn test_encoded_payload_size_fields() {
    let e = Env::default();
    let data = Bytes::from_slice(&e, b"hello cross-chain");

    let encoded = EncodedPayload {
        encoded_data: data,
        encoding_scheme: String::from_str(&e, "borsh"),
        compression_type: String::from_str(&e, "none"),
        original_size: 17,
        compressed_size: 17, // no compression applied
    };

    // For "none" compression the sizes are equal.
    assert_eq!(encoded.original_size, encoded.compressed_size);
}
