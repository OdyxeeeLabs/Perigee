#![cfg(test)]

//! Unit tests for the strategy analytics contract.
//!
//! Covers the two issue scopes:
//!
//! - [#657] risk framework — per-strategy caps, correlation-group caps, the
//!   weighted-risk ceiling and stress testing;
//! - [#658] performance tracking — APY annualisation, benchmark comparison and
//!   high-water-mark drawdown.
//!
//! Closes OdyxeeeLabs/Perigee#657
//! Closes OdyxeeeLabs/Perigee#658

use crate::{
    Error, Allocation, RiskLimits, StrategyAnalytics, StrategyAnalyticsClient, StressScenario,
    SECONDS_PER_YEAR,
};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

const LIMITS: RiskLimits = RiskLimits {
    max_portfolio_risk_score: 60,
    max_strategy_allocation_bps: 6_000,
    max_group_allocation_bps: 7_000,
    stress_tolerance_bps: 2_000,
};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let contract_id = env.register(StrategyAnalytics, ());
    let client = StrategyAnalyticsClient::new(&env, &contract_id);
    client.initialize(&admin, &LIMITS);

    (env, contract_id, admin)
}

fn register(
    env: &Env,
    client: &StrategyAnalyticsClient,
    admin: &Address,
    risk_score: u32,
    cap_bps: u32,
    group: u32,
) -> Address {
    let strategy = Address::generate(env);
    client.register_strategy(admin, &strategy, &risk_score, &cap_bps, &group);
    strategy
}

fn allocation(strategy: &Address, weight_bps: u32) -> Allocation {
    Allocation {
        strategy: strategy.clone(),
        weight_bps,
    }
}

// ── Setup ────────────────────────────────────────────────────────────────────

#[test]
fn initialize_records_the_admin_and_limits() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    assert_eq!(client.admin(), admin);
    assert_eq!(client.get_limits(), LIMITS);
    assert_eq!(client.strategy_count(), 0);
}

#[test]
fn initialize_can_only_run_once() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    assert_eq!(
        client.try_initialize(&admin, &LIMITS),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn an_over_scored_limit_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    let bad = RiskLimits {
        max_portfolio_risk_score: 101,
        ..LIMITS
    };
    assert_eq!(
        client.try_set_limits(&admin, &bad),
        Err(Ok(Error::InvalidRiskScore))
    );
}

// ── Risk profiles ────────────────────────────────────────────────────────────

#[test]
fn a_strategy_can_be_registered_and_read_back() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    let strategy = register(&env, &client, &admin, 30, 5_000, 1);

    let profile = client.get_risk_profile(&strategy);
    assert_eq!(profile.risk_score, 30);
    assert_eq!(profile.max_allocation_bps, 5_000);
    assert_eq!(profile.correlation_group, 1);
    assert_eq!(client.strategy_count(), 1);
}

#[test]
fn registering_the_same_strategy_twice_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    let strategy = register(&env, &client, &admin, 30, 5_000, 1);
    assert_eq!(
        client.try_register_strategy(&admin, &strategy, &20, &5_000, &1),
        Err(Ok(Error::StrategyAlreadyRegistered))
    );
}

#[test]
fn a_risk_score_above_the_scale_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);
    let strategy = Address::generate(&env);

    assert_eq!(
        client.try_register_strategy(&admin, &strategy, &101, &5_000, &1),
        Err(Ok(Error::InvalidRiskScore))
    );
}

#[test]
fn a_profile_can_be_updated() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);
    let strategy = register(&env, &client, &admin, 30, 5_000, 1);

    client.update_risk_profile(&admin, &strategy, &70, &3_000, &2);

    let profile = client.get_risk_profile(&strategy);
    assert_eq!(profile.risk_score, 70);
    assert_eq!(profile.max_allocation_bps, 3_000);
    assert_eq!(profile.correlation_group, 2);
}

// ── Allocation validation (#657) ─────────────────────────────────────────────

#[test]
fn a_balanced_book_passes_and_reports_its_risk_score() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    let a = register(&env, &client, &admin, 20, 6_000, 1);
    let b = register(&env, &client, &admin, 40, 6_000, 2);

    let allocations = vec![&env, allocation(&a, 5_000), allocation(&b, 5_000)];
    let report = client.validate_allocation(&allocations);

    // (5_000*20 + 5_000*40) / 10_000 = 30
    assert_eq!(report.portfolio_risk_score, 30);
    assert_eq!(report.largest_position_bps, 5_000);
    assert_eq!(report.strategy_count, 2);
}

#[test]
fn a_position_above_its_cap_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    let a = register(&env, &client, &admin, 20, 6_000, 1);
    let b = register(&env, &client, &admin, 40, 6_000, 2);

    let allocations = vec![&env, allocation(&a, 7_000), allocation(&b, 3_000)];
    assert_eq!(
        client.try_validate_allocation(&allocations),
        Err(Ok(Error::AllocationExceedsLimit))
    );
}

#[test]
fn a_concentrated_correlation_group_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    // Two strategies in the same group; the group cap is 7_000 bps.
    let a = register(&env, &client, &admin, 20, 6_000, 7);
    let b = register(&env, &client, &admin, 20, 6_000, 7);

    let allocations = vec![&env, allocation(&a, 5_000), allocation(&b, 4_000)];
    assert_eq!(
        client.try_validate_allocation(&allocations),
        Err(Ok(Error::GroupExceedsLimit))
    );
}

