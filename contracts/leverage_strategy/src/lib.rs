#![no_std]

//! Leveraged looping strategy with a normalized credit-market adapter.
//!
//! Gearbox and Notional are EVM protocols and are not callable directly from
//! Soroban. A Soroban market adapter must implement `LeverageMarket` and
//! bridge each protocol's collateral, debt, and risk data into the common
//! units documented below.
//!
//! Source: contract.md § Yield Strategies & Integrations (#24).

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

#[cfg(test)]
mod test;

/// Basis points: 10_000 = 100%.
pub const BPS_SCALE: i128 = 10_000;

const TTL_LEDGERS: u32 = 17_280;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    InvalidMaxLtv = 5,
    MaxLtvExceeded = 6,
    LiquidationRisk = 7,
    Insolvent = 8,
    InvalidAccount = 9,
    InvalidConfiguration = 10,
    Overflow = 11,
}

/// Leveraged-yield protocols supported through configured Soroban adapters.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Gearbox,
    Notional,
}

/// Credit-account values returned by a leverage-market adapter.
///
/// `net_asset_value` is collateral minus debt, `ltv_bps` is the debt-to-
/// collateral-value ratio in basis points, and `liquidation_threshold_bps` is
/// the protocol's liquidation threshold in basis points. An adapter that does
/// not expose a threshold should report `0`.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CreditAccount {
    pub collateral: i128,
    pub debt: i128,
    pub net_asset_value: i128,
    pub ltv_bps: i128,
    pub liquidation_threshold_bps: i128,
}

/// Common interface implemented by a Soroban adapter for a leverage market.
///
/// Adapters are responsible for protocol-specific token movement, looping,
/// and for normalizing credit-account data into `CreditAccount`.
#[soroban_sdk::contractclient(name = "LeverageMarketClient")]
pub trait LeverageMarket {
    fn open_loop(
        e: Env,
        collateral_asset: Address,
        collateral_amount: i128,
        debt_asset: Address,
        borrow_amount: i128,
        account: Address,
    );

    fn deleverage(
        e: Env,
        debt_asset: Address,
        repay_amount: i128,
        collateral_asset: Address,
        withdraw_amount: i128,
        account: Address,
    );

    fn credit_account(e: Env, account: Address) -> CreditAccount;
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
    MaxLtvBps,
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

fn validate_account(account: &CreditAccount) -> Result<(), Error> {
    if account.collateral < 0
        || account.debt < 0
        || account.ltv_bps < 0
        || account.ltv_bps > BPS_SCALE
        || account.liquidation_threshold_bps < 0
        || account.liquidation_threshold_bps > BPS_SCALE
    {
        return Err(Error::InvalidAccount);
    }
    if account.net_asset_value < 0 || (account.debt > 0 && account.net_asset_value == 0) {
        return Err(Error::Insolvent);
    }
    Ok(())
}

fn read_account(env: &Env) -> Result<CreditAccount, Error> {
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    let market: Address = load(env, &DataKey::Market)?;
    let account_address = env.current_contract_address();
    let account = LeverageMarketClient::new(env, &market).credit_account(&account_address);
    validate_account(&account)?;
    Ok(account)
}

fn load_max_ltv_bps(env: &Env) -> Result<i128, Error> {
    load(env, &DataKey::MaxLtvBps)
}

fn ensure_risk_limits(env: &Env) -> Result<CreditAccount, Error> {
    let account = read_account(env)?;
    let max_ltv_bps = load_max_ltv_bps(env)?;
    if account.ltv_bps > max_ltv_bps {
        return Err(Error::MaxLtvExceeded);
    }
    if account.liquidation_threshold_bps > 0 && account.ltv_bps >= account.liquidation_threshold_bps
    {
        return Err(Error::LiquidationRisk);
    }
    Ok(account)
}

#[contract]
pub struct LeverageStrategy;

#[contractimpl]
impl LeverageStrategy {
    /// Configure the strategy and its protocol-specific Soroban adapter.
    ///
    /// `max_ltv_bps` must be strictly below 100%, leaving room for the
    /// protocol's liquidation threshold.
    pub fn initialize(
        env: Env,
        admin: Address,
        operator: Address,
        protocol: Protocol,
        market: Address,
        collateral_asset: Address,
        debt_asset: Address,
        max_ltv_bps: i128,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if collateral_asset == debt_asset {
            return Err(Error::InvalidConfiguration);
        }
        if max_ltv_bps <= 0 || max_ltv_bps >= BPS_SCALE {
            return Err(Error::InvalidMaxLtv);
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
            .set(&DataKey::MaxLtvBps, &max_ltv_bps);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("init"),),
            (admin, operator, protocol, market, max_ltv_bps),
        );
        Ok(())
    }

