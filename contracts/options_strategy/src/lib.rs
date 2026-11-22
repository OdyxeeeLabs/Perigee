#![no_std]

//! Options strategy with a normalized options-market adapter.
//!
//! Lyra and Dopex are EVM protocols and are not callable directly from
//! Soroban. A Soroban market adapter must implement `OptionsMarket` and bridge
//! each protocol's option positions and Greeks into the common units
//! documented below.
//!
//! Source: contract.md § Yield Strategies & Integrations (#25).

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

#[cfg(test)]
mod test;

/// Basis points: 10_000 = 100% of the underlying notional.
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
    InvalidStrike = 5,
    InvalidExpiry = 6,
    InvalidDelta = 7,
    DeltaTooHigh = 8,
    InsufficientUnderlying = 9,
    InvalidPosition = 10,
    InvalidConfiguration = 11,
    InvalidPremium = 12,
}

/// Options protocols supported through configured Soroban adapters.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Lyra,
    Dopex,
}

/// Option structures supported by the strategy.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum OptionKind {
    CoveredCall,
    ProtectivePut,
}

/// Position values returned by an options-market adapter.
///
/// `delta_bps` is the position delta normalized to the underlying notional in
/// basis points (`5_000` = half of the notional).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionPosition {
    pub kind: OptionKind,
    pub amount: i128,
    pub strike_price: i128,
    pub expiry: u64,
    pub premium: i128,
    pub delta_bps: i128,
}

/// Aggregate account values returned by an options-market adapter.
///
/// `net_delta_bps` is the account-level delta normalized to the underlying
/// notional in basis points.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OptionsAccount {
    pub underlying: i128,
    pub short_calls: i128,
    pub long_puts: i128,
    pub net_delta_bps: i128,
}

/// Common interface implemented by a Soroban adapter for an options market.
///
/// Adapters are responsible for protocol-specific token movement, collateral,
/// settlement, and for normalizing positions and Greeks.
#[soroban_sdk::contractclient(name = "OptionsMarketClient")]
pub trait OptionsMarket {
    fn write_covered_call(
        e: Env,
        underlying: Address,
        strike: Address,
        amount: i128,
        strike_price: i128,
        expiry: u64,
        account: Address,
    ) -> OptionPosition;

    fn buy_protective_put(
        e: Env,
        underlying: Address,
        strike: Address,
        amount: i128,
        strike_price: i128,
        expiry: u64,
        premium: i128,
        account: Address,
    ) -> OptionPosition;

    fn settle(e: Env, kind: OptionKind, amount: i128, account: Address) -> i128;

    fn hedge(e: Env, target_delta_bps: i128, account: Address);

    fn account(e: Env, account: Address) -> OptionsAccount;
}

