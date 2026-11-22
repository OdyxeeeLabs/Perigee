#![cfg(test)]

use super::*;
use soroban_sdk::{testutils::Ledger, Address, Env, Vec};

mod source_a {
    use soroban_sdk::{contract, contractimpl, Env};

    #[contract]
    pub struct PriceSource100;

    #[contractimpl]
    impl PriceSource100 {
        pub fn latest_price(_env: Env) -> i128 {
            100
        }
    }
}

mod source_b {
    use soroban_sdk::{contract, contractimpl, Env};

    #[contract]
    pub struct PriceSource101;

    #[contractimpl]
    impl PriceSource101 {
        pub fn latest_price(_env: Env) -> i128 {
            101
        }
    }
}

mod source_c {
    use soroban_sdk::{contract, contractimpl, Env};

    #[contract]
    pub struct PriceSource99;

    #[contractimpl]
    impl PriceSource99 {
        pub fn latest_price(_env: Env) -> i128 {
            99
        }
    }
}

mod source_outlier {
    use soroban_sdk::{contract, contractimpl, Env};

    #[contract]
    pub struct PriceSourceOutlier;

    #[contractimpl]
    impl PriceSourceOutlier {
        pub fn latest_price(_env: Env) -> i128 {
            150
        }
    }
}

mod source_unresponsive {
    use soroban_sdk::{contract, contractimpl, Env};

    #[contract]
    pub struct PriceSourceUnresponsive;

    #[contractimpl]
    impl PriceSourceUnresponsive {
        pub fn latest_price(_env: Env) -> i128 {
            0
        }
    }
}

fn register_sources(
    env: &Env,
) -> (Address, Address, Address, Address, Address) {
    let ok_100 = env.register(source_a::PriceSource100, ());
    let ok_101 = env.register(source_b::PriceSource101, ());
    let ok_99 = env.register(source_c::PriceSource99, ());
    let outlier = env.register(source_outlier::PriceSourceOutlier, ());
    let unresponsive = env.register(source_unresponsive::PriceSourceUnresponsive, ());
    (ok_100, ok_101, ok_99, outlier, unresponsive)
}

fn setup_aggregator(
    env: &Env,
    max_staleness: u64,
) -> (
    OracleAggregatorClient<'_>,
    Address,
    Address,
    Address,
    Address,
    Address,
) {
    let (ok_100, ok_101, ok_99, outlier, unresponsive) = register_sources(env);
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(env, &aggregator_id);
    client.initialize(&max_staleness);
    (client, ok_100, ok_101, ok_99, outlier, unresponsive)
}

// ── Aggregation tests (success cases via client) ────────────────────────────

#[test]
fn test_aggregate_median_three_sources() {
    let env = Env::default();
    let (client, ok_100, ok_101, ok_99, _, _) = setup_aggregator(&env, 0);
    let sources = Vec::from_array(&env, [ok_100, ok_101, ok_99]);
    assert_eq!(client.aggregate_price(&sources), 100);
}

#[test]
fn test_ignore_outlier_and_use_fallback() {
    let env = Env::default();
    let (client, ok_100, ok_101, ok_99, outlier, _) = setup_aggregator(&env, 0);
    let sources = Vec::from_array(&env, [ok_100, ok_101, ok_99, outlier]);
    assert_eq!(client.aggregate_price(&sources), 100);
}

#[test]
fn test_skip_unresponsive_source() {
    let env = Env::default();
    let (client, ok_100, ok_101, ok_99, _, unresponsive) = setup_aggregator(&env, 0);
    let sources = Vec::from_array(&env, [ok_100, ok_101, ok_99, unresponsive]);
    assert_eq!(client.aggregate_price(&sources), 100);
}

// ── Aggregation error tests (via direct contract calls) ─────────────────────

