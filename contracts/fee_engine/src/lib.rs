#![no_std]

//! Fee Engine contract for on-chain fee transparency, performance fees,
//! management fees, fee discount tokens, and fee switching/routing (#669, #670, #661, #662, #663).

mod test;

soroban_sdk::contractimport!(
    // Contract internals are included directly here.
);

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, Env, IntoVal, Symbol, Vec,
};

pub const BPS_DENOMINATOR: u32 = 10_000;
pub const MAX_FEE_BPS: u32 = 2_500; // 25%
pub const SECONDS_PER_YEAR: u64 = 31_536_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    FeeTooHigh = 4,
    InvalidFeeSplit = 5,
    InvalidTier = 6,
    InsufficientStake = 7,
    InvalidAmount = 8,
    SwitchInactive = 9,
    InvalidRecipient = 10,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeConfig {
    pub management_fee_bps: u32,
    pub performance_fee_bps: u32,
    pub protocol_share_bps: u32,
    pub vault_fee_recipient: Address,
    pub treasury: Address,
}

impl FeeConfig {
    pub fn validate(&self) -> Result<(), Error> {
        if self.management_fee_bps > MAX_FEE_BPS || self.performance_fee_bps > MAX_FEE_BPS {
            return Err(Error::FeeTooHigh);
        }
        if self.protocol_share_bps > BPS_DENOMINATOR {
            return Err(Error::InvalidFeeSplit);
        }
        if self.vault_fee_recipient.to_string().is_empty() || self.treasury.to_string().is_empty() {
            return Err(Error::InvalidRecipient);
        }
        Ok(())
    }
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DiscountTier {
    pub min_stake: i128,
    pub rebate_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PerformanceTier {
    pub min_profit: i128,
    pub fee_bps: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultState {
    pub deposit_total: i128,
    pub high_water_mark: i128,
    pub last_crystallized: i128,
    pub fees_paid: i128,
    pub last_accrual_timestamp: u64,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Stake {
    pub staked: i128,
    pub tier: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeBreakdown {
    pub management_fee: i128,
    pub performance_fee: i128,
    pub total: i128,
    pub discount: i128,
    pub net_total: i128,
    pub protocol_share: i128,
    pub vault_share: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    StakingToken,
    FeeConfig,
    DiscountTiers,
    PerfTiers,
    Vault(Address),
    Stake(Address),
}

#[contract]
pub struct FeeEngine;

#[contractimpl]
impl FeeEngine {
    pub fn initialize(env: Env, admin: Address, staking_token: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::StakingToken, &staking_token);
        Ok(())
    }

    pub fn admin(env: Env) -> Address {
        env.storage().instance().get(&DataKey::Admin).unwrap()
    }

    pub fn get_fee_config(env: Env) -> FeeConfig {
        env.storage()
            .instance()
            .get(&DataKey::FeeConfig)
            .unwrap_or(FeeConfig {
                management_fee_bps: 0,
                performance_fee_bps: 0,
                protocol_share_bps: 0,
                vault_fee_recipient: env.current_contract_address(),
                treasury: env.current_contract_address(),
            })
    }

    pub fn set_fee_config(env: Env, admin: Address, config: FeeConfig) -> Result<(), Error> {
        admin.require_auth();
        let current_admin: Address = Self::admin(env.clone());
        if admin != current_admin {
            return Err(Error::Unauthorized);
        }

        config.validate()?;

        // State preservation on failure: validation happens prior to mutation.
        env.storage().instance().set(&DataKey::FeeConfig, &config);

        // Emit auditable event for successful change
        env.events().publish(
            (Symbol::new(&env, "fee_engine"), Symbol::new(&env, "config_updated")),
            config,
        );

        Ok(())
    }

    pub fn set_discount_tiers(env: Env, admin: Address, tiers: Vec<DiscountTier>) -> Result<(), Error> {
        admin.require_auth();
        let current_admin: Address = Self::admin(env.clone());
        if admin != current_admin {
            return Err(Error::Unauthorized);
        }

        // Validate ascending order
        let mut prev_stake = -1i128;
        for tier in tiers.iter() {
            if tier.min_stake <= prev_stake || tier.rebate_bps > BPS_DENOMINATOR {
                return Err(Error::InvalidTier);
            }
            prev_stake = tier.min_stake;
        }

        env.storage().instance().set(&DataKey::DiscountTiers, &tiers);
        Ok(())
    }

    pub fn set_perf_tiers(env: Env, admin: Address, tiers: Vec<PerformanceTier>) -> Result<(), Error> {
        admin.require_auth();
        let current_admin: Address = Self::admin(env.clone());
        if admin != current_admin {
            return Err(Error::Unauthorized);
        }

        let mut prev_profit = -1i128;
        for tier in tiers.iter() {
            if tier.min_profit <= prev_profit || tier.fee_bps > MAX_FEE_BPS {
                return Err(Error::InvalidTier);
            }
            prev_profit = tier.min_profit;
        }

        env.storage().instance().set(&DataKey::PerfTiers, &tiers);
        Ok(())
    }

    pub fn record_deposit(env: Env, vault: Address, amount: i128) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut state: VaultState = env
            .storage()
            .persistent()
            .get(&DataKey::Vault(vault.clone()))
            .unwrap_or(VaultState {
                deposit_total: 0,
                high_water_mark: 0,
                last_crystallized: 0,
                fees_paid: 0,
                last_accrual_timestamp: env.ledger().timestamp(),
            });
        state.deposit_total += amount;
        if state.high_water_mark == 0 {
            state.high_water_mark = state.deposit_total;
            state.last_crystallized = state.deposit_total;
        }
        env.storage().persistent().set(&DataKey::Vault(vault), &state);
        Ok(())
    }

    pub fn record_withdrawal(env: Env, vault: Address, amount: i128) -> Result<(), Error> {
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut state: VaultState = env
            .storage()
            .persistent()
            .get(&DataKey::Vault(vault.clone()))
            .ok_or(Error::NotInitialized)?;
        if amount > state.deposit_total {
            return Err(Error::InvalidAmount);
        }
        state.deposit_total -= amount;
        env.storage().persistent().set(&DataKey::Vault(vault), &state);
        Ok(())
    }

    pub fn accrue_management_fee(env: Env, vault: Address, nav: i128) -> i128 {
        let config = Self::get_fee_config(env.clone());
        let mut state: VaultState = env
            .storage()
            .persistent()
            .get(&DataKey::Vault(vault.clone()))
            .unwrap_or(VaultState {
                deposit_total: nav,
                high_water_mark: nav,
                last_crystallized: nav,
                fees_paid: 0,
                last_accrual_timestamp: env.ledger().timestamp(),
            });

        let current_time = env.ledger().timestamp();
        let elapsed = current_time.saturating_sub(state.last_accrual_timestamp);
        if elapsed == 0 || config.management_fee_bps == 0 {
            return 0;
        }

        let fee = (nav
            * (config.management_fee_bps as i128)
            * (elapsed as i128))
            / ((BPS_DENOMINATOR as i128) * (SECONDS_PER_YEAR as i128));

        state.last_accrual_timestamp = current_time;
        env.storage().persistent().set(&DataKey::Vault(vault), &state);
        fee
    }

    pub fn crystallize(env: Env, vault: Address, nav: i128) -> i128 {
        let config = Self::get_fee_config(env.clone());
        let mut state: VaultState = env
            .storage()
            .persistent()
            .get(&DataKey::Vault(vault.clone()))
            .unwrap_or(VaultState {
                deposit_total: nav,
                high_water_mark: nav,
                last_crystallized: nav,
                fees_paid: 0,
                last_accrual_timestamp: env.ledger().timestamp(),
            });

        if nav > state.high_water_mark {
            let profit = nav - state.high_water_mark;
            let fee = (profit * (config.performance_fee_bps as i128)) / (BPS_DENOMINATOR as i128);
            state.high_water_mark = nav;
            state.last_crystallized = nav;
            state.fees_paid += fee;
            env.storage().persistent().set(&DataKey::Vault(vault), &state);
            fee
        } else if nav < state.last_crystallized {
            let loss = state.last_crystallized - nav;
            let clawback = (loss * (config.performance_fee_bps as i128)) / (BPS_DENOMINATOR as i128);
            state.fees_paid = state.fees_paid.saturating_sub(clawback);
            state.last_crystallized = nav;
            env.storage().persistent().set(&DataKey::Vault(vault), &state);
            0
        } else {
            0
        }
    }

    pub fn stake(env: Env, staker: Address, amount: i128) -> Result<(), Error> {
        staker.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut current_stake: Stake = env
            .storage()
            .persistent()
            .get(&DataKey::Stake(staker.clone()))
            .unwrap_or(Stake {
                staked: 0,
                tier: 0,
            });
        current_stake.staked += amount;

        let tiers: Vec<DiscountTier> = env
            .storage()
            .instance()
            .get(&DataKey::DiscountTiers)
            .unwrap_or(Vec::new(&env));

        let mut best_tier = 0u32;
        let mut idx = 1u32;
        for tier in tiers.iter() {
            if current_stake.staked >= tier.min_stake {
                best_tier = idx;
            }
            idx += 1;
        }
        current_stake.tier = best_tier;

        env.storage().persistent().set(&DataKey::Stake(staker), &current_stake);
        Ok(())
    }

    pub fn unstake(env: Env, staker: Address, amount: i128) -> Result<(), Error> {
        staker.require_auth();
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let mut current_stake: Stake = env
            .storage()
            .persistent()
            .get(&DataKey::Stake(staker.clone()))
            .ok_or(Error::InsufficientStake)?;
        if amount > current_stake.staked {
            return Err(Error::InsufficientStake);
        }
        current_stake.staked -= amount;

        let tiers: Vec<DiscountTier> = env
            .storage()
            .instance()
            .get(&DataKey::DiscountTiers)
            .unwrap_or(Vec::new(&env));

        let mut best_tier = 0u32;
        let mut idx = 1u32;
        for tier in tiers.iter() {
            if current_stake.staked >= tier.min_stake {
                best_tier = idx;
            }
            idx += 1;
        }
        current_stake.tier = best_tier;

        env.storage().persistent().set(&DataKey::Stake(staker), &current_stake);
        Ok(())
    }

    pub fn get_stake(env: Env, staker: Address) -> Stake {
        env.storage()
            .persistent()
            .get(&DataKey::Stake(staker))
            .unwrap_or(Stake {
                staked: 0,
                tier: 0,
            })
    }

    pub fn get_vault(env: Env, vault: Address) -> VaultState {
        env.storage()
            .persistent()
            .get(&DataKey::Vault(vault))
            .unwrap_or(VaultState {
                deposit_total: 0,
                high_water_mark: 0,
                last_crystallized: 0,
                fees_paid: 0,
                last_accrual_timestamp: 0,
            })
    }

    pub fn compute_fees(env: Env, vault: Address, nav: i128) -> FeeBreakdown {
        let management = Self::accrue_management_fee(env.clone(), vault.clone(), nav);
        let performance = Self::crystallize(env.clone(), vault.clone(), nav);
        let total = management + performance;

        let mut discount_bps = 0u32;
        // Lookup stakeholder rebate
        let tiers: Vec<DiscountTier> = env
            .storage()
            .instance()
            .get(&DataKey::DiscountTiers)
            .unwrap_or(Vec::new(&env));
        let stake_info = Self::get_stake(env.clone(), vault.clone());
        if stake_info.tier > 0 {
            if let Some(tier) = tiers.get(stake_info.tier - 1) {
                discount_bps = tier.rebate_bps;
            }
        }

        let discount = (total * (discount_bps as i128)) / (BPS_DENOMINATOR as i128);
        let net_total = total - discount;
        let config = Self::get_fee_config(env.clone());
        let (vault_share, protocol_share) = Self::split_fee(env.clone(), &net_total, &config.protocol_share_bps);

        FeeBreakdown {
            management_fee: management,
            performance_fee: performance,
            total,
            discount,
            net_total,
            protocol_share,
            vault_share,
        }
    }

    pub fn split_fee(_env: Env, fee: &i128, protocol_share_bps: &u32) -> (i128, i128) {
        let protocol = (fee * (*protocol_share_bps as i128)) / (BPS_DENOMINATOR as i128);
        let vault = fee - protocol;
        (vault, protocol)
    }
}
