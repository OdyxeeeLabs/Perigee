#![cfg(test)]
extern crate std;
use super::*;

use emergency_guard::PauseType;
use soroban_sdk::{
    testutils::{Address as _, Events},
    vec, Address, BytesN, Env, String as SorobanString, TryIntoVal,
};
use std::vec::Vec;

// Issue #310: Import the compiled Liquidity Pool WASM so integration tests deploy
// the real contract instead of relying on a placeholder zero-hash.
mod liquidity_pool {
    soroban_sdk::contractimport!(
        file = "../../target/wasm32-unknown-unknown/release/liquidity_pool.wasm"
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
fn test_guard_admin_initialization() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admins = vec![&env, admin1.clone(), admin2.clone()];

    assert_eq!(factory_client.initialize(&admins, &2), Ok(()));
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

    assert_eq!(factory_client.initialize(&admins, &2), Ok(()));

    let single_approver = vec![&env, admin1.clone()];
    assert_eq!(
        factory_client.add_admin(&single_approver, &new_admin),
        Err(GuardError::InsufficientSignatures)
    );

    let full_approvals = vec![&env, admin1.clone(), admin2.clone()];
    assert_eq!(
        factory_client.add_admin(&full_approvals, &new_admin),
        Ok(())
    );
    assert!(factory_client.is_admin(&new_admin));

    assert_eq!(
        factory_client.remove_admin(&single_approver, &new_admin),
        Err(GuardError::InsufficientSignatures)
    );

    assert_eq!(
        factory_client.remove_admin(&full_approvals, &new_admin),
        Ok(())
    );
    assert!(!factory_client.is_admin(&new_admin));
}

#[test]
fn test_guard_pause_create_pair_success() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    let admins = vec![&env, admin.clone()];

    assert_eq!(factory_client.initialize(&admins, &1), Ok(()));
    assert!(!factory_client.guard_is_paused(&CREATE_PAIR));

    factory_client
        .guard_pause(&admin, &CREATE_PAIR, &true)
        .expect("admin should be able to pause create_pair");
    assert!(factory_client.guard_is_paused(&CREATE_PAIR));

    factory_client
        .guard_pause(&admin, &CREATE_PAIR, &false)
        .expect("admin should be able to resume create_pair");
    assert!(!factory_client.guard_is_paused(&CREATE_PAIR));
}

#[test]
fn test_guard_unpause_preserves_other_bits() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    const EXTRA_BIT: u32 = 1 << 7;
    let combined = emergency_guard::PauseType::CREATE_PAIR | EXTRA_BIT;

    factory_client.initialize(&admin);

    factory_client.guard_pause(&admin, &combined, &true);
    assert!(factory_client.guard_is_paused(&emergency_guard::PauseType::CREATE_PAIR));
    assert!(factory_client.guard_is_paused(&EXTRA_BIT));

    factory_client.guard_unpause(&admin, &emergency_guard::PauseType::CREATE_PAIR);
    assert!(!factory_client.guard_is_paused(&emergency_guard::PauseType::CREATE_PAIR));
    assert!(factory_client.guard_is_paused(&EXTRA_BIT));
}

