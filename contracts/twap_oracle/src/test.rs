#![cfg(test)]

use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

use crate::{Error, TwapOracle, TwapOracleClient};
use soroban_sdk::testutils::{Address as _, Ledger};
use soroban_sdk::{Address, Env};

fn setup_pair(e: &Env) -> (Address, Address) {
    let token_a = Address::generate(e);
    let token_b = Address::generate(e);
    (token_a, token_b)
}

#[test]
fn test_initialize() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    assert_eq!(client.initialize(&token_a, &token_b, &60), ());
    assert_eq!(client.get_tokens(), (token_a, token_b));
    assert_eq!(client.get_max_price_age(), 0);
}

#[test]
fn test_not_initialized() {
    let e = Env::default();
    let contract_id = e.register(TwapOracle, ());
    let token_a = Address::generate(&e);
    let token_b = Address::generate(&e);

    client.initialize(&token_a, &token_b, &60);

    // Try to initialize again (direct call so the Err is observable).
    let result = e.as_contract(&contract_id, || {
        TwapOracle::initialize(e.clone(), token_a.clone(), token_b.clone(), 60)
    });
    assert_eq!(result, Err(Error::AlreadyInitialized));

    assert_eq!(
        e.as_contract(&contract_id, || TwapOracle::update_price(e.clone(), 100)),
        Err(Error::NotInitialized)
    );
    assert_eq!(
        e.as_contract(&contract_id, || TwapOracle::set_max_price_age(e.clone(), 60)),
        Err(Error::NotInitialized)
    );
}

#[test]
fn test_invalid_price() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    client.initialize(&token_a, &token_b, &60);

    assert_eq!(
        e.as_contract(&contract_id, || TwapOracle::update_price(e.clone(), 0)),
        Err(Error::InvalidPrice)
    );
    assert_eq!(
        e.as_contract(&contract_id, || TwapOracle::update_price(e.clone(), -5)),
        Err(Error::InvalidPrice)
    );
}
    // First update
    client.update_price(&100);
    assert_eq!(client.get_twap(), 0); // No time elapsed yet

#[test]
fn test_update_and_get_twap() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    client.initialize(&token_a, &token_b, &1);
    e.ledger().with_mut(|li| li.timestamp = 1000);
    client.update_price(&90);
    // Second update
    client.update_price(&110);
    // TWAP = (100 * 10) / 10 = 100
    assert_eq!(client.get_twap(), 100);

    // The first update seeds the accumulator; later updates weight prior
    // prices over their elapsed window.
    e.ledger().with_mut(|li| li.timestamp = 1100);
    client.update_price(&100);
    e.ledger().with_mut(|li| li.timestamp = 1200);
    client.update_price(&200);

    // cumulative = 90*100 + 100*100 = 19000 over total time 200 => 95.
    assert_eq!(client.get_twap(), 95);
    assert_eq!(client.latest_price(), 95);
    // Third update
    client.update_price(&120);
    // Cumulative = 100*10 + 110*15 = 1000 + 1650 = 2650
    // Total time = 10 + 15 = 25
    // TWAP = 2650 / 25 = 106
    assert_eq!(client.get_twap(), 106);
}

#[test]
fn test_update_too_soon() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 1000);
    client.initialize(&token_a, &token_b, &60);
    client.update_price(&100);

    assert_eq!(
        e.as_contract(&contract_id, || TwapOracle::update_price(e.clone(), 110)),
        Err(Error::InsufficientTimeElapsed)
    );

    // Advance time by 50 seconds.
    e.ledger().with_mut(|li| li.timestamp = 1050);
    assert_eq!(
        e.as_contract(&contract_id, || TwapOracle::update_price(e.clone(), 110)),
        Err(Error::InsufficientTimeElapsed)
    );

    // Advance to 1060.
    // Advance to 1060
    e.ledger().with_mut(|li| li.timestamp = 1060);
    assert_eq!(
        e.as_contract(&contract_id, || TwapOracle::update_price(e.clone(), 110)),
        Ok(())
    );
}

#[test]
fn test_staleness_disabled_by_default() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 1000);
    client.initialize(&token_a, &token_b, &60);
    client.update_price(&100);
    e.ledger().with_mut(|li| li.timestamp = 1100);
    client.update_price(&100);

    // Even after a long time, with checks disabled the feed stays fresh and
    // keeps reporting its stable TWAP (100).
    e.ledger().with_mut(|li| li.timestamp = 1000 + 24 * 60 * 60);
    assert_eq!(client.is_price_stale(), false);
    assert_eq!(client.latest_price(), 100);
}

