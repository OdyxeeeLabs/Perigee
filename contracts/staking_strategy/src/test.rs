#![cfg(test)]

use super::*;
use soroban_sdk::{contract, contractimpl, contracttype, testutils::Address as _, Address, Env};

#[contract]
pub struct MockStakingPool;

#[contracttype]
#[derive(Clone)]
enum MockKey {
    Position(Address),
    NextGrossRewards,
}

fn default_position() -> StakingPosition {
    StakingPosition {
        shares: 0,
        principal: 0,
        accrued_rewards: 0,
    }
}

fn load_position(env: &Env, account: &Address) -> StakingPosition {
    env.storage()
        .persistent()
        .get(&MockKey::Position(account.clone()))
        .unwrap_or_else(default_position)
}

fn store_position(env: &Env, account: &Address, position: &StakingPosition) {
    env.storage()
        .persistent()
        .set(&MockKey::Position(account.clone()), position);
}

#[contractimpl]
impl StakingPool for MockStakingPool {
    fn stake(e: Env, _asset: Address, amount: i128, account: Address) -> i128 {
        let mut position = load_position(&e, &account);
        position.shares += amount;
        position.principal += amount;
        store_position(&e, &account, &position);
        amount
    }

    fn unstake(e: Env, shares: i128, account: Address) -> i128 {
        let mut position = load_position(&e, &account);
        position.shares -= shares;
        let released = shares;
        position.principal = (position.principal - released).max(0);
        store_position(&e, &account, &position);
        released
    }

    fn claim_rewards(e: Env, account: Address) -> i128 {
        let mut position = load_position(&e, &account);
        let gross = match e
            .storage()
            .instance()
            .get::<_, i128>(&MockKey::NextGrossRewards)
        {
            Some(gross) => {
                e.storage().instance().remove(&MockKey::NextGrossRewards);
                gross
            }
            None => position.accrued_rewards,
        };
        position.accrued_rewards = (position.accrued_rewards - gross).max(0);
        store_position(&e, &account, &position);
        gross
    }

    fn position(e: Env, account: Address) -> StakingPosition {
        load_position(&e, &account)
    }
}

#[contractimpl]
impl MockStakingPool {
    pub fn set_position(env: Env, account: Address, position: StakingPosition) {
        store_position(&env, &account, &position);
    }

    pub fn set_next_gross_rewards(env: Env, gross: i128) {
        env.storage()
            .instance()
            .set(&MockKey::NextGrossRewards, &gross);
    }
}

fn setup(protocol: Protocol) -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let pool = env.register(MockStakingPool, ());
    let contract = env.register(StakingStrategy, ());
    let client = StakingStrategyClient::new(&env, &contract);
    client.initialize(
        &admin,
        &operator,
        &protocol,
        &pool,
        &Address::generate(&env),
        &1_000,
    );
    (env, contract, pool, admin)
}

fn strategy_client<'a>(env: &'a Env, contract: &'a Address) -> StakingStrategyClient<'a> {
    StakingStrategyClient::new(env, contract)
}

fn pool_client<'a>(env: &'a Env, pool: &'a Address) -> MockStakingPoolClient<'a> {
    MockStakingPoolClient::new(env, pool)
}

#[test]
fn stake_mints_shares_and_tracks_principal() {
    let (env, contract, _pool, _admin) = setup(Protocol::Lido);
    let client = strategy_client(&env, &contract);

    let position = client.stake(&1_000);
    assert_eq!(position.shares, 1_000);
    assert_eq!(position.principal, 1_000);
    assert_eq!(client.protocol(), Protocol::Lido);
}

#[test]
fn stake_rejects_non_positive_amounts() {
    let (env, contract, _pool, _admin) = setup(Protocol::RocketPool);
    let client = strategy_client(&env, &contract);

    assert_eq!(client.try_stake(&0), Err(Ok(Error::InvalidAmount)));
    assert_eq!(client.try_stake(&-1), Err(Ok(Error::InvalidAmount)));
}

#[test]
fn unstake_releases_assets_and_reduces_shares() {
    let (env, contract, _pool, _admin) = setup(Protocol::Lido);
    let client = strategy_client(&env, &contract);
    client.stake(&1_000);

    let (released, position) = client.unstake(&400);
    assert_eq!(released, 400);
    assert_eq!(position.shares, 600);
    assert_eq!(position.principal, 600);
}

