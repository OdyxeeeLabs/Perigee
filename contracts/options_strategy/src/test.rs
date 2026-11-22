#![cfg(test)]

use super::*;
use soroban_sdk::{contract, contractimpl, contracttype, testutils::Address as _, Address, Env};

#[contract]
pub struct MockOptionsMarket;

#[contracttype]
#[derive(Clone)]
enum MockKey {
    Account(Address),
    NextDelta,
    NextPnl,
}

fn default_account() -> OptionsAccount {
    OptionsAccount {
        underlying: 0,
        short_calls: 0,
        long_puts: 0,
        net_delta_bps: 0,
    }
}

fn load_account(env: &Env, account: &Address) -> OptionsAccount {
    env.storage()
        .persistent()
        .get(&MockKey::Account(account.clone()))
        .unwrap_or_else(default_account)
}

fn store_account(env: &Env, account: &Address, value: &OptionsAccount) {
    env.storage()
        .persistent()
        .set(&MockKey::Account(account.clone()), value);
}

fn take_next(env: &Env, key: &MockKey) -> Option<i128> {
    let value = env.storage().instance().get::<_, i128>(key);
    if value.is_some() {
        env.storage().instance().remove(key);
    }
    value
}

fn apply_next_delta(env: &Env, account: &mut OptionsAccount) {
    if let Some(delta) = take_next(env, &MockKey::NextDelta) {
        account.net_delta_bps = delta;
    }
}

#[contractimpl]
impl OptionsMarket for MockOptionsMarket {
    fn write_covered_call(
        e: Env,
        _underlying: Address,
        _strike: Address,
        amount: i128,
        strike_price: i128,
        expiry: u64,
        account: Address,
    ) -> OptionPosition {
        let mut value = load_account(&e, &account);
        value.short_calls += amount;
        apply_next_delta(&e, &mut value);
        store_account(&e, &account, &value);
        OptionPosition {
            kind: OptionKind::CoveredCall,
            amount,
            strike_price,
            expiry,
            premium: 0,
            delta_bps: value.net_delta_bps,
        }
    }

    fn buy_protective_put(
        e: Env,
        _underlying: Address,
        _strike: Address,
        amount: i128,
        strike_price: i128,
        expiry: u64,
        premium: i128,
        account: Address,
    ) -> OptionPosition {
        let mut value = load_account(&e, &account);
        value.long_puts += amount;
        apply_next_delta(&e, &mut value);
        store_account(&e, &account, &value);
        OptionPosition {
            kind: OptionKind::ProtectivePut,
            amount,
            strike_price,
            expiry,
            premium,
            delta_bps: value.net_delta_bps,
        }
    }

    fn settle(e: Env, kind: OptionKind, amount: i128, account: Address) -> i128 {
        let mut value = load_account(&e, &account);
        match kind {
            OptionKind::CoveredCall => {
                value.short_calls = (value.short_calls - amount).max(0);
            }
            OptionKind::ProtectivePut => {
                value.long_puts = (value.long_puts - amount).max(0);
            }
        }
        apply_next_delta(&e, &mut value);
        store_account(&e, &account, &value);
        take_next(&e, &MockKey::NextPnl).unwrap_or(0)
    }

    fn hedge(e: Env, target_delta_bps: i128, account: Address) {
        let mut value = load_account(&e, &account);
        value.net_delta_bps = target_delta_bps;
        store_account(&e, &account, &value);
    }

    fn account(e: Env, account: Address) -> OptionsAccount {
        load_account(&e, &account)
    }
}

#[contractimpl]
impl MockOptionsMarket {
    pub fn set_account(env: Env, account: Address, value: OptionsAccount) {
        store_account(&env, &account, &value);
    }

    pub fn set_next_delta(env: Env, delta_bps: i128) {
        env.storage()
            .instance()
            .set(&MockKey::NextDelta, &delta_bps);
    }

