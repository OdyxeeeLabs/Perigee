#![cfg(test)]
extern crate std;
use super::*;

use soroban_sdk::{BytesN, Env};

mod liquidity_pool {
    soroban_sdk::contractimport!(
        file = "../../target/wasm32-unknown-unknown/release/liquidity_pool.wasm"
    );
}

fn pool_wasm_hash(env: &Env) -> BytesN<32> {
    env.deployer().upload_contract_wasm(liquidity_pool::WASM)
}

#[test]
fn test_initialize() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);

    factory_client.initialize(&admin);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let result = factory_client.get_pair(&token_a, &token_b);
    assert_eq!(result, None);
}

#[test]
fn test_initialize_twice() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    factory_client.initialize(&admin);
    assert_eq!(
        factory_client.try_initialize(&admin),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn test_create_pair() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    factory_client.initialize(&admin);

    let wasm_hash = pool_wasm_hash(&env);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let pair_address = factory_client.create_pair(&token_a, &token_b, &wasm_hash);

    let stored = factory_client.get_pair(&token_a, &token_b);
    assert_eq!(stored, Some(pair_address));
}

#[test]
fn test_create_pair_duplicate() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    factory_client.initialize(&admin);

    let wasm_hash = pool_wasm_hash(&env);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    factory_client.create_pair(&token_a, &token_b, &wasm_hash);
    assert_eq!(
        factory_client.try_create_pair(&token_a, &token_b, &wasm_hash),
        Err(Ok(Error::PairAlreadyExists))
    );
}

#[test]
fn test_get_pair_sorted() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    factory_client.initialize(&admin);

    let wasm_hash = pool_wasm_hash(&env);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    let pair = factory_client.create_pair(&token_a, &token_b, &wasm_hash);

    // get_pair should work regardless of token order
    let stored = factory_client.get_pair(&token_b, &token_a);
    assert_eq!(stored, Some(pair));
}

#[test]
fn test_create_pair_normalizes_addresses() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    factory_client.initialize(&admin);

    let wasm_hash = pool_wasm_hash(&env);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    // Create with (token_b, token_a) order
    let pair = factory_client.create_pair(&token_b, &token_a, &wasm_hash);

    // Should be retrievable with both orders
    assert_eq!(
        factory_client.get_pair(&token_a, &token_b),
        Some(pair.clone())
    );
    assert_eq!(factory_client.get_pair(&token_b, &token_a), Some(pair));
}
