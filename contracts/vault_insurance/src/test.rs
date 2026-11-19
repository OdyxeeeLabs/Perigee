#![cfg(test)]

//! Unit tests for the vault insurance contract.
//!
//! The three properties that matter are the ones a naive implementation gets
//! wrong: the reserve must be able to back what it has promised, premiums must
//! only be charged for cover actually taken, and a claim may only cite a trigger
//! the policy already declared.
//!
//! Closes OdyxeeeLabs/Perigee#638

use crate::{Provider, Trigger, VaultInsurance, VaultInsuranceClient, Error, EPOCH_LEDGERS};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

/// Triggers a policy declares, covering all four trigger kinds.
macro_rules! triggers {
    ($env:expr) => {
        vec::Vec::from_array(
            $env,
            [
                Trigger::StrategyLoss,
                Trigger::OracleDeviation,
                Trigger::ContractFailure,
            ],
        )
    };
}

macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();
        env.ledger().with_mut(|li| li.sequence_number = 1_000);

        let admin = Address::generate(&env);
        let recipient = Address::generate(&env);
        let vault = Address::generate(&env);

        let contract_id = env.register(VaultInsurance, ());
        let client = VaultInsuranceClient::new(&env, &contract_id);
        client.initialize(&admin, &recipient);

        (env, contract_id, client, admin, recipient, vault)
    }};
}

/// Fund the reserve and write a policy, which is the precondition for most tests.
macro_rules! with_policy {
    ($env:expr, $client:expr, $admin:expr, $vault:expr) => {{
        $client.fund($admin, &1_000_000);
        $client.write(
            $admin,
            $vault,
            &Provider::NexusMutual,
            &500_000,
            &1_000_000,
            &365, // 1%/yr
            &triggers($env),
        );
    }};
}

// ── Policy ───────────────────────────────────────────────────────────────────

#[test]
fn a_policy_records_its_terms() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    let policy = client.get_policy(&vault);
    assert_eq!(policy.provider, Provider::NexusMutual);
    assert_eq!(policy.coverage, 500_000);
    assert_eq!(policy.insured_value, 1_000_000);
    assert_eq!(policy.premium_rate_bps, 365);
    assert!(policy.active);
    assert_eq!(policy.triggers.len(), 3);
}

#[test]
fn a_second_policy_for_the_same_vault_is_refused() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    assert_eq!(
        client.try_write(
            &admin,
            &vault,
            &Provider::InsurAce,
            &1,
            &1_000_000,
            &365,
            &triggers(&env)
        ),
        Err(Ok(Error::PolicyExists))
    );
}

#[test]
fn coverage_above_the_insured_value_is_refused() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    client.fund(&admin, &1_000_000);

    assert_eq!(
        client.try_write(
            &admin,
            &vault,
            &Provider::NexusMutual,
            &2_000_000,
            &1_000_000,
            &365,
            &triggers(&env)
        ),
        Err(Ok(Error::CoverageAboveCap))
    );
}

#[test]
fn a_premium_rate_above_one_hundred_percent_is_refused() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    client.fund(&admin, &1_000_000);

    assert_eq!(
        client.try_write(
            &admin,
            &vault,
            &Provider::NexusMutual,
            &500_000,
            &1_000_000,
            &10_001,
            &triggers(&env)
        ),
        Err(Ok(Error::PremiumRateTooHigh))
    );
}

#[test]
fn cancelling_a_policy_releases_its_coverage() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    assert_eq!(client.reserve().outstanding_coverage, 500_000);

    client.cancel(&admin, &vault);

    assert!(!client.get_policy(&vault).active);
    assert_eq!(client.reserve().outstanding_coverage, 0);
}

// ── The reserve must back what it promises ──────────────────────────────────

#[test]
fn coverage_beyond_the_reserve_is_refused() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();

    // No funding at all, so nothing is supportable.
    assert_eq!(
        client.try_write(
            &admin,
            &vault,
            &Provider::NexusMutual,
            &1,
            &1_000_000,
            &365,
            &triggers(&env)
        ),
        Err(Ok(Error::CoverageExceedsReserve))
    );
}

