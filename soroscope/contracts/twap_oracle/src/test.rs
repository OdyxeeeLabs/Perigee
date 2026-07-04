#![cfg(test)]
use soroban_sdk::{testutils::Address as _, Address, Env};

use crate::{Error, TwapOracle, TwapOracleClient};

#[test]
fn test_initialize() {
    let e = Env::default();
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    let token_a = Address::generate(&e);
    let token_b = Address::generate(&e);

    assert_eq!(client.initialize(&token_a, &token_b, &60), Ok(()));

    // Try to initialize again
    assert_eq!(client.initialize(&token_a, &token_b, &60), Err(Error::AlreadyInitialized));

    let (a, b) = client.get_tokens();
    assert_eq!(a, token_a);
    assert_eq!(b, token_b);
}

#[test]
fn test_update_and_get_twap() {
    let e = Env::default();
    e.ledger().with_mut(|li| li.timestamp = 1000);

    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    let token_a = Address::generate(&e);
    let token_b = Address::generate(&e);

    client.initialize(&token_a, &token_b, &10);

    // First update
    assert_eq!(client.update_price(&100), Ok(()));
    assert_eq!(client.get_twap(), 0); // No time elapsed yet

    // Advance time
    e.ledger().with_mut(|li| li.timestamp = 1010);

    // Second update
    assert_eq!(client.update_price(&110), Ok(()));
    // TWAP = (100 * 10) / 10 = 100
    assert_eq!(client.get_twap(), 100);

    // Advance time again
    e.ledger().with_mut(|li| li.timestamp = 1025);

    // Third update
    assert_eq!(client.update_price(&120), Ok(()));
    // Cumulative = 100*10 + 110*15 = 1000 + 1650 = 2650
    // Total time = 10 + 15 = 25
    // TWAP = 2650 / 25 = 106
    assert_eq!(client.get_twap(), 106);
}

#[test]
fn test_update_too_soon() {
    let e = Env::default();
    e.ledger().with_mut(|li| li.timestamp = 1000);

    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    let token_a = Address::generate(&e);
    let token_b = Address::generate(&e);

    client.initialize(&token_a, &token_b, &60);

    client.update_price(&100);

    // Try update immediately
    assert_eq!(client.update_price(&110), Err(Error::InsufficientTimeElapsed));

    // Advance time by 50 seconds
    e.ledger().with_mut(|li| li.timestamp = 1050);

    // Still not enough
    assert_eq!(client.update_price(&110), Err(Error::InsufficientTimeElapsed));

    // Advance to 1060
    e.ledger().with_mut(|li| li.timestamp = 1060);

    assert_eq!(client.update_price(&110), Ok(()));
}

#[test]
fn test_invalid_price() {
    let e = Env::default();
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    let token_a = Address::generate(&e);
    let token_b = Address::generate(&e);

    client.initialize(&token_a, &token_b, &60);

    assert_eq!(client.update_price(&0), Err(Error::InvalidPrice));
    assert_eq!(client.update_price(&-1), Err(Error::InvalidPrice));
}

#[test]
fn test_not_initialized() {
    let e = Env::default();
    let contract_id = e.register(TwapOracle, ());
    let client = TwapOracleClient::new(&e, &contract_id);

    assert_eq!(client.update_price(&100), Err(Error::NotInitialized));
    assert_eq!(client.get_twap(), 0);
}