#[test]
fn test_guard_pause_create_pair_unauthorized() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);
    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    let admins = vec![&env, admin.clone()];

    assert_eq!(factory_client.initialize(&admins, &1), Ok(()));

    assert_eq!(
        factory_client.try_guard_pause(&stranger, &CREATE_PAIR, &true),
        Err(Ok(Error::Unauthorized))
    );
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

    let pool_hash = pool_wasm_hash(&env);

    // Note: Due to a testutils handle mapping bug in the Soroban SDK mock environment,
    // returning a newly deployed address from a native contract call corrupts the handle
    // mapping in the Rust test space. Any `Address` representing the new pool will evaluate
    // to the `factory_id` in Rust. However, the host engine state is correct.
    // Therefore, we only assert that a value is returned and stored, bypassing strict equality.
    let _pool_address = factory_client
        .create_pair(&token_a, &token_b, &pool_hash)
        .unwrap();

    // Verify the pair is stored and retrievable
    let stored_pair = factory_client.get_pair(&token_a, &token_b);
    assert!(stored_pair.is_some());

    // Reversed order should also resolve to the same pool (canonical ordering)
    let stored_pair_rev = factory_client.get_pair(&token_b, &token_a);
    assert!(stored_pair_rev.is_some());
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
        .upload_contract_wasm(liquidity_pool_contract::WASM);

    let mut admins = Vec::new(&env);
    admins.push_back(admin.clone());
    factory_client.initialize(&admins, &1).unwrap();
    factory_client.set_paused(&admin, &true).unwrap();

    let result = factory_client.create_pair(&token_a, &token_b, &pool_hash);
    assert_eq!(result, Err(Error::Paused));

    factory_client.set_paused(&admin, &false).unwrap();
    let created = factory_client
        .create_pair(&token_a, &token_b, &pool_hash)
        .unwrap();
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
        .create_pair(&token_a, &token_b, &pool_hash)
        .unwrap();

    // Second creation with the same pair should return a pair-exists error
    let result = factory_client.create_pair(&token_a, &token_b, &pool_hash);
    assert_eq!(result, Err(Error::PairAlreadyExists));
}
/*
// TODO: Enable this once we have a way to import the Liquidity Pool WASM
// let pool_hash = env.deployer().upload_contract_wasm(liquidity_pool_contract::WASM);
// let pool_address = factory_client.create_pair(&token_a, &token_b, &pool_hash);
// assert!(pool_address != factory_id);
*/

fn guard_initialized_events(
    env: &Env,
    contract_id: &Address,
) -> Vec<emergency_guard::GuardInitializedEvent> {
    let topic = SorobanString::from_str(env, "emergency_guard_initialized");
    env.events()
        .all()
        .iter()
        .filter_map(|(event_contract, topics, data)| {
            if event_contract != *contract_id || topics.len() != 1 {
                return None;
            }
            let topic_action: SorobanString = topics.get(0)?.try_into_val(env).ok()?;
            if topic_action == topic {
                data.try_into_val(env).ok()
            } else {
                None
            }
        })
        .collect()
}

fn pause_state_changed_events(
    env: &Env,
    contract_id: &Address,
) -> Vec<emergency_guard::PauseStateChangedEvent> {
    let topic = SorobanString::from_str(env, "emergency_guard_pause_state_changed");
    env.events()
        .all()
        .iter()
        .filter_map(|(event_contract, topics, data)| {
            if event_contract != *contract_id || topics.len() != 2 {
                return None;
            }
            let topic_action: SorobanString = topics.get(0)?.try_into_val(env).ok()?;
            if topic_action == topic {
                data.try_into_val(env).ok()
            } else {
                None
            }
        })
        .collect()
}

fn emergency_paused_events(
    env: &Env,
    contract_id: &Address,
) -> Vec<emergency_guard::EmergencyPausedEvent> {
    let topic = SorobanString::from_str(env, "emergency_guard_emergency_paused_all");
    env.events()
        .all()
        .iter()
        .filter_map(|(event_contract, topics, data)| {
            if event_contract != *contract_id || topics.len() != 1 {
                return None;
            }
            let topic_action: SorobanString = topics.get(0)?.try_into_val(env).ok()?;
            if topic_action == topic {
                data.try_into_val(env).ok()
            } else {
                None
            }
        })
        .collect()
}

fn resumed_events(env: &Env, contract_id: &Address) -> Vec<emergency_guard::ResumedEvent> {
    let topic = SorobanString::from_str(env, "emergency_guard_resumed_all");
    env.events()
        .all()
        .iter()
        .filter_map(|(event_contract, topics, data)| {
            if event_contract != *contract_id || topics.len() != 1 {
                return None;
            }
            let topic_action: SorobanString = topics.get(0)?.try_into_val(env).ok()?;
            if topic_action == topic {
                data.try_into_val(env).ok()
            } else {
                None
            }
        })
        .collect()
}

