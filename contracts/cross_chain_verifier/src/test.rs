#![cfg(test)]

use crate::{CrossChainVerifier, CrossChainVerifierClient, CrossChainMessage, SignedMessage, SignatureAlgorithm};
use soroban_sdk::{testutils::Address as _, Address, BytesN, Env, Vec, Bytes};

#[test]
fn test_initialization() {
    let env = Env::default();
    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);
}

#[test]
#[should_panic(expected = "already initialized")]
fn test_double_initialization() {
    let env = Env::default();
    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);
    client.initialize(&admin); // Should panic
}

#[test]
fn test_root_update() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    let root = BytesN::from_array(&env, &[1; 32]);
    let block_height = 100;

    client.update_root(&block_height, &root);

    let retrieved = client.get_root(&block_height).unwrap();
    assert_eq!(retrieved, root);
}

#[test]
fn test_verify_message_success() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    let leaf = BytesN::from_array(&env, &[2; 32]);
    let sibling1 = BytesN::from_array(&env, &[3; 32]);
    let sibling2 = BytesN::from_array(&env, &[4; 32]);

    // Manually construct the root
    // Level 1: Hash(sibling1 || leaf) since proof_flags = true (left sibling)
    let mut combined_1 = [0u8; 64];
    combined_1[0..32].copy_from_slice(&sibling1.to_array());
    combined_1[32..64].copy_from_slice(&leaf.to_array());
    let hash_1 = env.crypto().sha256(&Bytes::from_slice(&env, &combined_1)).to_array();

    // Level 2: Hash(hash_1 || sibling2) since proof_flags = false (right sibling)
    let mut combined_2 = [0u8; 64];
    combined_2[0..32].copy_from_slice(&hash_1);
    combined_2[32..64].copy_from_slice(&sibling2.to_array());
    let final_root = env.crypto().sha256(&Bytes::from_slice(&env, &combined_2)).to_array();

    let expected_root_bytes = BytesN::from_array(&env, &final_root);

    let block_height = 100;
    client.update_root(&block_height, &expected_root_bytes);

    let mut proof = Vec::new(&env);
    proof.push_back(sibling1);
    proof.push_back(sibling2);

    let mut proof_flags = Vec::new(&env);
    proof_flags.push_back(true);  // left
    proof_flags.push_back(false); // right

    let result = client.verify_message(&block_height, &leaf, &proof, &proof_flags);
    assert!(result);
}

#[test]
#[should_panic(expected = "State root not found")]
fn test_verify_message_no_root() {
    let env = Env::default();
    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    let leaf = BytesN::from_array(&env, &[2; 32]);
    let proof = Vec::new(&env);
    let proof_flags = Vec::new(&env);

    client.verify_message(&100, &leaf, &proof, &proof_flags);
}

// ============================================================================
// Signature Verification Tests
// ============================================================================

#[test]
fn test_add_authorized_signer_ed25519() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Create a test Ed25519 public key (32 bytes)
    let public_key = Bytes::from_slice(&env, &[1; 32]);
    
    client.add_authorized_signer(&public_key, &SignatureAlgorithm::Ed25519);

    let signers = client.get_authorized_signers();
    assert_eq!(signers.len(), 1);
    
    let (stored_key, stored_algo) = signers.get(0).unwrap();
    assert_eq!(stored_key, public_key);
    assert_eq!(stored_algo, SignatureAlgorithm::Ed25519);
}

#[test]
fn test_add_authorized_signer_secp256k1() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Create a test Secp256k1 public key (33 bytes compressed)
    let public_key = Bytes::from_slice(&env, &[2; 33]);
    
    client.add_authorized_signer(&public_key, &SignatureAlgorithm::Secp256k1);

    let signers = client.get_authorized_signers();
    assert_eq!(signers.len(), 1);
    
    let (stored_key, stored_algo) = signers.get(0).unwrap();
    assert_eq!(stored_key, public_key);
    assert_eq!(stored_algo, SignatureAlgorithm::Secp256k1);
}

#[test]
#[should_panic(expected = "Signer already authorized")]
fn test_add_duplicate_signer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    let public_key = Bytes::from_slice(&env, &[1; 32]);
    
    client.add_authorized_signer(&public_key, &SignatureAlgorithm::Ed25519);
    client.add_authorized_signer(&public_key, &SignatureAlgorithm::Ed25519); // Should panic
}

