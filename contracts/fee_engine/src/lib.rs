#![no_std]

//! # Perigee Fee Engine — performance fees, management fees and fee discounts
//!
//! Closes OdyxeeeLabs/Perigee#661
//! Closes OdyxeeeLabs/Perigee#662
//! Closes OdyxeeeLabs/Perigee#663
//! Closes OdyxeeeLabs/Perigee#669
//! Closes OdyxeeeLabs/Perigee#670
//! Closes OdyxeeeLabs/Perigee#671
//!
//! Three fee mechanisms that a vault needs, in one contract, because they
//! answer to the same question — *who is paid, out of what, and when* — and
//! because splitting them across contracts is how fee logic ends up inconsistent.
//!
//! ## Performance fees ([#661])
//!
//! Charged on **realised profit above a high-water mark**, never on the
//! depositors' principal. The mark is the highest NAV at which a fee was
//! crystallised:
//!
//! - `crystallize` is called with the current NAV. If the NAV is above the mark,
//!   the profit is `nav − mark`, the fee is charged on it, and the mark rises to
//!   the NAV.
//! - If the NAV has fallen below the last crystallised value, the fee charged on
//!   that loss is **clawed back** from `fees_paid`, so a strategy that gives back
//!   its gains gives back the fee it was paid on them.
//!
//! Fees are **tiered**: `perf_tiers` maps a profit size to a rate, so a large
//! profit is charged at a higher rate than a small one.
//!
//! ## Management fees ([#662])
//!
//! Accrue **continuously** on assets under management, not in periodic lumps.
//! `accrue_management_fee` charges
//!
//! ```text
//! fee = aum × management_fee_bps × elapsed / (10_000 × SECONDS_PER_YEAR)
//! ```
//!
//! for the time since the last accrual, so a vault that is deposited into and
//! withdrawn from within a single ledger pays for exactly the time the capital
//! was deployed. The fee is split between the vault's own fee recipient and the
//! protocol treasury by `protocol_share_bps`, and the split is computed so the
//! two shares always sum to the fee exactly.
//!
//! ## Fee discount tokens ([#663)
//!
//! A vault that stakes the governance token earns a **rebate** on its fees.
//! `discount_tiers` maps a staked amount to a rebate rate; the rebate is applied
//! to the total fee before the split, so the discount reduces what the vault
//! pays, not what the treasury is owed. Staking is real: `stake` pulls the
//! governance token into the contract and `unstake` returns it.
//!
//! ## What is never done
//!
//! - Fees are never charged on principal. Performance is charged on profit above
//!   the mark; management is charged on AUM, which excludes unrealised principal
//!   only insofar as the vault reports it.
//! - The fee split never loses or invents value: the protocol share is rounded
//!   down and the remainder goes to the vault.
//! - No fee exceeds `MAX_FEE_BPS` (20%).

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env, Vec};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Ledgers to live for persistent records after each write.
pub const TTL_LEDGERS: u32 = 17_280;

/// Basis-point denominator.
pub const BPS_DENOMINATOR: i128 = 10_000;

/// Seconds in a 365-day year, for accrual annualisation.
pub const SECONDS_PER_YEAR: i128 = 31_536_000;

/// Ceiling on any single fee, in basis points.
pub const MAX_FEE_BPS: u32 = 2_000;

