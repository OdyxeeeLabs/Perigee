#![cfg(test)]

//! Unit tests for the strategy orchestration contracts.
//!
//! Covers the two issue scopes:
//!
//! - [#659] rotation — target weights, drift-based planning, the `max_legs` gas
//!   cap, dust filtering and the slippage gate;
//! - [#656] cross-chain yield — chain registration, bridge-driven deployment,
//!   relayer-confirmed settlement and recalls.
//!
//! Closes OdyxeeeLabs/Perigee#659
//! Closes OdyxeeeLabs/Perigee#656

use crate::{
    CrossChainYield, CrossChainYieldClient, Error, RotationEngine, RotationEngineClient,
    RotationPolicy, TargetWeight,
};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

mod mock_bridge {
    use soroban_sdk::{contract, contractimpl, Address, Env};

    /// A stand-in for a Wormhole/LayerZero-style bridge. It returns a
    /// deterministic message id so tests can assert the call happened.
    #[contract]
    pub struct MockBridge;

    #[contractimpl]
    impl MockBridge {
        pub fn send(
            _env: Env,
            _token: Address,
            amount: i128,
            chain_id: u32,
            _recipient: Address,
        ) -> u64 {
            amount as u64 + chain_id as u64
        }

        pub fn recall(_env: Env, chain_id: u32, amount: i128) -> u64 {
            amount as u64 + chain_id as u64
        }
    }
}

// ── RotationEngine ───────────────────────────────────────────────────────────

const POLICY: RotationPolicy = RotationPolicy {
    drift_threshold_bps: 500,
    slippage_bps: 100,
    max_legs: 2,
    min_leg_bps: 1_000,
};

fn rotation_setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(RotationEngine, ());
    let client = RotationEngineClient::new(&env, &contract_id);
    client.initialize(&admin);

    (env, contract_id, admin)
}

fn targets(env: &Env, a: &Address, b: &Address, a_bps: u32, b_bps: u32) -> Vec<TargetWeight> {
    vec![
        env,
        TargetWeight {
            strategy: a.clone(),
            target_bps: a_bps,
        },
        TargetWeight {
            strategy: b.clone(),
            target_bps: b_bps,
        },
    ]
}

fn current(env: &Env, a: &Address, b: &Address, a_bps: u32, b_bps: u32) -> Vec<TargetWeight> {
    targets(env, a, b, a_bps, b_bps)
}

#[test]
fn rotation_engine_initializes_once() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);

    assert_eq!(client.admin(), admin);
    assert_eq!(
        client.try_initialize(&admin),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn policy_can_be_set_and_read_back() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);

    client.set_policy(&admin, &vault, &POLICY);
    assert_eq!(client.get_policy(&vault), POLICY);
}

#[test]
fn an_invalid_policy_is_refused() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);

    let bad = RotationPolicy {
        max_legs: 0,
        ..POLICY
    };
    assert_eq!(
        client.try_set_policy(&admin, &vault, &bad),
        Err(Ok(Error::InvalidPolicy))
    );
}

#[test]
fn targets_must_sum_to_one_hundred_percent() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    let bad = targets(&env, &a, &b, 6_000, 3_000);
    assert_eq!(
        client.try_set_targets(&admin, &vault, &bad),
        Err(Ok(Error::InvalidTargets))
    );
}

#[test]
fn duplicate_targets_are_refused() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let a = Address::generate(&env);

    let dup = vec![
        &env,
        TargetWeight {
            strategy: a.clone(),
            target_bps: 5_000,
        },
        TargetWeight {
            strategy: a,
            target_bps: 5_000,
        },
    ];
    assert_eq!(
        client.try_set_targets(&admin, &vault, &dup),
        Err(Ok(Error::DuplicateStrategy))
    );
}

#[test]
fn a_balanced_book_needs_no_rotation() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    client.set_policy(&admin, &vault, &POLICY);
    client.set_targets(&admin, &vault, &targets(&env, &a, &b, 6_000, 4_000));

    let plan = client.plan_rotation(&vault, &current(&env, &a, &b, 6_000, 4_000));
    assert!(!plan.rotation_needed);
    assert_eq!(plan.legs.len(), 0);
    assert_eq!(plan.total_drift_bps, 0);
}

#[test]
fn a_drift_above_the_threshold_produces_a_leg() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    client.set_policy(&admin, &vault, &POLICY);
    client.set_targets(&admin, &vault, &targets(&env, &a, &b, 6_000, 4_000));

    // a drifts from 6_000 to 5_000 (1_000 bps), b from 4_000 to 5_000.
    let plan = client.plan_rotation(&vault, &current(&env, &a, &b, 5_000, 5_000));
    assert!(plan.rotation_needed);
    assert_eq!(plan.legs.len(), 2);
    assert_eq!(plan.total_drift_bps, 2_000);

    let first = plan.legs.get(0).unwrap();
    assert_eq!(first.strategy, a);
    assert_eq!(first.current_bps, 5_000);
    assert_eq!(first.target_bps, 6_000);
    assert_eq!(first.drift_bps, 1_000);
}

