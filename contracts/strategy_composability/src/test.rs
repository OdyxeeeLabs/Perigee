#![cfg(test)]

//! Unit tests for the strategy composability registry.
//!
//! Covers [#660]: strategy-of-strategies registration, recursive resolution
//! into a flat leaf allocation, cycle and depth guards, and layered fee
//! composition.
//!
//! Closes OdyxeeeLabs/Perigee#660

use crate::{
    ChildWeight, Error, StrategyComposability, StrategyComposabilityClient, MAX_DEPTH,
};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(StrategyComposability, ());
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    client.initialize(&admin);

    (env, contract_id, admin)
}

fn child(strategy: &Address, weight_bps: u32) -> ChildWeight {
    ChildWeight {
        child: strategy.clone(),
        weight_bps,
    }
}

fn register(
    env: &Env,
    client: &StrategyComposabilityClient,
    admin: &Address,
    address: &Address,
    children: Vec<ChildWeight>,
    fee_bps: u32,
) {
    client.register_composite(admin, &address, &children, &fee_bps);
}

// ── Registration ─────────────────────────────────────────────────────────────

#[test]
fn initializes_once() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);

    assert_eq!(client.admin(), admin);
    assert_eq!(
        client.try_initialize(&admin),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn a_composite_can_be_registered_and_read_back() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let composite = Address::generate(&env);
    let leaf = Address::generate(&env);

    register(
        &env,
        &client,
        &admin,
        &composite,
        vec![&env, child(&leaf, 10_000)],
        100,
    );

    let stored = client.get_composite(&composite);
    assert_eq!(stored.address, composite);
    assert_eq!(stored.fee_bps, 100);
    assert_eq!(stored.depth, 1);
    assert_eq!(stored.children.len(), 1);
    assert_eq!(client.composite_count(), 1);
}

#[test]
fn registering_the_same_composite_twice_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let composite = Address::generate(&env);
    let leaf = Address::generate(&env);

    register(
        &env,
        &client,
        &admin,
        &composite,
        vec![&env, child(&leaf, 10_000)],
        100,
    );
    assert_eq!(
        client.try_register_composite(&admin, &composite, &vec![&env, child(&leaf, 10_000)], &100),
        Err(Ok(Error::StrategyAlreadyRegistered))
    );
}

#[test]
fn children_that_do_not_sum_to_one_hundred_percent_are_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let composite = Address::generate(&env);
    let leaf = Address::generate(&env);

    let bad = vec![&env, child(&leaf, 9_000)];
    assert_eq!(
        client.try_register_composite(&admin, &composite, &bad, &100),
        Err(Ok(Error::InvalidChildren))
    );
}

#[test]
fn a_self_reference_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let composite = Address::generate(&env);

    let bad = vec![&env, child(&composite, 10_000)];
    assert_eq!(
        client.try_register_composite(&admin, &composite, &bad, &100),
        Err(Ok(Error::SelfReference))
    );
}

#[test]
fn duplicate_children_are_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let composite = Address::generate(&env);
    let leaf = Address::generate(&env);

    let bad = vec![&env, child(&leaf, 5_000), child(&leaf, 5_000)];
    assert_eq!(
        client.try_register_composite(&admin, &composite, &bad, &100),
        Err(Ok(Error::InvalidChildren))
    );
}

#[test]
fn children_can_be_replaced() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let composite = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    register(
        &env,
        &client,
        &admin,
        &composite,
        vec![&env, child(&a, 10_000)],
        100,
    );
    client.set_children(
        &admin,
        &composite,
        &vec![&env, child(&a, 6_000), child(&b, 4_000)],
    );

    let stored = client.get_composite(&composite);
    assert_eq!(stored.children.len(), 2);
}

// ── Resolution ───────────────────────────────────────────────────────────────

#[test]
fn an_unregistered_address_resolves_to_a_single_leaf() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let leaf = Address::generate(&env);

    let leaves = client.resolve_allocation(&leaf, &10_000);
    assert_eq!(leaves.len(), 1);
    assert_eq!(leaves.get(0).unwrap().strategy, leaf);
    assert_eq!(leaves.get(0).unwrap().amount, 10_000);
}

