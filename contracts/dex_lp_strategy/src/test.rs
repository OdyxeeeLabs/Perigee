#![cfg(test)]

use super::*;
use soroban_sdk::{contract, contractimpl, contracttype, testutils::Address as _, Address, Env};

#[contract]
pub struct MockLiquidityPool;

#[contracttype]
#[derive(Clone)]
enum MockKey {
    Position(Address),
    NextIlBps,
}

fn default_position() -> LiquidityPosition {
    LiquidityPosition {
        liquidity: 0,
        amount0: 0,
        amount1: 0,
        tick_lower: -60,
        tick_upper: 60,
        net_asset_value: 0,
        impermanent_loss_bps: 0,
    }
}

fn load_position(env: &Env, account: &Address) -> LiquidityPosition {
    env.storage()
        .persistent()
        .get(&MockKey::Position(account.clone()))
        .unwrap_or_else(default_position)
}

fn store_position(env: &Env, account: &Address, position: &LiquidityPosition) {
    env.storage()
        .persistent()
        .set(&MockKey::Position(account.clone()), position);
}

#[contractimpl]
impl LiquidityPool for MockLiquidityPool {
    fn mint(
        e: Env,
        _token_a: Address,
        _token_b: Address,
        _fee_tier: u32,
        tick_lower: i32,
        tick_upper: i32,
        amount_a: i128,
        amount_b: i128,
        account: Address,
    ) -> LiquidityPosition {
        let mut position = load_position(&e, &account);
        position.liquidity += amount_a + amount_b;
        position.amount0 += amount_a;
        position.amount1 += amount_b;
        position.net_asset_value += amount_a + amount_b;
        position.tick_lower = tick_lower;
        position.tick_upper = tick_upper;
        if let Some(next_il) = e.storage().instance().get::<_, i128>(&MockKey::NextIlBps) {
            position.impermanent_loss_bps = next_il;
            e.storage().instance().remove(&MockKey::NextIlBps);
        }
        store_position(&e, &account, &position);
        position
    }

    fn burn(e: Env, liquidity: i128, account: Address) -> (i128, i128) {
        let mut position = load_position(&e, &account);
        let amount0 = if position.liquidity == 0 {
            0
        } else {
            position.amount0 * liquidity / position.liquidity
        };
        let amount1 = if position.liquidity == 0 {
            0
        } else {
            position.amount1 * liquidity / position.liquidity
        };
        position.liquidity -= liquidity;
        position.amount0 -= amount0;
        position.amount1 -= amount1;
        position.net_asset_value -= amount0 + amount1;
        store_position(&e, &account, &position);
        (amount0, amount1)
    }

    fn rebalance(e: Env, tick_lower: i32, tick_upper: i32, account: Address) -> LiquidityPosition {
        let mut position = load_position(&e, &account);
        position.tick_lower = tick_lower;
        position.tick_upper = tick_upper;
        store_position(&e, &account, &position);
        position
    }

    fn position(e: Env, account: Address) -> LiquidityPosition {
        load_position(&e, &account)
    }
}

#[contractimpl]
impl MockLiquidityPool {
    pub fn set_next_il_bps(env: Env, impermanent_loss_bps: i128) {
        env.storage()
            .instance()
            .set(&MockKey::NextIlBps, &impermanent_loss_bps);
    }

    pub fn set_position(env: Env, account: Address, position: LiquidityPosition) {
        store_position(&env, &account, &position);
    }
}

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let pool = env.register(MockLiquidityPool, ());
    let contract = env.register(DexLpStrategy, ());
    let client = DexLpStrategyClient::new(&env, &contract);
    client.initialize(
        &admin,
        &operator,
        &Protocol::UniswapV3,
        &pool,
        &Address::generate(&env),
        &Address::generate(&env),
        &3_000,
        &2_000,
    );
    (env, contract, pool, admin)
}

fn strategy_client<'a>(env: &'a Env, contract: &'a Address) -> DexLpStrategyClient<'a> {
    DexLpStrategyClient::new(env, contract)
}

fn pool_client<'a>(env: &'a Env, pool: &'a Address) -> MockLiquidityPoolClient<'a> {
    MockLiquidityPoolClient::new(env, pool)
}

#[test]
fn deposit_mints_a_concentrated_position() {
    let (env, contract, _pool, _admin) = setup();
    let client = strategy_client(&env, &contract);

    let position = client.deposit(&1_000, &2_000, &-60, &60);
    assert_eq!(position.liquidity, 3_000);
    assert_eq!(position.amount0, 1_000);
    assert_eq!(position.amount1, 2_000);
    assert_eq!(position.tick_lower, -60);
    assert_eq!(position.tick_upper, 60);
    assert_eq!(client.protocol(), Protocol::UniswapV3);
}

#[test]
fn deposit_rejects_bad_amounts_and_tick_ranges() {
    let (env, contract, _pool, _admin) = setup();
    let client = strategy_client(&env, &contract);

    assert_eq!(
        client.try_deposit(&0, &0, &-60, &60),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        client.try_deposit(&-1, &100, &-60, &60),
        Err(Ok(Error::InvalidAmount))
    );
    assert_eq!(
        client.try_deposit(&100, &100, &60, &60),
        Err(Ok(Error::InvalidTickRange))
    );
    assert_eq!(
        client.try_deposit(&100, &100, &(MIN_TICK - 1), &60),
        Err(Ok(Error::InvalidTickRange))
    );
    assert_eq!(
        client.try_deposit(&100, &100, &-60, &(MAX_TICK + 1)),
        Err(Ok(Error::InvalidTickRange))
    );
}

