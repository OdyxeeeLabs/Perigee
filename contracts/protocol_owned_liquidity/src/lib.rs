#![no_std]

//! # Perigee Protocol-Owned Liquidity (POL)
//!
//! Closes OdyxeeeLabs/Perigee#665
//!
//! Source: contract.md § Tokenomics & Fees (#35)
//!
//! Olympus-style bonding and protocol-owned liquidity management:
//!
//! - **Bonding**: users deposit a quote asset at a discount to the oracle
//!   price and vest governance tokens over `vesting_ledgers`.
//! - **POL deployment**: governance deploys accumulated reserves into an
//!   external LP / pool address and tracks owned LP units.
//! - **Yield capture**: yield reported against deployed POL is recorded and
//!   can be swept to the treasury.
//!
//! Discounted bond pricing:
//!
//! ```text
//! tokens_out = deposit_value * (10_000 + discount_bps) / 10_000 / token_price
//! ```

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol};

pub const BPS_DENOMINATOR: u32 = 10_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    InvalidAmount = 4,
    InvalidConfig = 5,
    BondNotFound = 6,
    BondNotMature = 7,
    AlreadyClaimed = 8,
    InsufficientReserves = 9,
    MathOverflow = 10,
    NothingToSweep = 11,
    /// The quote's `expiry_ledger` is in the past.
    QuoteExpired = 12,
    /// Settlement would mint fewer tokens than the caller's `min_tokens_out`.
    SlippageExceeded = 13,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolConfig {
    pub governance_token: Address,
    pub treasury: Address,
    /// Discount granted to bonders, in bps (e.g. 500 = 5% below market).
    pub discount_bps: u32,
    pub vesting_ledgers: u32,
    /// Maximum outstanding unvested bond liability in token units.
    pub max_bond_liability: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BondPosition {
    pub bonder: Address,
    pub token_amount: i128,
    pub deposit_value: i128,
    pub start_ledger: u32,
    pub mature_ledger: u32,
    pub claimed: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolState {
    /// Quote-asset reserves held by the protocol.
    pub reserves: i128,
    /// LP units currently deployed.
    pub deployed_lp: i128,
    /// Cumulative yield captured.
    pub yield_captured: i128,
    /// Unswept yield balance.
    pub yield_pending: i128,
    /// Outstanding unvested bond tokens.
    pub bond_liability: i128,
    pub next_bond_id: u64,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Config,
    State,
    Bond(u64),
}

fn require_admin(env: &Env, caller: &Address) -> Result<(), Error> {
    let admin: Address = env
        .storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)?;
    if caller != &admin {
        return Err(Error::Unauthorized);
    }
    caller.require_auth();
    Ok(())
}

fn load_config(env: &Env) -> Result<PolConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

fn load_state(env: &Env) -> PolState {
    env.storage().instance().get(&DataKey::State).unwrap_or(PolState {
        reserves: 0,
        deployed_lp: 0,
        yield_captured: 0,
        yield_pending: 0,
        bond_liability: 0,
        next_bond_id: 1,
    })
}

fn save_state(env: &Env, state: &PolState) {
    env.storage().instance().set(&DataKey::State, state);
}

/// Pure bond pricing helper.
pub fn quote_bond_tokens(
    deposit_value: i128,
    token_price: i128,
    discount_bps: u32,
) -> Result<i128, Error> {
    if deposit_value <= 0 || token_price <= 0 {
        return Err(Error::InvalidAmount);
    }
    if discount_bps > BPS_DENOMINATOR {
        return Err(Error::InvalidConfig);
    }
    let boosted = deposit_value
        .checked_mul((BPS_DENOMINATOR + discount_bps) as i128)
        .ok_or(Error::MathOverflow)?
        / BPS_DENOMINATOR as i128;
    Ok(boosted
        .checked_mul(BPS_DENOMINATOR as i128)
        .ok_or(Error::MathOverflow)?
        / token_price)
}

#[contract]
pub struct ProtocolOwnedLiquidity;

#[contractimpl]
impl ProtocolOwnedLiquidity {
    pub fn initialize(env: Env, admin: Address, config: PolConfig) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if config.discount_bps > BPS_DENOMINATOR || config.vesting_ledgers == 0 {
            return Err(Error::InvalidConfig);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Config, &config);
        save_state(
            &env,
            &PolState {
                reserves: 0,
                deployed_lp: 0,
                yield_captured: 0,
                yield_pending: 0,
                bond_liability: 0,
                next_bond_id: 1,
            },
        );
        Ok(())
    }

    pub fn get_state(env: Env) -> PolState {
        load_state(&env)
    }

    pub fn get_config(env: Env) -> Result<PolConfig, Error> {
        load_config(&env)
    }

    /// Deposit `deposit_value` of quote asset at oracle `token_price` (both in
    /// 1e4 fixed-point style units) and open a vesting bond.
    pub fn bond(
        env: Env,
        bonder: Address,
        deposit_value: i128,
        token_price: i128,
    ) -> Result<(u64, i128), Error> {
        Self::bond_inner(&env, bonder, deposit_value, token_price)
    }

    /// Tokens a bond of `deposit_value` at `token_price` would settle for under
    /// the current configuration. Uses exactly the pricing `bond` settles with.
    pub fn quote_bond(env: Env, deposit_value: i128, token_price: i128) -> Result<i128, Error> {
        let config = load_config(&env)?;
        quote_bond_tokens(deposit_value, token_price, config.discount_bps)
    }

    /// Bond against a quote with an expiry and a slippage bound.
    ///
    /// Fails with [`Error::QuoteExpired`] when the current ledger is past
    /// `expiry_ledger` (the quote is valid *through* that ledger), and with
    /// [`Error::SlippageExceeded`] when settlement would mint fewer than
    /// `min_tokens_out`. Both checks run before any state is written.
    pub fn bond_with_quote(
        env: Env,
        bonder: Address,
        deposit_value: i128,
        token_price: i128,
        min_tokens_out: i128,
        expiry_ledger: u32,
    ) -> Result<(u64, i128), Error> {
        if env.ledger().sequence() > expiry_ledger {
            return Err(Error::QuoteExpired);
        }
        let config = load_config(&env)?;
        let tokens = quote_bond_tokens(deposit_value, token_price, config.discount_bps)?;
        if tokens < min_tokens_out {
            return Err(Error::SlippageExceeded);
        }
        Self::bond_inner(&env, bonder, deposit_value, token_price)
    }

    fn bond_inner(
        env: &Env,
        bonder: Address,
        deposit_value: i128,
        token_price: i128,
    ) -> Result<(u64, i128), Error> {
        let env = env.clone();
        bonder.require_auth();
        let config = load_config(&env)?;
        let tokens = quote_bond_tokens(deposit_value, token_price, config.discount_bps)?;
        // A deposit too small to buy a single token unit would open a zero-value
        // claim; reject it instead.
        if tokens <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut state = load_state(&env);

        let next_liability = state
            .bond_liability
            .checked_add(tokens)
            .ok_or(Error::MathOverflow)?;
        if next_liability > config.max_bond_liability {
            return Err(Error::InsufficientReserves);
        }

        let bond_id = state.next_bond_id;
        state.next_bond_id += 1;
        state.reserves = state
            .reserves
            .checked_add(deposit_value)
            .ok_or(Error::MathOverflow)?;
        state.bond_liability = next_liability;

        let start = env.ledger().sequence();
        let position = BondPosition {
            bonder: bonder.clone(),
            token_amount: tokens,
            deposit_value,
            start_ledger: start,
            mature_ledger: start + config.vesting_ledgers,
            claimed: false,
        };
        env.storage()
            .persistent()
            .set(&DataKey::Bond(bond_id), &position);
        save_state(&env, &state);

        env.events().publish(
            (Symbol::new(&env, "pol"), Symbol::new(&env, "bond")),
            (bond_id, tokens),
        );
        Ok((bond_id, tokens))
    }

    pub fn claim_bond(env: Env, bond_id: u64, bonder: Address) -> Result<i128, Error> {
        bonder.require_auth();
        let key = DataKey::Bond(bond_id);
        let mut position: BondPosition = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::BondNotFound)?;
        if position.bonder != bonder {
            return Err(Error::Unauthorized);
        }
        if position.claimed {
            return Err(Error::AlreadyClaimed);
        }
        if env.ledger().sequence() < position.mature_ledger {
            return Err(Error::BondNotMature);
        }

        position.claimed = true;
        env.storage().persistent().set(&key, &position);

        let mut state = load_state(&env);
        state.bond_liability = state
            .bond_liability
            .saturating_sub(position.token_amount);
        save_state(&env, &state);

        env.events().publish(
            (Symbol::new(&env, "pol"), Symbol::new(&env, "claim")),
            (bond_id, position.token_amount),
        );
        Ok(position.token_amount)
    }

    /// Deploy reserves into an LP position (accounting only — transfer is external).
    pub fn deploy_pol(
        env: Env,
        admin: Address,
        reserve_amount: i128,
        lp_units: i128,
    ) -> Result<PolState, Error> {
        require_admin(&env, &admin)?;
        if reserve_amount <= 0 || lp_units <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut state = load_state(&env);
        if reserve_amount > state.reserves {
            return Err(Error::InsufficientReserves);
        }
        state.reserves -= reserve_amount;
        state.deployed_lp = state
            .deployed_lp
            .checked_add(lp_units)
            .ok_or(Error::MathOverflow)?;
        save_state(&env, &state);
        env.events().publish(
            (Symbol::new(&env, "pol"), Symbol::new(&env, "deploy")),
            (reserve_amount, lp_units),
        );
        Ok(state)
    }

    /// Record yield earned by deployed POL.
    pub fn capture_yield(env: Env, admin: Address, amount: i128) -> Result<PolState, Error> {
        require_admin(&env, &admin)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut state = load_state(&env);
        state.yield_captured = state
            .yield_captured
            .checked_add(amount)
            .ok_or(Error::MathOverflow)?;
        state.yield_pending = state
            .yield_pending
            .checked_add(amount)
            .ok_or(Error::MathOverflow)?;
        save_state(&env, &state);
        Ok(state)
    }

    /// Sweep pending yield to the treasury accounting bucket (returns amount).
    pub fn sweep_yield(env: Env, admin: Address) -> Result<i128, Error> {
        require_admin(&env, &admin)?;
        let mut state = load_state(&env);
        if state.yield_pending <= 0 {
            return Err(Error::NothingToSweep);
        }
        let amount = state.yield_pending;
        state.yield_pending = 0;
        // Yield returns as reserves the treasury can redeploy.
        state.reserves = state
            .reserves
            .checked_add(amount)
            .ok_or(Error::MathOverflow)?;
        save_state(&env, &state);
        env.events().publish(
            (Symbol::new(&env, "pol"), Symbol::new(&env, "sweep")),
            amount,
        );
        Ok(amount)
    }

    pub fn get_bond(env: Env, bond_id: u64) -> Result<BondPosition, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Bond(bond_id))
            .ok_or(Error::BondNotFound)
    }
}

#[cfg(test)]
mod test;
