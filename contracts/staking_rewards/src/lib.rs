#![no_std]
use soroban_sdk::{contract, contractimpl, contracttype, token, Address, Env, String, Vec, vec};

use emergency_guard::{EmergencyGuard, PauseType};
pub use soroscope_error_codes::ContractError;
use soroscope_math::Fixed;
use emergency_guard::{DefaultEmergencyGuard, PauseType, EmergencyGuardTrait};

pub const SCALE: i128 = 1_000_000_000_000_000_000; // 18 decimals

// ── Storage Keys ──────────────────────────────────────────────

#[derive(Clone)]
#[contracttype]
pub enum DataKey {
    Config,
    UserState(Address),
    TotalStaked,
}

// ── Configuration Struct ──────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StakingConfig {
    pub owner: Address,
    pub staking_token: Address,
    pub reward_token: Address,
    pub initial_rate: Fixed, // r0
    pub decay_rate: Fixed,   // d (where alpha = 1 - d)
    pub start_block: u32,
    pub is_paused: bool,
}

// ── User Staking State ────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct UserStakingState {
    pub staked_amount: i128,
    pub accrued_rewards: i128,
    pub last_update_block: u32,
}

// ── Event Structs ─────────────────────────────────────────────

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct StakeEvent {
    pub user: Address,
    pub amount: i128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct WithdrawEvent {
    pub user: Address,
    pub amount: i128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct ClaimEvent {
    pub user: Address,
    pub amount: i128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct EmergencyWithdrawEvent {
    pub user: Address,
    pub amount: i128,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[contracttype]
pub struct PausedEvent {
    pub paused: bool,
}

// ── Helper Math Functions ─────────────────────────────────────

fn fixed_pow_int(base: Fixed, mut exp: u32) -> Result<Fixed, ContractError> {
    let mut temp = base;
    let mut ans = Fixed::ONE;
    while exp > 0 {
        if exp & 1 == 1 {
            ans = ans.mul(temp).map_err(|_| ContractError::Overflow)?;
        }
        temp = temp.mul(temp).map_err(|_| ContractError::Overflow)?;
        exp >>= 1;
    }
    Ok(ans)
}

fn mul_div(a: i128, b: i128, d: i128) -> Option<i128> {
    if d == 0 {
        return None;
    }
    let a_abs = a.unsigned_abs();
    let b_abs = b.unsigned_abs();
    let d_abs = d.unsigned_abs();

    let (res_abs, overflow) = mul_div_u128(a_abs, b_abs, d_abs);
    if overflow || res_abs > (i128::MAX as u128) {
        return None;
    }

    let res = res_abs as i128;
    if (a < 0) ^ (b < 0) ^ (d < 0) {
        Some(-res)
    } else {
        Some(res)
    }
}

fn mul_div_u128(a: u128, b: u128, d: u128) -> (u128, bool) {
    if let Some(prod) = a.checked_mul(b) {
        return (prod / d, false);
    }
    let a_low = a & 0xFFFFFFFFFFFFFFFF;
    let a_high = a >> 64;
    let b_low = b & 0xFFFFFFFFFFFFFFFF;
    let b_high = b >> 64;
    let p0 = a_low * b_low;
    let p1 = a_low * b_high;
    let p2 = a_high * b_low;
    let p3 = a_high * b_high;
    let mid = (p1 & 0xFFFFFFFFFFFFFFFF) + (p2 & 0xFFFFFFFFFFFFFFFF) + (p0 >> 64);
    let high = p3 + (p1 >> 64) + (p2 >> 64) + (mid >> 64);
    let low = (mid << 64) | (p0 & 0xFFFFFFFFFFFFFFFF);
    if high >= d {
        return (0, true);
    }
    let mut quotient = 0u128;
    let mut remainder = high;
    for i in (0..128).rev() {
        remainder = (remainder << 1) | ((low >> i) & 1);
        if remainder >= d {
            remainder -= d;
            quotient |= 1 << i;
        }
    }
    (quotient, false)
}

fn multiply_amount(amount: i128, multiplier: Fixed) -> Result<i128, ContractError> {
    mul_div(amount, multiplier.0, SCALE).ok_or(ContractError::Overflow)
}

// ── Compounding Multiplier Calculation ────────────────────────

fn calculate_multiplier(config: &StakingConfig, t1: u32, t2: u32) -> Result<Fixed, ContractError> {
    if t2 <= t1 {
        return Ok(Fixed::ONE);
    }

    let t_start = config.start_block;
    let t1_eff = t1.max(t_start);
    let t2_eff = t2.max(t_start);

    if t2_eff <= t1_eff {
        return Ok(Fixed::ONE);
    }

    let k1 = t1_eff - t_start;
    let k2 = t2_eff - t_start;

    if config.decay_rate.0 == 0 {
        // No decay case: alpha = 1
        let elapsed = (k2 - k1) as i128;
        let elapsed_fixed = Fixed::from_int(elapsed).map_err(|_| ContractError::Overflow)?;
        let exponent = config
            .initial_rate
            .mul(elapsed_fixed)
            .map_err(|_| ContractError::Overflow)?;
        let multiplier = exponent.exp().map_err(|_| ContractError::Overflow)?;
        Ok(multiplier)
    } else {
        // Decay case: alpha = 1 - d
        let alpha = Fixed::ONE
            .sub(config.decay_rate)
            .map_err(|_| ContractError::Overflow)?;
        if alpha.0 < 0 || alpha.0 > SCALE {
            return Err(ContractError::InvalidInput);
        }

        let a1 = fixed_pow_int(alpha, k1)?;
        let a2 = fixed_pow_int(alpha, k2)?;
        let diff = a1.sub(a2).map_err(|_| ContractError::Overflow)?;

        // exponent = r0 * diff / decay_rate
        let term = config
            .initial_rate
            .mul(diff)
            .map_err(|_| ContractError::Overflow)?;
        let exponent = term.div(config.decay_rate).map_err(|_| {
            if config.decay_rate.0 == 0 {
                ContractError::DivisionByZero
            } else {
                ContractError::Overflow
            }
        })?;

        let multiplier = exponent.exp().map_err(|_| ContractError::Overflow)?;
        Ok(multiplier)
    }
}

// ── Contract Implementation ───────────────────────────────────

#[contract]
pub struct StakingRewards;

#[contractimpl]
impl StakingRewards {
    /// Initializes the staking rewards contract with the config.
    pub fn initialize(
        e: Env,
        owner: Address,
        staking_token: Address,
        reward_token: Address,
        initial_rate: i128, // initial rate (Fixed point representation)
        decay_rate: i128,   // decay rate (Fixed point representation, d = 1 - alpha)
        start_block: u32,
    ) -> Result<(), ContractError> {
        if e.storage().instance().has(&DataKey::Config) {
            return Err(ContractError::AlreadyInitialized);
        }

        if !(0..=SCALE).contains(&decay_rate) {
            return Err(ContractError::InvalidInput);
        }

        if initial_rate < 0 {
            return Err(ContractError::InvalidInput);
        }

        let config = StakingConfig {
            owner: owner.clone(),
            staking_token,
            reward_token,
            initial_rate: Fixed(initial_rate),
            decay_rate: Fixed(decay_rate),
            start_block,
            is_paused: false,
        };

        e.storage().instance().set(&DataKey::Config, &config);
        e.storage().instance().set(&DataKey::TotalStaked, &0i128);
        e.storage().instance().extend_ttl(10000, 10000);

        // Initialize emergency guard with single admin and threshold of 1
        let admins = vec![&e, owner.clone()];
        DefaultEmergencyGuard::init_guard(&e, admins, 1)
            .map_err(|_| ContractError::AlreadyInitialized)?;
        // Initialize the embedded EmergencyGuard so granular pause checks
        // (e.g. PauseType::CLAIM_REWARDS) can be toggled by the owner.
        // Threshold of 1 means the single owner can trigger any pause.
        let admins = soroban_sdk::vec![&e, config.owner.clone()];
        EmergencyGuard::initialize(e, admins, 1).map_err(|_| ContractError::AlreadyInitialized)?;

        Ok(())
    }

    /// Stakes primary tokens in the contract.
    pub fn stake(e: Env, user: Address, amount: i128) -> Result<(), ContractError> {
        // Check if staking is paused using granular pause control
        DefaultEmergencyGuard::check_not_paused(&e, PauseType::STAKE)
            .map_err(|_| ContractError::Paused)?;

        if amount <= 0 {
            return Err(ContractError::InvalidInput);
        }

        user.require_auth();

        let config = Self::get_config(e.clone())?;
        let mut state = Self::update_user_rewards_internal(&e, &config, &user)?;

        // Transfer staking tokens from user to contract
        token::Client::new(&e, &config.staking_token).transfer(
            &user,
            &e.current_contract_address(),
            &amount,
        );

        state.staked_amount = state
            .staked_amount
            .checked_add(amount)
            .ok_or(ContractError::Overflow)?;

        // Update total staked
        let mut total_staked: i128 = e
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        total_staked = total_staked
            .checked_add(amount)
            .ok_or(ContractError::Overflow)?;
        e.storage()
            .instance()
            .set(&DataKey::TotalStaked, &total_staked);

        e.storage()
            .persistent()
            .set(&DataKey::UserState(user.clone()), &state);
        e.storage()
            .persistent()
            .extend_ttl(&DataKey::UserState(user.clone()), 10000, 10000);
        e.storage().instance().extend_ttl(10000, 10000);

        e.events().publish(
            (String::from_str(&e, "stake"), user.clone()),
            StakeEvent { user, amount },
        );

        Ok(())
    }

    /// Withdraws staked principal tokens.
    pub fn withdraw(e: Env, user: Address, amount: i128) -> Result<(), ContractError> {
        // Check if staking is paused using granular pause control
        DefaultEmergencyGuard::check_not_paused(&e, PauseType::STAKE)
            .map_err(|_| ContractError::Paused)?;

        if amount <= 0 {
            return Err(ContractError::InvalidInput);
        }

        user.require_auth();

        let config = Self::get_config(e.clone())?;
        let mut state = Self::update_user_rewards_internal(&e, &config, &user)?;

        if state.staked_amount < amount {
            return Err(ContractError::InsufficientBalance);
        }

        state.staked_amount = state
            .staked_amount
            .checked_sub(amount)
            .ok_or(ContractError::Overflow)?;

        // Update total staked
        let mut total_staked: i128 = e
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        total_staked = total_staked
            .checked_sub(amount)
            .ok_or(ContractError::Overflow)?;
        e.storage()
            .instance()
            .set(&DataKey::TotalStaked, &total_staked);

        if state.staked_amount == 0 && state.accrued_rewards == 0 {
            e.storage()
                .persistent()
                .remove(&DataKey::UserState(user.clone()));
        } else {
            e.storage()
                .persistent()
                .set(&DataKey::UserState(user.clone()), &state);
            e.storage()
                .persistent()
                .extend_ttl(&DataKey::UserState(user.clone()), 10000, 10000);
        }
        e.storage().instance().extend_ttl(10000, 10000);

        // Transfer staking tokens back to user
        token::Client::new(&e, &config.staking_token).transfer(
            &e.current_contract_address(),
            &user,
            &amount,
        );

        e.events().publish(
            (String::from_str(&e, "withdraw"), user.clone()),
            WithdrawEvent { user, amount },
        );

        Ok(())
    }

    /// Claims accrued rewards.
    pub fn claim(e: Env, user: Address) -> Result<i128, ContractError> {
        // Check if staking is paused using granular pause control
        DefaultEmergencyGuard::check_not_paused(&e, PauseType::STAKE)
            .map_err(|_| ContractError::Paused)?;

        if EmergencyGuard::is_paused(e.clone(), PauseType::CLAIM_REWARDS) {
            return Err(ContractError::Paused);
        }

        user.require_auth();

        let config = Self::get_config(e.clone())?;
        let mut state = Self::update_user_rewards_internal(&e, &config, &user)?;
        let reward_amount = state.accrued_rewards;

        if reward_amount <= 0 {
            return Ok(0);
        }

        state.accrued_rewards = 0;

        if state.staked_amount == 0 {
            e.storage()
                .persistent()
                .remove(&DataKey::UserState(user.clone()));
        } else {
            e.storage()
                .persistent()
                .set(&DataKey::UserState(user.clone()), &state);
            e.storage()
                .persistent()
                .extend_ttl(&DataKey::UserState(user.clone()), 10000, 10000);
        }
        e.storage().instance().extend_ttl(10000, 10000);

        // Transfer reward tokens to user
        token::Client::new(&e, &config.reward_token).transfer(
            &e.current_contract_address(),
            &user,
            &reward_amount,
        );

        e.events().publish(
            (String::from_str(&e, "claim"), user.clone()),
            ClaimEvent {
                user,
                amount: reward_amount,
            },
        );

        Ok(reward_amount)
    }

    /// Emergency withdraw: pulls all principal stakings and forfeits all rewards.
    /// Operates even when paused or if the reward token pool is completely dry.
    pub fn emergency_withdraw(e: Env, user: Address) -> Result<i128, ContractError> {
        user.require_auth();

        let config = Self::get_config(e.clone())?;
        let state_key = DataKey::UserState(user.clone());

        if !e.storage().persistent().has(&state_key) {
            return Ok(0);
        }

        let state: UserStakingState = e.storage().persistent().get(&state_key).unwrap();
        let staked_amount = state.staked_amount;

        // Update total staked
        let mut total_staked: i128 = e
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        total_staked = total_staked
            .checked_sub(staked_amount)
            .ok_or(ContractError::Overflow)?;
        e.storage()
            .instance()
            .set(&DataKey::TotalStaked, &total_staked);

        if staked_amount <= 0 {
            return Ok(0);
        }

        // Wipe user state entirely (forfeiting rewards)
        e.storage().persistent().remove(&state_key);
        e.storage().instance().extend_ttl(10000, 10000);

        // Transfer staking tokens back to user
        token::Client::new(&e, &config.staking_token).transfer(
            &e.current_contract_address(),
            &user,
            &staked_amount,
        );

        e.events().publish(
            (String::from_str(&e, "emergency_withdraw"), user.clone()),
            EmergencyWithdrawEvent {
                user,
                amount: staked_amount,
            },
        );

        Ok(staked_amount)
    }

    /// Pause staking operations (admin only).
    pub fn pause_staking(e: Env) -> Result<(), ContractError> {
        let config = Self::get_config(e.clone())?;
    /// Sets the global paused state (owner only).
    pub fn set_paused(e: Env, paused: bool) -> Result<(), ContractError> {
        let mut config = Self::get_config(e.clone())?;
        config.owner.require_auth();

        DefaultEmergencyGuard::set_pause_state(&e, PauseType::STAKE, true)
            .map_err(|_| ContractError::Paused)?;

        e.events().publish(
            (String::from_str(&e, "pause_staking"),),
            PausedEvent { paused: true },
        );

        Ok(())
    }

    /// Resume staking operations (admin only).
    pub fn resume_staking(e: Env) -> Result<(), ContractError> {
        let config = Self::get_config(e.clone())?;
        config.owner.require_auth();

        DefaultEmergencyGuard::set_pause_state(&e, PauseType::STAKE, false)
            .map_err(|_| ContractError::Paused)?;

        e.events().publish(
            (String::from_str(&e, "resume_staking"),),
            PausedEvent { paused: false },
        );

        Ok(())
    }

    /// Emergency pause all operations (requires multi-sig approval).
    pub fn emergency_pause_all(e: Env, approvers: Vec<Address>) -> Result<(), ContractError> {
        DefaultEmergencyGuard::emergency_pause_all(&e, approvers)
            .map_err(|_| ContractError::Paused)?;

        e.events().publish(
            (String::from_str(&e, "emergency_pause_all"),),
            PausedEvent { paused: true },
        );

        Ok(())
    }

    /// Resume all paused operations (requires multi-sig approval).
    pub fn resume_all(e: Env, approvers: Vec<Address>) -> Result<(), ContractError> {
        DefaultEmergencyGuard::resume_all(&e, approvers)
            .map_err(|_| ContractError::Paused)?;

        e.events().publish(
            (String::from_str(&e, "resume_all"),),
            PausedEvent { paused: false },
        );

        Ok(())
    }

    /// Get current pause state.
    pub fn get_pause_state(e: Env) -> u32 {
        DefaultEmergencyGuard::get_pause_state(&e)
    }

    /// Check if staking is paused.
    pub fn is_staking_paused(e: Env) -> bool {
        let state = DefaultEmergencyGuard::get_pause_state(&e);
        let pause_type = PauseType::new(state);
        pause_type.is_paused(PauseType::STAKE)
    }

    /// Get list of admins.
    pub fn get_admins(e: Env) -> Vec<Address> {
        DefaultEmergencyGuard::get_admins(&e)
    }

    /// Get multi-sig threshold.
    pub fn get_threshold(e: Env) -> u32 {
        DefaultEmergencyGuard::get_threshold(&e)
    }

    /// Add new admin (multi-sig required).
    pub fn add_admin(e: Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), ContractError> {
        DefaultEmergencyGuard::add_admin(&e, approvers, new_admin)
            .map_err(|_| ContractError::Paused)
    }

    /// Remove admin (multi-sig required).
    pub fn remove_admin(e: Env, approvers: Vec<Address>, admin: Address) -> Result<(), ContractError> {
        DefaultEmergencyGuard::remove_admin(&e, approvers, admin)
            .map_err(|_| ContractError::Paused)
    }

    /// Rotate admin (multi-sig required).
    pub fn rotate_admin(e: Env, approvers: Vec<Address>, old_admin: Address, new_admin: Address) -> Result<(), ContractError> {
        DefaultEmergencyGuard::rotate_admin(&e, approvers, old_admin, new_admin)
    /// Granularly pause or unpause the claim_rewards operation (owner only).
    /// This is independent of the global `is_paused` flag and uses the
    /// embedded EmergencyGuard bitmask (PauseType::CLAIM_REWARDS).
    pub fn set_claim_rewards_paused(e: Env, paused: bool) -> Result<(), ContractError> {
        let config = Self::get_config(e.clone())?;
        // `EmergencyGuard::set_pause` performs the ownership auth check itself,
        // so we pass the owner through directly to avoid double-auth failures
        // when the same signer is reused within the same transaction.
        EmergencyGuard::set_pause(e, config.owner, PauseType::CLAIM_REWARDS, paused)
            .map_err(|_| ContractError::Paused)
    }

    // ── View Functions ──────────────────────────────────────────

    /// Returns the staked principal balance of the user.
    pub fn get_staked_balance(e: Env, user: Address) -> i128 {
        let state_key = DataKey::UserState(user);
        if e.storage().persistent().has(&state_key) {
            let state: UserStakingState = e.storage().persistent().get(&state_key).unwrap();
            state.staked_amount
        } else {
            0
        }
    }

    /// Returns the accrued rewards saved during the last update.
    pub fn get_accrued_rewards(e: Env, user: Address) -> i128 {
        let state_key = DataKey::UserState(user);
        if e.storage().persistent().has(&state_key) {
            let state: UserStakingState = e.storage().persistent().get(&state_key).unwrap();
            state.accrued_rewards
        } else {
            0
        }
    }

    /// Returns the real-time pending rewards (accrued + interest accumulated since last update).
    pub fn get_pending_rewards(e: Env, user: Address) -> i128 {
        let config_res = Self::get_config(e.clone());
        if config_res.is_err() {
            return 0;
        }
        let config = config_res.unwrap();
        let state_key = DataKey::UserState(user);

        if !e.storage().persistent().has(&state_key) {
            return 0;
        }

        let state: UserStakingState = e.storage().persistent().get(&state_key).unwrap();
        let t_curr = e.ledger().sequence();

        if state.staked_amount > 0 && t_curr > state.last_update_block {
            // Time-based reward calculation: V_new = V_old * multiplier, where
            // multiplier = exp(integral of reward rate over time). Rewards are
            // computed as R_new = V_new - staked_amount to avoid rounding errors.
            let multiplier_res = calculate_multiplier(&config, state.last_update_block, t_curr);
            if let Ok(multiplier) = multiplier_res {
                let v_old_res = state.staked_amount.checked_add(state.accrued_rewards);
                if let Some(v_old) = v_old_res {
                    let v_new_res = multiply_amount(v_old, multiplier);
                    if let Ok(v_new) = v_new_res {
                        let r_new_res = v_new.checked_sub(state.staked_amount);
                        if let Some(r_new) = r_new_res {
                            return r_new;
                        }
                    }
                }
            }
        }

        state.accrued_rewards
    }

    /// Returns the contract's configuration.
    pub fn get_config(e: Env) -> Result<StakingConfig, ContractError> {
        e.storage()
            .instance()
            .get(&DataKey::Config)
            .ok_or(ContractError::NotInitialized)
    }

    // ── Internal Helpers ────────────────────────────────────────

    fn update_user_rewards_internal(
        e: &Env,
        config: &StakingConfig,
        user: &Address,
    ) -> Result<UserStakingState, ContractError> {
        let state_key = DataKey::UserState(user.clone());
        let mut state = if e.storage().persistent().has(&state_key) {
            e.storage().persistent().get(&state_key).unwrap()
        } else {
            UserStakingState {
                staked_amount: 0,
                accrued_rewards: 0,
                last_update_block: e.ledger().sequence().max(config.start_block),
            }
        };

        let t_curr = e.ledger().sequence();

        if state.staked_amount > 0 && t_curr > state.last_update_block {
            // Time-based reward calculation: V_new = V_old * multiplier, where
            // multiplier = exp(integral of reward rate over time). Rewards are
            // computed as R_new = V_new - staked_amount to avoid rounding errors.
            let multiplier = calculate_multiplier(config, state.last_update_block, t_curr)?;

            // Virtual Balance V = S + R
            let v_old = state
                .staked_amount
                .checked_add(state.accrued_rewards)
                .ok_or(ContractError::Overflow)?;

            // V_new = v_old * multiplier
            let v_new = multiply_amount(v_old, multiplier)?;

            // R_new = V_new - S
            let r_new = v_new
                .checked_sub(state.staked_amount)
                .ok_or(ContractError::Overflow)?;

            state.accrued_rewards = r_new;
        }

        state.last_update_block = t_curr.max(config.start_block);
        Ok(state)
    }
}
mod test;