#[test]
fn withdraw_releases_proportional_amounts() {
    let (env, contract, _pool, _admin) = setup();
    let client = strategy_client(&env, &contract);
    client.deposit(&1_000, &2_000, &-60, &60);

    let (amount0, amount1) = client.withdraw(&1_500);
    assert_eq!(amount0, 500);
    assert_eq!(amount1, 1_000);
    assert_eq!(client.position().liquidity, 1_500);
}

#[test]
fn withdraw_rejects_bad_liquidity() {
    let (env, contract, _pool, _admin) = setup();
    let client = strategy_client(&env, &contract);
    client.deposit(&1_000, &2_000, &-60, &60);

    assert_eq!(client.try_withdraw(&0), Err(Ok(Error::InvalidAmount)));
    assert_eq!(
        client.try_withdraw(&3_001),
        Err(Ok(Error::InsufficientLiquidity))
    );
}

#[test]
fn withdrawals_stay_available_when_impermanent_loss_exceeds_tolerance() {
    let (env, contract, pool, _admin) = setup();
    let client = strategy_client(&env, &contract);

    pool_client(&env, &pool).set_next_il_bps(&3_000);
    let position = client.deposit(&1_000, &1_000, &-60, &60);
    assert_eq!(position.impermanent_loss_bps, 3_000);

    let (amount0, amount1) = client.withdraw(&2_000);
    assert_eq!(amount0, 1_000);
    assert_eq!(amount1, 1_000);
}

#[test]
fn rebalance_moves_the_position_to_the_new_range() {
    let (env, contract, _pool, _admin) = setup();
    let client = strategy_client(&env, &contract);
    client.deposit(&1_000, &1_000, &-60, &60);

    let position = client.rebalance(&-120, &120);
    assert_eq!(position.tick_lower, -120);
    assert_eq!(position.tick_upper, 120);
}

#[test]
fn rebalance_requires_liquidity_and_a_valid_range() {
    let (env, contract, _pool, _admin) = setup();
    let client = strategy_client(&env, &contract);

    assert_eq!(
        client.try_rebalance(&-120, &120),
        Err(Ok(Error::InsufficientLiquidity))
    );

    client.deposit(&1_000, &1_000, &-60, &60);
    assert_eq!(
        client.try_rebalance(&120, &120),
        Err(Ok(Error::InvalidTickRange))
    );
}

#[test]
fn rebalance_is_blocked_once_impermanent_loss_exceeds_the_tolerance() {
    let (env, contract, pool, _admin) = setup();
    let client = strategy_client(&env, &contract);

    pool_client(&env, &pool).set_next_il_bps(&2_001);
    client.deposit(&1_000, &1_000, &-60, &60);

    assert_eq!(
        client.try_rebalance(&-120, &120),
        Err(Ok(Error::ImpermanentLossTooHigh))
    );
    assert_eq!(client.position().tick_lower, -60);
}

#[test]
fn admin_controls_il_tolerance_and_operator() {
    let (env, contract, _pool, admin) = setup();
    let client = strategy_client(&env, &contract);
    let new_operator = Address::generate(&env);

    assert_eq!(
        client.try_set_max_impermanent_loss_bps(&admin, &0),
        Err(Ok(Error::InvalidImpermanentLoss))
    );
    assert_eq!(
        client.try_set_max_impermanent_loss_bps(&admin, &(BPS_SCALE + 1)),
        Err(Ok(Error::InvalidImpermanentLoss))
    );
    client.set_max_impermanent_loss_bps(&admin, &3_000);
    assert_eq!(client.max_impermanent_loss_bps(), 3_000);
    client.set_operator(&admin, &new_operator);
}

#[test]
fn initialize_validates_configuration() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let pool = env.register(MockLiquidityPool, ());
    let contract = env.register(DexLpStrategy, ());
    let client = DexLpStrategyClient::new(&env, &contract);
    let token_a = Address::generate(&env);
    let token_b = Address::generate(&env);

    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::UniswapV3,
            &pool,
            &token_a,
            &token_a,
            &3_000,
            &2_000,
        ),
        Err(Ok(Error::InvalidConfiguration))
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::UniswapV3,
            &pool,
            &token_a,
            &token_b,
            &0,
            &2_000,
        ),
        Err(Ok(Error::InvalidFeeTier))
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::UniswapV3,
            &pool,
            &token_a,
            &token_b,
            &(MAX_FEE_TIER + 1),
            &2_000,
        ),
        Err(Ok(Error::InvalidFeeTier))
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::UniswapV3,
            &pool,
            &token_a,
            &token_b,
            &3_000,
            &0,
        ),
        Err(Ok(Error::InvalidImpermanentLoss))
    );

    client.initialize(
        &admin,
        &operator,
        &Protocol::UniswapV3,
        &pool,
        &token_a,
        &token_b,
        &3_000,
        &2_000,
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::UniswapV3,
            &pool,
            &token_a,
            &token_b,
            &3_000,
            &2_000,
        ),
        Err(Ok(Error::AlreadyInitialized))
    );
}
