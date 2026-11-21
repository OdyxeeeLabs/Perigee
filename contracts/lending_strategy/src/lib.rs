#![no_std]

//! Lending strategy with a normalized market-adapter interface.
//!
//! Aave V3 and Compound V3 are EVM protocols and are not callable directly
//! from Soroban. A Soroban market adapter must implement `LendingMarket` and
//! bridge each protocol's position data into the common units documented below.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

#[cfg(test)]
mod test;

/// Health-factor values use six decimal places: 1.0 is `HEALTH_FACTOR_SCALE`.
pub const HEALTH_FACTOR_SCALE: i128 = 1_000_000;

const TTL_LEDGERS: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    InvalidHealthFactor = 5,
    HealthFactorTooLow = 6,
    InsufficientSupply = 7,
    InvalidRepayment = 8,
    NoCollateral = 9,
}

/// Lending protocols supported through configured Soroban market adapters.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    AaveV3,
    CompoundV3,
}

/// Position values returned by a market adapter.
///
/// `net_asset_value` is denominated in the strategy's collateral asset and
/// includes accrued interest, while `health_factor` is normalized to six
/// decimal places. An adapter with no outstanding debt should report
/// `i128::MAX` for the health factor.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct LendingPosition {
    pub supplied: i128,
    pub borrowed: i128,
    pub net_asset_value: i128,
    pub health_factor: i128,
}

/// Common interface implemented by a Soroban adapter for a lending market.
///
/// Adapters are responsible for protocol-specific token movement and for
/// normalizing position values into `LendingPosition`.
#[soroban_sdk::contractclient(name = "LendingMarketClient")]
pub trait LendingMarket {
    fn supply(e: Env, asset: Address, amount: i128, account: Address);
    fn withdraw(e: Env, asset: Address, amount: i128, recipient: Address);
    fn borrow(e: Env, asset: Address, amount: i128, recipient: Address);
    fn repay(e: Env, asset: Address, amount: i128, account: Address) -> i128;
    fn position(e: Env, account: Address) -> LendingPosition;
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Operator,
    Market,
    Protocol,
    CollateralAsset,
    DebtAsset,
    MinHealthFactor,
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

fn read_position(env: &Env) -> Result<LendingPosition, Error> {
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    let account = env.current_contract_address();
    let market: Address = env
        .storage()
        .instance()
        .get(&DataKey::Market)
        .ok_or(Error::NotInitialized)?;
    let position = LendingMarketClient::new(env, &market).position(&account);
    if position.supplied < 0
        || position.borrowed < 0
        || position.net_asset_value < 0
        || position.health_factor < 0
    {
        return Err(Error::InvalidHealthFactor);
    }
    Ok(position)
}

fn ensure_healthy(env: &Env) -> Result<LendingPosition, Error> {
    let position = read_position(env)?;
    let minimum: i128 = env
        .storage()
        .instance()
        .get(&DataKey::MinHealthFactor)
        .ok_or(Error::NotInitialized)?;
    if position.health_factor < minimum {
        return Err(Error::HealthFactorTooLow);
    }
    Ok(position)
}

fn load_market(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Market)
        .ok_or(Error::NotInitialized)
}

fn load_collateral_asset(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&DataKey::CollateralAsset)
        .ok_or(Error::NotInitialized)
}

fn load_debt_asset(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&DataKey::DebtAsset)
        .ok_or(Error::NotInitialized)
}

#[contract]
pub struct LendingStrategy;