#[test]
fn unstake_rejects_bad_share_amounts() {
    let (env, contract, _pool, _admin) = setup(Protocol::RocketPool);
    let client = strategy_client(&env, &contract);
    client.stake(&1_000);

    assert_eq!(client.try_unstake(&0), Err(Ok(Error::InvalidAmount)));
    assert_eq!(
        client.try_unstake(&1_001),
        Err(Ok(Error::InsufficientShares))
    );
}

#[test]
fn claim_applies_commission_to_gross_rewards() {
    let (env, contract, pool, _admin) = setup(Protocol::Lido);
    let client = strategy_client(&env, &contract);

    pool_client(&env, &pool).set_position(
        &contract,
        &StakingPosition {
            shares: 1_000,
            principal: 1_000,
            accrued_rewards: 1_000,
        },
    );

    let (gross, commission, position) = client.claim();
    assert_eq!(gross, 1_000);
    assert_eq!(commission, 100);
    assert_eq!(position.accrued_rewards, 0);
}

#[test]
fn claim_rounds_commission_down() {
    let (env, contract, pool, _admin) = setup(Protocol::RocketPool);
    let client = strategy_client(&env, &contract);

    pool_client(&env, &pool).set_position(
        &contract,
        &StakingPosition {
            shares: 1_000,
            principal: 1_000,
            accrued_rewards: 999,
        },
    );

    let (gross, commission, _position) = client.claim();
    assert_eq!(gross, 999);
    assert_eq!(commission, 99);
}

#[test]
fn claim_without_accrued_rewards_is_a_noop() {
    let (env, contract, _pool, _admin) = setup(Protocol::Lido);
    let client = strategy_client(&env, &contract);
    client.stake(&1_000);

    let (gross, commission, position) = client.claim();
    assert_eq!(gross, 0);
    assert_eq!(commission, 0);
    assert_eq!(position.shares, 1_000);
}

#[test]
fn claim_rejects_negative_adapter_rewards() {
    let (env, contract, pool, _admin) = setup(Protocol::Lido);
    let client = strategy_client(&env, &contract);

    pool_client(&env, &pool).set_position(
        &contract,
        &StakingPosition {
            shares: 1_000,
            principal: 1_000,
            accrued_rewards: 100,
        },
    );
    pool_client(&env, &pool).set_next_gross_rewards(&-1);

    assert_eq!(client.try_claim(), Err(Ok(Error::InvalidRewards)));
}

#[test]
fn admin_controls_commission_and_operator() {
    let (env, contract, _pool, admin) = setup(Protocol::Lido);
    let client = strategy_client(&env, &contract);
    let new_operator = Address::generate(&env);

    assert_eq!(
        client.try_set_commission_bps(&admin, &-1),
        Err(Ok(Error::InvalidCommission))
    );
    assert_eq!(
        client.try_set_commission_bps(&admin, &(MAX_COMMISSION_BPS + 1)),
        Err(Ok(Error::InvalidCommission))
    );
    client.set_commission_bps(&admin, &2_000);
    assert_eq!(client.commission_bps(), 2_000);
    client.set_operator(&admin, &new_operator);
}

#[test]
fn initialize_validates_commission() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let operator = Address::generate(&env);
    let pool = env.register(MockStakingPool, ());
    let asset = Address::generate(&env);
    let contract = env.register(StakingStrategy, ());
    let client = StakingStrategyClient::new(&env, &contract);

    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::Lido,
            &pool,
            &asset,
            &(MAX_COMMISSION_BPS + 1),
        ),
        Err(Ok(Error::InvalidCommission))
    );
    assert_eq!(
        client.try_initialize(&admin, &operator, &Protocol::Lido, &pool, &asset, &-1,),
        Err(Ok(Error::InvalidCommission))
    );

    client.initialize(
        &admin,
        &operator,
        &Protocol::Lido,
        &pool,
        &asset,
        &MAX_COMMISSION_BPS,
    );
    assert_eq!(
        client.try_initialize(
            &admin,
            &operator,
            &Protocol::RocketPool,
            &pool,
            &asset,
            &1_000,
        ),
        Err(Ok(Error::AlreadyInitialized))
    );
}
