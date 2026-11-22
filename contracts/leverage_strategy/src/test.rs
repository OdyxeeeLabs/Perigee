#![cfg(test)]

use super::*;
use soroban_sdk::{contract, contractimpl, contracttype, testutils::Address as _, Address, Env};

#[contract]
pub struct MockLeverageMarket;

#[contracttype]
#[derive(Clone)]
enum MockKey {
    Account(Address),
    NextLiquidationThreshold,
}

fn default_account() -> CreditAccount {
    CreditAccount {
        collateral: 0,
        debt: 0,
        net_asset_value: 0,
        ltv_bps: 0,
        liquidation_threshold_bps: 8_500,
    }
}

fn load_account(env: &Env, account: &Address) -> CreditAccount {
    env.storage()
        .persistent()
        .get(&MockKey::Account(account.clone()))
        .unwrap_or_else(default_account)
}

fn store_account(env: &Env, account: &Address, value: &CreditAccount) {
    env.storage()
        .persistent()
        .set(&MockKey::Account(account.clone()), value);
}

fn refresh(account: &mut CreditAccount) {
    account.net_asset_value = account.collateral - account.debt;
    account.ltv_bps = if account.collateral == 0 {
        0
    } else {
        account.debt * BPS_SCALE / account.collateral
    };
}

fn apply_next_threshold(env: &Env, account: &mut CreditAccount) {
    if let Some(threshold) = env
        .storage()
        .instance()
        .get::<_, i128>(&MockKey::NextLiquidationThreshold)
    {
        account.liquidation_threshold_bps = threshold;
        env.storage()
            .instance()
            .remove(&MockKey::NextLiquidationThreshold);
    }
}

#[contractimpl]
impl LeverageMarket for MockLeverageMarket {
    fn open_loop(
        e: Env,
        _collateral_asset: Address,
        collateral_amount: i128,
        _debt_asset: Address,
        borrow_amount: i128,
        account: Address,
    ) {
        let mut value = load_account(&e, &account);
        value.collateral += collateral_amount;
        value.debt += borrow_amount;
        apply_next_threshold(&e, &mut value);
        refresh(&mut value);
        store_account(&e, &account, &value);
    }

    fn deleverage(
        e: Env,
        _debt_asset: Address,
        repay_amount: i128,
        _collateral_asset: Address,
        withdraw_amount: i128,
        account: Address,
    ) {
        let mut value = load_account(&e, &account);
        value.debt = (value.debt - repay_amount).max(0);
        value.collateral = (value.collateral - withdraw_amount).max(0);
        refresh(&mut value);
        store_account(&e, &account, &value);
    }

    fn credit_account(e: Env, account: Address) -> CreditAccount {
        load_account(&e, &account)
    }
}

#[contractimpl]
impl MockLeverageMarket {
    pub fn set_account(env: Env, account: Address, value: CreditAccount) {
        store_account(&env, &account, &value);
    }

    pub fn set_next_liquidation_threshold(env: Env, threshold_bps: i128) {
        env.storage()
            .instance()
            .set(&MockKey::NextLiquidationThreshold, &threshold_bps);
    }
}

fn setup(protocol: Protocol) -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let market = env.register(MockLeverageMarket, ());
    let contract = env.register(LeverageStrategy, ());
    let client = LeverageStrategyClient::new(&env, &contract);
    client.initialize(
        &admin,
        &operator,
        &protocol,
        &market,
        &Address::generate(&env),
        &Address::generate(&env),
        &7_500,
    );
    (env, contract, market, admin)
}

fn strategy_client<'a>(env: &'a Env, contract: &'a Address) -> LeverageStrategyClient<'a> {
    LeverageStrategyClient::new(env, contract)
}

fn market_client<'a>(env: &'a Env, market: &'a Address) -> MockLeverageMarketClient<'a> {
    MockLeverageMarketClient::new(env, market)
}

#[test]
fn loop_opens_a_position_within_the_ltv_limit() {
    let (env, contract, _market, _admin) = setup(Protocol::Gearbox);
    let client = strategy_client(&env, &contract);

    let account = client.loop_position(&1_000, &500);
    assert_eq!(account.collateral, 1_000);
    assert_eq!(account.debt, 500);
    assert_eq!(account.net_asset_value, 500);
    assert_eq!(account.ltv_bps, 5_000);
    assert_eq!(client.protocol(), Protocol::Gearbox);
}

#[test]
fn loop_rejects_operations_that_breach_max_ltv() {
    let (env, contract, _market, _admin) = setup(Protocol::Gearbox);
    let client = strategy_client(&env, &contract);

    assert_eq!(
        client.try_loop_position(&1_000, &800),
        Err(Ok(Error::MaxLtvExceeded))
    );
    assert_eq!(client.credit_account().debt, 0);
}

#[test]
fn loop_rejects_operations_within_the_liquidation_threshold() {
    let (env, contract, market, _admin) = setup(Protocol::Notional);
    let client = strategy_client(&env, &contract);

    market_client(&env, &market).set_next_liquidation_threshold(&6_000);
    assert_eq!(
        client.try_loop_position(&1_000, &650),
        Err(Ok(Error::LiquidationRisk))
    );
}

