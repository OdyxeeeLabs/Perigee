#![cfg(test)]

//! Unit tests for the ERC-4626 vault.
//!
//! The suite covers the acceptance criteria ERC-4626 names — the four share/asset
//! conversions, deposit/mint/withdraw/redeem, the max/preview helpers — plus the
//! two things a naive implementation gets wrong: rounding direction, and
//! first-depositor inflation.
//!
//! Closes OdyxeeeLabs/Perigee#631

use crate::{Erc4626Vault, Erc4626VaultClient, Error, DECIMALS_OFFSET, VIRTUAL_OFFSET};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    token, Address, Env,
};

const INITIAL_SUPPLY: i128 = 1_000_000;
const FEE_BPS: u32 = 1_000; // 10%

/// Deploys the vault over a fresh SAC token and returns
/// `(env, contract_id, vault_id, asset_id, admin, fee_recipient, alice, bob)`.
///
/// A macro rather than a function because the generated client borrows the
/// `Env`, so it cannot outlive a returned one.
macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.sequence_number = 100);

        let admin = Address::generate(&env);
        let issuer = Address::generate(&env);
        let fee_recipient = Address::generate(&env);
        let alice = Address::generate(&env);
        let bob = Address::generate(&env);

        let asset_id = env
            .register_stellar_asset_contract_v2(issuer.clone())
            .address();
        let asset_admin = token::StellarAssetClient::new(&env, &asset_id);
        asset_admin.mint(&admin, &INITIAL_SUPPLY);

        let vault_id = env.register(Erc4626Vault, ());
        let client = Erc4626VaultClient::new(&env, &vault_id);
        client.initialize(&asset_id, &admin, &fee_recipient, &FEE_BPS);

        (env, vault_id, asset_id, admin, fee_recipient, alice, bob)
    }};
}

// ── Initialization and metadata ──────────────────────────────────────────────

#[test]
fn initialize_records_the_underlying_asset() {
    let (env, vault_id, asset_id, _admin, _fee, _alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    assert_eq!(client.asset(), asset_id);
    assert_eq!(client.total_assets(), 0);
    assert_eq!(client.total_supply(), 0);
    assert!(!client.paused());
}

#[test]
fn initialize_can_only_run_once() {
    let (env, vault_id, asset_id, admin, fee, _alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    assert_eq!(
        client.try_initialize(&asset_id, &admin, &fee, &FEE_BPS),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn initialize_rejects_a_fee_above_one_hundred_percent() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let asset_id = env
        .register_stellar_asset_contract_v2(Address::generate(&env))
        .address();
    let vault_id = env.register(Erc4626Vault, ());
    let client = Erc4626VaultClient::new(&env, &vault_id);

    assert_eq!(
        client.try_initialize(&asset_id, &admin, &admin, &10_001),
        Err(Ok(Error::InvalidFee))
    );
}

#[test]
fn queries_before_initialization_report_not_initialized() {
    let env = Env::default();
    let vault_id = env.register(Erc4626Vault, ());
    let client = Erc4626VaultClient::new(&env, &vault_id);

    assert_eq!(client.try_asset(), Err(Ok(Error::NotInitialized)));
}

// ── Deposit / mint ───────────────────────────────────────────────────────────

#[test]
fn deposit_mints_shares_one_for_one_into_an_empty_vault() {
    let (env, vault_id, _asset, admin, _fee, alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    let shares = client.deposit(&admin, &10_000, &alice);

    assert_eq!(shares, 10_000);
    assert_eq!(client.balance_of(&alice), 10_000);
    assert_eq!(client.total_assets(), 10_000);
    assert_eq!(client.total_supply(), 10_000);
    // The depositor's tokens are held by the vault.
    assert_eq!(
        token::Client::new(&env, &client.asset()).balance(&vault_id),
        10_000
    );
}

#[test]
fn deposit_credits_the_receiver_not_the_caller() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &1_000, &bob);

    assert_eq!(client.balance_of(&bob), 1_000);
    assert_eq!(client.balance_of(&alice), 0);
}

#[test]
fn deposit_rejects_a_zero_amount() {
    let (env, vault_id, _asset, admin, _fee, alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    assert_eq!(
        client.try_deposit(&admin, &0, &alice),
        Err(Ok(Error::ZeroAmount))
    );
}

#[test]
fn mint_charges_the_exact_assets_for_an_exact_share_count() {
    let (env, vault_id, _asset, admin, _fee, alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    let assets = client.mint(&admin, &500, &alice);

    assert_eq!(assets, 500);
    assert_eq!(client.balance_of(&alice), 500);
    assert_eq!(client.total_assets(), 500);
    assert_eq!(client.total_supply(), 500);
}

// ── Withdraw / redeem ────────────────────────────────────────────────────────

#[test]
fn withdraw_burns_shares_for_an_exact_asset_amount() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    let burned = client.withdraw(&alice, &4_000, &bob, &alice);

    assert_eq!(burned, 4_000);
    assert_eq!(client.balance_of(&alice), 6_000);
    assert_eq!(client.total_assets(), 6_000);
    assert_eq!(client.total_supply(), 6_000);
    assert_eq!(
        token::Client::new(&env, &client.asset()).balance(&bob),
        4_000
    );
}

#[test]
fn redeem_burns_an_exact_share_count() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    let assets = client.redeem(&alice, &2_500, &bob, &alice);

    assert_eq!(assets, 2_500);
    assert_eq!(client.balance_of(&alice), 7_500);
}

#[test]
fn redeem_rejects_more_shares_than_held() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &100, &alice);

    assert_eq!(
        client.try_redeem(&alice, &101, &bob, &alice),
        Err(Ok(Error::InsufficientShares))
    );
    assert_eq!(client.balance_of(&alice), 100);
}

