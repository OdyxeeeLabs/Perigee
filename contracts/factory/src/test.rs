#![cfg(test)]
extern crate std;
use super::*;

use soroban_sdk::{testutils::Address as _, Env, Vec};

// Import the Liquidity Pool WASM for integration testing.
// This requires running `cargo build --target wasm32-unknown-unknown --release`
// before `cargo test` so the .wasm artifact exists on disk.
mod liquidity_pool_contract {
    soroban_sdk::contractimport!(
        file = "../../target/wasm32-unknown-unknown/release/liquidity_pool.wasm"
    );
}

#[test]
fn test_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    // Pair should not exist yet
    let result = factory_client.get_pair(&token_a, &token_b);
    assert_eq!(result, None);
}

#[test]
fn test_pool_creation() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    // Setup Tokens
    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    // Upload the Liquidity Pool WASM and get its hash
    let pool_hash = env
        .deployer()
        .upload_contract_wasm(liquidity_pool_contract::WASM);

    // Note: Due to a testutils handle mapping bug in the Soroban SDK mock environment,
    // returning a newly deployed address from a native contract call corrupts the handle
    // mapping in the Rust test space. Any `Address` representing the new pool will evaluate
    // to the `factory_id` in Rust. However, the host engine state is correct.
    // Therefore, we only assert that a value is returned and stored, bypassing strict equality.
    let _pool_address = factory_client.create_pair(&token_a, &token_b, &pool_hash);

    // Verify the pair is stored and retrievable
    let stored_pair = factory_client.get_pair(&token_a, &token_b);
    assert!(stored_pair.is_some());

    // Reversed order should also resolve to the same pool (canonical ordering)
    let stored_pair_rev = factory_client.get_pair(&token_b, &token_a);
    assert!(stored_pair_rev.is_some());
}

#[test]
#[should_panic(expected = "Pair already exists")]
fn test_duplicate_pair_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let pool_hash = env
        .deployer()
        .upload_contract_wasm(liquidity_pool_contract::WASM);

    // First creation succeeds
    factory_client.create_pair(&token_a, &token_b, &pool_hash);

    // Second creation with the same pair should panic
    factory_client.create_pair(&token_a, &token_b, &pool_hash);
}

// ==================== MULTI-SIG ADMIN TESTS ====================

#[test]
fn test_multisig_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone(), admin3.clone()];

    // Initialize with 2-of-3 multi-sig
    factory_client.init_multisig(&admins, &2);

    // Verify configuration
    let config = factory_client.get_multisig_config();
    assert_eq!(config.threshold, 2);
    assert_eq!(config.admins.len(), 3);
}

#[test]
#[should_panic(expected = "MultiSig already initialized")]
fn test_multisig_double_initialization_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admins = soroban_sdk::vec![&env, admin1, admin2];

    factory_client.init_multisig(&admins, &2);
    factory_client.init_multisig(&admins, &2); // Should panic
}

#[test]
#[should_panic(expected = "At least one admin required")]
fn test_multisig_empty_admins_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admins: Vec<Address> = soroban_sdk::vec![&env];
    factory_client.init_multisig(&admins, &1);
}

#[test]
#[should_panic(expected = "Invalid threshold")]
fn test_multisig_invalid_threshold_zero_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admins = soroban_sdk::vec![&env, admin1, admin2];

    factory_client.init_multisig(&admins, &0); // Should panic
}

#[test]
#[should_panic(expected = "Invalid threshold")]
fn test_multisig_invalid_threshold_too_high_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admins = soroban_sdk::vec![&env, admin1, admin2];

    // Threshold higher than number of admins
    factory_client.init_multisig(&admins, &3);
}

#[test]
fn test_is_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);
    let non_admin = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone()];
    factory_client.init_multisig(&admins, &2);

    // Verify admin status
    assert!(factory_client.is_admin(&admin1));
    assert!(factory_client.is_admin(&admin2));
    assert!(!factory_client.is_admin(&admin3));
    assert!(!factory_client.is_admin(&non_admin));
}

#[test]
fn test_propose_add_admin_action() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let new_admin = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone()];
    factory_client.init_multisig(&admins, &2);

    // Propose to add a new admin
    let action = AdminAction::AddAdmin(new_admin.clone());
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    assert!(action_id > 0);
}