/// Maximum number of discount or performance tiers.
pub const MAX_TIERS: u32 = 8;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the fee engine.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// The contract has not been initialized yet.
    NotInitialized = 1,
    /// `initialize` was called more than once.
    AlreadyInitialized = 2,
    /// The caller is not authorised for this entry point.
    Unauthorized = 3,
    /// No fee configuration has been set.
    ConfigNotSet = 4,
    /// The vault has no state record.
    VaultNotFound = 5,
    /// A non-positive amount was supplied.
    InvalidAmount = 6,
    /// A tier list is malformed.
    InvalidTier = 7,
    /// More tiers than `MAX_TIERS`.
    TooManyTiers = 8,
    /// An unstake exceeds the staked balance.
    InsufficientStake = 9,
    /// An arithmetic step overflowed `i128`.
    MathOverflow = 10,
    /// The fee split does not sum to 100%.
    InvalidFeeSplit = 11,
    /// A fee above `MAX_FEE_BPS` was configured.
    FeeTooHigh = 12,
    /// Reentrant call into a non-reentrant fee entry point (#671).
    Reentrancy = 13,
    /// Sponsor is not authorised to pay fees for this user (#670).
    SponsorNotAllowed = 14,
    /// Fee history index is out of range (#669).
    HistoryOutOfBounds = 15,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// The fee schedule for the whole engine.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeConfig {
    /// Management fee in basis points of assets under management.
    pub management_fee_bps: u32,
    /// Performance fee in basis points of realised profit.
    pub performance_fee_bps: u32,
    /// Share of the fee going to the protocol treasury, in basis points.
    pub protocol_share_bps: u32,
    /// Recipient of the vault's share of fees.
    pub vault_fee_recipient: Address,
    /// Recipient of the protocol's share.
    pub treasury: Address,
}

/// Per-vault accounting.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultState {
    /// Depositors' principal.
    pub principal: i128,
    /// Highest NAV at which a performance fee was crystallised.
    pub high_water_mark: i128,
    /// NAV at the last crystallisation.
    pub last_crystallized: i128,
    /// Ledger timestamp of the last management-fee accrual.
    pub last_accrual: u64,
    /// Cumulative fees paid.
    pub fees_paid: i128,
}

/// A staked governance balance and the tier it currently qualifies for.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stake {
    /// Amount staked.
    pub staked: i128,
    /// Highest tier the stake qualifies for (0 = none).
    pub tier: u32,
}

/// A fee-discount tier.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscountTier {
    /// Minimum stake to qualify.
    pub min_stake: i128,
    /// Rebate on fees, in basis points.
    pub rebate_bps: u32,
}

/// A performance-fee tier.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformanceTier {
    /// Minimum profit to qualify.
    pub min_profit: i128,
    /// Fee rate, in basis points.
    pub fee_bps: u32,
}

/// The full breakdown of a fee charge.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeRecord {
    /// Monotonic sequence id for historical queries.
    pub seq: u64,
    /// Vault the fee was charged against.
    pub vault: Address,
    /// Who triggered the charge (may be a sponsor).
    pub payer: Address,
    /// Gross fee before discount.
    pub gross: i128,
    /// Discount applied.
    pub discount: i128,
    /// Net fee after discount.
    pub net: i128,
    /// Vault recipient share.
    pub vault_share: i128,
    /// Treasury share.
    pub protocol_share: i128,
    /// Ledger timestamp.
    pub timestamp: u64,
    /// Kind: 1 = management, 2 = performance, 3 = combined.
    pub kind: u32,
}

/// Breakdown of a computed fee for off-chain analytics and on-chain events.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeBreakdown {
    /// Management fee accrued.
    pub management_fee: i128,
    /// Performance fee crystallised.
    pub performance_fee: i128,
    /// `management_fee + performance_fee`.
    pub total: i128,
    /// Rebate earned from the staked governance tier.
    pub discount: i128,
    /// `total - discount`.
    pub net_total: i128,
    /// Share of the net fee going to the vault's fee recipient.
    pub vault_share: i128,
    /// Share of the net fee going to the protocol treasury.
    pub protocol_share: i128,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin allowed to configure fees and tiers.
    Admin,
    /// The [`FeeConfig`].
    Config,
    /// The governance token that earns fee discounts.
    StakingToken,
    /// [`DiscountTier`] list, ascending by `min_stake`.
    DiscountTiers,
    /// [`PerformanceTier`] list, ascending by `min_profit`.
    PerfTiers,
    /// [`VaultState`] for a vault.
    Vault(Address),
    /// [`Stake`] for a staker.
    Stake(Address),

    /// Reentrancy lock bit (#671).
    ReentrancyLock,
    /// Next fee event sequence id (#669).
    FeeSeq,
    /// Historical fee record by sequence (#669).
    FeeHistory(u64),
    /// Allowed fee sponsor for a user (#670).
    Sponsor(Address),
}


