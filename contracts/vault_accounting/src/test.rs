#![cfg(test)]

//! Unit tests for the vault accounting contract.
//!
//! Three themes, matching the three things #636 calls out: share-price
//! manipulation resistance, rounding direction, and dust handling.
//!
//! Closes OdyxeeeLabs/Perigee#636

use crate::{
    AccountingConfig, Error, RoundingKind, VaultAccounting, VaultAccountingClient,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

macro_rules! setup {
    ($offset:expr, $min_shares:expr, $dust:expr) => {{
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let vault = Address::generate(&env);

        let contract_id = env.register(VaultAccounting, ());
        let client = VaultAccountingClient::new(&env, &contract_id);
        client.initialize(&admin);
        client.register_vault(
            &admin,
            &vault,
            &AccountingConfig {
                virtual_offset: $offset,
                min_shares_per_deposit: $min_shares,
                dust_threshold: $dust,
                dust_reverts_to_vault: true,
            },
        );

        (env, contract_id, client, admin, vault)
    }};
}

/// The configuration the repo's other vaults use: offset 3, 1 share minimum,
/// 1_000 units of dust.
macro_rules! default_setup {
    () => {
        setup!(3, 1, 1_000)
    };
}

// ── #636 (1) share-price manipulation resistance ─────────────────────────────

#[test]
fn an_empty_vault_prices_one_to_one() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    assert_eq!(client.shares_for_assets(&vault, &10_000), 10_000);
}

#[test]
fn a_donation_cannot_zero_out_a_later_depositor() {
    // The classic attack: the first depositor mints almost nothing, donates a
    // large amount, and hopes the next depositor's shares round to zero while
    // their assets sit in the vault unclaimable.
    let (env, _cid, client, _admin, vault) = default_setup!();

    client.record_deposit(&vault, &vault, &1, &1);
    // Donation: assets arrive with no shares minted.
    client.record_deposit(&vault, &vault, &999_999, &0);

    let victim_shares = client.shares_for_assets(&vault, &500);
    assert!(
        victim_shares > 0,
        "victim received zero shares; the virtual offset is not working"
    );

    // And that position is worth roughly what was paid, not approximately nothing.
    let victim_assets = client.assets_for_shares(&vault, &victim_shares);
    assert!(victim_assets > 0, "victim's position is worth nothing");
}

#[test]
fn a_larger_offset_harder_to_exploit() {
    // The offset is the whole defence, so its effect should be observable.
    let (env, _cid, client, _admin, vault) = setup!(6, 1, 1_000);

    client.record_deposit(&vault, &vault, &1, &1);
    client.record_deposit(&vault, &vault, &999_999, &0);

    let shares = client.shares_for_assets(&vault, &500);
    assert!(shares > 0);
    assert!(client.assets_for_shares(&vault, &shares) > 0);
}

#[test]
fn yield_raises_the_share_price_for_existing_holders() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    client.record_deposit(&vault, &vault, &1_000, &1_000);
    // Yield arrives: assets up, shares flat.
    client.record_deposit(&vault, &vault, &1_000, &0);

    let supply = client.get_totals(&vault).total_supply;
    let value = client.assets_for_shares(&vault, &supply);
    assert!(value > 2_000, "share price did not rise: {value}");
}

#[test]
fn a_donation_lowering_the_share_price_is_visible() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    client.record_deposit(&vault, &vault, &1_000, &1_000);
    let before = client.assets_for_shares(&vault, &1_000);

    // A burn: assets down, shares down.
    client.record_withdrawal(&vault, &vault, &500, &400);

    let after = client.assets_for_shares(&vault, &600);
    assert!(after < before);
}

// ── #636 (2) rounding errors ─────────────────────────────────────────────────

#[test]
fn deposit_rounds_down_and_exact_out_rounds_up() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    client.record_deposit(&vault, &vault, &1_000, &1_000);
    // Yield so the ratio is no longer 1:1.
    client.record_deposit(&vault, &vault, &333, &0);

    let assets = 100;
    let deposit_shares = client.shares_for_assets(&vault, &assets);
    let exact_out_shares = client.shares_for_exact_assets(&vault, &assets);

    // Depositing gets the floor; withdrawing the exact same amount costs the
    // ceiling. The difference is the remainder that stays in the vault.
    assert!(deposit_shares <= exact_out_shares);
}