#[test]
fn test_stale_detection_after_max_age() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 1000);
    client.initialize(&token_a, &token_b, &60);
    client.update_price(&100);
    e.ledger().with_mut(|li| li.timestamp = 1100);
    client.update_price(&100);
    client.set_max_price_age(&300);

    // Fresh right at the boundary: 1100 + 300 = 1400 is not over the window.
    e.ledger().with_mut(|li| li.timestamp = 1400);
    assert_eq!(client.is_price_stale(), false);
    assert_eq!(client.latest_price(), 100);

    // Just over the window.
    e.ledger().with_mut(|li| li.timestamp = 1401);
    assert_eq!(client.is_price_stale(), true);
    assert_eq!(client.latest_price(), 0);

    // Updating the feed revives it; TWAP stays at 100.
    client.update_price(&110);
    assert_eq!(client.is_price_stale(), false);
    assert_eq!(client.latest_price(), 100);
}

#[test]
fn test_never_updated_feed_is_stale() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 1000);
    client.initialize(&token_a, &token_b, &60);
    client.set_max_price_age(&300);

    assert_eq!(client.is_price_stale(), true);
    assert_eq!(client.latest_price(), 0);
    let zero = || TwapOracle::update_price(e.clone(), 0);
    let negative = || TwapOracle::update_price(e.clone(), -1);
    assert_eq!(e.as_contract(&contract_id, zero), Err(Error::InvalidPrice));
    assert_eq!(e.as_contract(&contract_id, negative), Err(Error::InvalidPrice));
}

#[test]
fn test_set_max_price_age_toggles_checks() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 1000);
    client.initialize(&token_a, &token_b, &60);
    client.update_price(&100);

    assert_eq!(client.set_max_price_age(&10), ());
    assert_eq!(client.get_max_price_age(), 10);

    e.ledger().with_mut(|li| li.timestamp = 1010);
    assert_eq!(client.is_price_stale(), false);

    e.ledger().with_mut(|li| li.timestamp = 1011);
    assert_eq!(client.is_price_stale(), true);

    // Disabling the check reports a fresh feed again.
    client.set_max_price_age(&0);
    assert_eq!(client.is_price_stale(), false);
    let update = || TwapOracle::update_price(e.clone(), 100);
    assert_eq!(e.as_contract(&contract_id, update), Err(Error::NotInitialized));
    assert_eq!(client.get_twap(), 0);
}

#[test]
fn test_state_getter_reflects_updates() {
    let e = Env::default();
    e.ledger().with_mut(|li| li.timestamp = 5000);

    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    let token_a = Address::generate(&e);
    let token_b = Address::generate(&e);

    client.initialize(&token_a, &token_b, &10);
    client.update_price(&100);

    // Advance time and update again.
    e.ledger().with_mut(|li| li.timestamp = 5010);
    client.update_price(&110);

    let state = client.get_state();
    assert_eq!(state.last_price, 110);
    assert_eq!(state.last_update_timestamp, 5010);
    assert_eq!(state.cumulative_price, 100 * 10);
    assert_eq!(state.total_time, 10);
}
// ── #876 — TWAP oracle staleness and window boundaries ───────────────────────

/// A price submitted exactly at the minimum-window boundary (staleness limit)
/// is still considered fresh; one second beyond it is stale.
#[test]
fn test_staleness_exact_boundary() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 2000);
    client.initialize(&token_a, &token_b, &1);
    client.update_price(&100);

    // Set a staleness window of 500 seconds.
    client.set_max_price_age(&500);

    // At exactly 2000 + 500 = 2500 the feed is still within the window.
    e.ledger().with_mut(|li| li.timestamp = 2500);
    assert_eq!(client.is_price_stale(), false,
        "feed must be fresh exactly at the staleness boundary");

    // One second past the boundary → stale.
    e.ledger().with_mut(|li| li.timestamp = 2501);
    assert_eq!(client.is_price_stale(), true,
        "feed must be stale one second past the boundary");
}

/// When no price has ever been recorded the feed is considered stale as soon
/// as a non-zero max_age is configured, regardless of elapsed time.
#[test]
fn test_uninitialized_feed_is_always_stale_when_max_age_set() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 0);
    client.initialize(&token_a, &token_b, &1);
    client.set_max_price_age(&1_000_000);

    // No update has happened — feed must be stale even with a generous window.
    assert_eq!(client.is_price_stale(), true,
        "unrecorded feed must be stale when staleness checks are enabled");
    assert_eq!(client.latest_price(), 0,
        "latest_price must return 0 for a stale feed");
}

