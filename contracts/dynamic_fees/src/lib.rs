#![no_std]

//! # Perigee Dynamic Fee Adjustment
//!
//! Closes OdyxeeeLabs/Perigee#667
//!
//! Source: contract.md § Tokenomics & Fees (#37)
//!
//! Computes a protocol fee in basis points from three independent signals:
//!
//! 1. **Utilization** — higher pool utilization → higher fee (discourages
//!    draining thin liquidity).
//! 2. **Volatility** — higher observed volatility → higher fee (prices risk).
//! 3. **Competitor reference** — optional external benchmark; the local fee
//!    is clamped so it cannot undercut by more than `max_undercut_bps` or
//!    overshoot by more than `max_premium_bps`.
//!
//! The final fee is always clamped to `[min_fee_bps, max_fee_bps]`.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env};

pub const BPS_DENOMINATOR: u32 = 10_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    InvalidConfig = 4,
    InvalidInput = 5,
    MathOverflow = 6,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DynamicFeeConfig {
    pub base_fee_bps: u32,
    pub min_fee_bps: u32,
    pub max_fee_bps: u32,
    /// Fee added at 100% utilization.
    pub utilization_weight_bps: u32,
    /// Fee added per 100% annualised volatility (scaled by 10_000).
    pub volatility_weight_bps: u32,
    /// How far below a competitor quote we may go.
    pub max_undercut_bps: u32,
    /// How far above a competitor quote we may go.
    pub max_premium_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeSignals {
    /// Pool utilization in bps (0 = empty, 10_000 = fully utilised).
    pub utilization_bps: u32,
    /// Observed volatility in bps (e.g. 1_500 = 15%).
    pub volatility_bps: u32,
    /// Optional competitor fee in bps; `None` encoded as 0 means unused.
    pub competitor_fee_bps: u32,
    pub competitor_enabled: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeQuote {
    pub fee_bps: u32,
    pub utilization_component: u32,
    pub volatility_component: u32,
    pub after_competitor_clamp: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Config,
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

fn load_config(env: &Env) -> Result<DynamicFeeConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

fn validate_config(c: &DynamicFeeConfig) -> Result<(), Error> {
    if c.min_fee_bps > c.max_fee_bps {
        return Err(Error::InvalidConfig);
    }
    if c.base_fee_bps > c.max_fee_bps || c.max_fee_bps > BPS_DENOMINATOR {
        return Err(Error::InvalidConfig);
    }
    Ok(())
}

fn clamp(v: u32, lo: u32, hi: u32) -> u32 {
    if v < lo {
        lo
    } else if v > hi {
        hi
    } else {
        v
    }
}

/// Pure fee computation — unit-tested without the contract host.
pub fn compute_dynamic_fee(config: &DynamicFeeConfig, signals: &FeeSignals) -> Result<FeeQuote, Error> {
    if signals.utilization_bps > BPS_DENOMINATOR {
        return Err(Error::InvalidInput);
    }

    // utilization_component = weight * utilization / 10_000
    let utilization_component = (config.utilization_weight_bps as u64)
        .saturating_mul(signals.utilization_bps as u64)
        / BPS_DENOMINATOR as u64;

    // volatility_component = weight * volatility / 10_000
    let volatility_component = (config.volatility_weight_bps as u64)
        .saturating_mul(signals.volatility_bps as u64)
        / BPS_DENOMINATOR as u64;

    let mut fee = (config.base_fee_bps as u64)
        .saturating_add(utilization_component)
        .saturating_add(volatility_component);

    // Intermediate sums are u64 and can exceed u32::MAX for extreme inputs; saturate
    // before any narrowing cast so a huge value can never wrap to a small fee.
    fee = fee.min(u32::MAX as u64);
    let mut after_competitor = fee as u32;
    if signals.competitor_enabled {
        let comp = signals.competitor_fee_bps as u64;
        let floor = comp.saturating_sub(config.max_undercut_bps as u64);
        let ceil = comp.saturating_add(config.max_premium_bps as u64);
        if fee < floor {
            fee = floor;
        } else if fee > ceil {
            fee = ceil;
        }
        fee = fee.min(u32::MAX as u64);
        after_competitor = fee as u32;
    }

    let fee_bps = clamp(fee as u32, config.min_fee_bps, config.max_fee_bps);

    Ok(FeeQuote {
        fee_bps,
        utilization_component: utilization_component.min(u32::MAX as u64) as u32,
        volatility_component: volatility_component.min(u32::MAX as u64) as u32,
        after_competitor_clamp: after_competitor,
    })
}

#[contract]
pub struct DynamicFees;

#[contractimpl]
impl DynamicFees {
    pub fn initialize(env: Env, admin: Address, config: DynamicFeeConfig) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        validate_config(&config)?;
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    pub fn set_config(env: Env, admin: Address, config: DynamicFeeConfig) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        validate_config(&config)?;
        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    pub fn get_config(env: Env) -> Result<DynamicFeeConfig, Error> {
        load_config(&env)
    }

    pub fn quote(env: Env, signals: FeeSignals) -> Result<FeeQuote, Error> {
        let config = load_config(&env)?;
        compute_dynamic_fee(&config, &signals)
    }
}

#[cfg(test)]
mod test;