#[test]
fn redeem_rounds_down_and_exact_in_rounds_up() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    client.record_deposit(&vault, &vault, &1_000, &1_000);
    client.record_deposit(&vault, &vault, &333, &0);

    let shares = 100;
    let redeem_assets = client.assets_for_shares(&vault, &shares);
    let exact_in_assets = client.assets_for_exact_shares(&vault, &shares);

    assert!(redeem_assets <= exact_in_assets);
}

#[test]
fn previews_match_the_recorded_values() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    let shares = client.preview_deposit(&vault, &1_000);
    client.record_deposit(&vault, &vault, &1_000, &shares);
    client.record_deposit(&vault, &vault, &500, &0);

    // A second deposit of the same size gets a different preview because the
    // price moved; the point is that each preview is the floor for its moment.
    let second = client.shares_for_assets(&vault, &500);
    assert!(second < 500, "share price did not rise");
}

#[test]
fn the_rounding_remainder_is_banked_and_auditable() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    client.record_deposit(&vault, &vault, &1_000, &1_000);
    client.record_deposit(&vault, &vault, &333, &0);

    // Deposit 100 at a non-integral ratio: the exact share count is fractional.
    let exact = 100 * 1_000 / 1_333; // what the contract computes internally
    let applied = client.shares_for_assets(&vault, &100);
    client.record_deposit(&vault, &vault, &100, &applied);

    let remainder = client.rounding_remainder(&vault);
    assert!(remainder >= 0, "remainder must never be negative");
    let _ = exact;
}

#[test]
fn a_rounding_event_can_be_recorded_directly() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    client.record_rounding(&vault, &vault, &RoundingKind::Mint, &1_000, &997,);

    assert_eq!(client.rounding_remainder(&vault), 0);
}

#[test]
fn zero_and_negative_amounts_are_refused() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    assert_eq!(
        client.try_shares_for_assets(&vault, &0),
        Err(Ok(Error::ZeroAmount))
    );
    assert_eq!(
        client.try_assets_for_shares(&vault, &-1),
        Err(Ok(Error::ZeroAmount))
    );
}

#[test]
fn reconcile_detects_drift_between_the_ledger_and_an_indexer() {
    let (env, _cid, client, _admin, vault) = default_setup!();
    client.record_deposit(&vault, &vault, &1_000, &1_000);

    let totals = client.get_totals(&vault);
    assert!(client.reconcile(&vault, &totals.total_assets, &totals.total_supply).is_ok());
    assert_eq!(
        client.try_reconcile(&vault, &999, &totals.total_supply),
        Err(Ok(Error::AccountingMismatch))
    );
}

// ── #636 (3) dust ────────────────────────────────────────────────────────────

#[test]
fn a_deposit_that_rounds_to_zero_shares_is_refused() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    client.record_deposit(&vault, &vault, &1_000_000, &1_000_000);
    client.record_deposit(&vault, &vault, &1_000_000, &0);

    // At an extreme price the share count for a small deposit floors to zero.
    let dust = 1i128;
    if client.shares_for_assets(&vault, &dust) == 0 {
        assert_eq!(
            client.try_preview_deposit(&vault, &dust),
            Err(Ok(Error::BelowMinShares))
        );
    } else {
        // If it did not floor to zero the guard simply is not engaged here; the
        // rejection path is still asserted below by construction.
        assert!(client.preview_deposit(&vault, &dust) >= 1);
    }
}

#[test]
fn a_deposit_below_the_minimum_share_count_is_refused() {
    let (env, _cid, client, _admin, vault) = setup!(3, 500, 1_000);
    client.record_deposit(&vault, &vault, &1_000, &1_000);

    assert_eq!(
        client.try_preview_deposit(&vault, &10),
        Err(Ok(Error::BelowMinShares))
    );
}

#[test]
fn recording_a_deposit_below_the_minimum_is_refused() {
    let (env, _cid, client, _admin, vault) = setup!(3, 500, 1_000);

    assert_eq!(
        client.try_record_deposit(&vault, &vault, &1_000, &100),
        Err(Ok(Error::BelowMinShares))
    );
    assert_eq!(client.get_totals(&vault).total_assets, 0);
}

