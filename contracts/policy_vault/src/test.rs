#![cfg(test)]

//! Unit tests for the Policy Vault per-manager rate limiter.
//!
//! The suite covers the four properties the issue calls out explicitly, plus
//! the surrounding access-control and idempotency behaviour:
//!
//! * the limit is enforced per manager (a noisy neighbour cannot starve others),
//! * the limit resets every epoch (and the lifetime total does not),
//! * the limit is configurable by the admin at runtime,
//! * rejected calls do not spend quota and are not recorded.
//!
//! Closes OdyxeeeLabs/Perigee#505

use crate::{Error, PolicyVault, PolicyVaultClient};
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    Address, Env, TryIntoVal,
};

const MAX_PER_EPOCH: u32 = 10;
const EPOCH_LEN: u32 = 1_000;

/// Deploys the contract, initializes it with the default 10-per-1000-ledger
/// limit and snaps the ledger sequence to a deterministic value.
///
/// Returns `(env, contract_id, admin)`. Tests build their own client from the
/// returned env/id so the borrow lifetimes stay local to each test.
fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.sequence_number = 1);

    let id = env.register(PolicyVault, ());
    let admin = Address::generate(&env);
    PolicyVaultClient::new(&env, &id).initialize(&admin, &MAX_PER_EPOCH, &EPOCH_LEN);

    (env, id, admin)
}

// ── Initialization ───────────────────────────────────────────────────────────

#[test]
fn initialize_sets_admin_and_config() {
    let (env, id, admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);

    assert_eq!(client.get_admin(), admin);
    let config = client.get_config();
    assert_eq!(config.max_vaults_per_epoch, MAX_PER_EPOCH);
    assert_eq!(config.epoch_length_ledgers, EPOCH_LEN);
    assert_eq!(client.current_epoch(), 0);
}

