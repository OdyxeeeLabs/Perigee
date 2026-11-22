#![no_std]

//! # Perigee Referral System
//!
//! Closes OdyxeeeLabs/Perigee#664
//!
//! Source: contract.md § Tokenomics & Fees (#34)
//!
//! On-chain referral tracking with reward distribution and anti-sybil controls:
//!
//! - Each address may bind a single referrer **once** (immutable).
//! - Self-referral is rejected.
//! - Rewards accrue in bps of qualified volume attributed to the referee.
//! - Anti-sybil: a referrer may not exceed `max_referees`; a minimum
//!   `min_referee_age_ledgers` must elapse after the referee's first seen
//!   ledger before volume counts; `min_volume_for_reward` filters dust.
//! - Reward claims pull from an admin-funded reward pool.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Symbol,
};

pub const BPS_DENOMINATOR: u32 = 10_000;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    NotInitialized = 1,
    AlreadyInitialized = 2,
    Unauthorized = 3,
    AlreadyReferred = 4,
    SelfReferral = 5,
    InvalidAmount = 6,
    RefereeLimit = 7,
    RefereeTooNew = 8,
    VolumeTooLow = 9,
    InsufficientPool = 10,
    NothingToClaim = 11,
    MathOverflow = 12,
    ReferrerNotFound = 13,
    DuplicateAction = 14,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferralConfig {
    /// Share of qualified volume paid to the referrer, in bps.
    pub reward_bps: u32,
    pub max_referees: u32,
    pub min_referee_age_ledgers: u32,
    pub min_volume_for_reward: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ReferrerStats {
    pub referee_count: u32,
    pub qualified_volume: i128,
    pub rewards_accrued: i128,
    pub rewards_claimed: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RefereeRecord {
    pub referrer: Address,
    pub bound_ledger: u32,
    pub volume: i128,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Admin,
    Config,
    RewardPool,
    /// referee → RefereeRecord
    Referee(Address),
    /// referrer → ReferrerStats
    Referrer(Address),
    /// First ledger an address was observed (anti-sybil age).
    FirstSeen(Address),
    /// Action ids already credited through `attribute_action`.
    Action(BytesN<32>),
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

fn load_config(env: &Env) -> Result<ReferralConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

fn load_stats(env: &Env, referrer: &Address) -> ReferrerStats {
    env.storage()
        .persistent()
        .get(&DataKey::Referrer(referrer.clone()))
        .unwrap_or(ReferrerStats {
            referee_count: 0,
            qualified_volume: 0,
            rewards_accrued: 0,
            rewards_claimed: 0,
        })
}

fn save_stats(env: &Env, referrer: &Address, stats: &ReferrerStats) {
    env.storage()
        .persistent()
        .set(&DataKey::Referrer(referrer.clone()), stats);
}

fn note_first_seen(env: &Env, who: &Address) {
    let key = DataKey::FirstSeen(who.clone());
    if !env.storage().persistent().has(&key) {
        env.storage()
            .persistent()
            .set(&key, &env.ledger().sequence());
    }
}

fn first_seen(env: &Env, who: &Address) -> u32 {
    env.storage()
        .persistent()
        .get(&DataKey::FirstSeen(who.clone()))
        .unwrap_or(env.ledger().sequence())
}

#[contract]
pub struct Referral;

#[contractimpl]
impl Referral {
    pub fn initialize(env: Env, admin: Address, config: ReferralConfig) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        if config.reward_bps > BPS_DENOMINATOR {
            return Err(Error::InvalidAmount);
        }
        admin.require_auth();
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::Config, &config);
        env.storage().instance().set(&DataKey::RewardPool, &0i128);
        Ok(())
    }

    pub fn set_config(env: Env, admin: Address, config: ReferralConfig) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        if config.reward_bps > BPS_DENOMINATOR {
            return Err(Error::InvalidAmount);
        }
        env.storage().instance().set(&DataKey::Config, &config);
        Ok(())
    }

    pub fn fund_pool(env: Env, admin: Address, amount: i128) -> Result<i128, Error> {
        require_admin(&env, &admin)?;
        if amount <= 0 {
            return Err(Error::InvalidAmount);
        }
        let pool: i128 = env
            .storage()
            .instance()
            .get(&DataKey::RewardPool)
            .unwrap_or(0);
        let next = pool.checked_add(amount).ok_or(Error::MathOverflow)?;
        env.storage().instance().set(&DataKey::RewardPool, &next);
        Ok(next)
    }

    pub fn reward_pool(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::RewardPool)
            .unwrap_or(0)
    }

    /// Bind `referee` to `referrer` permanently.
    pub fn bind_referrer(
        env: Env,
        referee: Address,
        referrer: Address,
    ) -> Result<(), Error> {
        referee.require_auth();
        if referee == referrer {
            return Err(Error::SelfReferral);
        }
        let key = DataKey::Referee(referee.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::AlreadyReferred);
        }
        let config = load_config(&env)?;
        let mut stats = load_stats(&env, &referrer);
        if stats.referee_count >= config.max_referees {
            return Err(Error::RefereeLimit);
        }

        note_first_seen(&env, &referee);
        note_first_seen(&env, &referrer);

        let record = RefereeRecord {
            referrer: referrer.clone(),
            bound_ledger: env.ledger().sequence(),
            volume: 0,
        };
        env.storage().persistent().set(&key, &record);

        stats.referee_count += 1;
        save_stats(&env, &referrer, &stats);

        env.events().publish(
            (Symbol::new(&env, "referral"), Symbol::new(&env, "bind")),
            (referee, referrer),
        );
        Ok(())
    }

    /// Attribute volume to a referee and accrue referrer rewards when anti-sybil checks pass.
    ///
    /// This entry point has no notion of *which* action the volume came from, so
    /// calling it twice with the same amount credits it twice. Callers that need
    /// at-most-once attribution per action should use
    /// [`Referral::attribute_action`].
    pub fn attribute_volume(
        env: Env,
        referee: Address,
        volume: i128,
    ) -> Result<i128, Error> {
        Self::attribute_inner(&env, referee, volume)
    }

    /// Attribute volume for a uniquely identified action, at most once.
    ///
    /// A repeated `action_id` is rejected with [`Error::DuplicateAction`] before
    /// any state is touched. The id is only recorded when attribution succeeds,
    /// so a rejected attempt (too new, too small, unknown referee) does not burn
    /// the id and the action can be retried once it qualifies.
    pub fn attribute_action(
        env: Env,
        referee: Address,
        action_id: BytesN<32>,
        volume: i128,
    ) -> Result<i128, Error> {
        let key = DataKey::Action(action_id);
        if env.storage().persistent().has(&key) {
            return Err(Error::DuplicateAction);
        }
        let reward = Self::attribute_inner(&env, referee, volume)?;
        env.storage().persistent().set(&key, &true);
        Ok(reward)
    }

    fn attribute_inner(env: &Env, referee: Address, volume: i128) -> Result<i128, Error> {
        let env = env.clone();
        if volume <= 0 {
            return Err(Error::InvalidAmount);
        }
        let config = load_config(&env)?;
        if volume < config.min_volume_for_reward {
            return Err(Error::VolumeTooLow);
        }

        let key = DataKey::Referee(referee.clone());
        let mut record: RefereeRecord = env
            .storage()
            .persistent()
            .get(&key)
            .ok_or(Error::ReferrerNotFound)?;

        let age = env
            .ledger()
            .sequence()
            .saturating_sub(first_seen(&env, &referee));
        if age < config.min_referee_age_ledgers {
            return Err(Error::RefereeTooNew);
        }

        record.volume = record
            .volume
            .checked_add(volume)
            .ok_or(Error::MathOverflow)?;
        env.storage().persistent().set(&key, &record);

        let reward = volume
            .checked_mul(config.reward_bps as i128)
            .ok_or(Error::MathOverflow)?
            / BPS_DENOMINATOR as i128;

        let mut stats = load_stats(&env, &record.referrer);
        stats.qualified_volume = stats
            .qualified_volume
            .checked_add(volume)
            .ok_or(Error::MathOverflow)?;
        stats.rewards_accrued = stats
            .rewards_accrued
            .checked_add(reward)
            .ok_or(Error::MathOverflow)?;
        save_stats(&env, &record.referrer, &stats);

        Ok(reward)
    }

    pub fn claim_rewards(env: Env, referrer: Address) -> Result<i128, Error> {
        referrer.require_auth();
        let mut stats = load_stats(&env, &referrer);
        let pending = stats
            .rewards_accrued
            .checked_sub(stats.rewards_claimed)
            .ok_or(Error::MathOverflow)?;
        if pending <= 0 {
            return Err(Error::NothingToClaim);
        }
        let pool: i128 = env
            .storage()
            .instance()
            .get(&DataKey::RewardPool)
            .unwrap_or(0);
        if pending > pool {
            return Err(Error::InsufficientPool);
        }
        env.storage()
            .instance()
            .set(&DataKey::RewardPool, &(pool - pending));
        stats.rewards_claimed = stats
            .rewards_claimed
            .checked_add(pending)
            .ok_or(Error::MathOverflow)?;
        save_stats(&env, &referrer, &stats);
        env.events().publish(
            (Symbol::new(&env, "referral"), Symbol::new(&env, "claim")),
            (referrer, pending),
        );
        Ok(pending)
    }

    pub fn get_referrer(env: Env, referee: Address) -> Option<Address> {
        let record: Option<RefereeRecord> = env
            .storage()
            .persistent()
            .get(&DataKey::Referee(referee));
        record.map(|r| r.referrer)
    }

    pub fn get_stats(env: Env, referrer: Address) -> ReferrerStats {
        load_stats(&env, &referrer)
    }
}

#[cfg(test)]
mod test;
