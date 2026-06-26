#![cfg(test)]
extern crate std;
use super::*;

use soroban_sdk::{testutils::Address as _, Env};

// Import the compiled Liquidity Pool WASM.
// The WASM must be built before running tests:
//   cargo build -p liquidity_pool --target wasm32-unknown-unknown --release
//
// `contractimport!` reads the file at compile time and exposes a `WASM` constant
// (a `&[u8]` byte slice) and a generated `ContractClient` for the imported contract.
mod lp {
    soroban_sdk::contractimport!(
        file = "../../../target/wasm32-unknown-unknown/release/liquidity_pool.wasm"
    );
}

#[test]
fn test_create_pair() {
    let env = Env::default();
    env.mock_all_auths();

    // Register the factory contract
    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    // Upload the compiled Liquidity Pool WASM and get its hash
    let pool_hash = env.deployer().upload_contract_wasm(lp::WASM);

    // Set up two distinct token addresses
    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    // Create a liquidity pool for the token pair
    let pool_address = factory_client.create_pair(&token_a, &token_b, &pool_hash);

    // The deployed pool should have its own address, different from the factory
    assert!(pool_address != factory_id);

    // `get_pair` should return the same address regardless of token order
    let found = factory_client.get_pair(&token_a, &token_b);
    assert_eq!(found, Some(pool_address.clone()));

    let found_reversed = factory_client.get_pair(&token_b, &token_a);
    assert_eq!(found_reversed, Some(pool_address));
}

#[test]
#[should_panic(expected = "Pair already exists")]
fn test_create_duplicate_pair_panics() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let pool_hash = env.deployer().upload_contract_wasm(lp::WASM);

    let token_admin = Address::generate(&env);
    let token_a = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();
    let token_b = env
        .register_stellar_asset_contract_v2(token_admin.clone())
        .address();

    // First creation succeeds
    factory_client.create_pair(&token_a, &token_b, &pool_hash);

    // Second creation with same pair (reversed order) should panic
    factory_client.create_pair(&token_b, &token_a, &pool_hash);
}