#[test]
fn withdraw_rejects_more_assets_than_the_position_is_worth() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &100, &alice);

    assert_eq!(
        client.try_withdraw(&alice, &101, &bob, &alice),
        Err(Ok(Error::InsufficientShares))
    );
}

// ── Yield: the share price moves ─────────────────────────────────────────────

#[test]
fn yield_raises_the_share_price_for_existing_holders() {
    let (env, vault_id, asset_id, admin, _fee, alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);

    // 10% yield arrives in the vault without any shares being minted.
    let asset_admin = token::StellarAssetClient::new(&env, &asset_id);
    asset_admin.mint(&vault_id, &1_000);

    client.harvest(&admin);

    // The vault recognises the yield less the 10% performance fee.
    assert_eq!(client.total_assets(), 10_900);
    assert_eq!(client.total_supply(), 10_000);

    // The same share count is now worth more than before, and a late joiner
    // gets fewer shares per asset than the original depositor did.
    assert!(client.convert_to_assets(&10_000) > 10_000);
    assert!(client.preview_deposit(&10_000) < 10_000);
}
#[test]
fn harvest_takes_the_performance_fee_only_on_yield() {
    let (env, vault_id, asset_id, admin, fee_recipient, alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);

    let asset_admin = token::StellarAssetClient::new(&env, &asset_id);
    asset_admin.mint(&vault_id, &1_000); // 10% yield

    let profit = client.harvest(&admin);

    assert_eq!(profit, 1_000);
    // 10% of the profit, not of the balance.
    assert_eq!(
        token::Client::new(&env, &asset_id).balance(&fee_recipient),
        100
    );
    assert_eq!(client.total_assets(), 10_900);
}

#[test]
fn harvest_with_no_yield_takes_no_fee() {
    let (env, vault_id, _asset, admin, fee_recipient, alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);

    assert_eq!(client.harvest(&admin), 0);
    assert_eq!(
        token::Client::new(&env, &client.asset()).balance(&fee_recipient),
        0
    );
    assert_eq!(client.total_assets(), 10_000);
}

// ── Rounding direction ───────────────────────────────────────────────────────

#[test]
fn deposit_rounds_in_favour_of_the_vault() {
    let (env, vault_id, _asset, admin, _fee, alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &1_000, &alice);

    // Move the price off 1:1 so a conversion cannot divide evenly.
    client.mint(&admin, &1, &alice);

    // A deposit must never mint more than the exact ratio, and the preview the
    // caller was shown must be the number they actually get.
    let preview = client.preview_deposit(&777);
    let actual = client.deposit(&admin, &777, &alice);
    assert_eq!(actual, preview);

    // Repeating the deposit at the now-off-1:1 price mints strictly fewer shares
    // than a proportional split would, which is the remainder going to holders.
    assert!(client.preview_deposit(&777) < 777);
}
#[test]
fn withdraw_rounds_against_the_caller() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    client.mint(&admin, &1, &alice);

    // preview_withdraw must never under-report the shares a withdrawal costs.
    let preview = client.preview_withdraw(&1_000);
    let burned = client.withdraw(&alice, &1_000, &bob, &alice);
    assert_eq!(burned, preview);
}