#[contracttype]
#[derive(Clone)]
enum DataKey {
    Admin,
    Operator,
    Market,
    Protocol,
    UnderlyingAsset,
    StrikeAsset,
    MaxAbsDeltaBps,
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

fn validate_delta(delta_bps: i128) -> Result<(), Error> {
    if delta_bps < -BPS_SCALE || delta_bps > BPS_SCALE {
        return Err(Error::InvalidDelta);
    }
    Ok(())
}

fn abs(value: i128) -> i128 {
    if value < 0 {
        -value
    } else {
        value
    }
}

fn validate_position(
    position: &OptionPosition,
    kind: OptionKind,
    amount: i128,
    strike_price: i128,
    expiry: u64,
) -> Result<(), Error> {
    if position.kind != kind
        || position.amount != amount
        || position.amount <= 0
        || position.strike_price != strike_price
        || position.strike_price <= 0
        || position.expiry != expiry
        || position.premium < 0
    {
        return Err(Error::InvalidPosition);
    }
    validate_delta(position.delta_bps)
}

fn read_account(env: &Env) -> Result<OptionsAccount, Error> {
    env.storage()
        .instance()
        .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
    let market: Address = load(env, &DataKey::Market)?;
    let account_address = env.current_contract_address();
    let account = OptionsMarketClient::new(env, &market).account(&account_address);
    if account.underlying < 0 || account.short_calls < 0 || account.long_puts < 0 {
        return Err(Error::InvalidPosition);
    }
    validate_delta(account.net_delta_bps)?;
    Ok(account)
}

fn max_abs_delta_bps(env: &Env) -> Result<i128, Error> {
    load(env, &DataKey::MaxAbsDeltaBps)
}

fn ensure_delta_band(env: &Env) -> Result<OptionsAccount, Error> {
    let account = read_account(env)?;
    if abs(account.net_delta_bps) > max_abs_delta_bps(env)? {
        return Err(Error::DeltaTooHigh);
    }
    Ok(account)
}

fn load_assets(env: &Env) -> Result<(Address, Address), Error> {
    Ok((
        load(env, &DataKey::UnderlyingAsset)?,
        load(env, &DataKey::StrikeAsset)?,
    ))
}

#[contract]
pub struct OptionsStrategy;

#[contractimpl]
impl OptionsStrategy {
    /// Configure the strategy and its protocol-specific Soroban adapter.
    ///
    /// `max_abs_delta_bps` is the absolute delta band the operator may carry
    /// before hedging is required, normalized to the underlying notional.
    pub fn initialize(
        env: Env,
        admin: Address,
        operator: Address,
        protocol: Protocol,
        market: Address,
        underlying_asset: Address,
        strike_asset: Address,
        max_abs_delta_bps: i128,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if underlying_asset == strike_asset {
            return Err(Error::InvalidConfiguration);
        }
        if max_abs_delta_bps <= 0 || max_abs_delta_bps > BPS_SCALE {
            return Err(Error::InvalidDelta);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Operator, &operator);
        env.storage().instance().set(&DataKey::Protocol, &protocol);
        env.storage().instance().set(&DataKey::Market, &market);
        env.storage()
            .instance()
            .set(&DataKey::UnderlyingAsset, &underlying_asset);
        env.storage()
            .instance()
            .set(&DataKey::StrikeAsset, &strike_asset);
        env.storage()
            .instance()
            .set(&DataKey::MaxAbsDeltaBps, &max_abs_delta_bps);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("init"),),
            (admin, operator, protocol, market, max_abs_delta_bps),
        );
        Ok(())
    }

    /// Write a covered call against the strategy's underlying balance.
    pub fn write_covered_call(
        env: Env,
        amount: i128,
        strike_price: i128,
        expiry: u64,
    ) -> Result<(OptionPosition, OptionsAccount), Error> {
        require_operator(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if strike_price <= 0 {
            return Err(Error::InvalidStrike);
        }
        if expiry <= env.ledger().timestamp() {
            return Err(Error::InvalidExpiry);
        }

        let before = read_account(&env)?;
        if amount > before.underlying {
            return Err(Error::InsufficientUnderlying);
        }

        let market: Address = load(&env, &DataKey::Market)?;
        let (underlying, strike) = load_assets(&env)?;
        let account = env.current_contract_address();
        let position = OptionsMarketClient::new(&env, &market).write_covered_call(
            &underlying,
            &strike,
            &amount,
            &strike_price,
            &expiry,
            &account,
        );
        validate_position(
            &position,
            OptionKind::CoveredCall,
            amount,
            strike_price,
            expiry,
        )?;
        let options_account = ensure_delta_band(&env)?;

        env.events()
            .publish((symbol_short!("call"),), (amount, strike_price, expiry));
        Ok((position, options_account))
    }

    /// Buy a protective put, paying the quoted premium.
    pub fn buy_protective_put(
        env: Env,
        amount: i128,
        strike_price: i128,
        expiry: u64,
        premium: i128,
    ) -> Result<(OptionPosition, OptionsAccount), Error> {
        require_operator(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        if strike_price <= 0 {
            return Err(Error::InvalidStrike);
        }
        if expiry <= env.ledger().timestamp() {
            return Err(Error::InvalidExpiry);
        }
        if premium < 0 {
            return Err(Error::InvalidPremium);
        }

        let market: Address = load(&env, &DataKey::Market)?;
        let (underlying, strike) = load_assets(&env)?;
        let account = env.current_contract_address();
        let position = OptionsMarketClient::new(&env, &market).buy_protective_put(
            &underlying,
            &strike,
            &amount,
            &strike_price,
            &expiry,
            &premium,
            &account,
        );
        validate_position(
            &position,
            OptionKind::ProtectivePut,
            amount,
            strike_price,
            expiry,
        )?;
        let options_account = ensure_delta_band(&env)?;

        env.events()
            .publish((symbol_short!("put"),), (amount, strike_price, premium));
        Ok((position, options_account))
    }

    /// Settle expiring options and return the realized profit or loss.
    pub fn settle(
        env: Env,
        kind: OptionKind,
        amount: i128,
    ) -> Result<(i128, OptionsAccount), Error> {
        require_operator(&env)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }

        let market: Address = load(&env, &DataKey::Market)?;
        let account = env.current_contract_address();
        let pnl = OptionsMarketClient::new(&env, &market).settle(&kind, &amount, &account);
        let options_account = ensure_delta_band(&env)?;

        env.events()
            .publish((symbol_short!("settle"),), (kind, amount, pnl));
        Ok((pnl, options_account))
    }

    /// Re-hedge the net delta to a target inside the configured band.
    pub fn hedge(env: Env, target_delta_bps: i128) -> Result<OptionsAccount, Error> {
        require_operator(&env)?;
        validate_delta(target_delta_bps)?;
        if abs(target_delta_bps) > max_abs_delta_bps(&env)? {
            return Err(Error::DeltaTooHigh);
        }

        let market: Address = load(&env, &DataKey::Market)?;
        let account = env.current_contract_address();
        OptionsMarketClient::new(&env, &market).hedge(&target_delta_bps, &account);
        let options_account = ensure_delta_band(&env)?;

        env.events()
            .publish((symbol_short!("hedge"),), target_delta_bps);
        Ok(options_account)
    }

    /// Read the aggregate account from the configured options adapter.
    pub fn account(env: Env) -> Result<OptionsAccount, Error> {
        read_account(&env)
    }

    /// The configured protocol identifier.
    pub fn protocol(env: Env) -> Result<Protocol, Error> {
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        load(&env, &DataKey::Protocol)
    }

    /// The configured absolute delta band in basis points.
    pub fn max_abs_delta_bps(env: Env) -> Result<i128, Error> {
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        max_abs_delta_bps(&env)
    }

    /// Change the delta band. Only the admin can adjust it.
    pub fn set_max_abs_delta_bps(
        env: Env,
        admin: Address,
        max_abs_delta_bps: i128,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if max_abs_delta_bps <= 0 || max_abs_delta_bps > BPS_SCALE {
            return Err(Error::InvalidDelta);
        }
        env.storage()
            .instance()
            .set(&DataKey::MaxAbsDeltaBps, &max_abs_delta_bps);
        env.storage()
            .instance()
            .extend_ttl(TTL_LEDGERS, TTL_LEDGERS);
        env.events()
            .publish((symbol_short!("max_delta"),), max_abs_delta_bps);
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