#[test]
fn a_drift_below_the_threshold_produces_no_leg() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    client.set_policy(&admin, &vault, &POLICY);
    client.set_targets(&admin, &vault, &targets(&env, &a, &b, 6_000, 4_000));

    // 400 bps of drift is under the 500 bps threshold.
    let plan = client.plan_rotation(&vault, &current(&env, &a, &b, 5_600, 4_400));
    assert!(!plan.rotation_needed);
    assert_eq!(plan.legs.len(), 0);
    assert_eq!(plan.total_drift_bps, 800);
}

#[test]
fn dust_below_the_leg_floor_is_never_rotated() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    client.set_policy(&admin, &vault, &POLICY);
    // a is 9_000, b is 1_000 — b is below the 1_000 bps leg floor.
    client.set_targets(&admin, &vault, &targets(&env, &a, &b, 9_000, 1_000));

    // b drifts to 2_000 (1_000 bps) but is still dust on both sides.
    let plan = client.plan_rotation(&vault, &current(&env, &a, &b, 8_000, 2_000));
    assert!(!plan.rotation_needed);
    assert_eq!(plan.legs.len(), 0);
}

#[test]
fn the_leg_cap_bounds_how_many_strategies_move() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);

    client.set_policy(&admin, &vault, &POLICY);
    let target_list = vec![
        &env,
        TargetWeight {
            strategy: a.clone(),
            target_bps: 4_000,
        },
        TargetWeight {
            strategy: b.clone(),
            target_bps: 3_000,
        },
        TargetWeight {
            strategy: c.clone(),
            target_bps: 3_000,
        },
    ];
    client.set_targets(&admin, &vault, &target_list);

    // Every strategy drifts by 1_000 bps, but max_legs is 2.
    let observed = vec![
        &env,
        TargetWeight {
            strategy: a.clone(),
            target_bps: 3_000,
        },
        TargetWeight {
            strategy: b.clone(),
            target_bps: 4_000,
        },
        TargetWeight {
            strategy: c.clone(),
            target_bps: 2_000,
        },
    ];
    let plan = client.plan_rotation(&vault, &observed);
    assert!(plan.rotation_needed);
    assert_eq!(plan.legs.len(), 2);
    assert_eq!(plan.legs_skipped, 1);
}

#[test]
fn a_rotation_within_the_slippage_limit_is_recorded() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let from = Address::generate(&env);
    let to = Address::generate(&env);

    client.set_policy(&admin, &vault, &POLICY);
    // 1% slippage: 1_000 → 990 is exactly at the limit.
    client.record_rotation(&admin, &vault, &from, &to, &1_000, &990);
    assert_eq!(client.rotation_count(&vault), 1);
    assert_eq!(client.volume_moved(&vault), 1_000);
}

#[test]
fn a_rotation_beyond_the_slippage_limit_is_refused() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let from = Address::generate(&env);
    let to = Address::generate(&env);

    client.set_policy(&admin, &vault, &POLICY);
    assert_eq!(
        client.try_record_rotation(&admin, &vault, &from, &to, &1_000, &989),
        Err(Ok(Error::SlippageExceeded))
    );
    assert_eq!(client.rotation_count(&vault), 0);
}

#[test]
fn a_rotation_requires_a_configured_policy() {
    let (env, contract_id, admin) = rotation_setup();
    let client = RotationEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);
    let from = Address::generate(&env);
    let to = Address::generate(&env);

    assert_eq!(
        client.try_record_rotation(&admin, &vault, &from, &to, &1_000, &1_000),
        Err(Ok(Error::PolicyNotSet))
    );
}

// ── CrossChainYield ──────────────────────────────────────────────────────────

fn chain_setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let relayer = Address::generate(&env);
    let token = Address::generate(&env);
    let contract_id = env.register(CrossChainYield, ());
    let client = CrossChainYieldClient::new(&env, &contract_id);
    client.initialize(&admin, &token, &relayer);

    (env, contract_id, admin, relayer)
}

fn add_test_chain(
    env: &Env,
    client: &CrossChainYieldClient,
    admin: &Address,
    chain_id: u32,
) -> Address {
    let bridge = Address::generate(env);
    let remote_vault = Address::generate(env);
    client.add_chain(admin, &chain_id, &bridge, &remote_vault, &5_000);
    bridge
}

