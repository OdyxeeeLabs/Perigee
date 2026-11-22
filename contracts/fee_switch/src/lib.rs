#![no_std]

//! # Perigee Fee Switch — governance-controlled fee activation & revenue use
//!
//! Closes OdyxeeeLabs/Perigee#666
//!
//! Source: contract.md § Tokenomics & Fees (#36)
//!
//! The fee switch is the governance-gated mechanism that turns protocol fees
//! *on*, routes accrued revenue, and optionally buys back / burns the
//! governance token. It does **not** charge the fees themselves — that is the
//! job of `fee_engine` — it only decides whether collection is live and where
//! the proceeds go.
//!
//! ## Activation
//!
//! - Fees start **off**. Only the governance admin (or a bound governor
//!   contract) may call `activate` / `deactivate`.
//! - While inactive, `collect` and `distribute` refuse to run.
//!
//! ## Revenue sharing
//!
//! Accrued fees are split by configurable basis-point weights that always sum
//! to 10_000:
//!
//! - `treasury_bps` — protocol treasury
//! - `stakers_bps` — staker / veToken rewards
//! - `buyback_bps` — buyback-and-burn budget
//!
//! ## Buyback / burn
//!
//! `execute_buyback` spends the buyback budget against a quoted token amount
//! and records burned supply. The actual DEX swap is left to an external
//! executor; this contract tracks the budget and the burn accounting.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, Symbol};