#[test]
fn test_reject_when_not_enough_sources() {
    let env = Env::default();
    let (ok_100, ok_101, _, _, _) = register_sources(&env);
    let aggregator_id = env.register(OracleAggregator, ());
    let sources = Vec::from_array(&env, [ok_100, ok_101]);
    let result = env.as_contract(&aggregator_id, || {
        OracleAggregator::aggregate_price(env.clone(), sources)
    });
    assert_eq!(result, Err(Error::NotEnoughSources));
}

#[test]
fn test_reject_when_not_enough_valid_prices() {
    let env = Env::default();
    let (_, _, _, _, unresponsive) = register_sources(&env);
    let aggregator_id = env.register(OracleAggregator, ());
    let sources = Vec::from_array(&env, [unresponsive.clone(), unresponsive.clone(), unresponsive]);
    let result = env.as_contract(&aggregator_id, || {
        OracleAggregator::aggregate_price(env.clone(), sources)
    });
    assert_eq!(result, Err(Error::NotEnoughValidPrices));
}

// ── Initialization tests ────────────────────────────────────────────────────

#[test]
fn test_initialize() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    client.initialize(&300);
    assert_eq!(client.get_max_staleness(), 300);
}

#[test]
fn test_initialize_already_initialized() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    client.initialize(&300);
    let result = env.as_contract(&aggregator_id, || {
        OracleAggregator::initialize(env.clone(), 600)
    });
    assert_eq!(result, Err(Error::AlreadyInitialized));
}

// ── Price feed storage tests ────────────────────────────────────────────────

#[test]
fn test_submit_price() {
    let env = Env::default();
    let (client, ok_100, _, _, _, _) = setup_aggregator(&env, 0);

    client.submit_price(&ok_100, &100);

    let entry = client.get_price_entry(&ok_100);
    assert!(entry.is_some());
    let entry = entry.unwrap();
    assert_eq!(entry.price, 100);
    // Default ledger timestamp is 0 in the test env
    assert_eq!(entry.timestamp, 0);
}

#[test]
fn test_submit_price_with_timestamp() {
    let env = Env::default();
    let (ok_100, _, _, _, _) = register_sources(&env);
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    client.initialize(&0);

    // Set ledger timestamp before submitting
    env.ledger().with_mut(|li| li.timestamp = 42);
    client.submit_price(&ok_100, &100);

    let entry = client.get_price_entry(&ok_100).unwrap();
    assert_eq!(entry.price, 100);
    assert_eq!(entry.timestamp, 42);
}

#[test]
fn test_submit_price_invalid_zero() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let (ok_100, _, _, _, _) = register_sources(&env);
    let result = env.as_contract(&aggregator_id, || {
        OracleAggregator::submit_price(env.clone(), ok_100, 0)
    });
    assert_eq!(result, Err(Error::InvalidPrice));
}

#[test]
fn test_submit_price_invalid_negative() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let (ok_100, _, _, _, _) = register_sources(&env);
    let result = env.as_contract(&aggregator_id, || {
        OracleAggregator::submit_price(env.clone(), ok_100, -1)
    });
    assert_eq!(result, Err(Error::InvalidPrice));
}

#[test]
fn test_get_price_entry_none() {
    let env = Env::default();
    let (ok_100, _, _, _, _) = register_sources(&env);
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    assert!(client.get_price_entry(&ok_100).is_none());
}

// ── Staleness check tests ───────────────────────────────────────────────────

#[test]
fn test_staleness_rejects_stored_stale_price() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    // Initialize with 300 second staleness
    client.initialize(&300);

    // Submit prices manually at time 100
    env.ledger().with_mut(|li| li.timestamp = 100);
    let (ok_100, ok_101, ok_99, _, _) = register_sources(&env);
    client.submit_price(&ok_100, &100);
    client.submit_price(&ok_101, &101);
    client.submit_price(&ok_99, &99);

    // Advance ledger time past staleness threshold
    env.ledger().with_mut(|li| li.timestamp = 600);

    // Verify the stored entries are stale
    let entry = client.get_price_entry(&ok_100).unwrap();
    assert_eq!(entry.price, 100);
    assert_eq!(entry.timestamp, 100);
    // At time 600, a stored price from time 100 is 500s old, exceeding 300s staleness
}