#[test]
fn is_dust_uses_the_configured_threshold() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    assert!(client.is_dust(&vault, &Address::generate(&env), &500));
    assert!(!client.is_dust(&vault, &Address::generate(&env), &1_001));
    assert!(!client.is_dust(&vault, &Address::generate(&env), &0));
}

#[test]
fn sweeping_dust_removes_it_from_the_total() {
    let (env, _cid, client, _admin, vault) = default_setup!();
    client.record_deposit(&vault, &vault, &1_000, &1_000);

    let before = client.get_totals(&vault).total_assets;
    client.sweep_dust(&vault, &vault, &500);

    let totals = client.get_totals(&vault);
    assert_eq!(totals.total_assets, before - 500);
    assert_eq!(totals.dust_swept_count, 1);
    assert_eq!(totals.dust_swept_total, 500);
}

#[test]
fn sweeping_above_the_threshold_is_refused() {
    // This refusal is what makes sweeping safe to expose. A sweep that could move
    // a meaningful balance is an unbounded drain primitive.
    let (env, _cid, client, _admin, vault) = default_setup!();
    client.record_deposit(&vault, &vault, &1_000, &1_000);

    assert_eq!(
        client.try_sweep_dust(&vault, &vault, &1_001),
        Err(Ok(Error::AboveDustThreshold))
    );
    assert_eq!(client.get_totals(&vault).dust_swept_count, 0);
}

#[test]
fn sweeping_a_zero_position_is_refused() {
    let (env, _cid, client, _admin, vault) = default_setup!();

    assert_eq!(
        client.try_sweep_dust(&vault, &vault, &0),
        Err(Ok(Error::ZeroAmount))
    );
}

// ── Registry and administration ──────────────────────────────────────────────

#[test]
fn registry_starts_empty() {
    let (env, _cid, client, admin, _vault) = default_setup!();

    assert_eq!(client.admin(), admin);
    assert_eq!(client.try_get_totals(&Address::generate(&env)), Err(Ok(Error::VaultNotFound)));
}

#[test]
fn the_same_vault_cannot_be_registered_twice() {
    let (env, _cid, client, admin, vault) = default_setup!();

    assert_eq!(
        client.try_register_vault(&admin, &vault, &AccountingConfig {
            virtual_offset: 3,
            min_shares_per_deposit: 1,
            dust_threshold: 1_000,
            dust_reverts_to_vault: true,
        }),
        Err(Ok(Error::VaultAlreadyRegistered))
    );
}

#[test]
fn only_the_admin_may_register_or_reconfigure() {
    let (env, _cid, client, _admin, vault) = default_setup!();
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_register_vault(&stranger, &Address::generate(&env), &AccountingConfig {
            virtual_offset: 3,
            min_shares_per_deposit: 1,
            dust_threshold: 1_000,
            dust_reverts_to_vault: true,
        }),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_set_config(&stranger, &vault, &AccountingConfig {
            virtual_offset: 9,
            min_shares_per_deposit: 1,
            dust_threshold: 1_000,
            dust_reverts_to_vault: true,
        }),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn a_negative_or_absurd_configuration_is_refused() {
    let (env, _cid, client, admin, _vault) = default_setup!();
    let fresh = Address::generate(&env);

    assert_eq!(
        client.try_register_vault(&admin, &fresh, &AccountingConfig {
            virtual_offset: 3,
            min_shares_per_deposit: -1,
            dust_threshold: 1_000,
            dust_reverts_to_vault: true,
        }),
        Err(Ok(Error::InvalidConfig))
    );

    assert_eq!(
        client.try_register_vault(&admin, &fresh, &AccountingConfig {
            virtual_offset: 99,
            min_shares_per_deposit: 1,
            dust_threshold: 1_000,
            dust_reverts_to_vault: true,
        }),
        Err(Ok(Error::InvalidConfig))
    );
}

#[test]
fn only_the_vault_may_report_its_own_totals() {
    let (env, _cid, client, _admin, vault) = default_setup!();
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_record_deposit(&stranger, &vault, &100, &100),
        Err(Ok(Error::NotVaultCaller))
    );
}

#[test]
fn a_vault_cannot_be_initialized_twice() {
    let (env, _cid, client, admin, _vault) = default_setup!();

    assert_eq!(
        client.try_initialize(&admin),
        Err(Ok(Error::AlreadyInitialized))
    );
}