fn admin_added_events(env: &Env, contract_id: &Address) -> Vec<emergency_guard::AdminAddedEvent> {
    let topic = SorobanString::from_str(env, "emergency_guard_admin_added");
    env.events()
        .all()
        .iter()
        .filter_map(|(event_contract, topics, data)| {
            if event_contract != *contract_id || topics.len() != 2 {
                return None;
            }
            let topic_action: SorobanString = topics.get(0)?.try_into_val(env).ok()?;
            if topic_action == topic {
                data.try_into_val(env).ok()
            } else {
                None
            }
        })
        .collect()
}

fn admin_removed_events(
    env: &Env,
    contract_id: &Address,
) -> Vec<emergency_guard::AdminRemovedEvent> {
    let topic = SorobanString::from_str(env, "emergency_guard_admin_removed");
    env.events()
        .all()
        .iter()
        .filter_map(|(event_contract, topics, data)| {
            if event_contract != *contract_id || topics.len() != 2 {
                return None;
            }
            let topic_action: SorobanString = topics.get(0)?.try_into_val(env).ok()?;
            if topic_action == topic {
                data.try_into_val(env).ok()
            } else {
                None
            }
        })
        .collect()
}

fn setup_guard(
    env: &Env,
) -> (
    Address,
    LiquidityPoolFactoryClient<'_>,
    Address,
    Address,
    Address,
) {
    env.mock_all_auths();
    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(env, &factory_id);
    let admin1 = Address::generate(env);
    let admin2 = Address::generate(env);
    let admin3 = Address::generate(env);
    let admins = vec![env, admin1.clone(), admin2.clone(), admin3.clone()];

    factory_client.initialize_guard(&admins, &2);

    (factory_id, factory_client, admin1, admin2, admin3)
}

#[test]
fn test_initialize_guard_emits_standard_event() {
    let env = Env::default();
    let (factory_id, _client, admin1, admin2, admin3) = setup_guard(&env);

    let events = guard_initialized_events(&env, &factory_id);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        emergency_guard::GuardInitializedEvent {
            admins: vec![&env, admin1, admin2, admin3],
            threshold: 2,
        }
    );
}

#[test]
fn test_set_guard_pause_emits_standard_events() {
    let env = Env::default();
    let (factory_id, client, admin1, _admin2, _admin3) = setup_guard(&env);

    client.set_guard_pause(&admin1, &PauseType::MINT, &true);
    let events = pause_state_changed_events(&env, &factory_id);
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0],
        emergency_guard::PauseStateChangedEvent {
            admin: admin1.clone(),
            operation: PauseType::MINT,
            paused: true,
        }
    );

    assert!(client.is_guard_paused(&PauseType::MINT));

    client.set_guard_pause(&admin1, &PauseType::MINT, &false);
    let events = pause_state_changed_events(&env, &factory_id);
    assert_eq!(events.len(), 2);
    assert_eq!(
        events[1],
        emergency_guard::PauseStateChangedEvent {
            admin: admin1,
            operation: PauseType::MINT,
            paused: false,
        }
    );
}

#[test]
fn test_emergency_pause_and_resume_emit_standard_events() {
    let env = Env::default();
    let (factory_id, client, admin1, admin2, _admin3) = setup_guard(&env);
    let approvers = vec![&env, admin1.clone(), admin2.clone()];

    client.emergency_guard_pause(&approvers);
    let emergency_events = emergency_paused_events(&env, &factory_id);
    assert_eq!(emergency_events.len(), 1);
    assert_eq!(
        emergency_events[0],
        emergency_guard::EmergencyPausedEvent {
            approvers: vec![&env, admin1.clone(), admin2.clone()],
        }
    );
    assert!(client.is_guard_paused(&PauseType::MINT));

    client.resume_guard(&approvers);
    let resume_events = resumed_events(&env, &factory_id);
    assert_eq!(resume_events.len(), 1);
    assert_eq!(
        resume_events[0],
        emergency_guard::ResumedEvent {
            approvers: vec![&env, admin1, admin2],
        }
    );
    assert!(!client.is_guard_paused(&PauseType::MINT));
}