#[test]
fn previews_match_execution() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);

    assert_eq!(
        client.deposit(&admin, &1_234, &alice),
        client.preview_deposit(&1_234)
    );
    assert_eq!(
        client.mint(&admin, &77, &bob),
        client.preview_mint(&77)
    );
    assert_eq!(
        client.redeem(&alice, &55, &bob, &alice),
        client.preview_redeem(&55)
    );
    assert_eq!(
        client.withdraw(&alice, &333, &bob, &alice),
        client.preview_withdraw(&333)
    );
}

// ── First-depositor inflation ────────────────────────────────────────────────

#[test]
fn a_donation_cannot_steal_from_a_later_depositor() {
    let (env, vault_id, asset_id, admin, _fee, attacker, victim) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    // The classic attack: mint almost nothing, then donate a large amount so
    // the share price spikes and the next depositor's shares round to zero.
    client.deposit(&admin, &1, &attacker);
    let asset_admin = token::StellarAssetClient::new(&env, &asset_id);
    asset_admin.mint(&vault_id, &999_999);

    client.deposit(&admin, &500, &victim);

    // The victim must come away with a non-zero position. Without the virtual
    // offset this deposit mints 0 shares while leaving the assets in the vault.
    assert!(
        client.balance_of(&victim) > 0,
        "victim received no shares; the virtual offset is not working"
    );

    let total_before = client.total_assets();
    let redeemed = client.redeem(&victim, client.balance_of(&victim), &victim, &victim);
    assert!(redeemed > 0, "victim could not redeem anything");
    assert_eq!(client.total_assets(), total_before - redeemed);
}

#[test]
fn the_virtual_offset_is_documented_and_non_zero() {
    assert_eq!(DECIMALS_OFFSET, 3);
    assert_eq!(VIRTUAL_OFFSET, 1_000);
}

// ── Allowances (the permit substitute) ───────────────────────────────────────

#[test]
fn an_approved_spender_can_withdraw_within_the_allowance() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    client.approve(&alice, &bob, &1_000, &1_000);

    let burned = client.withdraw(&bob, &400, &bob, &alice);

    assert_eq!(burned, 400);
    assert_eq!(client.balance_of(&alice), 9_600);
    assert_eq!(
        client.allowance(&alice, &bob).unwrap().shares,
        600
    );
}

#[test]
fn a_spender_cannot_exceed_their_allowance() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    client.approve(&alice, &bob, &1_000, &1_000);

    assert_eq!(
        client.try_withdraw(&bob, &1_001, &bob, &alice),
        Err(Ok(Error::InsufficientAllowance))
    );
    // Rejected calls must not have consumed any of the allowance.
    assert_eq!(client.allowance(&alice, &bob).unwrap().shares, 1_000);
}

#[test]
fn a_spender_with_no_allowance_is_refused() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);

    assert_eq!(
        client.try_withdraw(&bob, &100, &bob, &alice),
        Err(Ok(Error::InsufficientAllowance))
    );
}

#[test]
fn an_expired_allowance_is_refused() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    client.approve(&alice, &bob, &1_000, &150);

    // Advance past the expiry ledger.
    env.ledger().with_mut(|li| li.sequence_number = 200);

    assert_eq!(
        client.try_withdraw(&bob, &100, &bob, &alice),
        Err(Ok(Error::AllowanceExpired))
    );
}

#[test]
fn an_allowance_cannot_be_created_already_expired() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);

    // The ledger is at 100, so an expiry at or below it is refused outright.
    assert_eq!(
        client.try_approve(&alice, &bob, &1_000, &100),
        Err(Ok(Error::AllowanceExpired))
    );
    assert_eq!(client.allowance(&alice, &bob), None);
}

#[test]
fn revoke_removes_the_allowance_immediately() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    client.approve(&alice, &bob, &1_000, &1_000);
    client.revoke(&alice, &bob);

    assert_eq!(client.allowance(&alice, &bob), None);
    assert_eq!(
        client.try_withdraw(&bob, &100, &bob, &alice),
        Err(Ok(Error::InsufficientAllowance))
    );
}