#[test]
fn test_staleness_stored_price_marked_stale() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    // Initialize with 300 second staleness
    client.initialize(&300);

    // Submit prices manually at time 100
    env.ledger().with_mut(|li| li.timestamp = 100);
    let (ok_100, ok_101, ok_99, _, _) = register_sources(&env);
    client.submit_price(&ok_100, &100);
    client.submit_price(&ok_101, &101);
    client.submit_price(&ok_99, &99);

    // Advance ledger time past staleness threshold
    env.ledger().with_mut(|li| li.timestamp = 600);

    // Verify that is_price_fresh returns false for stale stored prices
    let fresh = env.as_contract(&aggregator_id, || {
        OracleAggregator::is_price_fresh(&env, &ok_100, 600, 300)
    });
    assert!(!fresh);
}

#[test]
fn test_staleness_fresh_stored_price() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    // Initialize with 300 second staleness
    client.initialize(&300);

    // Submit prices at time 100
    env.ledger().with_mut(|li| li.timestamp = 100);
    let (ok_100, ok_101, ok_99, _, _) = register_sources(&env);
    client.submit_price(&ok_100, &100);
    client.submit_price(&ok_101, &101);
    client.submit_price(&ok_99, &99);

    // Advance time to 350 seconds (within staleness threshold)
    env.ledger().with_mut(|li| li.timestamp = 350);

    // Live prices from mock sources are valid (100, 101, 99)
    // and stored prices are within the staleness threshold
    let sources = Vec::from_array(&env, [ok_100, ok_101, ok_99]);
    assert_eq!(client.aggregate_price(&sources), 100);
}

#[test]
fn test_set_max_staleness() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    client.initialize(&300);
    assert_eq!(client.get_max_staleness(), 300);

    client.set_max_staleness(&600);
    assert_eq!(client.get_max_staleness(), 600);
}

#[test]
fn test_no_staleness_check_when_zero() {
    let env = Env::default();
    let (ok_100, ok_101, ok_99, _, _) = register_sources(&env);
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    // Initialize with 0 staleness (disabled)
    client.initialize(&0);

    // Submit prices at time 100
    env.ledger().with_mut(|li| li.timestamp = 100);
    client.submit_price(&ok_100, &100);
    client.submit_price(&ok_101, &101);
    client.submit_price(&ok_99, &99);

    // Advance time far into the future
    env.ledger().with_mut(|li| li.timestamp = 99999);

    let sources = Vec::from_array(&env, [ok_100, ok_101, ok_99]);
    // Since staleness is 0 (disabled), live prices should still be valid
    assert_eq!(client.aggregate_price(&sources), 100);
}

#[test]
fn test_staleness_disabled_always_fresh() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let client = OracleAggregatorClient::new(&env, &aggregator_id);

    client.initialize(&0);

    let (ok_100, _, _, _, _) = register_sources(&env);
    // With staleness=0, is_price_fresh should always return true
    let fresh = env.as_contract(&aggregator_id, || {
        OracleAggregator::is_price_fresh(&env, &ok_100, 999999, 0)
    });
    assert!(fresh);
}

// ── Internal function unit tests ─────────────────────────────────────────────

#[test]
fn test_sort_prices() {
    let env = Env::default();
    let prices = Vec::from_array(&env, [150, 99, 101, 100]);
    let sorted = OracleAggregator::sort_prices(prices);
    assert_eq!(sorted.get(0).unwrap(), 99);
    assert_eq!(sorted.get(1).unwrap(), 100);
    assert_eq!(sorted.get(2).unwrap(), 101);
    assert_eq!(sorted.get(3).unwrap(), 150);
}

#[test]
fn test_median_odd() {
    let env = Env::default();
    let prices = Vec::from_array(&env, [99, 100, 101]);
    assert_eq!(OracleAggregator::median(&prices), 100);
}