#[test]
fn a_composite_resolves_into_a_flat_leaf_allocation() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let composite = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    register(
        &env,
        &client,
        &admin,
        &composite,
        vec![&env, child(&a, 6_000), child(&b, 4_000)],
        100,
    );

    let leaves = client.resolve_allocation(&composite, &10_000);
    assert_eq!(leaves.len(), 2);
    assert_eq!(leaves.get(0).unwrap().strategy, a);
    assert_eq!(leaves.get(0).unwrap().amount, 6_000);
    assert_eq!(leaves.get(1).unwrap().strategy, b);
    assert_eq!(leaves.get(1).unwrap().amount, 4_000);
}

#[test]
fn a_nested_composite_resolves_recursively() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let inner = Address::generate(&env);
    let outer = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);

    register(
        &env,
        &client,
        &admin,
        &inner,
        vec![&env, child(&a, 5_000), child(&b, 5_000)],
        100,
    );
    register(
        &env,
        &client,
        &admin,
        &outer,
        vec![&env, child(&inner, 6_000), child(&c, 4_000)],
        200,
    );

    let leaves = client.resolve_allocation(&outer, &10_000);
    assert_eq!(leaves.len(), 3);
    // inner gets 6_000 → a 3_000, b 3_000; c gets 4_000.
    assert_eq!(leaves.get(0).unwrap().strategy, a);
    assert_eq!(leaves.get(0).unwrap().amount, 3_000);
    assert_eq!(leaves.get(1).unwrap().strategy, b);
    assert_eq!(leaves.get(1).unwrap().amount, 3_000);
    assert_eq!(leaves.get(2).unwrap().strategy, c);
    assert_eq!(leaves.get(2).unwrap().amount, 4_000);
}

#[test]
fn a_cycle_is_rejected_rather_than_expanded_forever() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let a = Address::generate(&env);
    let b = Address::generate(&env);

    register(
        &env,
        &client,
        &admin,
        &a,
        vec![&env, child(&b, 10_000)],
        100,
    );
    register(
        &env,
        &client,
        &admin,
        &b,
        vec![&env, child(&a, 10_000)],
        100,
    );

    assert_eq!(
        client.try_resolve_allocation(&a, &10_000),
        Err(Ok(Error::CyclicReference))
    );
}

#[test]
fn a_tree_deeper_than_the_limit_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);

    // Build a chain of MAX_DEPTH composites, then a leaf at the bottom.
    let mut nodes: Vec<Address> = Vec::new(&env);
    for _ in 0..=MAX_DEPTH {
        nodes.push_back(Address::generate(&env));
    }
    for i in 0..MAX_DEPTH {
        let parent = nodes.get(i).unwrap();
        let child_addr = nodes.get(i + 1).unwrap();
        register(
            &env,
            &client,
            &admin,
            &parent,
            vec![&env, child(&child_addr, 10_000)],
            0,
        );
    }

    // Resolving the root must exceed MAX_DEPTH before reaching the leaf.
    assert_eq!(
        client.try_resolve_allocation(&nodes.get(0).unwrap(), &10_000),
        Err(Ok(Error::MaxDepthExceeded))
    );
}

// ── Fee layering ─────────────────────────────────────────────────────────────

#[test]
fn a_leaf_has_no_effective_fee() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let leaf = Address::generate(&env);

    assert_eq!(client.effective_fee_bps(&leaf), 0);
}

#[test]
fn a_composite_layers_its_fee_over_its_children() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let inner = Address::generate(&env);
    let outer = Address::generate(&env);
    let a = Address::generate(&env);
    let b = Address::generate(&env);
    let c = Address::generate(&env);

    // inner charges 100 bps over two leaves.
    register(
        &env,
        &client,
        &admin,
        &inner,
        vec![&env, child(&a, 5_000), child(&b, 5_000)],
        100,
    );
    // outer charges 200 bps, with 60% flowing through inner.
    register(
        &env,
        &client,
        &admin,
        &outer,
        vec![&env, child(&inner, 6_000), child(&c, 4_000)],
        200,
    );

    // effective(inner) = 100
    assert_eq!(client.effective_fee_bps(&inner), 100);
    // effective(outer) = 200 + 6_000×100/10_000 = 260
    assert_eq!(client.effective_fee_bps(&outer), 260);
}

#[test]
fn a_negative_amount_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyComposabilityClient::new(&env, &contract_id);
    let leaf = Address::generate(&env);

    assert_eq!(
        client.try_resolve_allocation(&leaf, &-1),
        Err(Ok(Error::InvalidAmount))
    );
}