#[test]
fn the_reserve_must_back_the_sum_of_outstanding_coverage() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.sequence_number = 1_000);

    let admin = Address::generate(&env);
    let first = Address::generate(&env);
    let second = Address::generate(&env);

    let contract_id = env.register(VaultInsurance, ());
    let client = VaultInsuranceClient::new(&env, &contract_id);
    client.initialize(&admin, &Address::generate(&env));

    // Fund enough for one policy only.
    client.fund(&admin, &600_000);
    client.write(
        &admin,
        &first,
        &Provider::NexusMutual,
        &500_000,
        &1_000_000,
        &365,
        &triggers(&env),
    );

    // A second 500k policy would put 1_000_000 outstanding against 600_000 held.
    assert_eq!(
        client.try_write(
            &admin,
            &second,
            &Provider::NexusMutual,
            &500_000,
            &1_000_000,
            &365,
            &triggers(&env)
        ),
        Err(Ok(Error::CoverageExceedsReserve))
    );
}

#[test]
fn assess_coverage_tells_an_integrator_the_limit_up_front() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    // 1_000_000 held, 500_000 already outstanding.
    assert_eq!(client.assess_coverage(&1_000_000), 500_000);
    // And it never exceeds the requested insured value.
    assert_eq!(client.assess_coverage(&100), 100);
}

#[test]
fn premiums_increase_the_cover_that_can_be_written() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    client.fund(&admin, &1_000_000);

    assert_eq!(client.assess_coverage(&1_000_000), 1_000_000);
    assert!(client
        .write(
            &admin,
            &vault,
            &Provider::SelfFunded,
            &1_000_000,
            &1_000_000,
            &365,
            &triggers(&env)
        )
        .is_ok());
}

// ── Premiums ─────────────────────────────────────────────────────────────────

#[test]
fn no_premium_is_assessed_before_an_epoch_elapses() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    assert_eq!(client.assess_premium(&vault), 0);
}

#[test]
fn a_premium_is_assessed_per_elapsed_epoch() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    // 1%/yr of 1_000_000 is 10_000/yr, so ~27 per day.
    env.ledger()
        .with_mut(|li| li.sequence_number += EPOCH_LEDGERS * 3);
    let premium = client.assess_premium(&vault);

    assert!(premium > 0);
    assert!(
        (70..=90).contains(&premium),
        "three epochs of 1%/yr on 1_000_000 should be ~81, got {premium}"
    );
}

#[test]
fn a_deposit_accrues_the_premium_so_far() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    env.ledger()
        .with_mut(|li| li.sequence_number += EPOCH_LEDGERS);
    client.deposit(&vault, &1_000_000);

    assert!(client.get_policy(&vault).accrued_premium > 0);
}

#[test]
fn withdrawing_collects_the_accrued_premium_and_returns_the_rest() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    env.ledger()
        .with_mut(|li| li.sequence_number += EPOCH_LEDGERS);
    client.deposit(&vault, &1_000_000);

    let accrued = client.get_policy(&vault).accrued_premium;
    let (premium, net) = client.withdraw(&vault, &10_000);

    assert_eq!(premium, accrued);
    assert_eq!(net, 10_000 - accrued);
    assert_eq!(client.get_policy(&vault).accrued_premium, 0);
    assert_eq!(client.reserve().premiums_in, accrued);
}

#[test]
fn a_depositor_who_leaves_at_once_pays_nothing() {
    // The premium is assessed, not charged, so an immediate exit costs nothing.
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    let (premium, net) = client.withdraw(&vault, &10_000);

    assert_eq!(premium, 0);
    assert_eq!(net, 10_000);
}