    pub fn set_next_pnl(env: Env, pnl: i128) {
        env.storage().instance().set(&MockKey::NextPnl, &pnl);
    }
}

fn setup(protocol: Protocol) -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let market = env.register(MockOptionsMarket, ());
    let contract = env.register(OptionsStrategy, ());
    let client = OptionsStrategyClient::new(&env, &contract);
    client.initialize(
        &admin,
        &operator,
        &protocol,
        &market,
        &Address::generate(&env),
        &Address::generate(&env),
        &5_000,
    );
    (env, contract, market, admin)
}

fn strategy_client<'a>(env: &'a Env, contract: &'a Address) -> OptionsStrategyClient<'a> {
    OptionsStrategyClient::new(env, contract)
}

fn market_client<'a>(env: &'a Env, market: &'a Address) -> MockOptionsMarketClient<'a> {
    MockOptionsMarketClient::new(env, market)
}

fn account_with_underlying(underlying: i128, delta_bps: i128) -> OptionsAccount {
    OptionsAccount {
        underlying,
        short_calls: 0,
        long_puts: 0,
        net_delta_bps: delta_bps,
    }
}

#[test]
fn write_covered_call_requires_and_records_underlying() {
    let (env, contract, market, _admin) = setup(Protocol::Lyra);
    let client = strategy_client(&env, &contract);
    market_client(&env, &market).set_account(&contract, &account_with_underlying(1_000, 0));

    let (position, account) = client.write_covered_call(&600, &2_000, &1_000);
    assert_eq!(position.kind, OptionKind::CoveredCall);
    assert_eq!(position.amount, 600);
    assert_eq!(position.strike_price, 2_000);
    assert_eq!(position.expiry, 1_000);
    assert_eq!(account.short_calls, 600);
    assert_eq!(client.protocol(), Protocol::Lyra);
}

#[test]
fn write_covered_call_must_be_fully_collateralized() {
    let (env, contract, market, _admin) = setup(Protocol::Dopex);
    let client = strategy_client(&env, &contract);
    market_client(&env, &market).set_account(&contract, &account_with_underlying(100, 0));

    assert_eq!(
        client.try_write_covered_call(&101, &2_000, &1_000),
        Err(Ok(Error::InsufficientUnderlying))
    );
}

#[test]
fn write_covered_call_validates_inputs() {
    let (env, contract, market, _admin) = setup(Protocol::Lyra);
    let client = strategy_client(&env, &contract);
    market_client(&env, &market).set_account(&contract, &account_with_underlying(1_000, 0));

    assert_eq!(
        client.try_write_covered_call(&0, &2_000, &1_000),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        client.try_write_covered_call(&100, &0, &1_000),
        Err(Ok(Error::InvalidStrike))
    );
    assert_eq!(
        client.try_write_covered_call(&100, &2_000, &0),
        Err(Ok(Error::InvalidExpiry))
    );
}

#[test]
fn write_covered_call_reverts_when_delta_leaves_the_band() {
    let (env, contract, market, _admin) = setup(Protocol::Lyra);
    let client = strategy_client(&env, &contract);
    market_client(&env, &market).set_account(&contract, &account_with_underlying(1_000, 0));
    market_client(&env, &market).set_next_delta(&6_000);

    assert_eq!(
        client.try_write_covered_call(&100, &2_000, &1_000),
        Err(Ok(Error::DeltaTooHigh))
    );
}

#[test]
fn protective_put_records_premium_and_long_exposure() {
    let (env, contract, _market, _admin) = setup(Protocol::Dopex);
    let client = strategy_client(&env, &contract);

    let (position, account) = client.buy_protective_put(&500, &1_500, &1_000, &40);
    assert_eq!(position.kind, OptionKind::ProtectivePut);
    assert_eq!(position.amount, 500);
    assert_eq!(position.premium, 40);
    assert_eq!(account.long_puts, 500);

    assert_eq!(
        client.try_buy_protective_put(&500, &1_500, &1_000, &-1),
        Err(Ok(Error::InvalidPremium))
    );
    assert_eq!(
        client.try_buy_protective_put(&500, &1_500, &0, &40),
        Err(Ok(Error::InvalidExpiry))
    );
}