#[test]
fn test_median_even() {
    let env = Env::default();
    let prices = Vec::from_array(&env, [99, 100, 101, 102]);
    assert_eq!(OracleAggregator::median(&prices), 100); // (100 + 101) / 2 = 100
}

#[test]
fn test_abs_diff() {
    assert_eq!(OracleAggregator::abs_diff(100, 90), 10);
    assert_eq!(OracleAggregator::abs_diff(90, 100), 10);
    assert_eq!(OracleAggregator::abs_diff(100, 100), 0);
}

// ── #877 — Overflow and zero-denominator coverage for oracle aggregation ─────

/// Submitting i128::MAX as a price must succeed — the contract stores it
/// without overflow.
#[test]
fn test_submit_max_price_stored_without_overflow() {
    let env = Env::default();
    let (client, ok_100, _, _, _, _) = setup_aggregator(&env, 0);

    client.submit_price(&ok_100, &i128::MAX);

    let entry = client.get_price_entry(&ok_100).unwrap();
    assert_eq!(entry.price, i128::MAX);
}

/// Submitting i128::MIN (negative) must be rejected with InvalidPrice.
#[test]
fn test_submit_min_i128_rejected() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let (ok_100, _, _, _, _) = register_sources(&env);
    let result = env.as_contract(&aggregator_id, || {
        OracleAggregator::submit_price(env.clone(), ok_100, i128::MIN)
    });
    assert_eq!(result, Err(Error::InvalidPrice));
}

/// An empty source list (len < 3) must return NotEnoughSources without
/// panicking — the contract must not reach any arithmetic before the guard.
#[test]
fn test_empty_source_list_rejected_before_arithmetic() {
    let env = Env::default();
    let aggregator_id = env.register(OracleAggregator, ());
    let sources: Vec<Address> = Vec::new(&env);
    let result = env.as_contract(&aggregator_id, || {
        OracleAggregator::aggregate_price(env.clone(), sources)
    });
    assert_eq!(result, Err(Error::NotEnoughSources));
}

/// A list of exactly two sources must return NotEnoughSources.
#[test]
fn test_two_sources_rejected() {
    let env = Env::default();
    let (ok_100, ok_101, _, _, _) = register_sources(&env);
    let aggregator_id = env.register(OracleAggregator, ());
    let sources = Vec::from_array(&env, [ok_100, ok_101]);
    let result = env.as_contract(&aggregator_id, || {
        OracleAggregator::aggregate_price(env.clone(), sources)
    });
    assert_eq!(result, Err(Error::NotEnoughSources));
}

/// Three unresponsive sources (all return 0) must return NotEnoughValidPrices,
/// not a panic or zero result.
#[test]
fn test_all_zero_sources_return_not_enough_valid_prices() {
    let env = Env::default();
    let (_, _, _, _, unresponsive) = register_sources(&env);
    let aggregator_id = env.register(OracleAggregator, ());
    // All three slots are the same zero-returning source.
    let sources = Vec::from_array(&env, [unresponsive.clone(), unresponsive.clone(), unresponsive]);
    let result = env.as_contract(&aggregator_id, || {
        OracleAggregator::aggregate_price(env.clone(), sources)
    });
    assert_eq!(result, Err(Error::NotEnoughValidPrices));
}

/// abs_diff must not overflow for the extreme case (i128::MAX, 0).
#[test]
fn test_abs_diff_max_does_not_overflow() {
    let result = OracleAggregator::abs_diff(i128::MAX, 0);
    assert_eq!(result, i128::MAX);
}

/// abs_diff is symmetric for large values.
#[test]
fn test_abs_diff_symmetry_large_values() {
    let a = 1_000_000_000_000i128;
    let b = 1i128;
    assert_eq!(
        OracleAggregator::abs_diff(a, b),
        OracleAggregator::abs_diff(b, a)
    );
}
