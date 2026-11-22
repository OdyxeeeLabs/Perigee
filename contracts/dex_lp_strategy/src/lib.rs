#![no_std]

//! Concentrated-liquidity DEX LP strategy with a normalized pool adapter.
//!
//! Uniswap V3 is an EVM protocol and is not callable directly from Soroban. A
//! Soroban pool adapter must implement `LiquidityPool` and bridge each
//! protocol's position data into the common units documented below.
//!
//! Source: contract.md § Yield Strategies & Integrations (#22).

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

#[cfg(test)]
mod test;

/// Basis points used by the impermanent-loss guard: 10_000 = 100%.
pub const BPS_SCALE: i128 = 10_000;

/// Uniswap V3 fee tiers are expressed in hundredths of a bip.
pub const MAX_FEE_TIER: u32 = 1_000_000;

/// Uniswap V3 tick bounds.
pub const MIN_TICK: i32 = -887_272;
pub const MAX_TICK: i32 = 887_272;

const TTL_LEDGERS: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    InvalidTickRange = 5,
    InvalidFeeTier = 6,
    InvalidImpermanentLoss = 7,
    ImpermanentLossTooHigh = 8,
    InsufficientLiquidity = 9,
    InvalidPosition = 10,
    InvalidConfiguration = 11,
}

/// Concentrated-liquidity protocols supported through Soroban pool adapters.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    UniswapV3,
}

/// Position values returned by a pool adapter.
///
/// `amount0` and `amount1` are the underlying token amounts backing the
/// position, `net_asset_value` is denominated in the strategy's quote asset,
/// and `impermanent_loss_bps` is the loss relative to holding the tokens,
/// normalized to basis points.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LiquidityPosition {
    pub liquidity: i128,
    pub amount0: i128,
    pub amount1: i128,
    pub tick_lower: i32,
    pub tick_upper: i32,
    pub net_asset_value: i128,
    pub impermanent_loss_bps: i128,
}

/// Common interface implemented by a Soroban adapter for a concentrated
/// liquidity pool.
///
/// Adapters are responsible for protocol-specific token movement, tick
/// alignment, and for normalizing position values into `LiquidityPosition`.
#[soroban_sdk::contractclient(name = "LiquidityPoolClient")]
pub trait LiquidityPool {
    fn mint(
        e: Env,
        token_a: Address,
        token_b: Address,
        fee_tier: u32,
        tick_lower: i32,
        tick_upper: i32,
        amount_a: i128,
        amount_b: i128,
        account: Address,
    ) -> LiquidityPosition;

    fn burn(e: Env, liquidity: i128, account: Address) -> (i128, i128);

    fn rebalance(e: Env, tick_lower: i32, tick_upper: i32, account: Address) -> LiquidityPosition;

