#![no_std]

use emergency_guard::{DefaultEmergencyGuard, EmergencyGuardTrait, GuardError, GuardDataKey, PauseType};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, vec, Address, Env, String, Vec,
};


#[cfg(test)]
mod fuzz_test;
#[cfg(test)]
mod test;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    InsufficientBalance = 4,
    InsufficientLiquidity = 5,
    InsufficientShares = 6,
    InsufficientAllowance = 7,
    SlippageExceeded = 8,
    InvalidFee = 9,
    OracleNotConfigured = 10,
    PendingFeeUpdateExists = 11,
    TimelockNotElapsed = 12,
    NoPendingFeeUpdate = 13,
    Paused = 14,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DepositEvent {
    pub user: Address,
    pub amount_a: i128,
    pub amount_b: i128,
    pub shares_minted: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SwapEvent {
    pub user: Address,
    pub token_in: Address,
    pub token_out: Address,
    pub amount_in: i128,
    pub amount_out: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WithdrawEvent {
    pub user: Address,
    pub shares_burned: i128,
    pub amount_a: i128,
    pub amount_b: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BurnEvent {
    pub user: Address,
    pub shares_burned: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeChangedEvent {
    pub admin: Address,
    pub old_fee_bps: i128,
    pub new_fee_bps: i128,
}

pub const MAX_FEE_BPS: i128 = 100;
pub const DEFAULT_BASE_FEE_BPS: i128 = 30;
pub const DEFAULT_FEE_TIMELOCK_LEDGERS: u32 = 120;

pub const LOW_VOLATILITY_THRESHOLD_BPS: i128 = 100;
pub const MEDIUM_VOLATILITY_THRESHOLD_BPS: i128 = 250;
pub const HIGH_VOLATILITY_THRESHOLD_BPS: i128 = 500;

pub const LOW_VOLATILITY_FEE_BPS: i128 = 40;
pub const MEDIUM_VOLATILITY_FEE_BPS: i128 = 70;
pub const HIGH_VOLATILITY_FEE_BPS: i128 = 100;

pub mod pause_op {
    pub const SWAP: u32 = 1 << 0;
    pub const DEPOSIT: u32 = 1 << 1;
    pub const WITHDRAW: u32 = 1 << 2;
    pub const TRANSFER: u32 = 1 << 3;
    pub const MINT: u32 = 1 << 4;
    pub const BURN: u32 = 1 << 5;
    pub const ALL: u32 = u32::MAX;
}

#[soroban_sdk::contractclient(name = "PriceOracleClient")]
pub trait PriceOracle {
    fn latest_price(e: Env) -> i128;
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PoolState {
    pub token_a: Address,
    pub token_b: Address,
    pub reserve_a: i128,
    pub reserve_b: i128,
    pub total_shares: i128,
    pub fee_bps: i128,
    pub base_fee_bps: i128,
    pub admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingFeeUpdate {
    pub new_fee_bps: i128,
    pub executable_after_ledger: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleConfig {
    pub oracle_id: Address,
    pub base_fee_bps: i128,
    pub timelock_ledgers: u32,
}

#[derive(Clone)]
#[contracttype]
pub struct AllowanceDataKey {
    pub from: Address,
    pub spender: Address,
}

#[derive(Clone)]
#[contracttype]
pub struct AllowanceValue {
    pub amount: i128,
    pub expiration_ledger: u32,
}

#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    Pool,
    Admin,
    Balance(Address),
    Allowance(AllowanceDataKey),
    OracleConfig,
    LastOraclePrice,
    LastVolatilityBps,
    PendingFeeUpdate,
    // Staking
    StakedBalance(Address),
    StakeStartLedger(Address),
    TotalStaked,
    PendingRewards(Address),
}


fn sqrt(x: i128) -> i128 {
    if x == 0 {
        return 0;
    }
    let mut z = (x + 1) / 2;
    let mut y = x;
    while z < y {
        y = z;
        z = (x / z + z) / 2;
    }
    y
}

fn load_pool(e: &Env) -> Result<PoolState, Error> {
    e.storage()
        .instance()
        .get(&DataKey::Pool)
        .ok_or(Error::NotInitialized)
}

fn save_pool(e: &Env, pool: &PoolState) {
    e.storage().instance().set(&DataKey::Pool, pool);
}

fn target_fee_from_volatility(base_fee_bps: i128, volatility_bps: i128) -> i128 {
    let dynamic = if volatility_bps >= HIGH_VOLATILITY_THRESHOLD_BPS {
        HIGH_VOLATILITY_FEE_BPS
    } else if volatility_bps >= MEDIUM_VOLATILITY_THRESHOLD_BPS {
        MEDIUM_VOLATILITY_FEE_BPS
    } else if volatility_bps >= LOW_VOLATILITY_THRESHOLD_BPS {
        LOW_VOLATILITY_FEE_BPS
    } else {
        base_fee_bps
    };
    if dynamic > MAX_FEE_BPS {
        MAX_FEE_BPS
    } else {
        dynamic
    }
}

fn guard_init(e: &Env, admin: Address) {
    let admins = vec![e, admin];
    e.storage().instance().set(&GuardDataKey::Admins, &admins);
    e.storage().instance().set(&GuardDataKey::SignatureThreshold, &1u32);
    e.storage().instance().set(&GuardDataKey::PauseState, &PauseType::new(0));
}

fn guard_pause_state(e: &Env) -> u32 {
    let ps: PauseType = e
        .storage()
        .instance()
        .get(&GuardDataKey::PauseState)
        .unwrap_or(PauseType::new(0));
    ps.as_u32()
}

fn read_admin(e: &Env) -> Result<Address, Error> {
    e.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)
}

fn read_guard_admins(e: &Env) -> Vec<Address> {
    e.storage()
        .instance()
        .get(&GuardDataKey::Admins)
        .unwrap_or_else(|| Vec::new(e))
}

fn write_guard_admins(e: &Env, admins: &Vec<Address>) {
    e.storage().instance().set(&GuardDataKey::Admins, admins);
}

/// Return `Err(Error::Paused)` if `op` is currently paused.
fn guard_check_not_paused(e: &Env, op: u32) -> Result<(), Error> {
    if guard_pause_state(e) & op != 0 {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

fn guard_admins(e: &Env) -> Vec<Address> {
    e.storage()
        .instance()
        .get(&GuardDataKey::Admins)
        .unwrap_or_else(|| Vec::new(e))
}

fn guard_is_admin(e: &Env, addr: &Address) -> bool {
    guard_admins(e).iter().any(|a| a == *addr)
}

fn guard_require_admin(e: &Env, caller: &Address) -> Result<(), Error> {
    caller.require_auth();
    if !guard_is_admin(e, caller) {
        return Err(Error::Unauthorized);
    }
    Ok(())
}

fn guard_require_multisig(e: &Env, approvers: &Vec<Address>) -> Result<(), Error> {
    let threshold: u32 = e
        .storage()
        .instance()
        .get(&GuardDataKey::SignatureThreshold)
        .unwrap_or(1u32);

    let mut valid = 0u32;
    let mut seen = Vec::new(e);

    for addr in approvers.iter() {
        if seen.iter().any(|a| a == addr) {
            continue;
        }
        seen.push_back(addr.clone());
        if guard_is_admin(e, &addr) {
            addr.require_auth();
            valid += 1;
        }
    }

    if valid < threshold {
        return Err(Error::Unauthorized);
    }
    Ok(())
}

fn guard_set_ops(e: &Env, ops: u32, paused: bool) {
    let current_raw = guard_pause_state(e);
    let new_raw = if paused {
        current_raw | ops
    } else {
        current_raw & !ops
    };
    e.storage()
        .instance()
        .set(&GuardDataKey::PauseState, &PauseType::new(new_raw));
}

fn set_primary_admin(e: &Env, admin: Address) -> Result<(), Error> {
    e.storage().instance().set(&DataKey::Admin, &admin);
    let mut pool = load_pool(e)?;
    pool.admin = admin;
    save_pool(e, &pool);
    Ok(())
}

/// Implement EmergencyGuardTrait for LiquidityPool by delegating to DefaultEmergencyGuard
impl EmergencyGuardTrait for LiquidityPool {
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError> {
        DefaultEmergencyGuard::check_not_paused(env, operation)
    }

    fn get_pause_state(env: &Env) -> u32 {
        DefaultEmergencyGuard::get_pause_state(env)
    }

    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        DefaultEmergencyGuard::set_pause_state(env, operation, paused)
    }

    fn unpause(env: &Env, operation: u32) -> Result<(), GuardError> {
        DefaultEmergencyGuard::unpause(env, operation)
    }

    fn unpause_all(env: &Env) -> Result<(), GuardError> {
        DefaultEmergencyGuard::unpause_all(env)
    }

    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        DefaultEmergencyGuard::emergency_pause_all(env, approvers)
    }

    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        DefaultEmergencyGuard::resume_all(env, approvers)
    }

    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        DefaultEmergencyGuard::init_guard(env, admins, threshold)
    }

    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError> {
        DefaultEmergencyGuard::add_admin(env, approvers, new_admin)
    }

    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        DefaultEmergencyGuard::remove_admin(env, approvers, admin)
    }

    fn rotate_admin(env: &Env, approvers: Vec<Address>, old_admin: Address, new_admin: Address) -> Result<(), GuardError> {
        DefaultEmergencyGuard::rotate_admin(env, approvers, old_admin, new_admin)
    }

    fn get_admins(env: &Env) -> Vec<Address> {
        DefaultEmergencyGuard::get_admins(env)
    }

    fn get_threshold(env: &Env) -> u32 {
        DefaultEmergencyGuard::get_threshold(env)
    }

    fn is_admin(env: &Env, addr: Address) -> bool {
        DefaultEmergencyGuard::is_admin(env, addr)
    }
}



#[contract]
pub struct LiquidityPool;

#[contractimpl]
impl LiquidityPool {
    pub fn initialize(
        e: Env,
        admin: Address,
        token_a: Address,
        token_b: Address,
    ) -> Result<(), Error> {
        if e.storage().instance().has(&DataKey::Pool) {
            return Err(Error::AlreadyInitialized);
        }

        e.storage().instance().set(&DataKey::Admin, &admin);
        save_pool(
            &e,
            &PoolState {
                token_a,
                token_b,
                reserve_a: 0,
                reserve_b: 0,
                total_shares: 0,
                fee_bps: DEFAULT_BASE_FEE_BPS,
                base_fee_bps: DEFAULT_BASE_FEE_BPS,
                admin: admin.clone(),
            },
        );
        guard_init(&e, admin);
        Ok(())
    }

    // ── Granular pause (EmergencyGuard admin interface) ───────────────────────

    /// Pause or resume one operation bit. Any current guard admin may call this.
    pub fn set_operation_paused(
        e: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), Error> {
        guard_require_admin(&e, &admin)?;
        guard_set_ops(&e, operation, paused);
        Ok(())
    }

    /// Backward-compatible granular pause entry point.
    pub fn guard_pause(
        e: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), Error> {
        Self::set_operation_paused(e, admin, operation, paused)
    }

    /// Pause only swap operations.
    pub fn pause_swaps(e: Env) -> Result<(), Error> {
        let admin = read_admin(&e)?;
        Self::set_operation_paused(e, admin, pause_op::SWAP, true)
    }

    pub fn resume_swaps(e: Env) -> Result<(), Error> {
        let admin = read_admin(&e)?;
        Self::set_operation_paused(e, admin, pause_op::SWAP, false)
    }

    pub fn pause_deposits(e: Env) -> Result<(), Error> {
        let admin = read_admin(&e)?;
        Self::set_operation_paused(e, admin, pause_op::DEPOSIT, true)
    }

    pub fn resume_deposits(e: Env) -> Result<(), Error> {
        let admin = read_admin(&e)?;
        Self::set_operation_paused(e, admin, pause_op::DEPOSIT, false)
    }

    pub fn pause_withdrawals(e: Env) -> Result<(), Error> {
        let admin = read_admin(&e)?;
        Self::set_operation_paused(e, admin, pause_op::WITHDRAW, true)
    }

    pub fn resume_withdrawals(e: Env) -> Result<(), Error> {
        let admin = read_admin(&e)?;
        Self::set_operation_paused(e, admin, pause_op::WITHDRAW, false)
    }

    /// Emergency: pause all guarded operations via multi-sig approval.
    pub fn emergency_pause(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        guard_require_multisig(&e, &approvers)?;
        guard_set_ops(&e, pause_op::ALL, true);
        Ok(())
    }

    /// Alias for callers that use the explicit "all" naming.
    pub fn emergency_pause_all(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        Self::emergency_pause(e, approvers)
    }

    /// Unpause all guarded operations via multi-sig approval.
    ///
    /// This replaces the old single-admin boolean unpause flow. The approver
    /// list is validated by the same guard threshold used for emergency pause.
    pub fn guard_unpause(e: Env, admin: Address, operation: u32) -> Result<(), Error> {
        guard_require_admin(&e, &admin)?;
        guard_set_ops(&e, operation, false);
        Ok(())
    }

    /// Backward-compatible resume entry point for existing callers.
    pub fn resume(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        guard_require_multisig(&e, &approvers)?;
        guard_set_ops(&e, pause_op::ALL, false);
        Ok(())
    }

    /// Alias for callers that use the explicit "all" naming.
    pub fn resume_all(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        Self::resume(e, approvers)
    }

    /// Returns the raw pause-state bitmask.
    pub fn get_pause_state(e: Env) -> u32 {
        guard_pause_state(&e)
    }

    /// Returns `true` when `operation` is currently paused.
    pub fn guard_is_paused(e: Env, operation: u32) -> bool {
        guard_pause_state(&e) & operation != 0
    }

    /// Alias for callers that use operation-centric naming.
    pub fn is_paused_op(e: Env, operation: u32) -> bool {
        Self::guard_is_paused(e, operation)
    }

    /// Simple boolean pause/unpause for all operations (single admin).
    pub fn set_paused(e: Env, paused: bool) -> Result<(), Error> {
        let admin = read_admin(&e)?;
        admin.require_auth();
        if paused {
            guard_set_ops(&e, pause_op::ALL, true);
        } else {
            guard_set_ops(&e, pause_op::ALL, false);
        }
        Ok(())
    }

    /// Returns the list of authorized guard admins.
    pub fn get_admins(e: Env) -> Vec<Address> {
        read_guard_admins(&e)
    }

    /// Alias for callers that use guard-specific naming.
    pub fn get_guard_admins(e: Env) -> Vec<Address> {
        read_guard_admins(&e)
    }

    /// Returns the current multi-sig approval threshold.
    pub fn get_admin_threshold(e: Env) -> u32 {
        e.storage()
            .instance()
            .get(&GuardDataKey::SignatureThreshold)
            .unwrap_or(0)
    }

    /// Alias for callers that use guard-specific naming.
    pub fn get_guard_threshold(e: Env) -> u32 {
        Self::get_admin_threshold(e)
    }

    /// Returns the current primary pool admin.
    pub fn get_admin(e: Env) -> Result<Address, Error> {
        read_admin(&e)
    }

    /// Add a new guard admin. Requires the current multi-sig threshold.
    pub fn add_admin(e: Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), Error> {
        guard_require_multisig(&e, &approvers)?;
        let mut admins = read_guard_admins(&e);
        if !admins.iter().any(|a| a == new_admin) {
            admins.push_back(new_admin);
            write_guard_admins(&e, &admins);
        }
        Ok(())
    }

    /// Alias for callers that use guard-specific naming.
    pub fn add_guard_admin(
        e: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), Error> {
        Self::add_admin(e, approvers, new_admin)
    }

    /// Remove a guard admin. The admin set cannot be reduced below threshold.
    pub fn remove_admin(e: Env, approvers: Vec<Address>, admin: Address) -> Result<(), Error> {
        guard_require_multisig(&e, &approvers)?;
        let admins = read_guard_admins(&e);
        let threshold = Self::get_admin_threshold(e.clone());
        if admins.len() as u32 <= threshold {
            return Err(Error::Unauthorized);
        }

        let mut next_admins = Vec::new(&e);
        let mut found = false;
        for candidate in admins.iter() {
            if candidate == admin {
                found = true;
            } else {
                next_admins.push_back(candidate);
            }
        }
        if !found {
            return Err(Error::Unauthorized);
        }

        write_guard_admins(&e, &next_admins);
        if read_admin(&e)? == admin {
            if let Some(next_primary) = next_admins.get(0) {
                e.storage().instance().set(&DataKey::Admin, &next_primary);
            }
        }
        Ok(())
    }

    /// Alias for callers that use guard-specific naming.
    pub fn remove_guard_admin(
        e: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), Error> {
        Self::remove_admin(e, approvers, admin)
    }

    /// Replace one guard admin with another and update the primary pool admin
    /// when the replaced address is the current primary admin.
    pub fn rotate_admin(
        e: Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), Error> {
        guard_require_multisig(&e, &approvers)?;
        if !guard_is_admin(&e, &old_admin) {
            return Err(Error::Unauthorized);
        }

        let admins = read_guard_admins(&e);
        let mut next_admins = Vec::new(&e);
        for candidate in admins.iter() {
            if candidate == old_admin {
                if !next_admins.iter().any(|a| a == new_admin) {
                    next_admins.push_back(new_admin.clone());
                }
            } else if !next_admins.iter().any(|a| a == candidate) {
                next_admins.push_back(candidate);
            }
        }

        write_guard_admins(&e, &next_admins);
        if read_admin(&e)? == old_admin {
            e.storage().instance().set(&DataKey::Admin, &new_admin);
        }
        Ok(())
    }



    pub fn get_fee(e: Env) -> i128 {
        load_pool(&e)
            .map(|p| p.fee_bps)
            .unwrap_or(DEFAULT_BASE_FEE_BPS)
    }

    pub fn set_fee(e: Env, fee_bps: i128) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&fee_bps) {
            return Err(Error::InvalidFee);
        }

        let admin = read_admin(&e)?;
        admin.require_auth();
        let mut pool = load_pool(&e)?;
        let old_fee = pool.fee_bps;
        pool.fee_bps = fee_bps;
        save_pool(&e, &pool);

        e.events().publish(
            (String::from_str(&e, "fee_changed"), admin.clone()),
            FeeChangedEvent {
                admin,
                old_fee_bps: old_fee,
                new_fee_bps: fee_bps,
            },
        );
        Ok(())
    }

    /// Admin-only: configure the price oracle for dynamic fee adjustment.
    pub fn configure_fee_oracle(
        e: Env,
        oracle_id: Address,
        base_fee_bps: i128,
        timelock_ledgers: u32,
    ) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&base_fee_bps) {
            return Err(Error::InvalidFee);
        }
        let admin = read_admin(&e)?;
        admin.require_auth();
        e.storage().instance().set(
            &DataKey::OracleConfig,
            &OracleConfig {
                oracle_id,
                base_fee_bps,
                timelock_ledgers,
            },
        );
        Ok(())
    }

    pub fn sync_fee_from_oracle(e: Env) -> Result<Option<PendingFeeUpdate>, Error> {
        let cfg: OracleConfig = e
            .storage()
            .instance()
            .get(&DataKey::OracleConfig)
            .ok_or(Error::OracleNotConfigured)?;

        let oracle = PriceOracleClient::new(&e, &cfg.oracle_id);
        let current_price = oracle.latest_price();

        let Some(last_price) = e
            .storage()
            .instance()
            .get::<_, i128>(&DataKey::LastOraclePrice)
        else {
            e.storage()
                .instance()
                .set(&DataKey::LastOraclePrice, &current_price);
            return Ok(None);
        };

        let delta = if current_price >= last_price {
            current_price - last_price
        } else {
            last_price - current_price
        };
        let volatility_bps = if last_price == 0 {
            0
        } else {
            delta * 10_000 / last_price
        };

        e.storage()
            .instance()
            .set(&DataKey::LastOraclePrice, &current_price);
        e.storage()
            .instance()
            .set(&DataKey::LastVolatilityBps, &volatility_bps);

        let pool = load_pool(&e)?;
        let target_fee = target_fee_from_volatility(cfg.base_fee_bps, volatility_bps);
        if target_fee == pool.fee_bps {
            return Ok(None);
        }

        let pending = PendingFeeUpdate {
            new_fee_bps: target_fee,
            executable_after_ledger: e.ledger().sequence() + cfg.timelock_ledgers,
        };
        e.storage()
            .instance()
            .set(&DataKey::PendingFeeUpdate, &pending);
        Ok(Some(pending))
    }

    pub fn execute_fee_update(e: Env) -> Result<i128, Error> {
        let pending: PendingFeeUpdate = e
            .storage()
            .instance()
            .get(&DataKey::PendingFeeUpdate)
            .ok_or(Error::NoPendingFeeUpdate)?;

        if e.ledger().sequence() < pending.executable_after_ledger {
            return Err(Error::TimelockNotElapsed);
        }

        let mut pool = load_pool(&e)?;
        pool.fee_bps = pending.new_fee_bps;
        save_pool(&e, &pool);
        e.storage().instance().remove(&DataKey::PendingFeeUpdate);
        Ok(pending.new_fee_bps)
    }

    pub fn get_pending_fee_update(e: Env) -> Option<PendingFeeUpdate> {
        e.storage().instance().get(&DataKey::PendingFeeUpdate)
    }

    pub fn get_last_volatility_bps(e: Env) -> i128 {
        e.storage()
            .instance()
            .get(&DataKey::LastVolatilityBps)
            .unwrap_or(0)
    }

    pub fn deposit(e: Env, to: Address, amount_a: i128, amount_b: i128) -> Result<i128, Error> {
        guard_check_not_paused(&e, pause_op::DEPOSIT)?;
        to.require_auth();

        let mut pool = load_pool(&e)?;
        let client_a = soroban_sdk::token::Client::new(&e, &pool.token_a);
        let client_b = soroban_sdk::token::Client::new(&e, &pool.token_b);
        client_a.transfer(&to, &e.current_contract_address(), &amount_a);
        client_b.transfer(&to, &e.current_contract_address(), &amount_b);

        let shares = if pool.total_shares == 0 {
            sqrt(
                amount_a
                    .checked_mul(amount_b)
                    .ok_or(Error::InsufficientLiquidity)?,
            )
        } else {
            let share_a = amount_a
                .checked_mul(pool.total_shares)
                .ok_or(Error::InsufficientLiquidity)?
                / pool.reserve_a;
            let share_b = amount_b
                .checked_mul(pool.total_shares)
                .ok_or(Error::InsufficientLiquidity)?
                / pool.reserve_b;
            if share_a < share_b {
                share_a
            } else {
                share_b
            }
        };

        let user_key = DataKey::Balance(to.clone());
        let current = e
            .storage()
            .persistent()
            .get::<_, i128>(&user_key)
            .unwrap_or(0);
        e.storage().persistent().set(&user_key, &(current + shares));
        e.storage().persistent().extend_ttl(&user_key, 100, 100);

        pool.total_shares += shares;
        pool.reserve_a += amount_a;
        pool.reserve_b += amount_b;
        save_pool(&e, &pool);

        e.events().publish(
            (String::from_str(&e, "deposit"), to.clone()),
            DepositEvent {
                user: to,
                amount_a,
                amount_b,
                shares_minted: shares,
            },
        );

        Ok(shares)
    }

    pub fn swap(e: Env, to: Address, buy_a: bool, out: i128, in_max: i128) -> Result<i128, Error> {
        guard_check_not_paused(&e, pause_op::SWAP)?;
        to.require_auth();

        let mut pool = load_pool(&e)?;
        let (reserve_in, reserve_out, token_in, token_out) = if buy_a {
            (
                pool.reserve_b,
                pool.reserve_a,
                pool.token_b.clone(),
                pool.token_a.clone(),
            )
        } else {
            (
                pool.reserve_a,
                pool.reserve_b,
                pool.token_a.clone(),
                pool.token_b.clone(),
            )
        };

        if out >= reserve_out {
            return Err(Error::InsufficientLiquidity);
        }

        let fee_scale = 10_000i128 - pool.fee_bps;
        let numerator = reserve_in
            .checked_mul(out)
            .ok_or(Error::InsufficientLiquidity)?
            .checked_mul(10_000)
            .ok_or(Error::InsufficientLiquidity)?;
        let denominator = (reserve_out - out)
            .checked_mul(fee_scale)
            .ok_or(Error::InsufficientLiquidity)?;
        let amount_in = (numerator / denominator) + 1;

        if amount_in > in_max {
            return Err(Error::SlippageExceeded);
        }

        soroban_sdk::token::Client::new(&e, &token_in).transfer(
            &to,
            &e.current_contract_address(),
            &amount_in,
        );
        soroban_sdk::token::Client::new(&e, &token_out).transfer(
            &e.current_contract_address(),
            &to,
            &out,
        );

        if buy_a {
            pool.reserve_a -= out;
            pool.reserve_b += amount_in;
        } else {
            pool.reserve_a += amount_in;
            pool.reserve_b -= out;
        }
        save_pool(&e, &pool);

        e.events().publish(
            (String::from_str(&e, "swap"), to.clone()),
            SwapEvent {
                user: to,
                token_in,
                token_out,
                amount_in,
                amount_out: out,
            },
        );

        Ok(amount_in)
    }

    pub fn withdraw(e: Env, to: Address, share_amount: i128) -> Result<(i128, i128), Error> {
        guard_check_not_paused(&e, pause_op::WITHDRAW)?;
        to.require_auth();

        let mut pool = load_pool(&e)?;
        let user_key = DataKey::Balance(to.clone());
        let current = e
            .storage()
            .persistent()
            .get::<_, i128>(&user_key)
            .unwrap_or(0);
        if share_amount > current {
            return Err(Error::InsufficientShares);
        }

        let amount_a = share_amount * pool.reserve_a / pool.total_shares;
        let amount_b = share_amount * pool.reserve_b / pool.total_shares;

        e.storage()
            .persistent()
            .set(&user_key, &(current - share_amount));
        e.storage().persistent().extend_ttl(&user_key, 100, 100);

        pool.total_shares -= share_amount;
        pool.reserve_a -= amount_a;
        pool.reserve_b -= amount_b;
        let token_a = pool.token_a.clone();
        let token_b = pool.token_b.clone();
        save_pool(&e, &pool);

        soroban_sdk::token::Client::new(&e, &token_a).transfer(
            &e.current_contract_address(),
            &to,
            &amount_a,
        );
        soroban_sdk::token::Client::new(&e, &token_b).transfer(
            &e.current_contract_address(),
            &to,
            &amount_b,
        );

        e.events().publish(
            (String::from_str(&e, "withdraw"), to.clone()),
            WithdrawEvent {
                user: to,
                shares_burned: share_amount,
                amount_a,
                amount_b,
            },
        );

        Ok((amount_a, amount_b))
    }

    pub fn burn(e: Env, from: Address, amount: i128) -> Result<(), Error> {
        guard_check_not_paused(&e, pause_op::BURN)?;
        from.require_auth();

        let mut pool = load_pool(&e)?;
        let user_key = DataKey::Balance(from.clone());
        let current = e
            .storage()
            .persistent()
            .get::<_, i128>(&user_key)
            .unwrap_or(0);
        if amount > current {
            return Err(Error::InsufficientShares);
        }

        e.storage().persistent().set(&user_key, &(current - amount));
        e.storage().persistent().extend_ttl(&user_key, 100, 100);

        pool.total_shares -= amount;
        save_pool(&e, &pool);

        e.events().publish(
            (String::from_str(&e, "burn"), from.clone()),
            BurnEvent {
                user: from,
                shares_burned: amount,
            },
        );

        Ok(())
    }

    pub fn name(e: Env) -> String {
        String::from_str(&e, "Liquidity Pool Share")
    }

    pub fn symbol(e: Env) -> String {
        String::from_str(&e, "LPS")
    }

    pub fn decimals(_e: Env) -> u32 {
        7
    }

    pub fn balance(e: Env, id: Address) -> i128 {
        e.storage()
            .persistent()
            .get(&DataKey::Balance(id))
            .unwrap_or(0)
    }

    pub fn total_supply(e: Env) -> i128 {
        load_pool(&e).map(|p| p.total_shares).unwrap_or(0)
    }

    pub fn transfer(e: Env, from: Address, to: Address, amount: i128) -> Result<(), Error> {
        guard_check_not_paused(&e, pause_op::TRANSFER)?;
        from.require_auth();

        let from_key = DataKey::Balance(from.clone());
        let to_key = DataKey::Balance(to);

        let from_balance = e
            .storage()
            .persistent()
            .get::<_, i128>(&from_key)
            .unwrap_or(0);
        if from_balance < amount {
            return Err(Error::InsufficientBalance);
        }

        e.storage()
            .persistent()
            .set(&from_key, &(from_balance - amount));
        e.storage().persistent().extend_ttl(&from_key, 100, 100);

        let to_balance = e
            .storage()
            .persistent()
            .get::<_, i128>(&to_key)
            .unwrap_or(0);
        e.storage()
            .persistent()
            .set(&to_key, &(to_balance + amount));
        e.storage().persistent().extend_ttl(&to_key, 100, 100);

        Ok(())
    }

    pub fn approve(
        e: Env,
        from: Address,
        spender: Address,
        amount: i128,
        expiration_ledger: u32,
    ) -> Result<(), Error> {
        from.require_auth();

        let key = DataKey::Allowance(AllowanceDataKey {
            from: from.clone(),
            spender: spender.clone(),
        });
        e.storage().persistent().set(
            &key,
            &AllowanceValue {
                amount,
                expiration_ledger,
            },
        );
        e.storage().persistent().extend_ttl(&key, 100, 100);

        Ok(())
    }

    pub fn allowance(e: Env, from: Address, spender: Address) -> i128 {
        let key = DataKey::Allowance(AllowanceDataKey { from, spender });
        match e.storage().persistent().get::<_, AllowanceValue>(&key) {
            Some(a) if e.ledger().sequence() <= a.expiration_ledger => a.amount,
            _ => 0,
        }
    }

    pub fn transfer_from(
        e: Env,
        spender: Address,
        from: Address,
        to: Address,
        amount: i128,
    ) -> Result<(), Error> {
        guard_check_not_paused(&e, pause_op::TRANSFER)?;
        spender.require_auth();

        let current_allowance = Self::allowance(e.clone(), from.clone(), spender.clone());
        if current_allowance < amount {
            return Err(Error::InsufficientAllowance);
        }

        let new_allowance = current_allowance - amount;
        let key = DataKey::Allowance(AllowanceDataKey {
            from: from.clone(),
            spender: spender.clone(),
        });

        if new_allowance > 0 {
            let current_val = e
                .storage()
                .persistent()
                .get::<_, AllowanceValue>(&key)
                .unwrap();
            e.storage().persistent().set(
                &key,
                &AllowanceValue {
                    amount: new_allowance,
                    expiration_ledger: current_val.expiration_ledger,
                },
            );
            e.storage().persistent().extend_ttl(&key, 100, 100);
        } else {
            e.storage().persistent().remove(&key);
        }

        Self::transfer(e, from, to, amount)
    }

    // ── Staking ───────────────────────────────────────────────────────────────

    /// Stake `amount` of LP shares to earn rewards.
    /// The shares are locked from the user's balance.
    pub fn stake(e: Env, user: Address, amount: i128) -> Result<(), Error> {
        user.require_auth();
        guard_check_not_paused(&e, pause_op::ALL)?;

        // Deduct shares from user balance
        let bal = e
            .storage()
            .persistent()
            .get::<_, i128>(&DataKey::Balance(user.clone()))
            .unwrap_or(0);
        if bal < amount {
            return Err(Error::InsufficientBalance);
        }

        // Before changing stake, accumulate any pending rewards
        Self::_accrue_rewards(&e, &user);

        e.storage()
            .persistent()
            .set(&DataKey::Balance(user.clone()), &(bal - amount));

        let staked: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::StakedBalance(user.clone()))
            .unwrap_or(0);
        e.storage()
            .persistent()
            .set(&DataKey::StakedBalance(user.clone()), &(staked + amount));

        // Record the current ledger as the stake start (for new stakers)
        if staked == 0 {
            let current_ledger = e.ledger().sequence();
            e.storage()
                .persistent()
                .set(&DataKey::StakeStartLedger(user.clone()), &current_ledger);
        }

        // Update total staked
        let total: i128 = e
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        e.storage()
            .instance()
            .set(&DataKey::TotalStaked, &(total + amount));

        Ok(())
    }

    /// Unstake `amount` of LP shares, returning them to the user's balance.
    pub fn unstake(e: Env, user: Address, amount: i128) -> Result<(), Error> {
        user.require_auth();
        guard_check_not_paused(&e, pause_op::ALL)?;

        let staked: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::StakedBalance(user.clone()))
            .unwrap_or(0);
        if staked < amount {
            return Err(Error::InsufficientShares);
        }

        // Accrue rewards before unstaking
        Self::_accrue_rewards(&e, &user);

        e.storage()
            .persistent()
            .set(&DataKey::StakedBalance(user.clone()), &(staked - amount));

        let bal: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::Balance(user.clone()))
            .unwrap_or(0);
        e.storage()
            .persistent()
            .set(&DataKey::Balance(user.clone()), &(bal + amount));

        // Update total staked
        let total: i128 = e
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        e.storage()
            .instance()
            .set(&DataKey::TotalStaked, &(total - amount));

        // Clear stake ledger if fully unstaked
        if staked == amount {
            e.storage()
                .persistent()
                .remove(&DataKey::StakeStartLedger(user.clone()));
        }

        Ok(())
    }

    /// Claim accrued staking rewards. Returns the reward amount (in token_a units).
    pub fn claim_rewards(e: Env, user: Address) -> Result<i128, Error> {
        user.require_auth();
        guard_check_not_paused(&e, pause_op::ALL)?;

        // Accrue up to now
        Self::_accrue_rewards(&e, &user);

        let pending: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::PendingRewards(user.clone()))
            .unwrap_or(0);

        // Reset pending
        e.storage()
            .persistent()
            .set(&DataKey::PendingRewards(user.clone()), &0i128);

        Ok(pending)
    }

    /// Return the staked LP share balance for `user`.
    pub fn get_staked_balance(e: Env, user: Address) -> i128 {
        // Accrue without auth to get accurate pending
        e.storage()
            .persistent()
            .get(&DataKey::StakedBalance(user))
            .unwrap_or(0)
    }

    /// Return the total LP shares staked across all users.
    pub fn get_total_staked(e: Env) -> i128 {
        e.storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0)
    }

    /// Return the pending (unclaimed) rewards for `user` without claiming them.
    pub fn get_pending_rewards(e: Env, user: Address) -> i128 {
        // We peek at what has accrued up to now without persisting
        let staked: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::StakedBalance(user.clone()))
            .unwrap_or(0);

        if staked == 0 {
            return 0;
        }

        let start_ledger: u32 = e
            .storage()
            .persistent()
            .get(&DataKey::StakeStartLedger(user.clone()))
            .unwrap_or(e.ledger().sequence());

        let current_ledger = e.ledger().sequence();
        let ledgers_elapsed = (current_ledger.saturating_sub(start_ledger)) as i128;

        // Simple linear reward: 1 stroop per staked share per ledger
        let accrued = staked.saturating_mul(ledgers_elapsed);

        let already_pending: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::PendingRewards(user))
            .unwrap_or(0);

        already_pending + accrued
    }

    /// Internal: accumulate rewards from `StakeStartLedger` to current ledger.
    fn _accrue_rewards(e: &Env, user: &Address) {
        let staked: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::StakedBalance(user.clone()))
            .unwrap_or(0);

        if staked == 0 {
            return;
        }

        let start_ledger: u32 = e
            .storage()
            .persistent()
            .get(&DataKey::StakeStartLedger(user.clone()))
            .unwrap_or(e.ledger().sequence());

        let current_ledger = e.ledger().sequence();
        let ledgers_elapsed = (current_ledger.saturating_sub(start_ledger)) as i128;

        // 1 stroop per staked share per ledger
        let accrued = staked.saturating_mul(ledgers_elapsed);

        let pending: i128 = e
            .storage()
            .persistent()
            .get(&DataKey::PendingRewards(user.clone()))
            .unwrap_or(0);

        e.storage()
            .persistent()
            .set(&DataKey::PendingRewards(user.clone()), &(pending + accrued));

        // Reset the start ledger to now
        e.storage()
            .persistent()
            .set(&DataKey::StakeStartLedger(user.clone()), &current_ledger);
    }
}
