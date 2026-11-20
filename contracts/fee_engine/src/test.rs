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
//! [CONTRACT-18] Property-style tests for rounding, the fee bound, and the
//! high-water mark, using integer arithmetic only.
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

/// Deterministic xorshift PRNG so property-style tests are reproducible
/// without pulling in an external randomness crate.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Rng(seed.max(1))
    }
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    /// Uniform-ish value in `[lo, hi]` (inclusive).
    fn range(&mut self, lo: i128, hi: i128) -> i128 {
        let span = (hi - lo + 1) as u64;
        lo + (self.next() % span) as i128
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

/// Repeated small accruals must never total more than the defined annual rate
/// applied to the NAV over the elapsed period (the documented fee bound), no
/// matter how the period is split into steps.
#[test]
fn repeated_small_accruals_never_exceed_the_annual_bound() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    let cfg = config(&env, &treasury);
    let rate_bps = cfg.management_fee_bps as i128;
    client.set_fee_config(&admin, &cfg);

    let nav: i128 = 1_000_003; // deliberately not a round number
    env.ledger().with_mut(|l| l.timestamp = 0);
    client.record_deposit(&vault, &nav);

    let mut rng = Rng::new(0xC0FFEE);
    let mut now: i128 = 0;
    let mut total_charged: i128 = 0;
    // Many small, irregular steps covering exactly one year.
    while now < SECONDS_PER_YEAR {
        let step = rng.range(1, 3_600).min(SECONDS_PER_YEAR - now);
        now += step;
        env.ledger().with_mut(|l| l.timestamp = now as u64);
        total_charged += client.accrue_management_fee(&vault, &nav);
    }

    // Ceiling on the full-year charge: nav * rate / BPS, rounded up.
    let bound = (nav * rate_bps + BPS_DENOMINATOR - 1) / BPS_DENOMINATOR;
    assert!(
        total_charged <= bound,
        "charged {} exceeds annual bound {}",
        total_charged,
        bound
    );
    // And rounding loss should be bounded: at most one unit per step is lost,
    // so the total is never more than one unit below the exact annual fee.
    let exact_floor = nav * rate_bps / BPS_DENOMINATOR;
    assert!(total_charged + 1 >= exact_floor.min(bound));
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

/// Loss recovery: after a drawdown, no performance fee may be charged until the
/// NAV climbs back above the high-water mark. The mark must never move down.
#[test]
fn no_performance_fee_below_the_high_water_mark_during_recovery() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));
    client.record_deposit(&vault, &10_000);
    // Establish a mark of 12_000.
    client.crystallize(&vault, &12_000);
    let mark = client.get_vault(&vault).high_water_mark;
    assert_eq!(mark, 12_000);

    // Recovery path: NAV oscillates below the mark. No performance fee may be
    // charged on any step, and the mark must hold.
    let path: [i128; 6] = [11_000, 11_900, 11_500, 11_999, 11_200, 11_999];
    for nav in path {
        let before = client.get_vault(&vault).fees_paid;
        let fee = client.crystallize(&vault, &nav);
        let after = client.get_vault(&vault);
        // Any change in fees_paid while below the mark can only be a clawback
        // (which reduces it), never a positive performance charge.
        assert_eq!(fee, 0, "performance fee charged below mark at nav {}", nav);
        assert!(after.fees_paid <= before, "fees rose below mark at nav {}", nav);
        assert_eq!(after.high_water_mark, mark, "mark moved at nav {}", nav);
    }
}

/// Exact threshold crossing: a NAV equal to the mark is not a profit; one unit
/// above it is. The mark advances only on the crossing, never on the equality.
#[test]
fn exact_threshold_crossing_advances_the_mark_only_above() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));
    client.record_deposit(&vault, &10_000);

    // Exactly at the mark: no fee, mark stays.
    assert_eq!(client.crystallize(&vault, &10_000), 0);
    assert_eq!(client.get_vault(&vault).high_water_mark, 10_000);

    // One unit above: profit of 1 at 1_000 bps floors to 0 but the mark still
    // advances to the new NAV.
    client.crystallize(&vault, &10_001);
    assert_eq!(client.get_vault(&vault).high_water_mark, 10_001);
}

/// Property: across a random walk of NAVs, the high-water mark is monotonic
/// non-decreasing and no positive performance fee is ever charged below it.
#[test]
fn high_water_mark_is_monotonic_and_fees_respect_it_under_random_walk() {
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);
    let vault = Address::generate(&env);

    client.set_fee_config(&admin, &config(&env, &treasury));
    client.record_deposit(&vault, &10_000);

    let mut rng = Rng::new(0xBADC0DE);
    let mut nav: i128 = 10_000;
    let mut prev_mark = client.get_vault(&vault).high_water_mark;

    for _ in 0..500 {
        // Walk NAV by up to ±5% per step.
        let delta = rng.range(-500, 500);
        nav = (nav + nav * delta / BPS_DENOMINATOR).max(1);

        let before_mark = client.get_vault(&vault).high_water_mark;
        let fee = client.crystallize(&vault, &nav);
        let state = client.get_vault(&vault);

        // Mark never decreases.
        assert!(state.high_water_mark >= prev_mark);
        // If NAV did not exceed the prior mark, no performance fee was charged.
        if nav <= before_mark {
            assert_eq!(fee, 0);
        }
        // Mark advances only when NAV strictly exceeds it.
        if nav > before_mark {
            assert_eq!(state.high_water_mark, nav);
        } else {
            assert_eq!(state.high_water_mark, before_mark);
        }
        prev_mark = state.high_water_mark;
    }
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
}
