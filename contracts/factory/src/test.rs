#![cfg(test)]
extern crate std;
use super::*;

use emergency_guard::PauseType;
use soroban_sdk::{
    vec, Address, BytesN, Env,
};

mod liquidity_pool {
    soroban_sdk::contractimport!(
        file = "../../target/wasm32-unknown-unknown/release/liquidity_pool.wasm"
    );
}

fn pool_wasm_hash(env: &Env) -> BytesN<32> {
    env.deployer()
        .upload_contract_wasm(liquidity_pool::WASM)
}

fn setup_tokens(env: &Env, admin: &Address) -> (Address, Address) {
    let token_a = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    (token_a, token_b)
}

#[test]
fn test_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let token_admin = Address::generate(&env);
    let (token_a, token_b) = setup_tokens(&env, &token_admin);

    let result = factory_client.get_pair(&token_a, &token_b);
    assert_eq!(result, None);
}

#[test]
fn test_factory_initialize() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    factory_client.initialize(&admin);

    let admins = factory_client.get_admins();
    assert_eq!(admins.len(), 1);
    assert_eq!(admins.get(0).unwrap(), admin);
}

#[test]
fn test_create_pair_succeeds_when_not_paused() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let (token_a, token_b) = setup_tokens(&env, &admin);
    let pool_hash = pool_wasm_hash(&env);

    factory_client.initialize(&admin);

    let _pool_address = factory_client
        .create_pair(&token_a, &token_b, &pool_hash);

    let stored_pair = factory_client.get_pair(&token_a, &token_b);
    assert!(stored_pair.is_some());
}

#[test]
fn test_create_pair_fails_when_paused() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let (token_a, token_b) = setup_tokens(&env, &admin);
    let pool_hash = pool_wasm_hash(&env);

    factory_client.initialize(&admin);

    // Pause CREATE_PAIR
    let _ = factory_client.set_pause_state(&PauseType::CREATE_PAIR, &true);

    // create_pair should fail with Paused error
    let result = factory_client.try_create_pair(&token_a, &token_b, &pool_hash);
    assert_eq!(result, Err(Ok(Error::Paused)));
}

#[test]
fn test_create_pair_succeeds_after_unpause() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let (token_a, token_b) = setup_tokens(&env, &admin);
    let pool_hash = pool_wasm_hash(&env);

    factory_client.initialize(&admin);

    // Pause then unpause CREATE_PAIR
    let _ = factory_client.set_pause_state(&PauseType::CREATE_PAIR, &true);
    let _ = factory_client.set_pause_state(&PauseType::CREATE_PAIR, &false);

    // create_pair should succeed after unpause
    let _pool_address = factory_client
        .create_pair(&token_a, &token_b, &pool_hash);

    let stored_pair = factory_client.get_pair(&token_a, &token_b);
    assert!(stored_pair.is_some());
}

#[test]
fn test_other_operations_independent_of_create_pair() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let (token_a, token_b) = setup_tokens(&env, &admin);
    let pool_hash = pool_wasm_hash(&env);

    factory_client.initialize(&admin);

    // Pause SWAP (shouldn't affect CREATE_PAIR)
    let _ = factory_client.set_pause_state(&PauseType::SWAP, &true);

    // create_pair should still succeed since SWAP is a different flag
    let _pool_address = factory_client
        .create_pair(&token_a, &token_b, &pool_hash);

    let stored_pair = factory_client.get_pair(&token_a, &token_b);
    assert!(stored_pair.is_some());
}

#[test]
fn test_duplicate_pair_errors() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let (token_a, token_b) = setup_tokens(&env, &admin);
    let pool_hash = pool_wasm_hash(&env);

    factory_client.initialize(&admin);

    factory_client
        .create_pair(&token_a, &token_b, &pool_hash);

    let result = factory_client.try_create_pair(&token_a, &token_b, &pool_hash);
    assert_eq!(result, Err(Ok(Error::PairAlreadyExists)));
}

// ── Guard Management Tests ────────────────────────────────────────────────

#[test]
fn test_guard_admin_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admins = vec![&env, admin1.clone(), admin2.clone()];

    let _ = factory_client.init_guard(&admins, &2);
    assert_eq!(factory_client.get_threshold(), 2);
    assert_eq!(factory_client.get_admins().len(), 2);
    assert!(factory_client.is_admin(&admin1));
    assert!(factory_client.is_admin(&admin2));
}

#[test]
fn test_guard_admin_threshold_checks() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let admins = vec![&env, admin1.clone(), admin2.clone()];

    let _ = factory_client.init_guard(&admins, &2);

    let single_approver = vec![&env, admin1.clone()];
    let result = factory_client.try_add_admin(&single_approver, &new_admin);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));

    let full_approvals = vec![&env, admin1.clone(), admin2.clone()];
    let _ = factory_client.add_admin(&full_approvals, &new_admin);
    assert!(factory_client.is_admin(&new_admin));

    let result = factory_client.try_remove_admin(&single_approver, &new_admin);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));

    let _ = factory_client.remove_admin(&full_approvals, &new_admin);
    assert!(!factory_client.is_admin(&new_admin));
}
