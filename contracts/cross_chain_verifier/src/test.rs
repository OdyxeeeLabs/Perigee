#![cfg(test)]

use crate::{CrossChainVerifier, CrossChainVerifierClient};
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
