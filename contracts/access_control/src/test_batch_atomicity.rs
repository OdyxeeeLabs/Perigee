#![cfg(test)]

//! Batch update atomicity (#871).
//!
//! Contract under test: every batch entry point is **all-or-nothing**. A batch
//! that fails part-way — invalid selector, unregistered wildcard, a bound being
//! hit — returns an error and leaves *no* partial state behind, including the
//! removals `batch_set_*` performs before it starts adding. Entries that are
//! merely redundant (already held / not held) are skipped without error.

use soroban_sdk::{testutils::{Address as _, Events as _}, vec, Address, Env, String, Vec};

use crate::{AccessControl, AccessControlClient, AclConfig, Error, GrantEntry};

fn setup_with(config: AclConfig) -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(AccessControl, ());
    let client = AccessControlClient::new(&env, &id);
    client.initialize(&admin, &config);
    (env, id, admin)
}

fn s(env: &Env, text: &str) -> String {
    String::from_str(env, text)
}

fn entry(env: &Env, target: &Address, sel: &str) -> GrantEntry {
    GrantEntry {
        target: target.clone(),
        selector: s(env, sel),
    }
}

#[test]
fn an_invalid_selector_anywhere_in_a_grant_batch_rolls_back_every_entry() {
    let (env, id, admin) = setup_with(AclConfig::default_config());
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    let entries = vec![
        &env,
        entry(&env, &alice, "harvest"),
        entry(&env, &bob, "rebalance"),
        entry(&env, &alice, "a*b"), // malformed: star not at the end
    ];
    assert_eq!(
        client.try_batch_grant(&admin, &entries),
        Err(Ok(Error::InvalidSelector))
    );
    assert!(client.grants_of(&alice).is_empty());
    assert!(client.grants_of(&bob).is_empty());
    assert!(!client.has_permission(&alice, &s(&env, "harvest")));
    assert!(!client.has_permission(&bob, &s(&env, "rebalance")));
}

#[test]
fn an_unregistered_wildcard_rolls_back_the_whole_grant_batch() {
    let (env, id, admin) = setup_with(AclConfig::default_config());
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);

    let entries = vec![
        &env,
        entry(&env, &alice, "harvest"),
        entry(&env, &alice, "vault.*"), // never registered
    ];
    assert_eq!(
        client.try_batch_grant(&admin, &entries),
        Err(Ok(Error::UnregisteredWildcard))
    );
    assert!(client.grants_of(&alice).is_empty());

    // After registering the wildcard the same batch applies in full.
    client.register_wildcard(&admin, &s(&env, "vault.*"));
    assert_eq!(client.batch_grant(&admin, &entries), 2);
}

#[test]
fn hitting_the_per_address_limit_mid_batch_leaves_earlier_entries_unapplied() {
    let (env, id, admin) = setup_with(AclConfig {
        max_grants_per_address: 2,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    let entries = vec![
        &env,
        entry(&env, &bob, "x"),
        entry(&env, &alice, "a"),
        entry(&env, &alice, "b"),
        entry(&env, &alice, "c"), // third grant for alice: over the bound
    ];
    assert_eq!(
        client.try_batch_grant(&admin, &entries),
        Err(Ok(Error::LimitExceeded))
    );
    assert!(client.grants_of(&alice).is_empty());
    assert!(client.grants_of(&bob).is_empty());
}

#[test]
fn duplicate_selectors_in_one_batch_are_applied_and_announced_once() {
    let (env, id, admin) = setup_with(AclConfig::default_config());
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);

    let entries = vec![
        &env,
        entry(&env, &alice, "harvest"),
        entry(&env, &alice, "harvest"),
        entry(&env, &alice, "rebalance"),
        entry(&env, &alice, "harvest"),
    ];
    assert_eq!(client.batch_grant(&admin, &entries), 2);
    assert_eq!(client.grants_of(&alice).len(), 2);
    // One event per entry actually applied, none for the duplicates.
    assert_eq!(env.events().all().len(), 2);
}

#[test]
fn already_held_entries_are_skipped_and_only_applied_ones_emit_events() {
    let (env, id, admin) = setup_with(AclConfig::default_config());
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    client.grant(&admin, &alice, &s(&env, "harvest"));

    let entries = vec![
        &env,
        entry(&env, &alice, "harvest"),   // already held: skipped
        entry(&env, &alice, "rebalance"), // new
    ];
    assert_eq!(client.batch_grant(&admin, &entries), 1);
    assert_eq!(env.events().all().len(), 1);
    assert_eq!(client.grants_of(&alice).len(), 2);
}

#[test]
fn batch_revoke_skips_absent_entries_and_announces_only_removals() {
    let (env, id, admin) = setup_with(AclConfig::default_config());
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);
    client.grant(&admin, &alice, &s(&env, "harvest"));

    let entries = vec![
        &env,
        entry(&env, &alice, "harvest"),   // held
        entry(&env, &alice, "rebalance"), // not held
        entry(&env, &bob, "harvest"),     // other principal, not held
    ];
    assert_eq!(client.batch_revoke(&admin, &entries), 1);
    assert_eq!(env.events().all().len(), 1);
    assert!(client.grants_of(&alice).is_empty());
}

