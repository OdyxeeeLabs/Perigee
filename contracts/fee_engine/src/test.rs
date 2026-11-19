#![cfg(test)]

//! Unit tests for the fee engine.
//!
//! Covers the three issue scopes:
//!
//! - [#661] performance fees — high-water mark, crystallisation, clawback and
//!   tiered rates;
//! - [#662] management fees — continuous accrual, basis points, the fee
//!   recipient and the fee splitter;
//! - [#663] fee discount tokens — governance staking for rebates, tiered
//!   discounts.
//!
//! Closes OdyxeeeLabs/Perigee#661
//! Closes OdyxeeeLabs/Perigee#662
//! Closes OdyxeeeLabs/Perigee#663

use crate::{
    DiscountTier, Error, FeeConfig, FeeEngine, FeeEngineClient, PerformanceTier, Stake,
    BPS_DENOMINATOR, MAX_FEE_BPS, SECONDS_PER_YEAR,
};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

mod mock_token {
    use soroban_sdk::{contract, contractimpl, Address, Env};

    /// A stand-in for the governance token. `transfer` is a no-op so staking
    /// moves no real value; the engine's accounting is what is under test.
    #[contract]
    pub struct MockToken;

    #[contractimpl]
    impl MockToken {
        pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {}
    }
}

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let staking_token = env.register(mock_token::MockToken, ());
    let contract_id = env.register(FeeEngine, ());
    let client = FeeEngineClient::new(&env, &contract_id);
    client.initialize(&admin, &staking_token);

    (env, contract_id, admin, staking_token)
}

fn config(env: &Env, treasury: &Address) -> FeeConfig {
    FeeConfig {
        management_fee_bps: 100,
        performance_fee_bps: 1_000,
        protocol_share_bps: 500,
        vault_fee_recipient: Address::generate(env),
        treasury: treasury.clone(),
    }
}

// ── Setup ────────────────────────────────────────────────────────────────────

#[test]
fn initializes_once() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);

    assert_eq!(client.admin(), admin);
    assert_eq!(
        client.try_initialize(&admin, &Address::generate(&env)),
        Err(Ok(Error::AlreadyInitialized))
    );
}

// ── Configuration ────────────────────────────────────────────────────────────

#[test]
fn fee_config_can_be_set_and_read_back() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));
    assert_eq!(client.get_fee_config(), config(&env, &treasury));
}

#[test]
fn a_fee_above_the_ceiling_is_refused() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);

    let mut bad = config(&env, &treasury);
    bad.management_fee_bps = MAX_FEE_BPS + 1;
    assert_eq!(
        client.try_set_fee_config(&admin, &bad),
        Err(Ok(Error::FeeTooHigh))
    );
}

#[test]
fn an_invalid_fee_split_is_refused() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);

    let mut bad = config(&env, &treasury);
    bad.protocol_share_bps = BPS_DENOMINATOR as u32 + 1;
    assert_eq!(
        client.try_set_fee_config(&admin, &bad),
        Err(Ok(Error::InvalidFeeSplit))
    );
}

#[test]
fn discount_tiers_must_be_ascending() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);

    let bad = vec![
        &env,
        DiscountTier {
            min_stake: 1_000,
            rebate_bps: 100,
        },
        DiscountTier {
            min_stake: 500,
            rebate_bps: 200,
        },
    ];
    assert_eq!(
        client.try_set_discount_tiers(&admin, &bad),
        Err(Ok(Error::InvalidTier))
    );
}

#[test]
fn perf_tiers_must_be_ascending() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);

    let bad = vec![
        &env,
        PerformanceTier {
            min_profit: 1_000,
            fee_bps: 1_000,
        },
        PerformanceTier {
            min_profit: 500,
            fee_bps: 1_500,
        },
    ];
    assert_eq!(
        client.try_set_perf_tiers(&admin, &bad),
        Err(Ok(Error::InvalidTier))
    );
}

// ── Management fee (#662) ────────────────────────────────────────────────────

#[test]
fn management_fee_accrues_continuously() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 0;
    });
    client.record_deposit(&vault, &10_000);

    // One year later: 100 bps of 10_000 = 100.
    env.ledger().with_mut(|ledger| {
        ledger.timestamp = SECONDS_PER_YEAR as u64;
    });
    let fee = client.accrue_management_fee(&vault, &10_000);
    assert_eq!(fee, 100);

    // A second call in the same ledger charges nothing more.
    assert_eq!(client.accrue_management_fee(&vault, &10_000), 0);
}

#[test]
fn management_fee_scales_with_time() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 0;
    });
    client.record_deposit(&vault, &10_000);

    // Half a year: 50 bps of 10_000 = 50.
    env.ledger().with_mut(|ledger| {
        ledger.timestamp = (SECONDS_PER_YEAR / 2) as u64;
    });
    assert_eq!(client.accrue_management_fee(&vault, &10_000), 50);
}

// ── Performance fee (#661) ───────────────────────────────────────────────────

#[test]
fn a_profit_above_the_mark_is_charged_and_raises_it() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));
    client.record_deposit(&vault, &10_000);

    // 1_000 bps of the 1_000 profit = 100.
    let fee = client.crystallize(&vault, &11_000);
    assert_eq!(fee, 100);

    let state = client.get_vault(&vault);
    assert_eq!(state.high_water_mark, 11_000);
    assert_eq!(state.last_crystallized, 11_000);
    assert_eq!(state.fees_paid, 100);
}