    /// Open or extend a leveraged loop and enforce the post-operation LTV.
    pub fn loop_position(
        env: Env,
        collateral_amount: i128,
        borrow_amount: i128,
    ) -> Result<CreditAccount, Error> {
        require_operator(&env)?;
        if collateral_amount <= 0 || borrow_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let market: Address = load(&env, &DataKey::Market)?;
        let collateral_asset: Address = load(&env, &DataKey::CollateralAsset)?;
        let debt_asset: Address = load(&env, &DataKey::DebtAsset)?;
        let account = env.current_contract_address();
        LeverageMarketClient::new(&env, &market).open_loop(
            &collateral_asset,
            &collateral_amount,
            &debt_asset,
            &borrow_amount,
            &account,
        );
        let credit_account = ensure_risk_limits(&env)?;

        env.events().publish(
            (symbol_short!("loop"),),
            (collateral_amount, borrow_amount, credit_account.ltv_bps),
        );
        Ok(credit_account)
    }

    /// Repay debt to improve the position's health.
    pub fn deleverage(env: Env, repay_amount: i128) -> Result<CreditAccount, Error> {
        require_operator(&env)?;
        if repay_amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let market: Address = load(&env, &DataKey::Market)?;
        let collateral_asset: Address = load(&env, &DataKey::CollateralAsset)?;
        let debt_asset: Address = load(&env, &DataKey::DebtAsset)?;
        let account = env.current_contract_address();
        LeverageMarketClient::new(&env, &market).deleverage(
            &debt_asset,
            &repay_amount,
            &collateral_asset,
            &0,
            &account,
        );
        let credit_account = ensure_risk_limits(&env)?;

        env.events().publish(
            (symbol_short!("delever"),),
            (repay_amount, credit_account.ltv_bps),
        );
        Ok(credit_account)
    }

    /// Withdraw collateral and reject the operation if it would breach the LTV.
    pub fn withdraw_collateral(env: Env, amount: i128) -> Result<CreditAccount, Error> {
        require_operator(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let market: Address = load(&env, &DataKey::Market)?;
        let collateral_asset: Address = load(&env, &DataKey::CollateralAsset)?;
        let debt_asset: Address = load(&env, &DataKey::DebtAsset)?;
        let account = env.current_contract_address();
        LeverageMarketClient::new(&env, &market).deleverage(
            &debt_asset,
            &0,
            &collateral_asset,
            &amount,
            &account,
        );
        let credit_account = ensure_risk_limits(&env)?;

        env.events().publish(
            (symbol_short!("withdraw"),),
            (amount, credit_account.ltv_bps),
        );
        Ok(credit_account)
    }

    /// Read the current credit account from the configured market adapter.
    pub fn credit_account(env: Env) -> Result<CreditAccount, Error> {
        read_account(&env)
    }

    /// The configured protocol identifier.
    pub fn protocol(env: Env) -> Result<Protocol, Error> {
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        load(&env, &DataKey::Protocol)
    }

    /// The configured maximum loan-to-value ratio in basis points.
    pub fn max_ltv_bps(env: Env) -> Result<i128, Error> {
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        load_max_ltv_bps(&env)
    }

    /// Change the maximum LTV. Only the admin can adjust it.
    pub fn set_max_ltv_bps(env: Env, admin: Address, max_ltv_bps: i128) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if max_ltv_bps <= 0 || max_ltv_bps >= BPS_SCALE {
            return Err(Error::InvalidMaxLtv);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxLtvBps, &max_ltv_bps);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        env.events()
            .publish((symbol_short!("max_ltv"),), max_ltv_bps);
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