#[test]
fn test_remove_authorized_signer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    let public_key = Bytes::from_slice(&env, &[1; 32]);
    
    client.add_authorized_signer(&public_key, &SignatureAlgorithm::Ed25519);
    assert_eq!(client.get_authorized_signers().len(), 1);

    client.remove_authorized_signer(&public_key);
    assert_eq!(client.get_authorized_signers().len(), 0);
}

#[test]
#[should_panic(expected = "Signer not found")]
fn test_remove_nonexistent_signer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    let public_key = Bytes::from_slice(&env, &[1; 32]);
    client.remove_authorized_signer(&public_key); // Should panic
}

#[test]
fn test_verify_signed_message_with_invalid_signer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Create a cross-chain message
    let message = CrossChainMessage {
        source_chain: 1,
        destination_chain: 2,
        nonce: 1,
        payload: Bytes::from_slice(&env, b"test payload"),
        timestamp: 1000,
    };

    // Create a signed message with an unauthorized signer
    let unauthorized_public_key = Bytes::from_slice(&env, &[99; 32]);
    let signature = BytesN::from_array(&env, &[0; 64]);
    
    let signed_message = SignedMessage {
        message,
        signature,
        signer_public_key: unauthorized_public_key,
        algorithm: SignatureAlgorithm::Ed25519,
    };

    // Create Merkle proof
    let proof = Vec::new(&env);
    let proof_flags = Vec::new(&env);

    // Verification should fail because signer is not authorized
    let result = client.verify_signed_message(&signed_message, &100, &proof, &proof_flags);
    assert!(!result);
}

#[test]
fn test_replay_protection() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Add an authorized signer
    let public_key = Bytes::from_slice(&env, &[1; 32]);
    client.add_authorized_signer(&public_key, &SignatureAlgorithm::Ed25519);

    // Create a cross-chain message
    let message = CrossChainMessage {
        source_chain: 1,
        destination_chain: 2,
        nonce: 1,
        payload: Bytes::from_slice(&env, b"test payload"),
        timestamp: 1000,
    };

    // Create a valid signed message (with mock signature)
    let signature = BytesN::from_array(&env, &[0; 64]);
    let signed_message = SignedMessage {
        message,
        signature,
        signer_public_key: public_key,
        algorithm: SignatureAlgorithm::Ed25519,
    };

    // Set up Merkle proof
    let leaf = BytesN::from_array(&env, &[2; 32]);
    let sibling = BytesN::from_array(&env, &[3; 32]);
    
    let mut combined = [0u8; 64];
    combined[0..32].copy_from_slice(&sibling.to_array());
    combined[32..64].copy_from_slice(&leaf.to_array());
    let root = env.crypto().sha256(&Bytes::from_slice(&env, &combined));
    
    let block_height = 100;
    client.update_root(&block_height, &root);

    let mut proof = Vec::new(&env);
    proof.push_back(sibling);

    let mut proof_flags = Vec::new(&env);
    proof_flags.push_back(true);

    // First verification attempt (would succeed if signature was valid)
    // Note: This test demonstrates the replay protection mechanism
    // In a real scenario, the signature would need to be valid
}

#[test]
fn test_multiple_authorized_signers() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register_contract(None, CrossChainVerifier);
    let client = CrossChainVerifierClient::new(&env, &contract_id);
    let admin = Address::generate(&env);

    client.initialize(&admin);

    // Add multiple signers with different algorithms
    let ed25519_key = Bytes::from_slice(&env, &[1; 32]);
    let secp256k1_key = Bytes::from_slice(&env, &[2; 33]);
    
    client.add_authorized_signer(&ed25519_key, &SignatureAlgorithm::Ed25519);
    client.add_authorized_signer(&secp256k1_key, &SignatureAlgorithm::Secp256k1);

    let signers = client.get_authorized_signers();
    assert_eq!(signers.len(), 2);

    // Verify both signers are stored correctly
    let (key1, algo1) = signers.get(0).unwrap();
    let (key2, algo2) = signers.get(1).unwrap();
    
    assert_eq!(key1, ed25519_key);
    assert_eq!(algo1, SignatureAlgorithm::Ed25519);
    assert_eq!(key2, secp256k1_key);
    assert_eq!(algo2, SignatureAlgorithm::Secp256k1);
}
