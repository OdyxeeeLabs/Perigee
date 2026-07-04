#![cfg(test)]
extern crate std;
use super::*;

use soroban_sdk::{
    testutils::Address as _,
    Address, BytesN, Env,
};

// Issue #310: Import the compiled Liquidity Pool WASM so integration tests deploy
// the real contract instead of relying on a placeholder zero-hash.
mod liquidity_pool {
    soroban_sdk::contractimport!(
        file = "../../target/wasm32v1-none/release/liquidity_pool.wasm"
    );
}

fn pool_wasm_hash(env: &Env) -> BytesN<32> {
    env.deployer().upload_contract_wasm(liquidity_pool::WASM)
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
fn test_pause_create_pair() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let pool_hash = env
        .deployer()
        .upload_contract_wasm(liquidity_pool::WASM);

    factory_client.initialize(&admin);

    const PAUSE_CREATE_PAIR_FLAG: u32 = 1 << 6;

    env.invoke_contract::<()>(
        &factory_id,
        &soroban_sdk::Symbol::new(&env, "set_pause_state"),
        soroban_sdk::vec![&env, PAUSE_CREATE_PAIR_FLAG.into_val(&env), true.into_val(&env)],
    );

    let result = factory_client.try_create_pair(&token_a, &token_b, &pool_hash);
    assert_eq!(result, Err(Ok(Error::Paused)));

    env.invoke_contract::<()>(
        &factory_id,
        &soroban_sdk::Symbol::new(&env, "set_pause_state"),
        soroban_sdk::vec![&env, PAUSE_CREATE_PAIR_FLAG.into_val(&env), false.into_val(&env)],
    );

    let created = factory_client.create_pair(&token_a, &token_b, &pool_hash);
    assert!(created != factory_id);
}

#[test]
fn test_duplicate_pair_errors() {
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

    let pool_hash = pool_wasm_hash(&env);

    // First creation succeeds
    factory_client
        .create_pair(&token_a, &token_b, &pool_hash);

    // Second creation with the same pair should return a pair-exists error
    let result = factory_client.try_create_pair(&token_a, &token_b, &pool_hash);
    assert_eq!(result, Err(Ok(Error::PairAlreadyExists)));
}
/*
// TODO: Enable this once we have a way to import the Liquidity Pool WASM
// let pool_hash = env.deployer().upload_contract_wasm(liquidity_pool_contract::WASM);
// let pool_address = factory_client.create_pair(&token_a, &token_b, &pool_hash);
// assert!(pool_address != factory_id);
*/