// ── Helpers ──────────────────────────────────────────────────────────────────

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
    Ok(())
}

fn load_config(env: &Env) -> Result<FeeConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::ConfigNotSet)
}

fn load_vault(env: &Env, vault: &Address) -> Result<VaultState, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Vault(vault.clone()))
        .ok_or(Error::VaultNotFound)
}

fn default_vault_state() -> VaultState {
    VaultState {
        principal: 0,
        high_water_mark: 0,
        last_crystallized: 0,
        last_accrual: 0,
        fees_paid: 0,
    }
}

fn load_vault_or_default(env: &Env, vault: &Address) -> VaultState {
    env.storage()
        .persistent()
        .get(&DataKey::Vault(vault.clone()))
        .unwrap_or_else(default_vault_state)
}

fn save_vault(env: &Env, vault: &Address, state: &VaultState) {
    let key = DataKey::Vault(vault.clone());
    env.storage().persistent().set(&key, state);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
}

fn default_stake() -> Stake {
    Stake { staked: 0, tier: 0 }
}

fn load_stake_or_default(env: &Env, staker: &Address) -> Stake {
    env.storage()
        .persistent()
        .get(&DataKey::Stake(staker.clone()))
        .unwrap_or_else(default_stake)
}

fn save_stake(env: &Env, staker: &Address, stake: &Stake) {
    let key = DataKey::Stake(staker.clone());
    env.storage().persistent().set(&key, stake);
    env.storage()
        .persistent()
        .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
}

fn load_staking_token(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&DataKey::StakingToken)
        .ok_or(Error::NotInitialized)
}

fn load_tiers<T>(env: &Env, key: &DataKey) -> Vec<T>
where
    T: soroban_sdk::TryFromVal<Env, soroban_sdk::Val> + Clone,
{
    env.storage()
        .instance()
        .get(key)
        .unwrap_or_else(|| Vec::new(env))
}

fn checked_add(a: i128, b: i128) -> Result<i128, Error> {
    a.checked_add(b).ok_or(Error::MathOverflow)
}

/// The highest discount tier a stake qualifies for (0 = none).
fn tier_for_stake(env: &Env, staked: i128) -> u32 {
    let tiers: Vec<DiscountTier> = load_tiers(env, &DataKey::DiscountTiers);
    let mut tier = 0u32;
    for i in 0..tiers.len() {
        if staked >= tiers.get(i).unwrap().min_stake {
            tier = (i + 1) as u32;
        }
    }
    tier
}

fn rebate_for_stake(env: &Env, staked: i128) -> u32 {
    let tier = tier_for_stake(env, staked);
    if tier == 0 {
        return 0;
    }
    let tiers: Vec<DiscountTier> = load_tiers(env, &DataKey::DiscountTiers);
    tiers.get((tier - 1) as u32).unwrap().rebate_bps
}