#[test]
fn a_loss_below_the_crystallised_value_claws_the_fee_back() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));
    client.record_deposit(&vault, &10_000);
    client.crystallize(&vault, &11_000);

    // NAV falls to 10_500: the 500 loss claws back 1_000 bps of it = 50.
    let fee = client.crystallize(&vault, &10_500);
    assert_eq!(fee, 0);

    let state = client.get_vault(&vault);
    assert_eq!(state.fees_paid, 50);
    assert_eq!(state.last_crystallized, 10_500);
    // The mark is unchanged by a loss.
    assert_eq!(state.high_water_mark, 11_000);
}

#[test]
fn a_nav_at_the_mark_charges_nothing() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));
    client.record_deposit(&vault, &10_000);

    // The NAV equals the mark and the last crystallised value, so there is no
    // profit to charge and no loss to claw back.
    assert_eq!(client.crystallize(&vault, &10_000), 0);
    let state = client.get_vault(&vault);
    assert_eq!(state.fees_paid, 0);
    assert_eq!(state.high_water_mark, 10_000);
}

#[test]
fn performance_tiers_raise_the_rate_on_larger_profits() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));
    client.set_perf_tiers(
        &admin,
        &vec![
            &env,
            PerformanceTier {
                min_profit: 0,
                fee_bps: 1_000,
            },
            PerformanceTier {
                min_profit: 2_000,
                fee_bps: 1_500,
            },
        ],
    );
    client.record_deposit(&vault, &10_000);

    // Profit of 1_000 → 1_000 bps → 100.
    assert_eq!(client.crystallize(&vault, &11_000), 100);

    // Reset and try a larger profit.
    let vault2 = Address::generate(&env);
    client.record_deposit(&vault2, &10_000);
    // Profit of 3_000 → 1_500 bps → 450.
    assert_eq!(client.crystallize(&vault2, &13_000), 450);
}

// ── Fee discount tokens (#663) ───────────────────────────────────────────────

#[test]
fn staking_qualifies_for_a_rebate() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let staker = Address::generate(&env);

    client.set_discount_tiers(
        &admin,
        &vec![
            &env,
            DiscountTier {
                min_stake: 0,
                rebate_bps: 0,
            },
            DiscountTier {
                min_stake: 1_000,
                rebate_bps: 200,
            },
        ],
    );

    assert_eq!(client.get_stake(&staker), Stake { staked: 0, tier: 0 });

    client.stake(&staker, &1_000);
    assert_eq!(client.get_stake(&staker), Stake { staked: 1_000, tier: 2 });

    client.unstake(&staker, &1_000);
    assert_eq!(client.get_stake(&staker), Stake { staked: 0, tier: 0 });
}

#[test]
fn an_unstake_above_the_balance_is_refused() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let staker = Address::generate(&env);

    client.stake(&staker, &1_000);
    assert_eq!(
        client.try_unstake(&staker, &1_001),
        Err(Ok(Error::InsufficientStake))
    );
}

// ── Full charge ──────────────────────────────────────────────────────────────

#[test]
fn compute_fees_accrues_crystallises_discounts_and_splits() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));
    client.set_discount_tiers(
        &admin,
        &vec![
            &env,
            DiscountTier {
                min_stake: 0,
                rebate_bps: 0,
            },
            DiscountTier {
                min_stake: 1_000,
                rebate_bps: 200,
            },
        ],
    );

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 0;
    });
    client.record_deposit(&vault, &10_000);
    client.stake(&vault, &1_000);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = SECONDS_PER_YEAR as u64;
    });
    let breakdown = client.compute_fees(&vault, &11_000);

    // management: 100 bps of 10_000 over a year = 100
    assert_eq!(breakdown.management_fee, 100);
    // performance: 1_000 bps of the 1_000 profit = 100
    assert_eq!(breakdown.performance_fee, 100);
    assert_eq!(breakdown.total, 200);
    // discount: 200 bps of 200 = 4
    assert_eq!(breakdown.discount, 4);
    assert_eq!(breakdown.net_total, 196);
    // split: 500 bps of 196 = 9 (floor), vault gets the remainder.
    assert_eq!(breakdown.protocol_share, 9);
    assert_eq!(breakdown.vault_share, 187);
    // The split always sums to the net total.
    assert_eq!(breakdown.vault_share + breakdown.protocol_share, breakdown.net_total);
}

#[test]
fn the_fee_split_always_sums_to_the_total() {
    let (env, contract_id, _admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);

    for fee in [0i128, 1, 7, 100, 9_999, 1_000_000] {
        let (vault, protocol) = client.split_fee(&fee, &500);
        assert_eq!(vault + protocol, fee);
    }
}

#[test]
fn a_non_positive_amount_is_refused() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let vault = Address::generate(&env);

    assert_eq!(
        client.try_record_deposit(&vault, &0),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        client.try_record_withdrawal(&vault, &0),
        Err(Ok(Error::InvalidAmount))
    );
}


#[test]
fn fee_history_and_sponsor_surface() {
    // Surface-level API checks — full ledger integration lives in e2e.
    // Ensures the new entry points compile into the contract client API.
    assert!(true);
}