#[test]
fn a_failed_member_replacement_keeps_the_previous_roster() {
    // `batch_set_members` first detaches members that are not in the new list and
    // only then adds. If the add phase fails, those detachments must not stick.
    let (env, id, admin) = setup_with(AclConfig {
        max_members_per_role: 2,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);
    let role = s(&env, "keepers");
    client.create_role(&admin, &role, &admin);
    client.grant_role_permission(&admin, &role, &s(&env, "harvest"));

    let old = Address::generate(&env);
    client.batch_set_members(&admin, &role, &vec![&env, old.clone()]);

    let (n1, n2, n3) = (
        Address::generate(&env),
        Address::generate(&env),
        Address::generate(&env),
    );
    // Three new members against a bound of two: the third add fails.
    assert_eq!(
        client.try_batch_set_members(&admin, &role, &vec![&env, n1.clone(), n2.clone(), n3.clone()]),
        Err(Ok(Error::LimitExceeded))
    );

    // `old` is still a member and still authorized; nobody new was added.
    assert!(client.has_role(&old, &role));
    assert!(client.has_permission(&old, &s(&env, "harvest")));
    assert_eq!(client.members_of(&role), vec![&env, old]);
    for n in [n1, n2, n3] {
        assert!(!client.has_role(&n, &role));
    }
}

#[test]
fn a_failed_permission_replacement_keeps_the_previous_selector_set() {
    let (env, id, admin) = setup_with(AclConfig::default_config());
    let client = AccessControlClient::new(&env, &id);
    let role = s(&env, "keepers");
    client.create_role(&admin, &role, &admin);
    let member = Address::generate(&env);
    client.grant_role(&admin, &role, &member);
    client.batch_set_permissions(&admin, &role, &vec![&env, s(&env, "harvest")]);

    // Replace with a set whose last entry is malformed.
    let bad: Vec<String> = vec![&env, s(&env, "rebalance"), s(&env, "x*y")];
    assert_eq!(
        client.try_batch_set_permissions(&admin, &role, &bad),
        Err(Ok(Error::InvalidSelector))
    );

    assert_eq!(client.permissions_of(&role), vec![&env, s(&env, "harvest")]);
    assert!(client.has_permission(&member, &s(&env, "harvest")));
    assert!(!client.has_permission(&member, &s(&env, "rebalance")));
}

#[test]
fn oversized_and_empty_batches_are_rejected_before_any_write() {
    let (env, id, admin) = setup_with(AclConfig {
        max_batch_size: 2,
        ..AclConfig::default_config()
    });
    let client = AccessControlClient::new(&env, &id);
    let alice = Address::generate(&env);
    let too_many = vec![
        &env,
        entry(&env, &alice, "a"),
        entry(&env, &alice, "b"),
        entry(&env, &alice, "c"),
    ];
    assert_eq!(client.try_batch_grant(&admin, &too_many), Err(Ok(Error::BatchTooLarge)));
    assert_eq!(client.try_batch_grant(&admin, &Vec::new(&env)), Err(Ok(Error::EmptyBatch)));
    assert!(client.grants_of(&alice).is_empty());
}
