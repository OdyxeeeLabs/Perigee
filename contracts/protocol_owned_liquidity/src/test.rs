#![cfg(test)]

use crate::{
    quote_bond_tokens, Error, PolConfig, ProtocolOwnedLiquidity, ProtocolOwnedLiquidityClient,
};
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env,
};

fn setup() -> (Env, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(ProtocolOwnedLiquidity, ());
    let client = ProtocolOwnedLiquidityClient::new(&env, &id);
    client.initialize(
        &admin,
        &PolConfig {
            governance_token: Address::generate(&env),
            treasury: Address::generate(&env),
            discount_bps: 500, // 5%
            vesting_ledgers: 10,
            max_bond_liability: 1_000_000,
        },
    );
    (env, id, admin)
}

#[test]
fn bond_pricing_applies_discount() {
    // deposit 10_000 value, price 10_000 (1:1), 5% discount → 10_500 tokens
    let tokens = quote_bond_tokens(10_000, 10_000, 500).unwrap();
    assert_eq!(tokens, 10_500);
}

#[test]
fn bond_and_claim_after_vesting() {
    let (env, id, _) = setup();
    let client = ProtocolOwnedLiquidityClient::new(&env, &id);
    let bonder = Address::generate(&env);

    let (bond_id, tokens) = client.bond(&bonder, &10_000, &10_000);
    assert_eq!(tokens, 10_500);
    assert_eq!(client.get_state().reserves, 10_000);
    assert_eq!(client.get_state().bond_liability, 10_500);

    assert_eq!(
        client.try_claim_bond(&bond_id, &bonder),
        Err(Ok(Error::BondNotMature))
    );

    env.ledger().with_mut(|l| l.sequence_number += 10);
    let claimed = client.claim_bond(&bond_id, &bonder);
    assert_eq!(claimed, 10_500);
    assert_eq!(client.get_state().bond_liability, 0);
}

#[test]
fn deploy_pol_and_capture_yield() {
    let (env, id, admin) = setup();
    let client = ProtocolOwnedLiquidityClient::new(&env, &id);
    let bonder = Address::generate(&env);
    client.bond(&bonder, &50_000, &10_000);

    let state = client.deploy_pol(&admin, &20_000, &100);
    assert_eq!(state.reserves, 30_000);
    assert_eq!(state.deployed_lp, 100);

    client.capture_yield(&admin, &1_500);
    let swept = client.sweep_yield(&admin);
    assert_eq!(swept, 1_500);
    assert_eq!(client.get_state().yield_pending, 0);
    assert_eq!(client.get_state().reserves, 31_500);
    assert_eq!(client.get_state().yield_captured, 1_500);
}

#[test]
fn cannot_deploy_more_than_reserves() {
    let (env, id, admin) = setup();
    let client = ProtocolOwnedLiquidityClient::new(&env, &id);
    assert_eq!(
        client.try_deploy_pol(&admin, &1, &1),
        Err(Ok(Error::InsufficientReserves))
    );
}

// --- Quote vs settlement (#869) ---

macro_rules! pol {
    () => {{
        let (env, id, admin) = setup();
        env.ledger().with_mut(|l| l.sequence_number = 1_000);
        let client = ProtocolOwnedLiquidityClient::new(&env, &id);
        (env, client, admin)
    }};
}

#[test]
fn quote_matches_settlement_for_a_grid_of_inputs() {
    let (env, client, _admin) = pol!();
    let bonder = Address::generate(&env);
    for (deposit, price) in [
        (1i128, 10_000i128),
        (3, 10_000),
        (10_000, 10_000),
        (10_001, 9_999),
        (123_457, 31_337),
        (99_999, 10_001),
    ] {
        let quoted = client.quote_bond(&deposit, &price);
        let (_, settled) = client.bond(&bonder, &deposit, &price);
        assert_eq!(quoted, settled, "deposit {deposit} price {price}");
        // The pure helper and the contract agree.
        assert_eq!(quoted, quote_bond_tokens(deposit, price, 500).unwrap());
    }
}

#[test]
fn rounding_always_floors_and_never_mints_extra() {
    // deposit 7 at price 10_000 with 5% discount: boosted = 7 * 10_500 / 10_000
    // = 7 (7.35 floored), tokens = 7 * 10_000 / 10_000 = 7. The exact value is
    // 7.35, so the floor is strictly less than the true amount.
    assert_eq!(quote_bond_tokens(7, 10_000, 500).unwrap(), 7);
    // Never more than the exact rational value deposit*(1+d)/price*1e4.
    for deposit in 1..200i128 {
        for price in [3_333i128, 10_000, 12_347, 99_999] {
            let tokens = quote_bond_tokens(deposit, price, 500).unwrap();
            // exact_num / exact_den >= tokens, compared without division.
            assert!(tokens * price * 10_000 <= deposit * 10_500 * 10_000);
        }
    }
}

