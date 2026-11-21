#![cfg(test)]

use super::*;
use soroban_sdk::{contract, contractimpl, contracttype, testutils::Address as _, Address, Env};

#[contract]
pub struct MockLendingMarket;

#[contracttype]
#[derive(Clone)]
enum MockKey {
    Position(Address),
    NextHealthFactor,
}

fn empty_position() -> LendingPosition {
    LendingPosition {
        supplied: 0,
        borrowed: 0,
        net_asset_value: 0,
        health_factor: i128::MAX,
    }
}

fn update_position(env: &Env, account: &Address, mutate: impl FnOnce(&mut LendingPosition)) {
    let mut position: LendingPosition = env
        .storage()
        .persistent()
        .get(&MockKey::Position(account.clone()))
        .unwrap_or_else(empty_position);
    mutate(&mut position);
    if let Some(next_health_factor) = env
        .storage()
        .instance()
        .get::<_, i128>(&MockKey::NextHealthFactor)
    {
        position.health_factor = next_health_factor;
        env.storage()
            .instance()
            .remove(&MockKey::NextHealthFactor);
    }
    env.storage()
        .persistent()
        .set(&MockKey::Position(account.clone()), &position);
}

#[contractimpl]
impl LendingMarket for MockLendingMarket {
    fn supply(e: Env, _asset: Address, amount: i128, account: Address) {
        update_position(&e, &account, |position| {
            position.supplied += amount;
            position.net_asset_value += amount;
        });
    }

    fn withdraw(e: Env, _asset: Address, amount: i128, account: Address) {
        update_position(&e, &account, |position| {
            position.supplied -= amount;
            position.net_asset_value -= amount;
        });
    }

    fn borrow(e: Env, _asset: Address, amount: i128, account: Address) {
        update_position(&e, &account, |position| {
            position.borrowed += amount;
        });
    }

    fn repay(e: Env, _asset: Address, amount: i128, account: Address) -> i128 {
        let mut repaid = 0;
        update_position(&e, &account, |position| {
            repaid = amount.min(position.borrowed);
            position.borrowed -= repaid;
        });
        repaid
    }

    fn position(e: Env, account: Address) -> LendingPosition {
        e.storage()
            .persistent()
            .get(&MockKey::Position(account))
            .unwrap_or_else(empty_position)
    }
}

#[contractimpl]
impl MockLendingMarket {
    pub fn set_next_health_factor(env: Env, health_factor: i128) {
        env.storage()
            .instance()
            .set(&MockKey::NextHealthFactor, &health_factor);
    }

    pub fn set_position(env: Env, account: Address, position: LendingPosition) {
        env.storage()
            .persistent()
            .set(&MockKey::Position(account), &position);
    }
}

fn setup(protocol: Protocol) -> (Env, Address, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let market = env.register(MockLendingMarket, ());
    let contract = env.register(LendingStrategy, ());
    let client = LendingStrategyClient::new(&env, &contract);
    client.initialize(
        &admin,
        &operator,
        &protocol,
        &market,
        &Address::generate(&env),
        &Address::generate(&env),
        &(HEALTH_FACTOR_SCALE * 3 / 2),
    );
    (env, contract, market, operator, admin)
}

fn strategy_client<'a>(env: &'a Env, contract: &'a Address) -> LendingStrategyClient<'a> {
    LendingStrategyClient::new(env, contract)
}

fn market_client<'a>(env: &'a Env, market: &'a Address) -> MockLendingMarketClient<'a> {
    MockLendingMarketClient::new(env, market)
}

#[test]
fn supply_and_borrow_flow_updates_position() {
    let (env, contract, market, _operator, _admin) = setup(Protocol::AaveV3);
    let client = strategy_client(&env, &contract);

    let supplied = client.supply(&1_000);
    assert_eq!(supplied.supplied, 1_000);
    assert_eq!(client.protocol(), Protocol::AaveV3);

    market_client(&env, &market).set_next_health_factor(&(HEALTH_FACTOR_SCALE * 2));
    let borrowed = client.borrow(&250);
    assert_eq!(borrowed.borrowed, 250);
    assert_eq!(borrowed.health_factor, HEALTH_FACTOR_SCALE * 2);
}

#[test]
fn borrow_reverts_when_post_borrow_health_factor_is_below_minimum() {
    let (env, contract, market, _operator, _admin) = setup(Protocol::CompoundV3);
    let client = strategy_client(&env, &contract);
    client.supply(&1_000);

    market_client(&env, &market).set_next_health_factor(&(HEALTH_FACTOR_SCALE * 14 / 10));
    assert_eq!(
        client.try_borrow(&100),
        Err(Ok(Error::HealthFactorTooLow))
    );
    assert_eq!(client.position().borrowed, 0);
}

#[test]
fn borrow_requires_collateral() {
    let (env, contract, _market, _operator, _admin) = setup(Protocol::AaveV3);
    let client = strategy_client(&env, &contract);

    assert_eq!(client.try_borrow(&100), Err(Ok(Error::NoCollateral)));
}

#[test]
fn withdrawal_is_rejected_if_it_would_leave_an_unsafe_position() {
    let (env, contract, market, _operator, _admin) = setup(Protocol::AaveV3);
    let client = strategy_client(&env, &contract);
    client.supply(&1_000);
    client.borrow(&100);

    market_client(&env, &market).set_next_health_factor(&(HEALTH_FACTOR_SCALE * 12 / 10));
    assert_eq!(
        client.try_withdraw(&100),
        Err(Ok(Error::HealthFactorTooLow))
    );
    assert_eq!(client.position().supplied, 1_000);
}

#[test]
fn supply_can_restore_a_position_below_the_health_threshold() {
    let (env, contract, market, _operator, _admin) = setup(Protocol::AaveV3);
    let client = strategy_client(&env, &contract);
    market_client(&env, &market).set_position(
        &contract,
        &LendingPosition {
            supplied: 1_000,
            borrowed: 500,
            net_asset_value: 500,
            health_factor: HEALTH_FACTOR_SCALE * 12 / 10,
        },
    );

    let position = client.supply(&100);
    assert_eq!(position.supplied, 1_100);
    assert_eq!(position.health_factor, HEALTH_FACTOR_SCALE * 12 / 10);
}

#[test]
fn repay_caps_to_outstanding_debt_and_rejects_bad_amounts() {
    let (env, contract, _market, _operator, _admin) = setup(Protocol::CompoundV3);
    let client = strategy_client(&env, &contract);
    client.supply(&1_000);
    client.borrow(&300);

    let (repaid, position) = client.repay(&500);
    assert_eq!(repaid, 300);
    assert_eq!(position.borrowed, 0);
    assert_eq!(
        client.try_repay(&0),
        Err(Ok(Error::InvalidAmount))
    );
}

#[test]
fn admin_controls_threshold_and_operator() {
    let (env, contract, _market, _operator, admin) = setup(Protocol::AaveV3);
    let client = strategy_client(&env, &contract);
    let new_operator = Address::generate(&env);

    assert_eq!(
        client.try_set_min_health_factor(&admin, &HEALTH_FACTOR_SCALE),
        Err(Ok(Error::InvalidHealthFactor))
    );
    client.set_min_health_factor(&admin, &(HEALTH_FACTOR_SCALE * 2));
    client.set_operator(&admin, &new_operator);
}
