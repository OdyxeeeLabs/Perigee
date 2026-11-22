#![no_std]

//! Liquid-staking strategy with a normalized staking-pool adapter.
//!
//! Lido and Rocket Pool are EVM protocols and are not callable directly from
//! Soroban. A Soroban pool adapter must implement `StakingPool` and bridge
//! each protocol's shares, principal, and reward accounting into the common
//! units documented below.
//!
//! Source: contract.md § Yield Strategies & Integrations (#23).

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

#[cfg(test)]
mod test;

/// Basis points: 10_000 = 100%.
pub const BPS_SCALE: i128 = 10_000;

/// Commission is capped at 50% of claimed rewards.
pub const MAX_COMMISSION_BPS: i128 = 5_000;

const TTL_LEDGERS: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    InvalidCommission = 5,
    InsufficientShares = 6,
    InvalidRewards = 7,
    InvalidPosition = 8,
    Overflow = 9,
}

/// Liquid-staking protocols supported through configured Soroban adapters.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Lido,
    RocketPool,
}

/// Position values returned by a staking-pool adapter.
///
/// `shares` is the liquid staking receipt balance, `principal` is the staked
/// asset value at deposit cost, and `accrued_rewards` is the gross reward
/// balance before commission.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakingPosition {
    pub shares: i128,
    pub principal: i128,
    pub accrued_rewards: i128,
}

/// Common interface implemented by a Soroban adapter for a liquid staking
/// protocol.
///
/// Adapters are responsible for protocol-specific token movement and for
/// normalizing balances into `StakingPosition`.
#[soroban_sdk::contractclient(name = "StakingPoolClient")]
pub trait StakingPool {
    fn stake(e: Env, asset: Address, amount: i128, account: Address) -> i128;
    fn unstake(e: Env, shares: i128, account: Address) -> i128;
    fn claim_rewards(e: Env, account: Address) -> i128;
    fn position(e: Env, account: Address) -> StakingPosition;
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Operator,
    Pool,
    Protocol,
    StakingAsset,
    CommissionBps,
}

fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
    let stored: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)?;
    if *admin != stored {
        return Err(Error::Unauthorized);
    }
    admin.require_auth();
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    Ok(())
}

fn require_operator(env: &Env) -> Result<(), Error> {
    let operator: Address = env
        .storage()
        .instance()
        .get(&DataKey::Operator)
        .ok_or(Error::NotInitialized)?;
    operator.require_auth();
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    Ok(())
}

fn load<T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val>>(
    env: &Env,
    key: &DataKey,
) -> Result<T, Error> {
    env.storage()
        .instance()
        .get(key)
        .ok_or(Error::NotInitialized)
}

fn validate_position(position: &StakingPosition) -> Result<(), Error> {
    if position.shares < 0 || position.principal < 0 || position.accrued_rewards < 0 {
        return Err(Error::InvalidPosition);
    }
    Ok(())
}

fn read_position(env: &Env) -> Result<StakingPosition, Error> {
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    let pool: Address = load(env, &DataKey::Pool)?;
    let account = env.current_contract_address();
    let position = StakingPoolClient::new(env, &pool).position(&account);
    validate_position(&position)?;
    Ok(position)
}

fn load_commission_bps(env: &Env) -> Result<i128, Error> {
    load(env, &DataKey::CommissionBps)
}

fn commission_on(gross: i128, commission_bps: i128) -> Result<i128, Error> {
    let numerator = gross.checked_mul(commission_bps).ok_or(Error::Overflow)?;
    numerator.checked_div(BPS_SCALE).ok_or(Error::Overflow)
}

#[contract]
pub struct StakingStrategy;