/// A minimum update interval of 0 means every price update is accepted regardless
/// of how little time has passed.
#[test]
fn test_min_interval_zero_accepts_immediate_updates() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 100);
    // min_update_interval = 0 → no cooldown required.
    client.initialize(&token_a, &token_b, &0);
    client.update_price(&50);

    // Update again at the same timestamp — must succeed.
    client.update_price(&60);

    // Both prices have been accepted; TWAP is 0 (no time has elapsed between them).
    let twap = client.get_twap();
    assert_eq!(twap, 0, "TWAP must be 0 when no time has elapsed between updates");
}

/// After a stale feed is refreshed with a new price the staleness flag clears.
#[test]
fn test_refreshed_feed_becomes_non_stale() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 1000);
    client.initialize(&token_a, &token_b, &10);
    client.update_price(&100);
    client.set_max_price_age(&200);

    // Push ledger past the staleness window.
    e.ledger().with_mut(|li| li.timestamp = 1201);
    assert_eq!(client.is_price_stale(), true, "feed must be stale past the window");

    // Refresh with a new price — must clear the stale flag.
    e.ledger().with_mut(|li| li.timestamp = 1220);
    client.update_price(&110);
    assert_eq!(client.is_price_stale(), false,
        "feed must be fresh after a new price update");
}

/// Disabling staleness checks (max_age = 0) renders the feed permanently fresh
/// even when no price has been recorded.
#[test]
fn test_disabled_staleness_always_fresh() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 0);
    client.initialize(&token_a, &token_b, &1);
    // max_age = 0 means disabled — no staleness checking.
    // Default is already 0 from initialize.
    assert_eq!(client.get_max_price_age(), 0);
    assert_eq!(client.is_price_stale(), false,
        "staleness checks disabled → feed always reported as fresh");
}

// ── #880 — Lending health-factor limits (oracle price boundary coverage) ──────

/// Submitting a valid price and querying it back must return the stored
/// price unchanged — the oracle provides the trusted price used for health
/// factor calculations in lending contracts.
#[test]
fn test_oracle_trusted_price_round_trip() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 5000);
    client.initialize(&token_a, &token_b, &1);
    client.update_price(&1_000);

    let state = client.get_state();
    assert_eq!(state.last_price, 1_000,
        "stored price must reflect the submitted collateral value");
    assert_eq!(state.last_update_timestamp, 5000);
}

/// Price of 1 is the minimum valid price boundary; 0 and negative values must
/// be rejected — mirrors a lending contract rejecting a zero or negative
/// collateral valuation which would produce an infinite or negative health factor.
#[test]
fn test_minimum_price_boundary_zero_rejected() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 100);
    client.initialize(&token_a, &token_b, &0);

    // Price = 1 is the minimum valid boundary — must succeed.
    client.update_price(&1);

    // Price = 0 must be rejected (would cause division by zero in health factor).
    assert_eq!(
        e.as_contract(&contract_id, || TwapOracle::update_price(e.clone(), 0)),
        Err(Error::InvalidPrice),
    );

    // Negative price must also be rejected.
    assert_eq!(
        e.as_contract(&contract_id, || TwapOracle::update_price(e.clone(), -100)),
        Err(Error::InvalidPrice),
    );
}

/// A stale oracle price (past staleness window) must yield 0 from latest_price,
/// signalling that the lending contract must not use the value and must refuse
/// any operations that would change the health factor.
#[test]
fn test_stale_oracle_price_unavailable_for_health_check() {
    let e = Env::default();
    let (token_a, token_b) = setup_pair(&e);
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    e.ledger().with_mut(|li| li.timestamp = 1000);
    client.initialize(&token_a, &token_b, &1);
    client.update_price(&500);
    client.set_max_price_age(&60);

    // Within the window the price is available.
    e.ledger().with_mut(|li| li.timestamp = 1060);
    assert_eq!(client.is_price_stale(), false);
    assert!(client.latest_price() > 0,
        "price must be available within the staleness window");

    // Past the window the price is unavailable — lending must reject it.
    e.ledger().with_mut(|li| li.timestamp = 1061);
    assert_eq!(client.is_price_stale(), true);
    assert_eq!(client.latest_price(), 0,
        "latest_price must return 0 when stale (unavailable for health checks)");
}