#[test]
fn loop_rejects_non_positive_amounts() {
    let (env, contract, _market, _admin) = setup(Protocol::Gearbox);
    let client = strategy_client(&env, &contract);

    assert_eq!(
        client.try_loop_position(&0, &100),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        client.try_loop_position(&100, &-1),
        Err(Ok(Error::InvalidAmount))
    );
}

#[test]
fn deleverage_improves_the_ltv() {
    let (env, contract, market, _admin) = setup(Protocol::Gearbox);
    let client = strategy_client(&env, &contract);

    market_client(&env, &market).set_account(
        &contract,
        &CreditAccount {
            collateral: 1_000,
            debt: 700,
            net_asset_value: 300,
            ltv_bps: 7_000,
            liquidation_threshold_bps: 8_500,
        },
    );

    let account = client.deleverage(&200);
    assert_eq!(account.debt, 500);
    assert_eq!(account.ltv_bps, 5_000);
    assert_eq!(client.try_deleverage(&0), Err(Ok(Error::InvalidAmount)));
}

#[test]
fn withdrawing_collateral_cannot_push_the_position_over_the_ltv() {
    let (env, contract, market, _admin) = setup(Protocol::Gearbox);
    let client = strategy_client(&env, &contract);

    market_client(&env, &market).set_account(
        &contract,
        &CreditAccount {
            collateral: 1_000,
            debt: 700,
            net_asset_value: 300,
            ltv_bps: 7_000,
            liquidation_threshold_bps: 8_500,
        },
    );

    assert_eq!(
        client.try_withdraw_collateral(&100),
        Err(Ok(Error::MaxLtvExceeded))
    );
    assert_eq!(client.credit_account().collateral, 1_000);

    let account = client.withdraw_collateral(&50);
    assert_eq!(account.collateral, 950);
    assert_eq!(account.ltv_bps, 7_368);
}

#[test]
fn invalid_adapter_accounts_are_rejected() {
    let (env, contract, market, _admin) = setup(Protocol::Notional);
    let client = strategy_client(&env, &contract);

    market_client(&env, &market).set_account(
        &contract,
        &CreditAccount {
            collateral: 1_000,
            debt: 1_200,
            net_asset_value: 300,
            ltv_bps: 12_000,
            liquidation_threshold_bps: 8_500,
        },
    );

    assert_eq!(client.try_credit_account(), Err(Ok(Error::InvalidAccount)));
}

#[test]
fn insolvent_positions_are_rejected() {
    let (env, contract, market, _admin) = setup(Protocol::Notional);
    let client = strategy_client(&env, &contract);

    market_client(&env, &market).set_account(
        &contract,
        &CreditAccount {
            collateral: 1_000,
            debt: 1_200,
            net_asset_value: -200,
            ltv_bps: 8_000,
            liquidation_threshold_bps: 8_500,
        },
    );

    assert_eq!(client.try_credit_account(), Err(Ok(Error::Insolvent)));
}

#[test]
fn admin_controls_max_ltv_and_operator() {
    let (env, contract, _market, admin) = setup(Protocol::Gearbox);
    let client = strategy_client(&env, &contract);
    let new_operator = Address::generate(&env);

    assert_eq!(
        client.try_set_max_ltv_bps(&admin, &0),
        Err(Ok(Error::InvalidMaxLtv))
    );
    assert_eq!(
        client.try_set_max_ltv_bps(&admin, &BPS_SCALE),
        Err(Ok(Error::InvalidMaxLtv))
    );
    client.set_max_ltv_bps(&admin, &5_000);
    assert_eq!(client.max_ltv_bps(), 5_000);
    client.set_operator(&admin, &new_operator);
}

#[test]
fn initialize_validates_configuration() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let market = env.register(MockLeverageMarket, ());
    let collateral_asset = Address::generate(&env);
    let debt_asset = Address::generate(&env);
    let contract = env.register(LeverageStrategy, ());
    let client = LeverageStrategyClient::new(&env, &contract);

    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::Gearbox,
            &market,
            &collateral_asset,
            &collateral_asset,
            &7_500,
        ),
        Err(Ok(Error::InvalidConfiguration))
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::Gearbox,
            &market,
            &collateral_asset,
            &debt_asset,
            &0,
        ),
        Err(Ok(Error::InvalidMaxLtv))
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::Gearbox,
            &market,
            &collateral_asset,
            &debt_asset,
            &BPS_SCALE,
        ),
        Err(Ok(Error::InvalidMaxLtv))
    );

    client.initialize(
        &admin,
        &operator,
        &Protocol::Notional,
        &market,
        &collateral_asset,
        &debt_asset,
        &7_500,
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::Notional,
            &market,
            &collateral_asset,
            &debt_asset,
            &7_500,
        ),
        Err(Ok(Error::AlreadyInitialized))
    );
}