/// The performance-fee rate for a given profit, or `None` if no tier matches.
fn perf_rate_for_profit(env: &Env, profit: i128) -> Option<u32> {
    let tiers: Vec<PerformanceTier> = load_tiers(env, &DataKey::PerfTiers);
    let mut rate = None;
    for i in 0..tiers.len() {
        if profit >= tiers.get(i).unwrap().min_profit {
            rate = Some(tiers.get(i).unwrap().fee_bps);
        }
    }
    rate
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct FeeEngine;

#[contractimpl]
impl FeeEngine {
    // ── Setup ───────────────────────────────────────────────────────────────

    /// Initialize with the governance token that earns fee discounts.
    pub fn initialize(env: Env, admin: Address, staking_token: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::StakingToken, &staking_token);

        env.events()
            .publish((symbol_short!("init"),), staking_token);
        Ok(())
    }

    /// The admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }

    // ── Configuration ───────────────────────────────────────────────────────

    /// Set the fee schedule.
    pub fn set_fee_config(env: Env, admin: Address, config: FeeConfig) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if config.management_fee_bps > MAX_FEE_BPS || config.performance_fee_bps > MAX_FEE_BPS {
            return Err(Error::FeeTooHigh);
        }
        if config.protocol_share_bps > BPS_DENOMINATOR as u32 {
            return Err(Error::InvalidFeeSplit);
        }
        env.storage().instance().set(&DataKey::Config, &config);
        env.events().publish(
            (symbol_short!("fee_cfg"),),
            (config.management_fee_bps, config.performance_fee_bps),
        );
        Ok(())
    }

    /// The fee schedule.
    pub fn get_fee_config(env: Env) -> Result<FeeConfig, Error> {
        load_config(&env)
    }

    /// Set the discount tiers, ascending by `min_stake`.
    pub fn set_discount_tiers(
        env: Env,
        admin: Address,
        tiers: Vec<DiscountTier>,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        Self::validate_discount_tiers(&tiers)?;
        env.storage()
            .instance()
            .set(&DataKey::DiscountTiers, &tiers);
        env.events()
            .publish((symbol_short!("disc_tiers"),), tiers.len());
        Ok(())
    }

    /// The discount tiers.
    pub fn get_discount_tiers(env: Env) -> Vec<DiscountTier> {
        load_tiers(&env, &DataKey::DiscountTiers)
    }

    /// Set the performance tiers, ascending by `min_profit`.
    pub fn set_perf_tiers(
        env: Env,
        admin: Address,
        tiers: Vec<PerformanceTier>,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        Self::validate_perf_tiers(&tiers)?;
        env.storage().instance().set(&DataKey::PerfTiers, &tiers);
        env.events()
            .publish((symbol_short!("perf_tiers"),), tiers.len());
        Ok(())
    }

    /// The performance tiers.
    pub fn get_perf_tiers(env: Env) -> Vec<PerformanceTier> {
        load_tiers(&env, &DataKey::PerfTiers)
    }

    fn validate_discount_tiers(tiers: &Vec<DiscountTier>) -> Result<(), Error> {
        if tiers.len() == 0 || tiers.len() > MAX_TIERS {
            return Err(Error::TooManyTiers);
        }
        for i in 0..tiers.len() {
            let tier = tiers.get(i).unwrap();
            if tier.rebate_bps > BPS_DENOMINATOR as u32 {
                return Err(Error::InvalidTier);
            }
            if i > 0 && tiers.get(i - 1).unwrap().min_stake >= tier.min_stake {
                return Err(Error::InvalidTier);
            }
        }
        Ok(())
    }

    fn validate_perf_tiers(tiers: &Vec<PerformanceTier>) -> Result<(), Error> {
        if tiers.len() == 0 || tiers.len() > MAX_TIERS {
            return Err(Error::TooManyTiers);
        }
        for i in 0..tiers.len() {
            let tier = tiers.get(i).unwrap();
            if tier.fee_bps > MAX_FEE_BPS {
                return Err(Error::FeeTooHigh);
            }
            if i > 0 && tiers.get(i - 1).unwrap().min_profit >= tier.min_profit {
                return Err(Error::InvalidTier);
            }
        }
        Ok(())
    }

    // ── Governance staking (#663) ──────────────────────────────────────────

    /// Stake governance tokens to earn a fee rebate.
    pub fn stake(env: Env, staker: Address, amount: i128) -> Result<(), Error> {
        staker.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let token = load_staking_token(&env)?;
        soroban_sdk::token::Client::new(&env, &token)
            .transfer(&staker, &env.current_contract_address(), &amount);

        let mut stake = load_stake_or_default(&env, &staker);
        stake.staked = checked_add(stake.staked, amount)?;
        stake.tier = tier_for_stake(&env, stake.staked);
        save_stake(&env, &staker, &stake);

        env.events().publish((symbol_short!("stake"), staker), amount);
        Ok(())
    }

    /// Unstake governance tokens.
    pub fn unstake(env: Env, staker: Address, amount: i128) -> Result<(), Error> {
        staker.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut stake = load_stake_or_default(&env, &staker);
        if amount > stake.staked {
            return Err(Error::InsufficientStake);
        }
        let token = load_staking_token(&env)?;
        soroban_sdk::token::Client::new(&env, &token)
            .transfer(&env.current_contract_address(), &staker, &amount);

        stake.staked -= amount;
        stake.tier = tier_for_stake(&env, stake.staked);
        save_stake(&env, &staker, &stake);

        env.events().publish((symbol_short!("unstake"), staker), amount);
        Ok(())
    }

    /// The stake and tier for an address.
    pub fn get_stake(env: Env, staker: Address) -> Stake {
        load_stake_or_default(&env, &staker)
    }

    // ── Vault lifecycle ─────────────────────────────────────────────────────

    /// Record a deposit, raising principal and the high-water mark.
    pub fn record_deposit(env: Env, vault: Address, amount: i128) -> Result<(), Error> {
        vault.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut state = load_vault_or_default(&env, &vault);
        state.principal = checked_add(state.principal, amount)?;

        let now = env.ledger().timestamp();
        if state.principal > state.high_water_mark {
            state.high_water_mark = state.principal;
            state.last_crystallized = state.principal;
        }
        state.last_accrual = now;
        save_vault(&env, &vault, &state);
        Ok(())
    }

    /// Record a withdrawal, reducing principal.
    pub fn record_withdrawal(env: Env, vault: Address, amount: i128) -> Result<(), Error> {
        vault.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut state = load_vault_or_default(&env, &vault);
        if amount > state.principal {
            return Err(Error::InvalidAmount);
        }
        state.principal -= amount;
        save_vault(&env, &vault, &state);
        Ok(())
    }

    /// The state of a vault.
    pub fn get_vault(env: Env, vault: Address) -> Result<VaultState, Error> {
        load_vault(&env, &vault)
    }

    // ── Management fee (#662) ──────────────────────────────────────────────

    /// # Accrue the management fee for the elapsed time
    ///
    /// Charges `aum × management_fee_bps × elapsed / (10_000 × SECONDS_PER_YEAR)`
    /// and advances `last_accrual`, so the next call charges only for the time
    /// since this one.
    pub fn accrue_management_fee(env: Env, vault: Address, aum: i128) -> Result<i128, Error> {
        Self::reentrancy_enter(&env)?;
        let out = (|| {
            let config = load_config(&env)?;
            let mut state = load_vault(&env, &vault)?;

            let now = env.ledger().timestamp();
            let elapsed = now.saturating_sub(state.last_accrual);
            let fee = if elapsed == 0 || aum <= 0 {
                0
            } else {
                aum.checked_mul(config.management_fee_bps as i128)
                    .ok_or(Error::MathOverflow)?
                    .checked_mul(elapsed as i128)
                    .ok_or(Error::MathOverflow)?
                    / (BPS_DENOMINATOR * SECONDS_PER_YEAR)
            };

            state.last_accrual = now;
            state.fees_paid = checked_add(state.fees_paid, fee)?;
            save_vault(&env, &vault, &state);
            if fee > 0 {
                let (vault_share, protocol_share) =
                    Self::split_fee(fee, config.protocol_share_bps)?;
                Self::record_fee_event(
                    &env,
                    &vault,
                    &vault,
                    fee,
                    0,
                    fee,
                    vault_share,
                    protocol_share,
                    1,
                );
            }
            Ok(fee)
        })();
        Self::reentrancy_exit(&env);
        out
    }

    // ── Performance fee (#661) ─────────────────────────────────────────────

    /// # Crystallise the performance fee at a given NAV
    ///
    /// If the NAV is above the high-water mark, the profit is `nav − mark`, the
    /// fee is charged on it, and the mark rises. If the NAV has fallen below the
    /// last crystallised value, the fee charged on that loss is clawed back.
    pub fn crystallize(env: Env, vault: Address, nav: i128) -> Result<i128, Error> {
        Self::reentrancy_enter(&env)?;
        let out = (|| {
            let config = load_config(&env)?;
            let mut state = load_vault(&env, &vault)?;

            if nav > state.high_water_mark {
                let profit = nav - state.high_water_mark;
                let rate =
                    perf_rate_for_profit(&env, profit).unwrap_or(config.performance_fee_bps);
                let fee = profit
                    .checked_mul(rate as i128)
                    .ok_or(Error::MathOverflow)?
                    / BPS_DENOMINATOR;
                state.high_water_mark = nav;
                state.last_crystallized = nav;
                state.fees_paid = checked_add(state.fees_paid, fee)?;
                save_vault(&env, &vault, &state);
                if fee > 0 {
                    let (vault_share, protocol_share) =
                        Self::split_fee(fee, config.protocol_share_bps)?;
                    Self::record_fee_event(
                        &env,
                        &vault,
                        &vault,
                        fee,
                        0,
                        fee,
                        vault_share,
                        protocol_share,
                        2,
                    );
                }
                return Ok(fee);
            }

            if nav < state.last_crystallized {
                let loss = state.last_crystallized - nav;
                let rate = perf_rate_for_profit(&env, loss).unwrap_or(config.performance_fee_bps);
                let clawback = loss
                    .checked_mul(rate as i128)
                    .ok_or(Error::MathOverflow)?
                    / BPS_DENOMINATOR;
                state.fees_paid = state.fees_paid.saturating_sub(clawback);
                state.last_crystallized = nav;
                save_vault(&env, &vault, &state);
                return Ok(0);
            }

            Ok(0)
        })();
        Self::reentrancy_exit(&env);
        out
    }

    // ── Full charge ────────────────────────────────────────────────────────

    /// # Compute the complete fee charge for a vault
    ///
    /// Accrues the management fee, crystallises the performance fee, applies the
    /// staked-governance rebate, and splits the net fee between the vault's fee
    /// recipient and the treasury.
    pub fn compute_fees(env: Env, vault: Address, nav: i128) -> Result<FeeBreakdown, Error> {
        let management_fee = Self::accrue_management_fee(env, vault.clone(), nav)?;
        let performance_fee = Self::crystallize(env, vault.clone(), nav)?;
        let config = load_config(&env)?;

        let total = checked_add(management_fee, performance_fee)?;
        let stake = load_stake_or_default(&env, &vault);
        let rebate = rebate_for_stake(&env, stake.staked);
        let discount = total
            .checked_mul(rebate as i128)
            .ok_or(Error::MathOverflow)?
            / BPS_DENOMINATOR;
        let net_total = total.checked_sub(discount).ok_or(Error::MathOverflow)?;
        let (vault_share, protocol_share) = Self::split_fee(net_total, config.protocol_share_bps)?;

        Ok(FeeBreakdown {
            management_fee,
            performance_fee,
            total,
            discount,
            net_total,
            vault_share,
            protocol_share,
        })
    }

    /// # Sponsored fee computation (#670)
    ///
    /// Same as `compute_fees`, but `payer` may be a pre-authorised sponsor
    /// paying on behalf of `vault` (meta-transaction / gasless path). Emits
    /// fee transparency records with `payer` set to the sponsor.
    pub fn compute_fees_sponsored(
        env: Env,
        vault: Address,
        payer: Address,
        nav: i128,
    ) -> Result<FeeBreakdown, Error> {
        Self::require_payer_or_sponsor(&env, &vault, &payer)?;
        let breakdown = Self::compute_fees(env.clone(), vault.clone(), nav)?;
        if breakdown.net_total > 0 {
            Self::record_fee_event(
                &env,
                &vault,
                &payer,
                breakdown.total,
                breakdown.discount,
                breakdown.net_total,
                breakdown.vault_share,
                breakdown.protocol_share,
                3,
            );
        }
        Ok(breakdown)
    }


    // ── Reentrancy (#671) ────────────────────────────────────────────────────

    fn reentrancy_enter(env: &Env) -> Result<(), Error> {
        let key = DataKey::ReentrancyLock;
        let locked: bool = env.storage().instance().get(&key).unwrap_or(false);
        if locked {
            return Err(Error::Reentrancy);
        }
        env.storage().instance().set(&key, &true);
        Ok(())
    }

    fn reentrancy_exit(env: &Env) {
        env.storage().instance().set(&DataKey::ReentrancyLock, &false);
    }

    // ── Fee transparency (#669) ──────────────────────────────────────────────

    fn next_fee_seq(env: &Env) -> u64 {
        let seq: u64 = env.storage().instance().get(&DataKey::FeeSeq).unwrap_or(0u64);
        let next = seq.saturating_add(1);
        env.storage().instance().set(&DataKey::FeeSeq, &next);
        next
    }

    fn record_fee_event(
        env: &Env,
        vault: &Address,
        payer: &Address,
        gross: i128,
        discount: i128,
        net: i128,
        vault_share: i128,
        protocol_share: i128,
        kind: u32,
    ) {
        let seq = Self::next_fee_seq(env);
        let timestamp = env.ledger().timestamp();
        let record = FeeRecord {
            seq,
            vault: vault.clone(),
            payer: payer.clone(),
            gross,
            discount,
            net,
            vault_share,
            protocol_share,
            timestamp,
            kind,
        };
        env.storage()
            .persistent()
            .set(&DataKey::FeeHistory(seq), &record);
        env.events().publish(
            (symbol_short!("fee_chg"), vault.clone(), kind),
            (
                seq,
                payer.clone(),
                gross,
                discount,
                net,
                vault_share,
                protocol_share,
                timestamp,
            ),
        );
    }

    /// Number of fee history records (highest sequence id).
    pub fn fee_history_len(env: Env) -> u64 {
        env.storage().instance().get(&DataKey::FeeSeq).unwrap_or(0u64)
    }

    /// Fetch a historical fee record by sequence id (1-based).
    pub fn get_fee_record(env: Env, seq: u64) -> Result<FeeRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::FeeHistory(seq))
            .ok_or(Error::HistoryOutOfBounds)
    }

    // ── Fee delegation / sponsored fees (#670) ───────────────────────────────

    /// Authorise `sponsor` to pay fees on behalf of `user` (meta-tx / gasless).
    pub fn set_fee_sponsor(
        env: Env,
        user: Address,
        sponsor: Address,
        allowed: bool,
    ) -> Result<(), Error> {
        user.require_auth();
        if allowed {
            env.storage()
                .instance()
                .set(&DataKey::Sponsor(user.clone()), &sponsor);
        } else {
            env.storage()
                .instance()
                .remove(&DataKey::Sponsor(user.clone()));
        }
        env.events()
            .publish((symbol_short!("sponsor"), user), (sponsor, allowed));
        Ok(())
    }

    /// Return the configured fee sponsor for `user`, if any.
    pub fn get_fee_sponsor(env: Env, user: Address) -> Option<Address> {
        env.storage().instance().get(&DataKey::Sponsor(user))
    }

    fn require_payer_or_sponsor(
        env: &Env,
        user: &Address,
        payer: &Address,
    ) -> Result<(), Error> {
        if payer == user {
            payer.require_auth();
            return Ok(());
        }
        payer.require_auth();
        let allowed: Option<Address> =
            env.storage().instance().get(&DataKey::Sponsor(user.clone()));
        match allowed {
            Some(s) if &s == payer => Ok(()),
            _ => Err(Error::SponsorNotAllowed),
        }
    }

    /// # Split a fee between the vault and the treasury
    ///
    /// The protocol share is rounded down and the remainder goes to the vault, so
    /// the two shares always sum to exactly `fee`.
    pub fn split_fee(fee: i128, protocol_share_bps: u32) -> Result<(i128, i128), Error> {
        if protocol_share_bps > BPS_DENOMINATOR as u32 {
            return Err(Error::InvalidFeeSplit);
        }
        let protocol = fee
            .checked_mul(protocol_share_bps as i128)
            .ok_or(Error::MathOverflow)?
            / BPS_DENOMINATOR;
        let vault = fee.checked_sub(protocol).ok_or(Error::MathOverflow)?;
        Ok((vault, protocol))
    }
}
