#![cfg(test)]

use crate::{Error, FeeSwitch, FeeSwitchClient, FeeSwitchConfig, BPS_DENOMINATOR};
use soroban_sdk::{testutils::Address as _, Address, Env};

fn setup() -> (Env, Address, Address, FeeSwitchConfig) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let config = FeeSwitchConfig {
        fee_bps: 30,
        treasury_bps: 5_000,
        stakers_bps: 3_000,
        buyback_bps: 2_000,
        treasury: Address::generate(&env),
        stakers_recipient: Address::generate(&env),
        governance_token: Address::generate(&env),
    };
    let id = env.register(FeeSwitch, ());
    let client = FeeSwitchClient::new(&env, &id);
    client.initialize(&admin, &config);
    (env, id, admin, config)
}

#[test]
fn starts_inactive_and_can_be_toggled() {
    let (env, id, admin, _) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    assert!(!client.is_active());
    client.activate(&admin);
    assert!(client.is_active());
    client.deactivate(&admin);
    assert!(!client.is_active());
}

#[test]
fn collect_requires_active_switch() {
    let (env, id, _admin, _) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    assert_eq!(client.try_collect(&1_000), Err(Ok(Error::SwitchInactive)));
}

#[test]
fn collect_splits_revenue_exactly() {
    let (env, id, admin, _) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    client.activate(&admin);
    let buckets = client.collect(&10_000);
    assert_eq!(buckets.treasury, 5_000);
    assert_eq!(buckets.stakers, 3_000);
    assert_eq!(buckets.buyback, 2_000);
    assert_eq!(
        buckets.treasury + buckets.stakers + buckets.buyback,
        buckets.total_collected
    );
}

#[test]
fn buyback_burns_and_reduces_budget() {
    let (env, id, admin, _) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    client.activate(&admin);
    client.collect(&10_000);
    let after = client.execute_buyback(&admin, &1_000, &50);
    assert_eq!(after.buyback, 1_000);
    assert_eq!(after.total_burned, 50);
}

#[test]
fn invalid_bps_split_is_rejected() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(FeeSwitch, ());
    let client = FeeSwitchClient::new(&env, &id);
    let bad = FeeSwitchConfig {
        fee_bps: 30,
        treasury_bps: 4_000,
        stakers_bps: 3_000,
        buyback_bps: 2_000, // sums to 9_000
        treasury: Address::generate(&env),
        stakers_recipient: Address::generate(&env),
        governance_token: Address::generate(&env),
    };
    assert_eq!(
        client.try_initialize(&admin, &bad),
        Err(Ok(Error::InvalidBps))
    );
    let _ = BPS_DENOMINATOR;
}

#[test]
fn quote_fee_zero_when_inactive() {
    let (env, id, admin, _) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    assert_eq!(client.quote_fee(&1_000_000), 0);
    client.activate(&admin);
    assert_eq!(client.quote_fee(&1_000_000), 3_000); // 30 bps
}

// --- Accounting across configuration changes (#866) ---

fn config_with(env: &Env, base: &FeeSwitchConfig, t: u32, s: u32, b: u32) -> FeeSwitchConfig {
    let _ = env;
    FeeSwitchConfig {
        treasury_bps: t,
        stakers_bps: s,
        buyback_bps: b,
        ..base.clone()
    }
}

#[test]
fn disabled_switch_accrues_nothing() {
    let (env, id, admin, _) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    assert_eq!(client.try_collect(&1_000), Err(Ok(Error::SwitchInactive)));
    let b = client.get_buckets();
    assert_eq!(b.total_collected, 0);
    assert_eq!(b.treasury + b.stakers + b.buyback, 0);

    // Deactivating again stops accrual and leaves earlier accruals untouched.
    client.activate(&admin);
    client.collect(&10_000);
    client.deactivate(&admin);
    assert_eq!(client.try_collect(&10_000), Err(Ok(Error::SwitchInactive)));
    let b = client.get_buckets();
    assert_eq!(b.total_collected, 10_000);
    assert_eq!(b.treasury, 5_000);
}

#[test]
fn each_accrual_uses_the_split_active_at_that_time() {
    let (env, id, admin, base) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    client.activate(&admin);

    // 50/30/20
    client.collect(&10_000);
    // Change to 10/10/80 — earlier accruals must not be rewritten.
    client.set_config(&admin, &config_with(&env, &base, 1_000, 1_000, 8_000));
    let after_first = client.get_buckets();
    assert_eq!(after_first.treasury, 5_000);
    assert_eq!(after_first.stakers, 3_000);
    assert_eq!(after_first.buyback, 2_000);

    client.collect(&10_000);
    let b = client.get_buckets();
    assert_eq!(b.treasury, 5_000 + 1_000);
    assert_eq!(b.stakers, 3_000 + 1_000);
    assert_eq!(b.buyback, 2_000 + 8_000);
    assert_eq!(b.total_collected, 20_000);
    assert_eq!(b.treasury + b.stakers + b.buyback, b.total_collected);

    // And back to a treasury-only split.
    client.set_config(&admin, &config_with(&env, &base, 10_000, 0, 0));
    client.collect(&500);
    let b = client.get_buckets();
    assert_eq!(b.treasury, 6_500);
    assert_eq!(b.stakers, 4_000);
    assert_eq!(b.buyback, 10_000);
    assert_eq!(b.treasury + b.stakers + b.buyback, b.total_collected);
}

#[test]
fn rounding_remainder_goes_to_buyback_and_totals_reconcile() {
    let (env, id, admin, base) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    client.activate(&admin);
    client.set_config(&admin, &config_with(&env, &base, 3_333, 3_333, 3_334));

    let mut expected_total = 0i128;
    for amount in [1i128, 2, 3, 7, 99, 101, 10_001] {
        client.collect(&amount);
        expected_total += amount;
        let b = client.get_buckets();
        assert_eq!(b.total_collected, expected_total);
        assert_eq!(b.treasury + b.stakers + b.buyback, expected_total);
    }

    // A single unit cannot be split: treasury and stakers round to zero and
    // the whole unit lands in the buyback bucket.
    let (env2, id2, admin2, base2) = setup();
    let c2 = FeeSwitchClient::new(&env2, &id2);
    c2.activate(&admin2);
    c2.set_config(&admin2, &config_with(&env2, &base2, 3_333, 3_333, 3_334));
    let b = c2.collect(&1);
    assert_eq!((b.treasury, b.stakers, b.buyback), (0, 0, 1));
}

#[test]
fn rejected_config_change_keeps_previous_split() {
    let (env, id, admin, base) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    client.activate(&admin);
    let bad = config_with(&env, &base, 5_000, 5_000, 5_000);
    assert_eq!(client.try_set_config(&admin, &bad), Err(Ok(Error::InvalidBps)));
    let b = client.collect(&10_000);
    assert_eq!((b.treasury, b.stakers, b.buyback), (5_000, 3_000, 2_000));
}

#[test]
fn config_change_emits_the_new_split() {
    use soroban_sdk::{testutils::Events, vec, IntoVal, Symbol};
    let (env, id, admin, base) = setup();
    let client = FeeSwitchClient::new(&env, &id);
    client.set_config(&admin, &config_with(&env, &base, 2_500, 2_500, 5_000));
    let events = env.events().all();
    assert_eq!(
        events,
        vec![
            &env,
            (
                id.clone(),
                (Symbol::new(&env, "fee_switch"), Symbol::new(&env, "config")).into_val(&env),
                (2_500u32, 2_500u32, 5_000u32).into_val(&env),
            )
        ]
    );
}