pub const BPS_DENOMINATOR: u32 = 10_000;
/// Maximum total fee rate the switch is allowed to enable (20%).
pub const MAX_FEE_BPS: u32 = 2_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    SwitchInactive = 4,
    InvalidBps = 5,
    InvalidAmount = 6,
    InsufficientBudget = 7,
    MathOverflow = 8,
    FeeTooHigh = 9,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeSwitchConfig {
    /// Swap / protocol fee in bps charged when the switch is on.
    pub fee_bps: u32,
    pub treasury_bps: u32,
    pub stakers_bps: u32,
    pub buyback_bps: u32,
    pub treasury: Address,
    pub stakers_recipient: Address,
    pub governance_token: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RevenueBuckets {
    pub treasury: i128,
    pub stakers: i128,
    pub buyback: i128,
    pub total_collected: i128,
    pub total_burned: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Active,
    Config,
    Buckets,
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

fn is_active(env: &Env) -> bool {
    env.storage()
        .instance()
        .get(&DataKey::Active)
        .unwrap_or(false)
}

fn load_config(env: &Env) -> Result<FeeSwitchConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

fn load_buckets(env: &Env) -> RevenueBuckets {
    env.storage().instance().get(&DataKey::Buckets).unwrap_or(
        RevenueBuckets {
            treasury: 0,
            stakers: 0,
            buyback: 0,
            total_collected: 0,
            total_burned: 0,
        },
    )
}

fn save_buckets(env: &Env, buckets: &RevenueBuckets) {
    env.storage().instance().set(&DataKey::Buckets, buckets);
}

fn validate_config(config: &FeeSwitchConfig) -> Result<(), Error> {
    if config.fee_bps > MAX_FEE_BPS {
        return Err(Error::FeeTooHigh);
    }
    let sum = config
        .treasury_bps
        .saturating_add(config.stakers_bps)
        .saturating_add(config.buyback_bps);
    if sum != BPS_DENOMINATOR {
        return Err(Error::InvalidBps);
    }
    Ok(())
}

#[contract]
pub struct FeeSwitch;

#[contractimpl]
impl FeeSwitch {
    pub fn initialize(env: Env, admin: Address, config: FeeSwitchConfig) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        validate_config(&config)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::Active, &false);
        save_buckets(
            &env,
            &RevenueBuckets {
                treasury: 0,
                stakers: 0,
                buyback: 0,
                total_collected: 0,
                total_burned: 0,
            },
        );
        Ok(())
    }

    pub fn activate(env: Env, admin: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::Active, &true);
        env.events()
            .publish((Symbol::new(&env, "fee_switch"), Symbol::new(&env, "on")), true);
        Ok(())
    }

    pub fn deactivate(env: Env, admin: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::Active, &false);
        env.events().publish(
            (Symbol::new(&env, "fee_switch"), Symbol::new(&env, "off")),
            false,
        );
        Ok(())
    }

    pub fn is_active(env: Env) -> bool {
        is_active(&env)
    }

    pub fn set_config(env: Env, admin: Address, config: FeeSwitchConfig) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        validate_config(&config)?;
        env.storage().instance().set(&DataKey::Config, &config);
        // Announce the split that applies to every accrual from now on so
        // indexers can tell which configuration was active for a given collect.
        env.events().publish(
            (Symbol::new(&env, "fee_switch"), Symbol::new(&env, "config")),
            (config.treasury_bps, config.stakers_bps, config.buyback_bps),
        );
        Ok(())
    }

    pub fn get_config(env: Env) -> Result<FeeSwitchConfig, Error> {
        load_config(&env)
    }

    pub fn get_buckets(env: Env) -> RevenueBuckets {
        load_buckets(&env)
    }

    /// Record fee revenue and split it into treasury / stakers / buyback buckets.
    pub fn collect(env: Env, amount: i128) -> Result<RevenueBuckets, Error> {
        if !is_active(&env) {
            return Err(Error::SwitchInactive);
        }
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let config = load_config(&env)?;
        let mut buckets = load_buckets(&env);

        let treasury_share = amount
            .checked_mul(config.treasury_bps as i128)
            .ok_or(Error::MathOverflow)?
            / BPS_DENOMINATOR as i128;
        let stakers_share = amount
            .checked_mul(config.stakers_bps as i128)
            .ok_or(Error::MathOverflow)?
            / BPS_DENOMINATOR as i128;
        // Remainder to buyback so shares always sum to `amount`.
        let buyback_share = amount
            .checked_sub(treasury_share)
            .ok_or(Error::MathOverflow)?
            .checked_sub(stakers_share)
            .ok_or(Error::MathOverflow)?;

        buckets.treasury = buckets
            .treasury
            .checked_add(treasury_share)
            .ok_or(Error::MathOverflow)?;
        buckets.stakers = buckets
            .stakers
            .checked_add(stakers_share)
            .ok_or(Error::MathOverflow)?;
        buckets.buyback = buckets
            .buyback
            .checked_add(buyback_share)
            .ok_or(Error::MathOverflow)?;
        buckets.total_collected = buckets
            .total_collected
            .checked_add(amount)
            .ok_or(Error::MathOverflow)?;

        save_buckets(&env, &buckets);
        env.events().publish(
            (Symbol::new(&env, "fee_switch"), Symbol::new(&env, "collect")),
            amount,
        );
        Ok(buckets)
    }

    /// Spend buyback budget and record burned governance-token amount.
    pub fn execute_buyback(
        env: Env,
        admin: Address,
        spend: i128,
        tokens_burned: i128,
    ) -> Result<RevenueBuckets, Error> {
        require_admin(&env, &admin)?;
        if !is_active(&env) {
            return Err(Error::SwitchInactive);
        }
        if spend <= 0 || tokens_burned <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut buckets = load_buckets(&env);
        if spend > buckets.buyback {
            return Err(Error::InsufficientBudget);
        }
        buckets.buyback -= spend;
        buckets.total_burned = buckets
            .total_burned
            .checked_add(tokens_burned)
            .ok_or(Error::MathOverflow)?;
        save_buckets(&env, &buckets);
        env.events().publish(
            (Symbol::new(&env, "fee_switch"), Symbol::new(&env, "burn")),
            tokens_burned,
        );
        Ok(buckets)
    }

    /// Quote the fee due on a notional amount when the switch is active.
    pub fn quote_fee(env: Env, notional: i128) -> Result<i128, Error> {
        if notional < 0 {
            return Err(Error::InvalidAmount);
        }
        if !is_active(&env) {
            return Ok(0);
        }
        let config = load_config(&env)?;
        Ok(notional
            .checked_mul(config.fee_bps as i128)
            .ok_or(Error::MathOverflow)?
            / BPS_DENOMINATOR as i128)
    }
}

#[cfg(test)]
mod test;