#[contractimpl]
impl LendingStrategy {
    /// Configure the strategy and its protocol-specific Soroban adapter.
    ///
    /// `min_health_factor` must be strictly greater than 1.0. The market
    /// adapter is expected to be configured for the selected protocol and
    /// assets, and to report values in this contract's normalized format.
    pub fn initialize(
        env: Env,
        admin: Address,
        operator: Address,
        protocol: Protocol,
        market: Address,
        collateral_asset: Address,
        debt_asset: Address,
        min_health_factor: i128,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if min_health_factor <= HEALTH_FACTOR_SCALE {
            return Err(Error::InvalidHealthFactor);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Operator, &operator);
        env.storage().instance().set(&DataKey::Protocol, &protocol);
        env.storage().instance().set(&DataKey::Market, &market);
        env.storage()
            .instance()
            .set(&DataKey::CollateralAsset, &collateral_asset);
        env.storage()
            .instance()
            .set(&DataKey::DebtAsset, &debt_asset);
        env.storage()
            .instance()
            .set(&DataKey::MinHealthFactor, &min_health_factor);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("init"),),
            (admin, operator, protocol, market, min_health_factor),
        );
        Ok(())
    }

    /// Supply collateral to the configured market on behalf of this strategy.
    pub fn supply(env: Env, amount: i128) -> Result<LendingPosition, Error> {
        require_operator(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let market = load_market(&env)?;
        let asset = load_collateral_asset(&env)?;
        let account = env.current_contract_address();
        LendingMarketClient::new(&env, &market).supply(&asset, &amount, &account);
        let position = read_position(&env)?;
        env.events()
            .publish((symbol_short!("supply"),), (asset, amount));
        Ok(position)
    }

    /// Withdraw collateral to the strategy after checking the resulting risk.
    pub fn withdraw(env: Env, amount: i128) -> Result<LendingPosition, Error> {
        require_operator(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let before = read_position(&env)?;
        if amount > before.supplied {
            return Err(Error::InsufficientSupply);
        }

        let market = load_market(&env)?;
        let asset = load_collateral_asset(&env)?;
        let account = env.current_contract_address();
        LendingMarketClient::new(&env, &market).withdraw(&asset, &amount, &account);
        let position = ensure_healthy(&env)?;
        env.events()
            .publish((symbol_short!("withdraw"),), (asset, amount));
        Ok(position)
    }

    /// Borrow the configured debt asset only if the resulting health factor is safe.
    pub fn borrow(env: Env, amount: i128) -> Result<LendingPosition, Error> {
        require_operator(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let before = ensure_healthy(&env)?;
        if before.supplied == 0 {
            return Err(Error::NoCollateral);
        }

        let market = load_market(&env)?;
        let asset = load_debt_asset(&env)?;
        let account = env.current_contract_address();
        LendingMarketClient::new(&env, &market).borrow(&asset, &amount, &account);
        let position = ensure_healthy(&env)?;
        env.events()
            .publish((symbol_short!("borrow"),), (asset, amount));
        Ok(position)
    }

    /// Repay debt from the strategy and return the amount the market accepted.
    pub fn repay(env: Env, amount: i128) -> Result<(i128, LendingPosition), Error> {
        require_operator(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let market = load_market(&env)?;
        let asset = load_debt_asset(&env)?;
        let account = env.current_contract_address();
        let repaid = LendingMarketClient::new(&env, &market).repay(&asset, &amount, &account);
        if repaid < 0 || repaid > amount {
            return Err(Error::InvalidRepayment);
        }
        let position = read_position(&env)?;
        env.events()
            .publish((symbol_short!("repay"),), (asset, repaid));
        Ok((repaid, position))
    }

    /// Read the current position from the configured market adapter.
    pub fn position(env: Env) -> Result<LendingPosition, Error> {
        read_position(&env)
    }

    /// The configured protocol identifier.
    pub fn protocol(env: Env) -> Result<Protocol, Error> {
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        env.storage()
            .instance()
            .get(&DataKey::Protocol)
            .ok_or(Error::NotInitialized)
    }

    /// Change the minimum health factor. Only the admin can lower or raise it.
    pub fn set_min_health_factor(
        env: Env,
        admin: Address,
        min_health_factor: i128,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if min_health_factor <= HEALTH_FACTOR_SCALE {
            return Err(Error::InvalidHealthFactor);
        }
        env.storage()
            .instance()
            .set(&DataKey::MinHealthFactor, &min_health_factor);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        env.events()
            .publish((symbol_short!("min_hf"),), min_health_factor);
        Ok(())
    }

    /// Transfer operation authority without changing the strategy admin.
    pub fn set_operator(
        env: Env,
        admin: Address,
        operator: Address,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::Operator, &operator);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        env.events()
            .publish((symbol_short!("operator"),), operator);
        Ok(())
    }
}