#[test]
fn dust_deposit_that_buys_nothing_is_rejected_without_state_change() {
    let (env, client, _admin) = pol!();
    let bonder = Address::generate(&env);
    let before = client.get_state();
    // 1 unit at price 20_000 floors to zero tokens.
    assert_eq!(client.quote_bond(&1, &20_000), 0);
    assert_eq!(
        client.try_bond(&bonder, &1, &20_000),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(client.get_state(), before);
    assert_eq!(client.try_get_bond(&1), Err(Ok(Error::BondNotFound)));
}

#[test]
fn non_positive_inputs_are_rejected() {
    let (env, client, _admin) = pol!();
    let bonder = Address::generate(&env);
    assert_eq!(client.try_bond(&bonder, &0, &10_000), Err(Ok(Error::InvalidAmount)));
    assert_eq!(client.try_bond(&bonder, &-5, &10_000), Err(Ok(Error::InvalidAmount)));
    assert_eq!(client.try_bond(&bonder, &1_000, &0), Err(Ok(Error::InvalidAmount)));
    assert_eq!(client.get_state().next_bond_id, 1);
}

#[test]
fn quote_is_valid_through_its_expiry_ledger_and_fails_after() {
    let (env, client, _admin) = pol!();
    let bonder = Address::generate(&env);
    let expiry = env.ledger().sequence() + 5;

    // Still valid on the expiry ledger itself.
    env.ledger().with_mut(|l| l.sequence_number = expiry);
    let (_, tokens) = client.bond_with_quote(&bonder, &10_000, &10_000, &10_500, &expiry);
    assert_eq!(tokens, 10_500);

    // One ledger later the same quote is dead and nothing changes.
    env.ledger().with_mut(|l| l.sequence_number = expiry + 1);
    let before = client.get_state();
    assert_eq!(
        client.try_bond_with_quote(&bonder, &10_000, &10_000, &10_500, &expiry),
        Err(Ok(Error::QuoteExpired))
    );
    assert_eq!(client.get_state(), before);
}

#[test]
fn slippage_bound_is_inclusive_and_rejection_leaves_no_state() {
    let (env, client, _admin) = pol!();
    let bonder = Address::generate(&env);
    let expiry = env.ledger().sequence() + 100;
    let before = client.get_state();

    // Asking for one token more than the settlement would give fails...
    assert_eq!(
        client.try_bond_with_quote(&bonder, &10_000, &10_000, &10_501, &expiry),
        Err(Ok(Error::SlippageExceeded))
    );
    assert_eq!(client.get_state(), before);

    // ...asking for exactly the settlement amount succeeds.
    let (_, tokens) = client.bond_with_quote(&bonder, &10_000, &10_000, &10_500, &expiry);
    assert_eq!(tokens, 10_500);
}

#[test]
fn settlement_cannot_exceed_the_quoted_bound_when_price_moves() {
    // The caller quotes at one price and settles after the price rose: the
    // settled amount shrinks, so a bound set from the earlier quote is violated.
    let (env, client, _admin) = pol!();
    let bonder = Address::generate(&env);
    let expiry = env.ledger().sequence() + 100;
    let quoted = client.quote_bond(&10_000, &10_000);
    assert_eq!(
        client.try_bond_with_quote(&bonder, &10_000, &11_000, &quoted, &expiry),
        Err(Ok(Error::SlippageExceeded))
    );
}

#[test]
fn liability_cap_rejection_leaves_state_untouched() {
    let (env, client, _admin) = pol!();
    let bonder = Address::generate(&env);
    // Cap is 1_000_000 tokens; this bond would mint far more.
    let before = client.get_state();
    assert_eq!(
        client.try_bond(&bonder, &2_000_000, &10_000),
        Err(Ok(Error::InsufficientReserves))
    );
    assert_eq!(client.get_state(), before);
}

#[test]
fn bond_maturity_boundary_is_inclusive() {
    let (env, client, _admin) = pol!();
    let bonder = Address::generate(&env);
    let (id, _) = client.bond(&bonder, &10_000, &10_000);
    let mature = client.get_bond(&id).mature_ledger;

    env.ledger().with_mut(|l| l.sequence_number = mature - 1);
    assert_eq!(client.try_claim_bond(&id, &bonder), Err(Ok(Error::BondNotMature)));
    env.ledger().with_mut(|l| l.sequence_number = mature);
    assert_eq!(client.claim_bond(&id, &bonder), 10_500);
    assert_eq!(client.try_claim_bond(&id, &bonder), Err(Ok(Error::AlreadyClaimed)));
}
