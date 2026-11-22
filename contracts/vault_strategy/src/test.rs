#![cfg(test)]

//! Unit tests for the vault strategy registry.
//!
//! Covers the three things #633 names: modular strategies, strategy rotation, and
//! performance fee distribution — plus the property that makes the fee schedule
//! non-predatory, that performance fees are charged on profit and never on
//! depositors' principal.
//!
//! Closes OdyxeeeLabs/Perigee#633

use crate::{Error, FeeConfig, VaultStrategy, VaultStrategyClient, MAX_FEE_BPS};
use soroban_sdk::{testutils::Address as _, Address, Env, String};

macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let treasury = Address::generate(&env);
        let vault = Address::generate(&env);

        let contract_id = env.register(VaultStrategy, ());
        let client = VaultStrategyClient::new(&env, &contract_id);
        client.initialize(&admin, &treasury);

        (env, contract_id, client, admin, treasury, vault)
    }};
}

fn label(env: &Env, value: &str) -> String {
    String::from_str(env, value)
}

fn default_fees(env: &Env, treasury: Address) -> FeeConfig {
    FeeConfig {
        management_fee_bps: 100,      // 1%
        performance_fee_bps: 1_000,   // 10%
        protocol_share_bps: 500,      // half to the protocol
        vault_fee_recipient: Address::generate(env),
        treasury,
    }
}

/// Register and approve a strategy, returning its address.
macro_rules! strategy {
    ($env:expr, $client:expr, $admin:expr, $name:expr) => {{
        let address = Address::generate($env);
        $client.register_strategy($admin, &address, &1, &label($env, $name));
        $client.approve_strategy($admin, &address, &true);
        address
    }};
}

// ── Registry ─────────────────────────────────────────────────────────────────

#[test]
fn registry_starts_empty() {
    let (env, _cid, client, admin, _treasury, _vault) = setup!();

    assert_eq!(client.admin(), admin);
    assert_eq!(client.strategy_count(), 0);
}