#[test]
fn settle_returns_realized_pnl_and_checks_delta() {
    let (env, contract, market, _admin) = setup(Protocol::Lyra);
    let client = strategy_client(&env, &contract);
    market_client(&env, &market).set_account(
        &contract,
        &OptionsAccount {
            underlying: 1_000,
            short_calls: 500,
            long_puts: 0,
            net_delta_bps: 1_000,
        },
    );
    market_client(&env, &market).set_next_pnl(&250);

    let (pnl, account) = client.settle(&OptionKind::CoveredCall, &500);
    assert_eq!(pnl, 250);
    assert_eq!(account.short_calls, 0);
    assert_eq!(
        client.try_settle(&OptionKind::CoveredCall, &0),
        Err(Ok(Error::InvalidAmount))
    );
}

#[test]
fn hedge_keeps_delta_inside_the_configured_band() {
    let (env, contract, market, _admin) = setup(Protocol::Lyra);
    let client = strategy_client(&env, &contract);
    market_client(&env, &market).set_account(&contract, &account_with_underlying(1_000, 4_000));

    let account = client.hedge(&1_000);
    assert_eq!(account.net_delta_bps, 1_000);

    assert_eq!(client.try_hedge(&6_000), Err(Ok(Error::DeltaTooHigh)));
    assert_eq!(
        client.try_hedge(&(BPS_SCALE + 1)),
        Err(Ok(Error::InvalidDelta))
    );
}

#[test]
fn invalid_adapter_delta_is_rejected() {
    let (env, contract, market, _admin) = setup(Protocol::Dopex);
    let client = strategy_client(&env, &contract);
    market_client(&env, &market).set_account(&contract, &account_with_underlying(1_000, 12_000));

    assert_eq!(client.try_account(), Err(Ok(Error::InvalidDelta)));
}

#[test]
fn admin_controls_delta_band_and_operator() {
    let (env, contract, _market, admin) = setup(Protocol::Lyra);
    let client = strategy_client(&env, &contract);
    let new_operator = Address::generate(&env);

    assert_eq!(
        client.try_set_max_abs_delta_bps(&admin, &0),
        Err(Ok(Error::InvalidDelta))
    );
    assert_eq!(
        client.try_set_max_abs_delta_bps(&admin, &(BPS_SCALE + 1)),
        Err(Ok(Error::InvalidDelta))
    );
    client.set_max_abs_delta_bps(&admin, &3_000);
    assert_eq!(client.max_abs_delta_bps(), 3_000);
    client.set_operator(&admin, &new_operator);
}

#[test]
fn initialize_validates_configuration() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let market = env.register(MockOptionsMarket, ());
    let underlying = Address::generate(&env);
    let strike = Address::generate(&env);
    let contract = env.register(OptionsStrategy, ());
    let client = OptionsStrategyClient::new(&env, &contract);

    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::Lyra,
            &market,
            &underlying,
            &underlying,
            &5_000,
        ),
        Err(Ok(Error::InvalidConfiguration))
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::Lyra,
            &market,
            &underlying,
            &strike,
            &0,
        ),
        Err(Ok(Error::InvalidDelta))
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::Lyra,
            &market,
            &underlying,
            &strike,
            &(BPS_SCALE + 1),
        ),
        Err(Ok(Error::InvalidDelta))
    );

    client.initialize(
        &admin,
        &operator,
        &Protocol::Dopex,
        &market,
        &underlying,
        &strike,
        &5_000,
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::Dopex,
            &market,
            &underlying,
            &strike,
            &5_000,
        ),
        Err(Ok(Error::AlreadyInitialized))
    );
}