#[test]
fn cross_chain_initializes_once() {
    let (env, contract_id, admin, relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    assert_eq!(client.admin(), admin);
    assert_eq!(client.total_managed(), 0);
    assert_eq!(client.chain_count(), 0);
}

#[test]
fn a_chain_can_be_registered_and_read_back() {
    let (env, contract_id, admin, _relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    let bridge = add_test_chain(&client, &admin, 1);
    let config = client.get_chain(&1);
    assert_eq!(config.chain_id, 1);
    assert_eq!(config.bridge, bridge);
    assert_eq!(config.max_allocation_bps, 5_000);
    assert!(config.active);
    assert_eq!(client.chain_count(), 1);
}

#[test]
fn registering_the_same_chain_twice_is_refused() {
    let (env, contract_id, admin, _relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    add_test_chain(&env, &client, &admin, 1);
    let bridge = Address::generate(&env);
    let remote_vault = Address::generate(&env);
    assert_eq!(
        client.try_add_chain(&admin, &1, &bridge, &remote_vault, &5_000),
        Err(Ok(Error::ChainAlreadyRegistered))
    );
}

#[test]
fn a_deactivated_chain_refuses_deployments() {
    let (env, contract_id, admin, _relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    add_test_chain(&env, &client, &admin, 1);
    client.set_total_managed(&admin, &10_000);
    client.set_chain_active(&admin, &1, &false);

    assert_eq!(
        client.try_deploy(&admin, &1, &1_000),
        Err(Ok(Error::ChainInactive))
    );
}

#[test]
fn a_deployment_within_the_cap_succeeds() {
    let (env, contract_id, admin, _relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    add_test_chain(&env, &client, &admin, 1);
    client.set_total_managed(&admin, &10_000);

    // Cap is 5_000 bps of 10_000 = 5_000.
    let message_id = client.deploy(&admin, &1, &4_000);
    assert_eq!(message_id, 4_001);

    let position = client.get_position(&1);
    assert_eq!(position.pending, 4_000);
    assert_eq!(position.confirmed, 0);
}

#[test]
fn a_deployment_beyond_the_cap_is_refused() {
    let (env, contract_id, admin, _relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    add_test_chain(&env, &client, &admin, 1);
    client.set_total_managed(&admin, &10_000);

    // 4_000 + 2_000 = 6_000 > 5_000 cap.
    client.deploy(&admin, &1, &4_000);
    assert_eq!(
        client.try_deploy(&admin, &1, &2_000),
        Err(Ok(Error::CapExceeded))
    );
}

#[test]
fn only_the_relayer_can_confirm_a_delivery() {
    let (env, contract_id, admin, relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    add_test_chain(&env, &client, &admin, 1);
    client.set_total_managed(&admin, &10_000);
    client.deploy(&admin, &1, &4_000);

    client.confirm(&1, &4_000);
    let position = client.get_position(&1);
    assert_eq!(position.pending, 0);
    assert_eq!(position.confirmed, 4_000);
}

#[test]
fn confirming_more_than_is_pending_is_refused() {
    let (env, contract_id, admin, _relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    add_test_chain(&env, &client, &admin, 1);
    client.set_total_managed(&admin, &10_000);
    client.deploy(&admin, &1, &4_000);

    assert_eq!(
        client.try_confirm(&1, &4_001),
        Err(Ok(Error::NothingPending))
    );
}

#[test]
fn a_withdraw_reduces_the_confirmed_balance() {
    let (env, contract_id, admin, relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    add_test_chain(&env, &client, &admin, 1);
    client.set_total_managed(&admin, &10_000);
    client.deploy(&admin, &1, &4_000);
    client.confirm(&1, &4_000);

    let message_id = client.withdraw(&admin, &1, &4_000);
    assert_eq!(message_id, 4_001);

    let position = client.get_position(&1);
    assert_eq!(position.confirmed, 0);
    assert_eq!(position.pending, 0);
}

#[test]
fn withdrawing_more_than_is_confirmed_is_refused() {
    let (env, contract_id, admin, relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    add_test_chain(&env, &client, &admin, 1);
    client.set_total_managed(&admin, &10_000);
    client.deploy(&admin, &1, &4_000);
    client.confirm(&1, &4_000);

    assert_eq!(
        client.try_withdraw(&admin, &1, &4_001),
        Err(Ok(Error::NothingPending))
    );
}

#[test]
fn a_non_positive_amount_is_refused() {
    let (env, contract_id, admin, _relayer) = chain_setup();
    let client = CrossChainYieldClient::new(&env, &contract_id);

    add_test_chain(&env, &client, &admin, 1);
    client.set_total_managed(&admin, &10_000);

    assert_eq!(
        client.try_deploy(&admin, &1, &0),
        Err(Ok(Error::InvalidAmount))
    );
}