#[test]
fn a_weighted_risk_above_the_ceiling_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    // Both score 90; the weighted average can only reach 60 if capped, so this
    // book is rejected on risk even though every per-strategy cap holds.
    let a = register(&env, &client, &admin, 90, 6_000, 1);
    let b = register(&env, &client, &admin, 90, 6_000, 2);

    let allocations = vec![&env, allocation(&a, 5_000), allocation(&b, 5_000)];
    assert_eq!(
        client.try_validate_allocation(&allocations),
        Err(Ok(Error::PortfolioRiskTooHigh))
    );
}

#[test]
fn weights_that_do_not_sum_to_one_hundred_percent_are_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    let a = register(&env, &client, &admin, 10, 6_000, 1);
    let allocations = vec![&env, allocation(&a, 9_000)];
    assert_eq!(
        client.try_validate_allocation(&allocations),
        Err(Ok(Error::InvalidAllocation))
    );
}

// ── Stress testing (#657) ────────────────────────────────────────────────────

#[test]
fn stress_test_reports_the_loss_and_the_tolerance() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    let a = register(&env, &client, &admin, 20, 6_000, 1);
    let b = register(&env, &client, &admin, 20, 6_000, 2);

    let allocations = vec![&env, allocation(&a, 5_000), allocation(&b, 5_000)];
    let scenarios = vec![
        &env,
        StressScenario {
            strategy: a.clone(),
            shock_bps: 5_000,
        },
        StressScenario {
            strategy: b.clone(),
            shock_bps: 1_000,
        },
    ];

    let result = client.stress_test(&allocations, &scenarios);
    // (5_000*5_000 + 5_000*1_000) / 10_000 = 3_000 bps
    assert_eq!(result.loss_bps, 3_000);
    assert_eq!(result.tolerance_bps, LIMITS.stress_tolerance_bps);
    assert!(result.breached);
}

#[test]
fn stress_test_can_pass_within_tolerance() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    let a = register(&env, &client, &admin, 20, 6_000, 1);
    let allocations = vec![&env, allocation(&a, 10_000)];
    let scenarios = vec![
        &env,
        StressScenario {
            strategy: a.clone(),
            shock_bps: 1_000,
        },
    ];

    let result = client.stress_test(&allocations, &scenarios);
    assert_eq!(result.loss_bps, 1_000);
    assert!(!result.breached);
}

#[test]
fn an_impossible_shock_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);

    let a = register(&env, &client, &admin, 20, 6_000, 1);
    let allocations = vec![&env, allocation(&a, 10_000)];
    let scenarios = vec![
        &env,
        StressScenario {
            strategy: a.clone(),
            shock_bps: 10_001,
        },
    ];

    assert_eq!(
        client.try_stress_test(&allocations, &scenarios),
        Err(Ok(Error::InvalidScenario))
    );
}

// ── Performance tracking (#658) ──────────────────────────────────────────────

#[test]
fn fewer_than_two_samples_cannot_report_performance() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);
    let a = register(&env, &client, &admin, 20, 6_000, 1);

    client.record_snapshot(&admin, &a, &1_000);
    assert_eq!(
        client.try_performance(&a),
        Err(Ok(Error::NotEnoughSamples))
    );
}

#[test]
fn apy_is_annualised_and_compared_against_the_benchmark() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);
    let a = register(&env, &client, &admin, 20, 6_000, 1);

    // Strategy: 1_000 → 1_100 over exactly one year = 10% = 1_000 bps.
    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 0;
    });
    client.record_snapshot(&admin, &a, &1_000);
    client.record_benchmark(&admin, &1_000);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = SECONDS_PER_YEAR;
    });
    client.record_snapshot(&admin, &a, &1_100);
    client.record_benchmark(&admin, &1_050);

    let report = client.performance(&a);
    assert_eq!(report.apy_bps, 1_000);
    assert_eq!(report.benchmark_apy_bps, 500);
    assert_eq!(report.excess_bps, 500);
    assert_eq!(report.samples, 2);
    assert_eq!(client.benchmark_apy(), 500);
}

#[test]
fn a_losing_strategy_reports_a_negative_apy() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);
    let a = register(&env, &client, &admin, 20, 6_000, 1);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 0;
    });
    client.record_snapshot(&admin, &a, &1_000);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = SECONDS_PER_YEAR;
    });
    client.record_snapshot(&admin, &a, &900);

    let report = client.performance(&a);
    assert_eq!(report.apy_bps, -1_000);
}

#[test]
fn drawdown_is_tracked_from_the_high_water_mark() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);
    let a = register(&env, &client, &admin, 20, 6_000, 1);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 0;
    });
    client.record_snapshot(&admin, &a, &1_000);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 100;
    });
    client.record_snapshot(&admin, &a, &1_200);

    env.ledger().with_mut(|ledger| {
        ledger.timestamp = 600;
    });
    let series = client.record_snapshot(&admin, &a, &900);

    assert_eq!(series.high_water_mark, 1_200);
    // (1_200 - 900) / 1_200 = 25% = 2_500 bps
    assert_eq!(series.max_drawdown_bps, 2_500);

    let report = client.performance(&a);
    assert_eq!(report.current_drawdown_bps, 2_500);
    assert_eq!(report.max_drawdown_bps, 2_500);
}

#[test]
fn a_non_positive_snapshot_is_refused() {
    let (env, contract_id, admin) = setup();
    let client = StrategyAnalyticsClient::new(&env, &contract_id);
    let a = register(&env, &client, &admin, 20, 6_000, 1);

    assert_eq!(
        client.try_record_snapshot(&admin, &a, &0),
        Err(Ok(Error::InvalidAmount))
    );
}