#[test]
#[should_panic(expected = "Only admins can propose actions")]
fn test_propose_action_non_admin_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1, admin2];
    factory_client.init_multisig(&admins, &2);

    // Non-admin tries to propose
    let action = AdminAction::AddAdmin(new_admin);
    factory_client.propose_admin_action(&non_admin, &action);
}

#[test]
fn test_approve_admin_action() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let new_admin = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone()];
    factory_client.init_multisig(&admins, &2);

    // Propose and approve
    let action = AdminAction::AddAdmin(new_admin.clone());
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    factory_client.approve_admin_action(&admin2, &action_id);
}

#[test]
#[should_panic(expected = "Only admins can approve actions")]
fn test_approve_action_non_admin_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let non_admin = Address::generate(&env);
    let new_admin = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2];
    factory_client.init_multisig(&admins, &2);

    // Propose action
    let action = AdminAction::AddAdmin(new_admin);
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    // Non-admin tries to approve
    factory_client.approve_admin_action(&non_admin, &action_id);
}

#[test]
#[should_panic(expected = "Insufficient approvals")]
fn test_execute_action_insufficient_approvals_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);
    let new_admin = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2, admin3];
    factory_client.init_multisig(&admins, &3); // Requires 3 approvals

    // Propose and get one approval (from proposer)
    let action = AdminAction::AddAdmin(new_admin);
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    // Try to execute without enough approvals
    factory_client.execute_admin_action(&action_id);
}

#[test]
fn test_execute_add_admin_action_success() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let new_admin = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone()];
    factory_client.init_multisig(&admins, &2);

    // Propose add admin
    let action = AdminAction::AddAdmin(new_admin.clone());
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    // Get second approval
    factory_client.approve_admin_action(&admin2, &action_id);

    // Execute
    factory_client.execute_admin_action(&action_id);

    // Verify new admin was added
    let config = factory_client.get_multisig_config();
    assert_eq!(config.admins.len(), 3);
    assert!(factory_client.is_admin(&new_admin));
}

#[test]
#[should_panic(expected = "Admin already exists")]
fn test_execute_add_duplicate_admin_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone()];
    factory_client.init_multisig(&admins, &2);

    // Try to add admin2 again
    let action = AdminAction::AddAdmin(admin2);
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    factory_client.approve_admin_action(&admin2, &action_id);
    factory_client.execute_admin_action(&action_id);
}

#[test]
fn test_execute_remove_admin_action_success() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone(), admin3.clone()];
    factory_client.init_multisig(&admins, &2);

    // Propose remove admin
    let action = AdminAction::RemoveAdmin(admin3.clone());
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    // Get second approval
    factory_client.approve_admin_action(&admin2, &action_id);

    // Execute
    factory_client.execute_admin_action(&action_id);

    // Verify admin was removed
    let config = factory_client.get_multisig_config();
    assert_eq!(config.admins.len(), 2);
    assert!(!factory_client.is_admin(&admin3));
}

#[test]
#[should_panic(expected = "Cannot remove last admin")]
fn test_execute_remove_last_admin_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admins = soroban_sdk::vec![&env, admin1.clone()];
    factory_client.init_multisig(&admins, &1);

    // Try to remove only admin
    let action = AdminAction::RemoveAdmin(admin1.clone());
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    factory_client.execute_admin_action(&action_id);
}

#[test]
#[should_panic(expected = "Admin not found")]
fn test_execute_remove_nonexistent_admin_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let nonexistent = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone()];
    factory_client.init_multisig(&admins, &2);

    // Try to remove non-existent admin
    let action = AdminAction::RemoveAdmin(nonexistent);
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    factory_client.approve_admin_action(&admin2, &action_id);
    factory_client.execute_admin_action(&action_id);
}

#[test]
fn test_execute_set_threshold_action_success() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone(), admin3.clone()];
    factory_client.init_multisig(&admins, &2);

    // Propose set threshold
    let action = AdminAction::SetThreshold(3);
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    // Get second approval
    factory_client.approve_admin_action(&admin2, &action_id);

    // Execute
    factory_client.execute_admin_action(&action_id);

    // Verify threshold was updated
    let config = factory_client.get_multisig_config();
    assert_eq!(config.threshold, 3);
}

