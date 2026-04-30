#![cfg(test)]

use soroban_sdk::testutils::Address as _;
use soroban_sdk::{Address, Env, String};

use crate::{Domain, Transfer, TypedDataAuth};

#[test]
fn test_domain_hash_is_nonzero() {
    let env = Env::default();
    let contract_address = Address::generate(&env);
    let domain = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address,
    };
    let hash = TypedDataAuth::compute_domain_hash(&env, &domain);
    assert!(!hash.is_empty());
}

#[test]
fn test_struct_hash_is_nonzero() {
    let env = Env::default();
    let from = Address::generate(&env);
    let to = Address::generate(&env);
    let transfer = Transfer { from, to, amount: 1000 };
    let hash = TypedDataAuth::compute_struct_hash(&env, &transfer);
    assert!(!hash.is_empty());
}

#[test]
fn test_message_hash_is_nonzero() {
    let env = Env::default();
    let contract_address = Address::generate(&env);
    let domain = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address,
    };
    let from = Address::generate(&env);
    let to = Address::generate(&env);
    let transfer = Transfer { from, to, amount: 500 };

    let domain_hash = TypedDataAuth::compute_domain_hash(&env, &domain);
    let struct_hash = TypedDataAuth::compute_struct_hash(&env, &transfer);
    let message_hash = TypedDataAuth::compute_message_hash(&env, domain_hash, struct_hash);
    assert!(!message_hash.is_empty());
}

#[test]
fn test_domain_separator_consistency() {
    let env = Env::default();
    let contract_address = Address::generate(&env);
    let domain1 = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address.clone(),
    };
    let domain2 = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address,
    };
    assert_eq!(
        TypedDataAuth::compute_domain_hash(&env, &domain1),
        TypedDataAuth::compute_domain_hash(&env, &domain2),
    );
}

#[test]
fn test_different_domains_produce_different_hashes() {
    let env = Env::default();
    let contract_address = Address::generate(&env);
    let domain1 = Domain {
        name: String::from_str(&env, "TestContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address.clone(),
    };
    let domain2 = Domain {
        name: String::from_str(&env, "OtherContract"),
        version: String::from_str(&env, "1.0"),
        chain_id: 1,
        verifying_contract: contract_address,
    };
    assert_ne!(
        TypedDataAuth::compute_domain_hash(&env, &domain1),
        TypedDataAuth::compute_domain_hash(&env, &domain2),
    );
}