    fn position(e: Env, account: Address) -> LiquidityPosition;
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Operator,
    Pool,
    Protocol,
    TokenA,
    TokenB,
    FeeTier,
    MaxIlBps,
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

fn validate_ticks(tick_lower: i32, tick_upper: i32) -> Result<(), Error> {
    if tick_lower >= tick_upper || tick_lower < MIN_TICK || tick_upper > MAX_TICK {
        return Err(Error::InvalidTickRange);
    }
    Ok(())
}

fn validate_position(position: &LiquidityPosition) -> Result<(), Error> {
    if position.liquidity < 0
        || position.amount0 < 0
        || position.amount1 < 0
        || position.net_asset_value < 0
        || position.impermanent_loss_bps < 0
        || position.impermanent_loss_bps > BPS_SCALE
        || position.tick_lower >= position.tick_upper
    {
        return Err(Error::InvalidPosition);
    }
    Ok(())
}

fn read_position(env: &Env) -> Result<LiquidityPosition, Error> {
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    let pool: Address = load(env, &DataKey::Pool)?;
    let account = env.current_contract_address();
    let position = LiquidityPoolClient::new(env, &pool).position(&account);
    validate_position(&position)?;
    Ok(position)
}

fn max_il_bps(env: &Env) -> Result<i128, Error> {
    load(env, &DataKey::MaxIlBps)
}

#[contract]
pub struct DexLpStrategy;

#[contractimpl]
impl DexLpStrategy {
    /// Configure the strategy and its protocol-specific Soroban pool adapter.
    ///
    /// `fee_tier` follows the Uniswap convention of hundredths of a bip, and
    /// `max_il_bps` is the impermanent-loss tolerance in basis points.
    pub fn initialize(
        env: Env,
        admin: Address,
        operator: Address,
        protocol: Protocol,
        pool: Address,
        token_a: Address,
        token_b: Address,
        fee_tier: u32,
        max_il_bps: i128,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if token_a == token_b {
            return Err(Error::InvalidConfiguration);
        }
        if fee_tier == 0 || fee_tier > MAX_FEE_TIER {
            return Err(Error::InvalidFeeTier);
        }
        if max_il_bps <= 0 || max_il_bps > BPS_SCALE {
            return Err(Error::InvalidImpermanentLoss);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Operator, &operator);
        env.storage().instance().set(&DataKey::Protocol, &protocol);
        env.storage().instance().set(&DataKey::Pool, &pool);
        env.storage().instance().set(&DataKey::TokenA, &token_a);
        env.storage().instance().set(&DataKey::TokenB, &token_b);
        env.storage().instance().set(&DataKey::FeeTier, &fee_tier);
        env.storage()
            .instance()
            .set(&DataKey::MaxIlBps, &max_il_bps);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("init"),),
            (admin, operator, protocol, pool, fee_tier, max_il_bps),
        );
        Ok(())
    }

    /// Deposit both tokens into the configured pool within a tick range.
    pub fn deposit(
        env: Env,
        amount_a: i128,
        amount_b: i128,
        tick_lower: i32,
        tick_upper: i32,
    ) -> Result<LiquidityPosition, Error> {
        require_operator(&env)?;
        if amount_a < 0 || amount_b < 0 || (amount_a == 0 && amount_b == 0) {
            return Err(Error::InvalidAmount);
        }
        validate_ticks(tick_lower, tick_upper)?;

        let pool: Address = load(&env, &DataKey::Pool)?;
        let token_a: Address = load(&env, &DataKey::TokenA)?;
        let token_b: Address = load(&env, &DataKey::TokenB)?;
        let fee_tier: u32 = load(&env, &DataKey::FeeTier)?;
        let account = env.current_contract_address();

        let position = LiquidityPoolClient::new(&env, &pool).mint(
            &token_a,
            &token_b,
            &fee_tier,
            &tick_lower,
            &tick_upper,
            &amount_a,
            &amount_b,
            &account,
        );
        validate_position(&position)?;

        env.events().publish(
            (symbol_short!("deposit"),),
            (amount_a, amount_b, tick_lower, tick_upper),
        );
        Ok(position)
    }

    /// Burn liquidity and return the amounts released by the pool adapter.
    ///
    /// Withdrawals are always available so liquidity providers are never
    /// trapped by the impermanent-loss guard; the guard only gates rebalancing.
    pub fn withdraw(env: Env, liquidity: i128) -> Result<(i128, i128), Error> {
        require_operator(&env)?;
        if liquidity <= 0 {
            return Err(Error::InvalidAmount);
        }
        let before = read_position(&env)?;
        if liquidity > before.liquidity {
            return Err(Error::InsufficientLiquidity);
        }

        let pool: Address = load(&env, &DataKey::Pool)?;
        let account = env.current_contract_address();
        let (amount0, amount1) = LiquidityPoolClient::new(&env, &pool).burn(&liquidity, &account);
        if amount0 < 0 || amount1 < 0 {
            return Err(Error::InvalidPosition);
        }

        env.events()
            .publish((symbol_short!("withdraw"),), (liquidity, amount0, amount1));
        Ok((amount0, amount1))
    }

    /// Move the position to a new tick range.
    ///
    /// Rebalancing is rejected once the adapter-reported impermanent loss
    /// exceeds the configured tolerance: the operator must withdraw instead of
    /// rotating a position that is already beyond the strategy's risk limit.
    pub fn rebalance(
        env: Env,
        tick_lower: i32,
        tick_upper: i32,
    ) -> Result<LiquidityPosition, Error> {
        require_operator(&env)?;
        validate_ticks(tick_lower, tick_upper)?;

        let before = read_position(&env)?;
        if before.liquidity == 0 {
            return Err(Error::InsufficientLiquidity);
        }
        if before.impermanent_loss_bps > max_il_bps(&env)? {
            return Err(Error::ImpermanentLossTooHigh);
        }

        let pool: Address = load(&env, &DataKey::Pool)?;
        let account = env.current_contract_address();
        let position =
            LiquidityPoolClient::new(&env, &pool).rebalance(&tick_lower, &tick_upper, &account);
        validate_position(&position)?;

        env.events().publish(
            (symbol_short!("rebalance"),),
            (tick_lower, tick_upper, position.impermanent_loss_bps),
        );
        Ok(position)
    }

    /// Read the current position from the configured pool adapter.
    pub fn position(env: Env) -> Result<LiquidityPosition, Error> {
        read_position(&env)
    }

    /// The configured protocol identifier.
    pub fn protocol(env: Env) -> Result<Protocol, Error> {
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        load(&env, &DataKey::Protocol)
    }

    /// The configured impermanent-loss tolerance in basis points.
    pub fn max_impermanent_loss_bps(env: Env) -> Result<i128, Error> {
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        max_il_bps(&env)
    }

    /// Change the impermanent-loss tolerance. Only the admin can adjust it.
    pub fn set_max_impermanent_loss_bps(
        env: Env,
        admin: Address,
        max_il_bps: i128,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if max_il_bps <= 0 || max_il_bps > BPS_SCALE {
            return Err(Error::InvalidImpermanentLoss);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxIlBps, &max_il_bps);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        env.events().publish((symbol_short!("max_il"),), max_il_bps);
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