#[test]
fn registry_can_only_be_initialized_once() {
    let (env, _cid, client, admin, treasury, _vault) = setup!();

    assert_eq!(
        client.try_initialize(&admin, &treasury),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn registering_records_the_strategy_without_approving_it() {
    let (env, _cid, client, admin, _treasury, _vault) = setup!();
    let strategy = Address::generate(&env);

    client.register_strategy(&admin, &strategy, &1, &label(&env, "aave"));

    let record = client.get_strategy(&strategy);
    assert_eq!(record.version, 1);
    assert_eq!(record.label, String::from_str(&env, "aave"));
    // Registration alone must not make a strategy live.
    assert!(!record.approved);
    assert_eq!(client.strategy_count(), 1);
}

#[test]
fn an_incompatible_strategy_version_is_refused() {
    let (env, _cid, client, admin, _treasury, _vault) = setup!();
    let strategy = Address::generate(&env);

    assert_eq!(
        client.try_register_strategy(&admin, &strategy, &99, &label(&env, "future")),
        Err(Ok(Error::IncompatibleStrategyVersion))
    );
    assert_eq!(client.strategy_count(), 0);
}

#[test]
fn registering_the_same_strategy_twice_is_refused() {
    let (env, _cid, client, admin, _treasury, _vault) = setup!();
    let strategy = strategy!(env, client, admin, "compound");

    assert_eq!(
        client.try_register_strategy(&admin, &strategy, &1, &label(&env, "again")),
        Err(Ok(Error::StrategyAlreadyRegistered))
    );
    assert_eq!(client.strategy_count(), 1);
}

#[test]
fn an_unknown_strategy_is_not_found() {
    let (env, _cid, client, _admin, _treasury, _vault) = setup!();

    assert_eq!(
        client.try_get_strategy(&Address::generate(&env)),
        Err(Ok(Error::StrategyNotFound))
    );
}

#[test]
fn only_the_admin_may_register_or_approve() {
    let (env, _cid, client, admin, _treasury, _vault) = setup!();
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_register_strategy(&stranger, &Address::generate(&env), &1, &label(&env, "x")),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_approve_strategy(&stranger, &Address::generate(&env), &true),
        Err(Ok(Error::Unauthorized))
    );

    let _ = admin;
}

// ── Vault lifecycle ──────────────────────────────────────────────────────────

#[test]
fn initializing_a_vault_requires_an_approved_strategy() {
    let (env, _cid, client, admin, _treasury, vault) = setup!();
    let registered_but_not_approved = Address::generate(&env);

    client.register_strategy(&admin, &registered_but_not_approved, &1, &label(&env, "x"));

    assert_eq!(
        client.try_initialize_vault(&admin, &vault, &registered_but_not_approved, &1_000),
        Err(Ok(Error::StrategyNotApproved))
    );
}

#[test]
fn initializing_a_vault_seeds_principal() {
    let (env, _cid, client, admin, _treasury, vault) = setup!();
    let strategy = strategy!(env, client, admin, "aave");

    client.initialize_vault(&admin, &vault, &strategy, &1_000);

    let state = client.get_vault(&vault);
    assert_eq!(state.active, strategy);
    assert_eq!(state.principal, 1_000);
    assert_eq!(state.idle, 1_000);
    assert_eq!(state.fees_paid, 0);
}

#[test]
fn deposits_and_withdrawals_move_principal() {
    let (env, _cid, client, admin, _treasury, vault) = setup!();
    let strategy = strategy!(env, client, admin, "aave");
    client.initialize_vault(&admin, &vault, &strategy, &1_000);

    client.add_deposit(&vault, &500);
    assert_eq!(client.get_vault(&vault).principal, 1_500);
    assert_eq!(client.get_vault(&vault).idle, 1_500);

    client.add_withdrawal(&vault, &200);
    assert_eq!(client.get_vault(&vault).principal, 1_300);
}

#[test]
fn a_withdrawal_beyond_principal_is_refused() {
    let (env, _cid, client, admin, _treasury, vault) = setup!();
    let strategy = strategy!(env, client, admin, "aave");
    client.initialize_vault(&admin, &vault, &strategy, &1_000);

    assert_eq!(
        client.try_add_withdrawal(&vault, &1_001),
        Err(Ok(Error::InsufficientPrincipal))
    );
    assert_eq!(client.get_vault(&vault).principal, 1_000);
}

#[test]
fn an_uninitialized_vault_is_reported() {
    let (env, _cid, client, _admin, _treasury, _vault) = setup!();

    assert_eq!(
        client.try_get_vault(&Address::generate(&env)),
        Err(Ok(Error::NoActiveStrategy))
    );
}

// ── Rotation ─────────────────────────────────────────────────────────────────

#[test]
fn rotation_moves_the_vault_to_an_approved_strategy() {
    let (env, _cid, client, admin, _treasury, vault) = setup!();
    let first = strategy!(env, client, admin, "aave");
    let second = strategy!(env, client, admin, "compound");
    client.initialize_vault(&admin, &vault, &first, &1_000);

    let result = client.rotate(&vault, &second, &250, &1_000, &1_250);

    assert_eq!(result.from, first);
    assert_eq!(result.to, second);
    assert_eq!(result.profit, 250);

    let state = client.get_vault(&vault);
    assert_eq!(state.active, second);
    assert_eq!(state.rotation_count, 1);
    assert_eq!(state.idle, 1_250);
    // Realised profit is value, not principal, so it is never fee-bearing.
    assert_eq!(state.principal, 1_000);
}

#[test]
fn rotation_into_an_unapproved_strategy_is_refused() {
    let (env, _cid, client, admin, _treasury, vault) = setup!();
    let first = strategy!(env, client, admin, "aave");
    let registered_only = Address::generate(&env);
    client.register_strategy(&admin, &registered_only, &1, &label(&env, "pending"));
    client.initialize_vault(&admin, &vault, &first, &1_000);

    assert_eq!(
        client.try_rotate(&vault, &registered_only, &0, &1_000, &1_000),
        Err(Ok(Error::StrategyNotApproved))
    );
    assert_eq!(client.get_vault(&vault).active, first);
}

#[test]
fn revoking_approval_blocks_rotation() {
    let (env, _cid, client, admin, _treasury, vault) = setup!();
    let first = strategy!(env, client, admin, "aave");
    let second = strategy!(env, client, admin, "compound");
    client.initialize_vault(&admin, &vault, &first, &1_000);

    client.approve_strategy(&admin, &second, &false);

    assert_eq!(
        client.try_rotate(&vault, &second, &0, &1_000, &1_000),
        Err(Ok(Error::StrategyNotApproved))
    );
}

#[test]
fn rotation_counts_accumulate() {
    let (env, _cid, client, admin, _treasury, vault) = setup!();
    let a = strategy!(env, client, admin, "a");
    let b = strategy!(env, client, admin, "b");
    let c = strategy!(env, client, admin, "c");
    client.initialize_vault(&admin, &vault, &a, &1_000);

    client.rotate(&vault, &b, &0, &1_000, &1_000);
    client.rotate(&vault, &c, &0, &1_000, &1_000);

    let state = client.get_vault(&vault);
    assert_eq!(state.rotation_count, 2);
    assert_eq!(state.active, c);
}

#[test]
fn rotating_to_the_current_strategy_is_refused() {
    let (env, _cid, client, admin, _treasury, vault) = setup!();
    let only = strategy!(env, client, admin, "aave");
    client.initialize_vault(&admin, &vault, &only, &1_000);

    assert_eq!(
        client.try_rotate(&vault, &only, &0, &1_000, &1_000),
        Err(Ok(Error::StrategyAlreadyRegistered))
    );
}

// ── Fees ─────────────────────────────────────────────────────────────────────

#[test]
fn fees_are_zero_when_there_is_no_profit() {
    let (env, _cid, client, admin, treasury, vault) = setup!();
    let strategy = strategy!(env, client, admin, "aave");
    client.initialize_vault(&admin, &vault, &strategy, &1_000);
    client.set_fee_config(&admin, &vault, &default_fees(&env, treasury));

    // Exactly principal: no yield, so no fee. A fee charged on the balance would
    // take depositors' own money here.
    let (total, _vault_share, _protocol) = client.compute_fees(&vault, &1_000);
    assert_eq!(total, 0);
}

#[test]
fn performance_fees_are_charged_on_profit_not_on_principal() {
    let (env, _cid, client, admin, treasury, vault) = setup!();
    let strategy = strategy!(env, client, admin, "aave");
    client.initialize_vault(&admin, &vault, &strategy, &1_000);

    // 10% management + 10% performance on a 200 profit.
    let fees = default_fees(&env, treasury);
    client.set_fee_config(&admin, &vault, &fees);

    let (total, vault_share, protocol_share) = client.compute_fees(&vault, &1_200);
    assert_eq!(total, 40); // 20 + 20
    assert_eq!(vault_share + protocol_share, total);
    assert_eq!(protocol_share, 20); // half
    assert_eq!(vault_share, 20);
}

#[test]
fn a_loss_is_never_charged() {
    let (env, _cid, client, admin, treasury, vault) = setup!();
    let strategy = strategy!(env, client, admin, "aave");
    client.initialize_vault(&admin, &vault, &strategy, &1_000);
    client.set_fee_config(&admin, &vault, &default_fees(&env, treasury));

    // A strategy that lost money must not produce a negative fee.
    let (total, _v, _p) = client.compute_fees(&vault, &800);
    assert_eq!(total, 0);
}

#[test]
fn the_fee_split_always_sums_to_the_total() {
    let (env, cid, _client, _admin, _treasury, _vault) = setup!();
    // No rounding can send fees to the wrong place, at any split point.
    for bps in [0u32, 1, 250, 5_000, 9_999, 10_000] {
        let (vault, protocol) = VaultStrategyClient::new(&env, &cid).split_fee(&1_000, &bps);
        assert_eq!(vault + protocol, 1_000, "split {bps} lost or created value");
        assert!(protocol >= 0);
        assert!(vault >= 0);
    }
}

#[test]
fn fees_above_the_ceiling_are_refused() {
    let (env, _cid, client, admin, treasury, vault) = setup!();
    let strategy = strategy!(env, client, admin, "aave");
    client.initialize_vault(&admin, &vault, &strategy, &1_000);

    let mut fees = default_fees(&env, treasury);
    fees.performance_fee_bps = MAX_FEE_BPS + 1;

    assert_eq!(
        client.try_set_fee_config(&admin, &vault, &fees),
        Err(Ok(Error::FeeTooHigh))
    );

    fees.performance_fee_bps = MAX_FEE_BPS;
    fees.management_fee_bps = MAX_FEE_BPS + 1;
    assert_eq!(
        client.try_set_fee_config(&admin, &vault, &fees),
        Err(Ok(Error::FeeTooHigh))
    );
}

#[test]
fn a_protocol_share_above_one_hundred_percent_is_refused() {
    let (env, _cid, client, admin, treasury, vault) = setup!();
    let strategy = strategy!(env, client, admin, "aave");
    client.initialize_vault(&admin, &vault, &strategy, &1_000);

    let mut fees = default_fees(&env, treasury);
    fees.protocol_share_bps = 10_001;

    assert_eq!(
        client.try_set_fee_config(&admin, &vault, &fees),
        Err(Ok(Error::InvalidFeeSplit))
    );
}

#[test]
fn paid_fees_accumulate() {
    let (env, _cid, client, admin, treasury, vault) = setup!();
    let strategy = strategy!(env, client, admin, "aave");
    client.initialize_vault(&admin, &vault, &strategy, &1_000);
    client.set_fee_config(&admin, &vault, &default_fees(&env, treasury));

    client.record_fees_paid(&vault, &40);
    client.record_fees_paid(&vault, &60);

    assert_eq!(client.get_vault(&vault).fees_paid, 100);
}

// ── Administration ───────────────────────────────────────────────────────────

#[test]
fn set_admin_rotates_the_registry_admin() {
    let (env, _cid, client, admin, _treasury, _vault) = setup!();
    let successor = Address::generate(&env);

    client.set_admin(&admin, &successor);

    assert_eq!(client.admin(), successor);
    assert_eq!(
        client.try_register_strategy(&admin, &Address::generate(&env), &1, &label(&env, "x")),
        Err(Ok(Error::Unauthorized))
    );
}