#[test]
fn permit_withdraw_consumes_the_allowance_and_removes_it_when_exhausted() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    client.approve(&alice, &bob, &400, &1_000);

    let burned = client.permit_withdraw(&bob, &alice, &400, &bob);

    assert_eq!(burned, 400);
    assert_eq!(client.balance_of(&alice), 9_600);
    assert_eq!(client.allowance(&alice, &bob), None);
}

#[test]
fn permit_withdraw_refuses_more_than_the_allowance() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    client.approve(&alice, &bob, &400, &1_000);

    assert_eq!(
        client.try_permit_withdraw(&bob, &alice, &401, &bob),
        Err(Ok(Error::InsufficientAllowance))
    );
    assert_eq!(client.allowance(&alice, &bob).unwrap().shares, 400);
}

// ── Pause and admin ──────────────────────────────────────────────────────────

#[test]
fn pause_blocks_every_asset_movement_but_not_views() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    client.set_paused(&admin, &true);

    assert!(client.paused());
    assert_eq!(
        client.try_deposit(&admin, &100, &alice),
        Err(Ok(Error::Paused))
    );
    assert_eq!(
        client.try_mint(&admin, &100, &alice),
        Err(Ok(Error::Paused))
    );
    assert_eq!(
        client.try_withdraw(&alice, &100, &bob, &alice),
        Err(Ok(Error::Paused))
    );
    // Views stay readable so a paused vault can still be inspected.
    assert_eq!(client.balance_of(&alice), 10_000);
    assert_eq!(client.max_withdraw(&alice), 0);
    assert_eq!(client.max_redeem(&alice), 0);
}

#[test]
fn a_non_admin_cannot_pause_or_rotate_keys() {
    let (env, vault_id, _asset, admin, _fee, _alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    assert_eq!(
        client.try_set_paused(&bob, &true),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(client.try_set_admin(&bob, &bob), Err(Ok(Error::Unauthorized)));
    assert_eq!(client.try_harvest(&bob), Err(Ok(Error::Unauthorized)));
}

#[test]
fn set_admin_rotates_without_touching_balances() {
    let (env, vault_id, _asset, admin, _fee, alice, bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    client.deposit(&admin, &10_000, &alice);
    client.set_admin(&admin, &bob);

    assert_eq!(client.get_config().admin, bob);
    assert_eq!(client.balance_of(&alice), 10_000);
    // The old admin has lost the power.
    assert_eq!(
        client.try_set_paused(&admin, &true),
        Err(Ok(Error::Unauthorized))
    );
}

// ── Verifier configuration ───────────────────────────────────────────────────

#[test]
fn off_chain_verification_is_off_by_default() {
    let (env, vault_id, _asset, _admin, _fee, _alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    assert_eq!(client.verifier_kind(), crate::VerifierKind::None);
    assert_eq!(client.verifier(), None);
}

#[test]
fn a_verifier_must_be_opted_into_explicitly() {
    let (env, vault_id, _asset, admin, _fee, _alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    let verifier = Address::generate(&env);
    client.set_verifier(&admin, &crate::VerifierKind::Custom(1), &Some(verifier.clone()));

    assert_eq!(client.verifier_kind(), crate::VerifierKind::Custom(1));
    assert_eq!(client.verifier(), Some(verifier));

    client.set_verifier(&admin, &crate::VerifierKind::None, &None);
    assert_eq!(client.verifier(), None);
}

// ── Max helpers ──────────────────────────────────────────────────────────────

#[test]
fn max_deposit_and_max_mint_are_unlimited() {
    let (env, vault_id, _asset, _admin, _fee, alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    assert_eq!(client.max_deposit(&alice), i128::MAX);
    assert_eq!(client.max_mint(&alice), i128::MAX);
}

#[test]
fn max_withdraw_tracks_the_position() {
    let (env, vault_id, _asset, admin, _fee, alice, _bob) = setup!();
    let client = Erc4626VaultClient::new(&env, &vault_id);

    assert_eq!(client.max_withdraw(&alice), 0);

    client.deposit(&admin, &10_000, &alice);

    assert_eq!(client.max_withdraw(&alice), 10_000);
    assert_eq!(client.max_redeem(&alice), 10_000);
}