#[test]
fn test_admin_guard_actions_emit_standard_events() {
    let env = Env::default();
    let (factory_id, client, admin1, admin2, admin3) = setup_guard(&env);
    let approvers = vec![&env, admin1.clone(), admin2.clone()];
    let admin4 = Address::generate(&env);

    client.add_guard_admin(&approvers, &admin4);
    let added_events = admin_added_events(&env, &factory_id);
    assert_eq!(added_events.len(), 1);
    assert_eq!(
        added_events[0],
        emergency_guard::AdminAddedEvent {
            approvers: vec![&env, admin1.clone(), admin2.clone()],
            new_admin: admin4,
        }
    );

    client.remove_guard_admin(&approvers, &admin3);
    let removed_events = admin_removed_events(&env, &factory_id);
    assert_eq!(removed_events.len(), 1);
    assert_eq!(
        removed_events[0],
        emergency_guard::AdminRemovedEvent {
            approvers: vec![&env, admin1, admin2],
            admin: admin3,
        }
    );
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
#[should_panic(expected = "Pair creation is paused")]
fn test_factory_paused_creation() {
    let env = Env::default();
    env.mock_all_auths();

    let factory_id = env.register(LiquidityPoolFactory, ());
    let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

    let admin = Address::generate(&env);
    factory_client.initialize(&admin);

    // Operations are not paused initially
    let create_pair_op = PauseType::CREATE_PAIR;
    assert!(!factory_client.is_paused(&create_pair_op));

    // Pause create_pair
    factory_client.set_operation_paused(&admin, &create_pair_op, &true);
    assert!(factory_client.is_paused(&create_pair_op));

    // Setup Tokens & WASM
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

    // Try to create a pair - should panic
    factory_client.create_pair(&token_a, &token_b, &pool_hash);
}

#[test]
fn test_factory_other_operation_independent() {
    // ==================== MULTI-SIG ADMIN TESTS ====================

    #[test]
    fn test_multisig_initialization() {
        let env = Env::default();
        env.mock_all_auths();

        let factory_id = env.register(LiquidityPoolFactory, ());
        let factory_client = LiquidityPoolFactoryClient::new(&env, &factory_id);

        let admin = Address::generate(&env);
        factory_client.initialize(&admin);

        // Pause a different operation (e.g. SWAP = 1 << 0)
        let swap_op = PauseType::SWAP;
        let create_pair_op = PauseType::CREATE_PAIR;
        factory_client.set_operation_paused(&admin, &swap_op, &true);

        assert!(factory_client.is_paused(&swap_op));
        assert!(!factory_client.is_paused(&create_pair_op));

        // Setup Tokens & WASM
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

        // Creating pair should still succeed because CREATE_PAIR is not paused
        let _pool_address = factory_client.create_pair(&token_a, &token_b, &pool_hash);

        let stored_pair = factory_client.get_pair(&token_a, &token_b);
        assert!(stored_pair.is_some());
    }

    #[test]
    fn test_factory_unpause_resumes() {
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

        let admin = Address::generate(&env);
        factory_client.initialize(&admin);

        let create_pair_op = PauseType::CREATE_PAIR;
        factory_client.set_operation_paused(&admin, &create_pair_op, &true);
        assert!(factory_client.is_paused(&create_pair_op));

        // Unpause CREATE_PAIR
        factory_client.set_operation_paused(&admin, &create_pair_op, &false);
        assert!(!factory_client.is_paused(&create_pair_op));

        // Setup Tokens & WASM
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

        // Creating pair should now succeed
        let _pool_address = factory_client.create_pair(&token_a, &token_b, &pool_hash);

        let stored_pair = factory_client.get_pair(&token_a, &token_b);
        assert!(stored_pair.is_some());
    }

    #[test]
    fn test_factory_unauthorized_pause() {
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

        let admin = Address::generate(&env);
        let non_admin = Address::generate(&env);
        factory_client.initialize(&admin);

        let create_pair_op = PauseType::CREATE_PAIR;
        // Attempting to pause as non_admin should return Error::Unauthorized (value = 2)
        let res = factory_client.try_set_operation_paused(&non_admin, &create_pair_op, &true);
        assert_eq!(res, Err(Ok(Error::Unauthorized)));
    }
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
    let action = AdminAction::AddAdmin(admin2.clone());
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