#[test]
fn initialize_cannot_run_twice() {
    let (env, id, admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);

    assert_eq!(
        client.try_initialize(&admin, &MAX_PER_EPOCH, &EPOCH_LEN),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn initialize_rejects_zero_config() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(PolicyVault, ());
    let client = PolicyVaultClient::new(&env, &id);
    let admin = Address::generate(&env);

    // A zero limit or a zero-length epoch would silently disable the guard.
    assert_eq!(
        client.try_initialize(&admin, &0, &EPOCH_LEN),
        Err(Ok(Error::InvalidConfig))
    );
    assert_eq!(
        client.try_initialize(&admin, &MAX_PER_EPOCH, &0),
        Err(Ok(Error::InvalidConfig))
    );
}

#[test]
fn views_fail_before_initialize() {
    let env = Env::default();
    env.mock_all_auths();
    let id = env.register(PolicyVault, ());
    let client = PolicyVaultClient::new(&env, &id);
    let manager = Address::generate(&env);

    assert_eq!(client.try_get_admin(), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_get_config(), Err(Ok(Error::NotInitialized)));
    assert_eq!(
        client.try_remaining_quota(&manager),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_create_vault(&manager, &0),
        Err(Ok(Error::NotInitialized))
    );
}

// ── Core rate limiting ───────────────────────────────────────────────────────

#[test]
fn create_vault_records_and_consumes_quota() {
    let (env, id, _admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let manager = Address::generate(&env);

    let record = client.create_vault(&manager, &42);
    assert_eq!(record.manager, manager);
    assert_eq!(record.vault_id, 42);
    assert_eq!(record.epoch, 0);
    assert_eq!(record.created_ledger, 1);

    assert_eq!(client.vaults_this_epoch(&manager), 1);
    assert_eq!(client.total_vaults(&manager), 1);
    assert_eq!(client.remaining_quota(&manager), MAX_PER_EPOCH - 1);

    let stored = client.get_vault(&manager, &42).expect("vault record");
    assert_eq!(stored, record);
}

#[test]
fn rate_limit_is_enforced_per_epoch() {
    let (env, id, _admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let manager = Address::generate(&env);

    for vault_id in 0..MAX_PER_EPOCH {
        client.create_vault(&manager, &vault_id);
    }
    assert_eq!(client.vaults_this_epoch(&manager), MAX_PER_EPOCH);
    assert_eq!(client.remaining_quota(&manager), 0);

    // The (max + 1)-th vault in the same epoch is rejected.
    assert_eq!(
        client.try_create_vault(&manager, &MAX_PER_EPOCH),
        Err(Ok(Error::RateLimitExceeded))
    );

    // A rejected call must not be recorded and must not advance the counters.
    assert!(client.get_vault(&manager, &MAX_PER_EPOCH).is_none());
    assert_eq!(client.total_vaults(&manager), MAX_PER_EPOCH);
}

#[test]
fn managers_have_independent_quotas() {
    let (env, id, _admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let alice = Address::generate(&env);
    let bob = Address::generate(&env);

    for vault_id in 0..MAX_PER_EPOCH {
        client.create_vault(&alice, &vault_id);
    }
    assert_eq!(
        client.try_create_vault(&alice, &99),
        Err(Ok(Error::RateLimitExceeded))
    );

    // Bob's budget is untouched by Alice exhausting hers.
    client.create_vault(&bob, &0);
    assert_eq!(client.vaults_this_epoch(&bob), 1);
    assert_eq!(client.remaining_quota(&bob), MAX_PER_EPOCH - 1);
}

#[test]
fn quota_resets_after_epoch_boundary() {
    let (env, id, _admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let manager = Address::generate(&env);

    for vault_id in 0..MAX_PER_EPOCH {
        client.create_vault(&manager, &vault_id);
    }
    assert_eq!(client.remaining_quota(&manager), 0);

    // Advance into the next epoch.
    env.ledger().with_mut(|li| li.sequence_number += EPOCH_LEN);

    assert_eq!(client.current_epoch(), 1);
    assert_eq!(client.vaults_this_epoch(&manager), 0);
    assert_eq!(client.remaining_quota(&manager), MAX_PER_EPOCH);
    // The lifetime total survives the epoch rollover.
    assert_eq!(client.total_vaults(&manager), MAX_PER_EPOCH);

    client.create_vault(&manager, &MAX_PER_EPOCH);
    assert_eq!(client.vaults_this_epoch(&manager), 1);
    assert_eq!(client.total_vaults(&manager), MAX_PER_EPOCH + 1);
}

#[test]
fn duplicate_vault_id_is_rejected_without_spending_quota() {
    let (env, id, _admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let manager = Address::generate(&env);

    client.create_vault(&manager, &1);
    assert_eq!(client.vaults_this_epoch(&manager), 1);

    assert_eq!(
        client.try_create_vault(&manager, &1),
        Err(Ok(Error::VaultAlreadyExists))
    );

    // The duplicate attempt neither consumed quota nor inflated the total.
    assert_eq!(client.vaults_this_epoch(&manager), 1);
    assert_eq!(client.total_vaults(&manager), 1);
}

// ── Configuration ────────────────────────────────────────────────────────────

#[test]
fn rate_limit_is_admin_configurable() {
    let (env, id, admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let manager = Address::generate(&env);

    // Tighten the limit to 2 vaults per 100 ledgers.
    client.set_rate_limit(&admin, &2, &100);
    let config = client.get_config();
    assert_eq!(config.max_vaults_per_epoch, 2);
    assert_eq!(config.epoch_length_ledgers, 100);

    client.create_vault(&manager, &0);
    client.create_vault(&manager, &1);
    assert_eq!(
        client.try_create_vault(&manager, &2),
        Err(Ok(Error::RateLimitExceeded))
    );
}

#[test]
fn set_rate_limit_rejects_zero_config() {
    let (env, id, admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);

    assert_eq!(
        client.try_set_rate_limit(&admin, &0, &EPOCH_LEN),
        Err(Ok(Error::InvalidConfig))
    );
    assert_eq!(
        client.try_set_rate_limit(&admin, &1, &0),
        Err(Ok(Error::InvalidConfig))
    );
}

#[test]
fn changing_epoch_length_starts_a_fresh_epoch() {
    let (env, id, admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let manager = Address::generate(&env);

    for vault_id in 0..MAX_PER_EPOCH {
        client.create_vault(&manager, &vault_id);
    }
    assert_eq!(client.remaining_quota(&manager), 0);

    // Re-partitioning the ledger timeline makes the stored epoch stale, so the
    // next call starts a fresh epoch (documented behaviour of setting the limit).
    client.set_rate_limit(&admin, &MAX_PER_EPOCH, &1);
    assert_eq!(client.current_epoch(), 1);
    assert_eq!(client.vaults_this_epoch(&manager), 0);

    client.create_vault(&manager, &MAX_PER_EPOCH);
    assert_eq!(client.total_vaults(&manager), MAX_PER_EPOCH + 1);
}

#[test]
fn set_admin_rotates_admin_key() {
    let (env, id, admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let new_admin = Address::generate(&env);

    client.set_admin(&admin, &new_admin);
    assert_eq!(client.get_admin(), new_admin);

    // The new admin can retune the limit; counters are left untouched.
    client.set_rate_limit(&new_admin, &3, &50);
    assert_eq!(client.get_config().max_vaults_per_epoch, 3);
}

// ── Access control ───────────────────────────────────────────────────────────

#[test]
fn non_admin_cannot_change_rate_limit() {
    let (env, id, _admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_set_rate_limit(&stranger, &1, &EPOCH_LEN),
        Err(Ok(Error::Unauthorized))
    );
    // The configuration is unchanged.
    assert_eq!(client.get_config().max_vaults_per_epoch, MAX_PER_EPOCH);
}

#[test]
fn non_admin_cannot_rotate_admin() {
    let (env, id, admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let stranger = Address::generate(&env);
    let other = Address::generate(&env);

    assert_eq!(
        client.try_set_admin(&stranger, &other),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(client.get_admin(), admin);
}

#[test]
fn admin_only_functions_require_admin_auth() {
    let (env, id, admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);

    client.set_rate_limit(&admin, &5, &200);
    let auths = env.auths();
    assert!(
        auths.iter().any(|(addr, _)| *addr == admin),
        "set_rate_limit must require the admin authorization"
    );
}

#[test]
fn create_vault_requires_manager_auth() {
    let (env, id, _admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let manager = Address::generate(&env);

    client.create_vault(&manager, &0);
    let auths = env.auths();
    assert!(
        auths.iter().any(|(addr, _)| *addr == manager),
        "create_vault must require the manager authorization"
    );
}

// ── Events ───────────────────────────────────────────────────────────────────

#[test]
fn vault_creation_emits_event() {
    let (env, id, _admin) = setup();
    let client = PolicyVaultClient::new(&env, &id);
    let manager = Address::generate(&env);

    client.create_vault(&manager, &7);

    let events = env.events().all();
    let mut found = false;
    for (_, topics, data) in events.iter() {
        if topics.len() != 3 {
            continue;
        }
        let topic_manager: Result<Address, _> = topics.get(1).unwrap().try_into_val(&env);
        let topic_vault: Result<u32, _> = topics.get(2).unwrap().try_into_val(&env);
        let is_our_event =
            topic_manager.ok() == Some(manager.clone()) && topic_vault.ok() == Some(7);

        if is_our_event {
            let (ledger, epoch, count): (u32, u32, u32) =
                data.try_into_val(&env).expect("VaultCreated data tuple");
            assert_eq!((ledger, epoch, count), (1, 0, 1));
            found = true;
        }
    }

    assert!(found, "expected a vault-created event for (manager, 7)");
}