#[contractimpl]
impl StakingStrategy {
    /// Configure the strategy and its protocol-specific Soroban adapter.
    ///
    /// `commission_bps` is the share of claimed staking rewards retained by
    /// the strategy (protocol commission plus performance fee) and is capped
    /// at `MAX_COMMISSION_BPS`.
    pub fn initialize(
        env: Env,
        admin: Address,
        operator: Address,
        protocol: Protocol,
        pool: Address,
        staking_asset: Address,
        commission_bps: i128,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if commission_bps < 0 || commission_bps > MAX_COMMISSION_BPS {
            return Err(Error::InvalidCommission);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Operator, &operator);
        env.storage().instance().set(&DataKey::Protocol, &protocol);
        env.storage().instance().set(&DataKey::Pool, &pool);
        env.storage()
            .instance()
            .set(&DataKey::StakingAsset, &staking_asset);
        env.storage()
            .instance()
            .set(&DataKey::CommissionBps, &commission_bps);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("init"),),
            (admin, operator, protocol, pool, commission_bps),
        );
        Ok(())
    }

    /// Stake the configured asset and return the resulting position.
    pub fn stake(env: Env, amount: i128) -> Result<StakingPosition, Error> {
        require_operator(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let pool: Address = load(&env, &DataKey::Pool)?;
        let asset: Address = load(&env, &DataKey::StakingAsset)?;
        let account = env.current_contract_address();
        let shares = StakingPoolClient::new(&env, &pool).stake(&asset, &amount, &account);
        if shares < 0 {
            return Err(Error::InvalidPosition);
        }
        let position = read_position(&env)?;

        env.events()
            .publish((symbol_short!("stake"),), (amount, shares));
        Ok(position)
    }

    /// Redeem staking shares and return the assets released by the adapter.
    pub fn unstake(env: Env, shares: i128) -> Result<(i128, StakingPosition), Error> {
        require_operator(&env)?;
        if shares <= 0 {
            return Err(Error::InvalidAmount);
        }
        let before = read_position(&env)?;
        if shares > before.shares {
            return Err(Error::InsufficientShares);
        }

        let pool: Address = load(&env, &DataKey::Pool)?;
        let account = env.current_contract_address();
        let released = StakingPoolClient::new(&env, &pool).unstake(&shares, &account);
        if released < 0 {
            return Err(Error::InvalidPosition);
        }
        let position = read_position(&env)?;

        env.events()
            .publish((symbol_short!("unstake"),), (shares, released));
        Ok((released, position))
    }

    /// Claim accrued rewards, apply the configured commission, and return
    /// `(gross, commission, net)`.
    ///
    /// The adapter moves gross rewards into the strategy account; the
    /// commission stays there for the treasury while the net amount is what
    /// the vault can account for as profit.
    pub fn claim(env: Env) -> Result<(i128, i128, StakingPosition), Error> {
        require_operator(&env)?;
        let before = read_position(&env)?;
        if before.accrued_rewards == 0 {
            return Ok((0, 0, before));
        }

        let pool: Address = load(&env, &DataKey::Pool)?;
        let account = env.current_contract_address();
        let gross = StakingPoolClient::new(&env, &pool).claim_rewards(&account);
        if gross < 0 {
            return Err(Error::InvalidRewards);
        }
        let commission = commission_on(gross, load_commission_bps(&env)?)?;
        let net = gross - commission;
        if net < 0 {
            return Err(Error::InvalidRewards);
        }
        let position = read_position(&env)?;

        env.events()
            .publish((symbol_short!("claim"),), (gross, commission, net));
        Ok((gross, commission, position))
    }

    /// Read the current position from the configured pool adapter.
    pub fn position(env: Env) -> Result<StakingPosition, Error> {
        read_position(&env)
    }

    /// The configured protocol identifier.
    pub fn protocol(env: Env) -> Result<Protocol, Error> {
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        load(&env, &DataKey::Protocol)
    }

    /// The configured commission in basis points.
    pub fn commission_bps(env: Env) -> Result<i128, Error> {
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        load_commission_bps(&env)
    }

    /// Change the commission applied to claimed rewards.
    pub fn set_commission_bps(env: Env, admin: Address, commission_bps: i128) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if commission_bps < 0 || commission_bps > MAX_COMMISSION_BPS {
            return Err(Error::InvalidCommission);
        }
        env.storage()
            .instance()
            .set(&DataKey::CommissionBps, &commission_bps);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        env.events()
            .publish((symbol_short!("comm"),), commission_bps);
        Ok(())
    }

    /// Transfer operation authority without changing the strategy admin.
    pub fn set_operator(env: Env, admin: Address, operator: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::Operator, &operator);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        env.events().publish((symbol_short!("operator"),), operator);
        Ok(())
    }
}