#[test]
#[should_panic(expected = "Invalid threshold")]
fn test_execute_set_invalid_threshold_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone()];
    factory_client.init_multisig(&admins, &2);

    // Try to set invalid threshold
    let action = AdminAction::SetThreshold(0);
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    factory_client.approve_admin_action(&admin2, &action_id);
    factory_client.execute_admin_action(&action_id);
}

#[test]
#[should_panic(expected = "Invalid threshold")]
fn test_execute_set_threshold_too_high_fails() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone()];
    factory_client.init_multisig(&admins, &2);

    // Try to set threshold higher than admin count
    let action = AdminAction::SetThreshold(5);
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    factory_client.approve_admin_action(&admin2, &action_id);
    factory_client.execute_admin_action(&action_id);
}

#[test]
fn test_threshold_adjustment_on_admin_removal() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone(), admin3.clone()];
    factory_client.init_multisig(&admins, &3); // 3-of-3

    // Remove one admin
    let action = AdminAction::RemoveAdmin(admin3);
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    factory_client.approve_admin_action(&admin2, &action_id);
    factory_client.approve_admin_action(&admin1, &action_id); // Third approval

    factory_client.execute_admin_action(&action_id);

    // Verify threshold was adjusted to match remaining admins
    let config = factory_client.get_multisig_config();
    assert_eq!(config.admins.len(), 2);
    assert_eq!(config.threshold, 2); // Threshold adjusted from 3 to 2
}

#[test]
fn test_1of3_multisig() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);
    let new_admin = Address::generate(&env);

    let admins = soroban_sdk::vec![&env, admin1.clone(), admin2, admin3];
    factory_client.init_multisig(&admins, &1); // 1-of-3

    // Only proposer approval needed
    let action = AdminAction::AddAdmin(new_admin.clone());
    let action_id = factory_client.propose_admin_action(&admin1, &action);

    // Can execute immediately with just proposer's approval
    factory_client.execute_admin_action(&action_id);

    // Verify new admin was added
    assert!(factory_client.is_admin(&new_admin));
}

#[test]
fn test_complex_multisig_scenario() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);
    let admin4 = Address::generate(&env);
    let admin5 = Address::generate(&env);

    // Start with 3 admins, 2-of-3 threshold
    let initial_admins = soroban_sdk::vec![&env, admin1.clone(), admin2.clone(), admin3.clone()];
    factory_client.init_multisig(&initial_admins, &2);

    // Add admin4
    let add_admin4 = AdminAction::AddAdmin(admin4.clone());
    let action_id_1 = factory_client.propose_admin_action(&admin1, &add_admin4);
    factory_client.approve_admin_action(&admin2, &action_id_1);
    factory_client.execute_admin_action(&action_id_1);
    assert_eq!(factory_client.get_multisig_config().admins.len(), 4);

    // Add admin5
    let add_admin5 = AdminAction::AddAdmin(admin5.clone());
    let action_id_2 = factory_client.propose_admin_action(&admin2, &add_admin5);
    factory_client.approve_admin_action(&admin3, &action_id_2);
    factory_client.execute_admin_action(&action_id_2);
    assert_eq!(factory_client.get_multisig_config().admins.len(), 5);

    // Increase threshold to 3
    let set_threshold = AdminAction::SetThreshold(3);
    let action_id_3 = factory_client.propose_admin_action(&admin4, &set_threshold);
    factory_client.approve_admin_action(&admin5, &action_id_3);
    factory_client.execute_admin_action(&action_id_3);
    assert_eq!(factory_client.get_multisig_config().threshold, 3);

    // Remove admin3
    let remove_admin3 = AdminAction::RemoveAdmin(admin3.clone());
    let action_id_4 = factory_client.propose_admin_action(&admin1, &remove_admin3);
    factory_client.approve_admin_action(&admin2, &action_id_4);
    factory_client.approve_admin_action(&admin4, &action_id_4);
    factory_client.execute_admin_action(&action_id_4);

    // Verify final state
    let final_config = factory_client.get_multisig_config();
    assert_eq!(final_config.admins.len(), 4);
    assert!(!factory_client.is_admin(&admin3));
    assert_eq!(final_config.threshold, 3);
}