#[test]
fn a_premium_never_exceeds_the_withdrawal() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    client.fund(&admin, &10_000_000);
    client.write(
        &admin,
        &vault,
        &Provider::NexusMutual,
        &5_000_000,
        &10_000_000,
        &3_650, // 10%/yr
        &triggers(&env),
    );

    // Run enough epochs that the accrued premium exceeds the withdrawal.
    env.ledger()
        .with_mut(|li| li.sequence_number += EPOCH_LEDGERS * 100);
    client.deposit(&vault, &10_000_000);

    let (premium, net) = client.withdraw(&vault, &500);

    assert_eq!(premium, 500);
    assert_eq!(net, 0);
    // A fully-insured withdrawal releases the policy.
    assert!(!client.get_policy(&vault).active);
}

#[test]
fn a_zero_withdrawal_is_refused() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    assert_eq!(
        client.try_withdraw(&vault, &0),
        Err(Ok(Error::ZeroAmount))
    );
}

// ── Claims ───────────────────────────────────────────────────────────────────

#[test]
fn a_claim_cites_a_declared_trigger() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    let claimant = Address::generate(&env);
    client.open_claim(&claimant, &vault, &Trigger::StrategyLoss, &100_000);

    let claim = client.get_claim(&vault);
    assert_eq!(claim.trigger, Trigger::StrategyLoss);
    assert_eq!(claim.amount, 100_000);
    assert!(!claim.settled);
}

#[test]
fn a_claim_on_an_undeclared_trigger_is_refused() {
    // This ordering is the point: a claim may not pick its own trigger, or a
    // vault could claim against its own mispricing by declaring it afterwards.
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    let claimant = Address::generate(&env);
    assert_eq!(
        client.try_open_claim(&claimant, &vault, &Trigger::LiquidationCascade, &1_000),
        Err(Ok(Error::TriggerNotCovered))
    );
}

#[test]
fn a_claim_beyond_the_coverage_is_refused() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    let claimant = Address::generate(&env);
    assert_eq!(
        client.try_open_claim(&claimant, &vault, &Trigger::StrategyLoss, &600_000),
        Err(Ok(Error::SettlementTooLarge))
    );
}

#[test]
fn only_one_claim_is_open_at_a_time() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    let claimant = Address::generate(&env);
    client.open_claim(&claimant, &vault, &Trigger::StrategyLoss, &100_000);

    assert_eq!(
        client.try_open_claim(&claimant, &vault, &Trigger::OracleDeviation, &50_000),
        Err(Ok(Error::ClaimAlreadyOpen))
    );
}

#[test]
fn settling_pays_out_of_the_reserve() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    let claimant = Address::generate(&env);
    client.open_claim(&claimant, &vault, &Trigger::StrategyLoss, &100_000);

    let before = client.reserve();
    client.settle(&admin, &vault, &100_000);
    let after = client.reserve();

    assert!(client.get_claim(&vault).settled);
    assert_eq!(after.payouts_out, before.payouts_out + 100_000);
    assert_eq!(after.unclaimed_premium, before.unclaimed_premium - 100_000);
}

#[test]
fn a_settlement_beyond_the_claim_is_refused() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    let claimant = Address::generate(&env);
    client.open_claim(&claimant, &vault, &Trigger::StrategyLoss, &100_000);

    assert_eq!(
        client.try_settle(&admin, &vault, &100_001),
        Err(Ok(Error::SettlementTooLarge))
    );
}

#[test]
fn a_settlement_beyond_the_reserve_is_refused() {
    let env = Env::default();
    env.mock_all_auths();
    env.ledger().with_mut(|li| li.sequence_number = 1_000);

    let admin = Address::generate(&env);
    let vault = Address::generate(&env);
    let claimant = Address::generate(&env);

    let contract_id = env.register(VaultInsurance, ());
    let client = VaultInsuranceClient::new(&env, &contract_id);
    client.initialize(&admin, &Address::generate(&env));

    // Fund and write, then drain the reserve with premium collection so the
    // claim's payout no longer fits.
    client.fund(&admin, &600_000);
    client.write(
        &admin,
        &vault,
        &Provider::NexusMutual,
        &500_000,
        &1_000_000,
        &3_650,
        &triggers(&env),
    );
    env.ledger()
        .with_mut(|li| li.sequence_number += EPOCH_LEDGERS);
    client.deposit(&vault, &1_000_000);
    client.withdraw(&vault, &550_000);

    client.open_claim(&claimant, &vault, &Trigger::StrategyLoss, &100_000);

    assert_eq!(
        client.try_settle(&admin, &vault, &100_000),
        Err(Ok(Error::SettlementTooLarge))
    );
}

#[test]
fn a_claim_can_be_closed_without_payout() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    let claimant = Address::generate(&env);
    client.open_claim(&claimant, &vault, &Trigger::StrategyLoss, &100_000);
    client.close_claim(&admin, &vault);

    assert_eq!(
        client.try_get_claim(&vault),
        Err(Ok(Error::NoOpenClaim))
    );
}

#[test]
fn a_vault_without_a_claim_has_none() {
    let (env, _cid, client, admin, _recipient, vault) = setup!();
    with_policy!(env, client, admin, vault);

    assert_eq!(
        client.try_get_claim(&vault),
        Err(Ok(Error::NoOpenClaim))
    );
}

// ── Administration ───────────────────────────────────────────────────────────

#[test]
fn only_the_admin_may_write_settle_or_fund() {
    let (env, _cid, client, _admin, _recipient, vault) = setup!();
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_fund(&stranger, &1_000),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_write(
            &stranger,
            &vault,
            &Provider::NexusMutual,
            &1,
            &1,
            &1,
            &triggers(&env)
        ),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_settle(&stranger, &vault, &1),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn providers_have_labels() {
    let (env, _cid, client, _admin, _recipient, _vault) = setup!();

    assert_eq!(
        client.provider_label(&Provider::NexusMutual),
        soroban_sdk::String::from_str(&env, "Nexus Mutual")
    );
    assert_eq!(
        client.provider_label(&Provider::InsurAce),
        soroban_sdk::String::from_str(&env, "InsurAce")
    );
}

#[test]
fn the_registry_can_only_be_initialized_once() {
    let (env, _cid, client, admin, recipient, _vault) = setup!();

    assert_eq!(
        client.try_initialize(&admin, &recipient),
        Err(Ok(Error::AlreadyInitialized))
    );
}

// ── #849 — initialization state ─────────────────────────────────────────────

#[test]
fn initialization_records_admin_recipient_and_empty_reserve() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let recipient = Address::generate(&env);
    let contract_id = env.register(VaultInsurance, ());
    let client = VaultInsuranceClient::new(&env, &contract_id);

    assert_eq!(client.try_admin(), Err(Ok(Error::NotInitialized)));

    client.initialize(&admin, &recipient);

    assert_eq!(client.admin(), admin);
    assert_eq!(client.premium_recipient(), recipient);
    let reserve = client.reserve();
    assert_eq!(reserve.premiums_in, 0);
    assert_eq!(reserve.payouts_out, 0);
    assert_eq!(reserve.unclaimed_premium, 0);
    assert_eq!(reserve.outstanding_coverage, 0);
}

#[test]
fn calls_before_initialization_are_rejected() {
    let env = Env::default();
    env.mock_all_auths();

    let caller = Address::generate(&env);
    let vault = Address::generate(&env);
    let recipient = Address::generate(&env);
    let contract_id = env.register(VaultInsurance, ());
    let client = VaultInsuranceClient::new(&env, &contract_id);

    assert_eq!(client.try_admin(), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_premium_recipient(), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_reserve(), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_get_policy(&vault), Err(Ok(Error::NoPolicy)));
    assert_eq!(
        client.try_fund(&caller, &1_000),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_initialize(&caller, &recipient),
        Ok(Ok(())),
        "a fresh registry must still accept its first initialization"
    );
}