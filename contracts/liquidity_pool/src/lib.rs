#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, vec, Address, Env, String, Vec,
};
use emergency_guard::{EmergencyGuardTrait, GuardError, PauseType};
use emergency_guard::{EmergencyGuard, PauseType};
use soroban_sdk::vec;
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String, Vec};
use emergency_guard::{
    emit_admin_added, emit_admin_removed, emit_emergency_paused_all, emit_guard_initialized,
    emit_pause_state_changed, emit_resumed_all, EmergencyGuardTrait, GuardError, PauseType,
    emit_pause_state_changed, emit_resumed_all, EmergencyGuard, GuardError, PauseType, DefaultEmergencyGuard, EmergencyGuardTrait
};
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String, Vec};
use emergency_guard::{EmergencyGuard, GuardError, PauseType};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, panic_with_error, vec, Address, Env,
    String, Vec,
};

#[cfg(test)]
mod fuzz_test;
#[cfg(test)]
mod test;

// ── Errors ────────────────────────────────────────────────────────────────────

/// Errors returned by the `LiquidityPool` contract.
///
/// Code assignments are stable on-chain ABI — do NOT renumber.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    InsufficientLiquidity = 2,
    SlippageExceeded = 3,
    InsufficientShares = 4,
    NotInitialized = 5,
    InsufficientBalance = 6,
    Unauthorized = 7,
    InsufficientAllowance = 8,
    InvalidFee = 9,
    OracleNotConfigured = 10,
    InvalidOraclePrice = 11,
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
    InsufficientLiquidity = 2,
    SlippageExceeded = 3,
    InsufficientShares = 4,
    NotInitialized = 5,
    InsufficientBalance = 6,
    Unauthorized = 7,
    InsufficientAllowance = 8,
    InvalidFee = 9,
    OracleNotConfigured = 10,
    InvalidOraclePrice = 11,
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

/// Event payload emitted after a successful stake.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StakeEvent {
    /// Address that staked LP shares.
    pub user: Address,
    /// LP shares staked.
    pub amount_staked: i128,
}

/// Event payload emitted after a successful unstake.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnstakeEvent {
    /// Address that unstaked LP shares.
    pub user: Address,
    /// LP shares unstaked.
    pub amount_unstaked: i128,
}

/// Event payload emitted after claiming rewards.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ClaimRewardsEvent {
    /// Address that claimed rewards.
    pub user: Address,
    /// Amount of rewards claimed.
    pub rewards_amount: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeChangedEvent {
    pub admin: Address,
    pub old_fee_bps: i128,
    pub new_fee_bps: i128,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeUpdateScheduledEvent {
    pub scheduled_by: Address,
    pub old_fee_bps: i128,
    pub new_fee_bps: i128,
    pub executable_after_ledger: u32,
    pub volatility_bps: i128,
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
pub struct GuardState {
    pub pause_state: u32,
    pub admins: Vec<Address>,
    pub signature_threshold: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleConfig {
    pub oracle: Address,
    pub last_price: i128,
    pub last_volatility_bps: i128,
    pub timelock_ledgers: u32,
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
// ── Oracle interface ──────────────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "PriceOracleClient")]
pub trait PriceOracle {
    fn latest_price(e: Env) -> i128;
}

// ── On-chain data types ───────────────────────────────────────────────────────

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
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeUpdateScheduledEvent {
    pub scheduled_by: Address,
    pub old_fee_bps: i128,
    pub new_fee_bps: i128,
    pub executable_after_ledger: u32,
    pub volatility_bps: i128,
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
pub struct GuardState {
    pub pause_state: u32,
    pub admins: Vec<Address>,
    pub signature_threshold: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleConfig {
    pub oracle: Address,
    pub last_price: i128,
    pub last_volatility_bps: i128,
    pub timelock_ledgers: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingFeeUpdate {
    pub new_fee_bps: i128,
    pub executable_after_ledger: u32,
    pub based_on_volatility_bps: i128,
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
    GuardAdmins,
    GuardThreshold,
    GuardPauseState,
    Balance(Address),
    Allowance(AllowanceDataKey),
    OracleConfig,
    LastOraclePrice,
    LastVolatilityBps,
    PendingFeeUpdate,
    Paused,
    StakedBalance(Address),
    TotalStaked,
    UserRewards(Address),
    LastRewardLedger,
    AccumulatedRewardPerShare,
}

fn sqrt(x: i128) -> i128 {
    if x == 0 {
        return 0;
    Guard,
    Oracle,
    PendingFeeUpdate,
/// Grouped pool state — stored as a single instance key to reduce storage footprint.
#[contracttype]
#[derive(Clone)]
pub struct PoolState {
    pub admin: Address,
    pub token_a: Address,
    pub token_b: Address,
    pub reserve_a: i128,
    pub reserve_b: i128,
    pub total_shares: i128,
    pub fee_bps: i128,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Single-entry key for all grouped pool state.
    Pool,
    /// Per-user LP share balance (persistent storage).
    Balance(Address),
    /// ERC-20-style allowances (persistent storage).
    Allowance(AllowanceDataKey),
}

fn load_pool(e: &Env) -> Result<PoolState, Error> {
    e.storage()
        .instance()
        .get::<_, PoolState>(&DataKey::Pool)
        .ok_or(Error::NotInitialized)
}

fn save_pool(e: &Env, pool: &PoolState) {
    e.storage().instance().set(&DataKey::Pool, pool);
}
// ── Constants ─────────────────────────────────────────────────────────────────

pub const MAX_FEE_BPS: i128 = 100;
pub const DEFAULT_BASE_FEE_BPS: i128 = 30;
pub const DEFAULT_FEE_TIMELOCK_LEDGERS: u32 = 120;

pub const LOW_VOLATILITY_THRESHOLD_BPS: i128 = 100;
pub const MEDIUM_VOLATILITY_THRESHOLD_BPS: i128 = 250;
pub const HIGH_VOLATILITY_THRESHOLD_BPS: i128 = 500;

pub const LOW_VOLATILITY_FEE_BPS: i128 = 40;
pub const MEDIUM_VOLATILITY_FEE_BPS: i128 = 70;
pub const HIGH_VOLATILITY_FEE_BPS: i128 = 100;

// ── Oracle trait ──────────────────────────────────────────────────────────────

#[soroban_sdk::contractclient(name = "PriceOracleClient")]
pub trait PriceOracle {
    fn latest_price(e: Env) -> i128;
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

    Admin,
    GuardAdmins,
    GuardThreshold,
    GuardPauseState,
    Balance(Address),
    Allowance(AllowanceDataKey),
    OracleConfig,
    LastOraclePrice,
    LastVolatilityBps,
    PendingFeeUpdate,
}
// ── Helpers ───────────────────────────────────────────────────────────────────
// ── Private helpers ───────────────────────────────────────────────────────────

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

fn load_guard(e: &Env) -> GuardState {
    e.storage()
        .instance()
        .get(&DataKey::Guard)
        .unwrap_or_else(|| GuardState {
            pause_state: 0,
            admins: Vec::new(e),
            signature_threshold: 0,
        })
}

fn save_guard(e: &Env, guard: &GuardState) {
    e.storage().instance().set(&DataKey::Guard, guard);
}

fn load_or_init_guard_from_pool(e: &Env, pool: &PoolState) -> GuardState {
    let guard = load_guard(e);
    if guard.signature_threshold == 0 && guard.admins.len() == 0 {
        GuardState {
            pause_state: guard.pause_state,
            admins: {
                let mut admins = Vec::new(e);
                admins.push_back(pool.admin.clone());
                admins
            },
            signature_threshold: 1,
        }
    } else {
        guard
    }
}

fn check_paused(pool: &PoolState, operation: u32, guard: &GuardState) -> Result<(), Error> {
    if guard.pause_state & operation != 0 {
fn check_not_paused(e: &Env, operation: u32) -> Result<(), Error> {
fn load_admin(e: &Env) -> Result<Address, Error> {
    e.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)
}

fn map_guard_err(err: GuardError) -> Error {
    match err {
        GuardError::Paused => Error::Paused,
        GuardError::NotInitialized => Error::NotInitialized,
        GuardError::Unauthorized
        | GuardError::InsufficientSignatures
        | GuardError::AdminNotFound
        | GuardError::InvalidThreshold => Error::Unauthorized,
        GuardError::AlreadyInitialized => Error::AlreadyInitialized,
    }
}

fn require_not_paused(e: &Env, operation: u32) -> Result<(), Error> {
    if EmergencyGuard::is_paused(e.clone(), operation) {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

/// Rewards per ledger: 1 LP token (with 7 decimals) = 10_000_000
pub const REWARDS_PER_LEDGER: i128 = 10_000_000;
/// Precision factor for reward calculations
pub const REWARD_PRECISION: i128 = 1_000_000_000_000;

pub trait PriceOracle {
    fn latest_price(e: Env) -> i128;
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

    if dynamic_fee > MAX_FEE_BPS {

    if dynamic_fee > MAX_FEE_BPS {
    if dynamic > MAX_FEE_BPS {
        MAX_FEE_BPS
    } else {
        dynamic
    }
}

fn guard_init(e: &Env, admin: Address) {
    let admins = vec![e, admin];
    e.storage().instance().set(&DataKey::GuardAdmins, &admins);
    e.storage().instance().set(&DataKey::GuardThreshold, &1u32);
}

fn guard_pause_state(e: &Env) -> u32 {
    e.storage()
        .instance()
        .get(&DataKey::GuardPauseState)
        .unwrap_or(0u32)
}
// ── pause_op aliases (kept for backwards compat with tests) ──────────────────

fn read_admin(e: &Env) -> Result<Address, Error> {
    e.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)
}

fn read_guard_admins(e: &Env) -> Vec<Address> {
    e.storage()
        .instance()
        .get(&DataKey::GuardAdmins)
        .unwrap_or_else(|| Vec::new(e))
}

fn write_guard_admins(e: &Env, admins: &Vec<Address>) {
    e.storage().instance().set(&DataKey::GuardAdmins, admins);
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
        .get(&DataKey::GuardAdmins)
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
        .get(&DataKey::GuardThreshold)
        .ok_or(Error::NotInitialized)?;
    let admins = guard_admins(e);

    let mut valid = 0u32;
fn primary_admin(e: &Env) -> Result<Address, GuardError> {
    let guard = load_guard(e);
    if let Some(admin) = guard.admins.get(0) {
        Ok(admin)
    } else {
        let pool = load_pool(e).map_err(|_| GuardError::NotInitialized)?;
        Ok(pool.admin)
    }
}

fn check_multi_sig(e: &Env, approvers: &Vec<Address>) -> Result<(), GuardError> {
    let guard = load_guard(e);
    if guard.signature_threshold == 0 {
        return Err(GuardError::NotInitialized);
    }

    if approvers.len() < guard.signature_threshold {
        return Err(GuardError::InsufficientSignatures);
    }

    let mut valid_approvers: u32 = 0;
    let mut seen = Vec::new(e);

    for addr in approvers.iter() {
        if seen.iter().any(|a| a == addr) {
            continue;
        }
        seen.push_back(addr.clone());
        if admins.iter().any(|a| a == addr) {
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
    let state = guard_pause_state(e);
    let next = if paused { state | ops } else { state & !ops };
    if next != state {
        e.storage().instance().set(&DataKey::GuardPauseState, &next);
    }
}
const CORE_PAUSE_OPS: [u32; 4] = [
    PauseType::SWAP,
    PauseType::DEPOSIT,
    PauseType::WITHDRAW,
    PauseType::BURN,
];

fn primary_admin(e: &Env) -> Result<Address, GuardError> {
    let guard = load_guard(e);
    if let Some(admin) = guard.admins.get(0) {
        Ok(admin)
    } else {
        let pool = load_pool(e).map_err(|_| GuardError::NotInitialized)?;
        Ok(pool.admin)
    }
}

fn check_multi_sig(e: &Env, approvers: &Vec<Address>) -> Result<(), GuardError> {
    let guard = load_guard(e);
    if guard.signature_threshold == 0 {
        return Err(GuardError::NotInitialized);
    }

    if approvers.len() < guard.signature_threshold {
        return Err(GuardError::InsufficientSignatures);
    }

    let mut valid_approvers: u32 = 0;
    let mut seen = Vec::new(e);

    for addr in approvers.iter() {
        if seen.iter().any(|a| a == addr) {
            continue;
        }
        seen.push_back(addr.clone());

        if guard.admins.iter().any(|a| a == addr) {
            addr.require_auth();
            valid_approvers += 1;
        }
    }

    if valid_approvers < guard.signature_threshold {
        Err(GuardError::InsufficientSignatures)
    } else {
        Ok(())
    }
fn set_primary_admin(e: &Env, admin: Address) -> Result<(), Error> {
    e.storage().instance().set(&DataKey::Admin, &admin);
    let mut pool = load_pool(e)?;
    pool.admin = admin;
    save_pool(e, &pool);
    Ok(())

        if guard.admins.iter().any(|a| a == addr) {
            addr.require_auth();
            valid_approvers += 1;
        }
    }

    if valid_approvers < guard.signature_threshold {
        Err(GuardError::InsufficientSignatures)
    } else {
        Ok(())
    }
}

#[contract]
pub struct LiquidityPool;

#[contractimpl]
impl LiquidityPool {
    // ── Initialisation ────────────────────────────────────────────────────────

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
                paused: false,
            },
        );

        save_guard(
            &e,
            &GuardState {
                pause_state: 0,
                admins: {
                    let mut admins = Vec::new(&e);
                    admins.push_back(admin);
                    admins
                },
                signature_threshold: 1,
            },
        );

        Ok(())
    }

    pub fn get_fee(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, PoolState>(&DataKey::Pool)
            .map(|p| p.fee_bps)
            .unwrap_or(DEFAULT_BASE_FEE_BPS)
    }

            },
        );

        EmergencyGuard::initialize(e.clone(), vec![&e, admin], 1).map_err(map_guard_err)?;
        Ok(())
    }

    // ── Admin accessors ─────────────────────────────────────────────────────────

    pub fn get_admin(e: Env) -> Address {
        load_admin(&e).expect("not initialized")
    }

    pub fn get_admins(e: Env) -> Vec<Address> {
        EmergencyGuard::get_admins(e)
    }

    pub fn get_admin_threshold(e: Env) -> u32 {
        EmergencyGuard::get_threshold(e)
    }

    // ── EmergencyGuard pause interface ──────────────────────────────────────────

    pub fn guard_pause(
        e: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), Error> {
    /// Pause or resume one operation bit. Any current guard admin may call this.
    pub fn guard_pause(e: Env, admin: Address, operation: u32, paused: bool) -> Result<(), Error> {
        EmergencyGuard::set_pause(e, admin, operation, paused).map_err(map_guard_err)
    }

    pub fn guard_is_paused(e: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(e, operation)
    pub fn guard_unpause(e: Env, admin: Address, operation: u32) -> Result<(), Error> {
        Self::guard_pause(e, admin, operation, false)
    }

    pub fn set_operation_paused(
        e: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), Error> {
        Self::guard_pause(e, admin, operation, paused)
    }

    /// Pause or unpause all core pool operations (swap, deposit, withdraw, burn).
    pub fn set_paused(e: Env, paused: bool) -> Result<(), Error> {
        let admin = load_admin(&e)?;
        for op in CORE_PAUSE_OPS {
            EmergencyGuard::set_pause(e.clone(), admin.clone(), op, paused).map_err(map_guard_err)?;
        for op in [
            PauseType::SWAP,
            PauseType::DEPOSIT,
            PauseType::WITHDRAW,
            PauseType::BURN,
        ] {
            EmergencyGuard::set_pause(e.clone(), admin.clone(), op, paused)
                .map_err(map_guard_err)?;
        }
        Ok(())
    }

    pub fn pause_swaps(e: Env) -> Result<(), Error> {
        let admin = load_admin(&e)?;
        EmergencyGuard::set_pause(e, admin, PauseType::SWAP, true).map_err(map_guard_err)
    }

    pub fn resume_swaps(e: Env) -> Result<(), Error> {
        let admin = load_admin(&e)?;
        EmergencyGuard::set_pause(e, admin, PauseType::SWAP, false).map_err(map_guard_err)
        let admin = Self::get_admin(e.clone())?;
        Self::guard_unpause(e, admin, pause_op::SWAP)
    }

    pub fn pause_deposits(e: Env) -> Result<(), Error> {
        let admin = load_admin(&e)?;
        EmergencyGuard::set_pause(e, admin, PauseType::DEPOSIT, true).map_err(map_guard_err)
    }

    pub fn resume_deposits(e: Env) -> Result<(), Error> {
        let admin = load_admin(&e)?;
        EmergencyGuard::set_pause(e, admin, PauseType::DEPOSIT, false).map_err(map_guard_err)
        let admin = Self::get_admin(e.clone())?;
        Self::guard_unpause(e, admin, pause_op::DEPOSIT)
    }

    pub fn pause_withdrawals(e: Env) -> Result<(), Error> {
        let admin = load_admin(&e)?;
        EmergencyGuard::set_pause(e, admin, PauseType::WITHDRAW, true).map_err(map_guard_err)
    }

    pub fn resume_withdrawals(e: Env) -> Result<(), Error> {
        let admin = load_admin(&e)?;
        EmergencyGuard::set_pause(e, admin, PauseType::WITHDRAW, false).map_err(map_guard_err)
    }

    pub fn emergency_pause(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::emergency_pause(e, approvers).map_err(map_guard_err)
    }

    pub fn resume(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::resume(e, approvers).map_err(map_guard_err)
        let admin = Self::get_admin(e.clone())?;
        Self::guard_unpause(e, admin, pause_op::WITHDRAW)
    }

    pub fn emergency_pause_all(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        Self::emergency_pause(e, approvers)
    }

    pub fn resume_all(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        Self::resume(e, approvers)
    }

    pub fn get_pause_state(e: Env) -> u32 {
        EmergencyGuard::get_pause_state(e)
    }
    pub fn get_pause_mask(e: Env) -> u32 {
        EmergencyGuard::get_pause_state(e)
    }

    pub fn get_pause_mask(e: Env) -> u32 {
        EmergencyGuard::get_pause_state(e)
    }

    /// Unpause all via multi-sig approvers (backward-compatible resume entry point).
    pub fn guard_unpause(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::resume(e, approvers).map_err(map_guard_err)
    }

    pub fn is_paused_op(e: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(e, operation)
    }

    pub fn add_admin(e: Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), Error> {
        EmergencyGuard::add_admin(e, approvers, new_admin).map_err(map_guard_err)
    }

    pub fn remove_admin(e: Env, approvers: Vec<Address>, admin: Address) -> Result<(), Error> {
        let pool_admin = load_admin(&e)?;
        EmergencyGuard::remove_admin(e.clone(), approvers, admin.clone()).map_err(map_guard_err)?;
        let admins = EmergencyGuard::get_admins(e.clone());
        if pool_admin == admin && admins.len() == 1 {
            if let Some(remaining) = admins.get(0) {
                e.storage().instance().set(&DataKey::Admin, &remaining);
            }
        }
        Ok(())
    }

    pub fn rotate_admin(
        e: Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), Error> {
        let pool_admin = load_admin(&e)?;
        if pool_admin != old_admin {
            return Err(Error::Unauthorized);
        }
        EmergencyGuard::add_admin(e.clone(), approvers.clone(), new_admin.clone())
            .map_err(map_guard_err)?;
        EmergencyGuard::remove_admin(e.clone(), approvers, old_admin).map_err(map_guard_err)?;
        e.storage().instance().set(&DataKey::Admin, &new_admin);
        Ok(())
    pub fn add_guard_admin(
        e: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), Error> {
        EmergencyGuard::add_admin(e, approvers, new_admin).map_err(map_guard_err)
    }

    pub fn remove_guard_admin(
        e: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), Error> {
        Self::remove_admin(e, approvers, admin)
    }

    pub fn get_guard_admins(e: Env) -> Vec<Address> {
        EmergencyGuard::get_admins(e)
    }

    pub fn get_guard_threshold(e: Env) -> u32 {
        EmergencyGuard::get_threshold(e)
    }

    // ── Fee management ────────────────────────────────────────────────────────

    pub fn get_fee(e: Env) -> i128 {
        load_pool(&e)
            .map(|p| p.fee_bps)
            .unwrap_or(DEFAULT_BASE_FEE_BPS)
    }

    pub fn set_fee(e: Env, fee_bps: i128) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&fee_bps) {
            return Err(Error::InvalidFee);
        }

        let admin = load_admin(&e)?;
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

    pub fn configure_fee_oracle(
        e: Env,
        oracle: Address,
        base_fee_bps: i128,
        timelock_ledgers: u32,
    ) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&base_fee_bps) {
            return Err(Error::InvalidFee);
        }

        let mut pool = load_pool(&e)?;
        pool.admin.require_auth();
        pool.base_fee_bps = base_fee_bps;
        save_pool(&e, &pool);

        e.storage().instance().set(
            &DataKey::Oracle,
            &OracleConfig {
                oracle,
                last_price: 0,
                last_volatility_bps: 0,
                timelock_ledgers,
            },
        );

        Ok(())
    }

    pub fn get_last_volatility_bps(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, OracleConfig>(&DataKey::Oracle)
            .map(|cfg| cfg.last_volatility_bps)
            .unwrap_or(0)
    }

    pub fn get_pending_fee_update(e: Env) -> Option<PendingFeeUpdate> {
        e.storage().instance().get(&DataKey::PendingFeeUpdate)
    }

    pub fn sync_fee_from_oracle(e: Env) -> Result<Option<PendingFeeUpdate>, Error> {
        let mut cfg: OracleConfig = e
            .storage()
            .instance()
            .get(&DataKey::Oracle)
            .ok_or(Error::OracleNotConfigured)?;

        let oracle_client = PriceOracleClient::new(&e, &cfg.oracle);
        let current_price = oracle_client.latest_price();
        if current_price <= 0 {
            return Err(Error::InvalidOraclePrice);
        }

        let previous_price = cfg.last_price;
        cfg.last_price = current_price;

        if previous_price <= 0 {
            cfg.last_volatility_bps = 0;
            e.storage().instance().set(&DataKey::Oracle, &cfg);
            return Ok(None);
        }

        let price_delta = if current_price >= previous_price {
            current_price - previous_price
        } else {
            previous_price - current_price
        };

        let volatility_bps = price_delta
            .checked_mul(10_000)
            .ok_or(Error::InvalidOraclePrice)?
            / previous_price;

        cfg.last_volatility_bps = volatility_bps;
        e.storage().instance().set(&DataKey::Oracle, &cfg);

        let pool = load_pool(&e)?;
        let target_fee = target_fee_from_volatility(pool.base_fee_bps, volatility_bps);
        if target_fee == pool.fee_bps {
            return Ok(None);
        }

        let execute_after = e.ledger().sequence().saturating_add(cfg.timelock_ledgers);
        let pending = PendingFeeUpdate {
            new_fee_bps: target_fee,
            executable_after_ledger: execute_after,
            based_on_volatility_bps: volatility_bps,
        };

        e.storage()
            .instance()
            .set(&DataKey::PendingFeeUpdate, &pending);

        e.storage()
            .instance()
            .set(&DataKey::PendingFeeUpdate, &pending);
        let scheduled_by = e.current_contract_address();
        e.events().publish(
            (
                String::from_str(&e, "fee_update_scheduled"),
                scheduled_by.clone(),
            ),
            FeeUpdateScheduledEvent {
                scheduled_by,
                old_fee_bps: pool.fee_bps,
                new_fee_bps: target_fee,
                executable_after_ledger: execute_after,
                volatility_bps,
            },
        );

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

        if !(0..=MAX_FEE_BPS).contains(&pending.new_fee_bps) {
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
    pub fn guard_unpause(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        guard_require_multisig(&e, &approvers)?;
        guard_set_ops(&e, pause_op::ALL, false);
        Ok(())
    }

    /// Backward-compatible resume entry point for existing callers.
    pub fn resume(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        Self::guard_unpause(e, approvers)
    }

    /// Alias for callers that use the explicit "all" naming.
    pub fn resume_all(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        Self::guard_unpause(e, approvers)
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
            .get(&DataKey::GuardThreshold)
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

    pub fn remove_admin(e: Env, approvers: Vec<Address>, admin: Address) -> Result<(), Error> {
        Self::remove_guard_admin(e, approvers, admin)
    }

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

        let admins = guard_admins(&e);
        let mut new_admins = Vec::new(&e);
        for a in admins.iter() {
            if a == old_admin {
                if !new_admins.iter().any(|existing| existing == new_admin) {
                    new_admins.push_back(new_admin.clone());
                }
            } else if !new_admins.iter().any(|existing| existing == a) {
                new_admins.push_back(a);
            }
        }

        e.storage()
            .instance()
            .set(&DataKey::GuardAdmins, &new_admins);

        if Self::get_admin(e.clone())? == old_admin {
            set_primary_admin(&e, new_admin)?;
        }
        Ok(())
    }

    pub fn get_fee(e: Env) -> i128 {
        load_pool(&e)
            .map(|p| p.fee_bps)
            .unwrap_or(DEFAULT_BASE_FEE_BPS)
    }


        save_guard(
            &e,
            &GuardState {
                pause_state: 0,
                admins: {
                    let mut admins = Vec::new(&e);
                    admins.push_back(admin);
                    admins
                },
                signature_threshold: 1,
            },
        );

        Ok(())
    }

    pub fn get_fee(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, PoolState>(&DataKey::Pool)
            .map(|p| p.fee_bps)
            .unwrap_or(DEFAULT_BASE_FEE_BPS)
    }

    pub fn set_fee(e: Env, fee_bps: i128) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&fee_bps) {
            return Err(Error::InvalidFee);
        }

        admin.require_auth();
        let pool = PoolState {
            admin: admin.clone(),
            token_a,
            token_b,
            reserve_a: 0,
            reserve_b: 0,
            total_shares: 0,
            fee_bps: 30,
        };
        save_pool(&e, &pool);
        // Initialize the EmergencyGuard with the admin as the sole approver.
        EmergencyGuard::initialize(e.clone(), vec![&e, admin], 1)
            .map_err(|_| Error::Unauthorized)?;
        Ok(())
    }

    /// Returns the current fee in basis points.
    pub fn get_fee(e: Env) -> Result<i128, Error> {
        let pool = load_pool(&e)?;
        Ok(pool.fee_bps)
    }

    /// Admin-only: update the swap fee. Valid range: 0–100 bps.
    pub fn set_fee(e: Env, fee_bps: i128) -> Result<(), Error> {
        if !(0..=100).contains(&fee_bps) {
            return Err(Error::InvalidFee);
        }
        let admin = read_admin(&e)?;
        admin.require_auth();
        let mut pool = load_pool(&e)?;
        let old_fee = pool.fee_bps;
        pool.fee_bps = fee_bps;
        save_pool(&e, &pool);

        save_pool(&e, &pool);
        e.events().publish(
            (String::from_str(&e, "fee_changed"), pool.admin.clone()),
            FeeChangedEvent {
                admin: pool.admin,
                old_fee_bps: old_fee,
                new_fee_bps: fee_bps,
            },
        );

        Ok(())
    }

    pub fn configure_fee_oracle(
        e: Env,
        oracle_id: Address,
        oracle: Address,
        base_fee_bps: i128,
        timelock_ledgers: u32,
    ) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&base_fee_bps) {
            return Err(Error::InvalidFee);
        }

        let mut pool = load_pool(&e)?;
        pool.admin.require_auth();
        pool.base_fee_bps = base_fee_bps;
        save_pool(&e, &pool);

        let admin = load_admin(&e)?;
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

    pub fn get_last_volatility_bps(e: Env) -> i128 {
        e.storage()
            .instance()
            .get(&DataKey::LastVolatilityBps)
            .unwrap_or(0)
    }

    pub fn get_pending_fee_update(e: Env) -> Option<PendingFeeUpdate> {
        e.storage().instance().get(&DataKey::PendingFeeUpdate)
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
        let maybe_last: Option<i128> = e.storage().instance().get(&DataKey::LastOraclePrice);
        let Some(last_price) = maybe_last else {
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
        if !(0..=MAX_FEE_BPS).contains(&pending.new_fee_bps) {
            return Err(Error::InvalidFee);
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

    /// Admin-only: emergency pause all operations (requires multi-sig).
    pub fn emergency_pause(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::emergency_pause(e, approvers).map_err(|_| Error::Unauthorized)
    }

    /// Admin-only: resume all paused operations (requires multi-sig).
    pub fn resume_all(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::resume(e, approvers).map_err(|_| Error::Unauthorized)
    }

    /// Check whether a specific operation is currently paused.
    pub fn is_paused(e: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(e, operation)
    }

    /// Deposits token A and token B into the pool and mints LP shares.
    pub fn deposit(e: Env, to: Address, amount_a: i128, amount_b: i128) -> Result<i128, Error> {
        guard_check_not_paused(&e, pause_op::DEPOSIT)?;
            &DataKey::Oracle,
            &OracleConfig {
                oracle,
                last_price: 0,
                last_volatility_bps: 0,
                timelock_ledgers,
            },
        );

        Ok(())
    }

    pub fn get_last_volatility_bps(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, OracleConfig>(&DataKey::Oracle)
            .map(|cfg| cfg.last_volatility_bps)
            .unwrap_or(0)
    }

    pub fn get_pending_fee_update(e: Env) -> Option<PendingFeeUpdate> {
        e.storage().instance().get(&DataKey::PendingFeeUpdate)
    }

    pub fn sync_fee_from_oracle(e: Env) -> Result<Option<PendingFeeUpdate>, Error> {
        let mut cfg: OracleConfig = e
            .storage()
            .instance()
            .get(&DataKey::Oracle)
            .ok_or(Error::OracleNotConfigured)?;

        let oracle_client = PriceOracleClient::new(&e, &cfg.oracle);
        let current_price = oracle_client.latest_price();
        if current_price <= 0 {
            return Err(Error::InvalidOraclePrice);
        }

        let previous_price = cfg.last_price;
        cfg.last_price = current_price;

        if previous_price <= 0 {
            cfg.last_volatility_bps = 0;
            e.storage().instance().set(&DataKey::Oracle, &cfg);
            return Ok(None);
        }

        let price_delta = if current_price >= previous_price {
            current_price - previous_price
        } else {
            previous_price - current_price
        };

        let volatility_bps = price_delta
            .checked_mul(10_000)
            .ok_or(Error::InvalidOraclePrice)?
            / previous_price;

        cfg.last_volatility_bps = volatility_bps;
        e.storage().instance().set(&DataKey::Oracle, &cfg);

        let pool = load_pool(&e)?;
        let target_fee = target_fee_from_volatility(pool.base_fee_bps, volatility_bps);
        if target_fee == pool.fee_bps {
            return Ok(None);
        }

        let execute_after = e.ledger().sequence().saturating_add(cfg.timelock_ledgers);
        let pending = PendingFeeUpdate {
            new_fee_bps: target_fee,
            executable_after_ledger: execute_after,
            based_on_volatility_bps: volatility_bps,
        };

        e.storage()
            .instance()
            .set(&DataKey::PendingFeeUpdate, &pending);

        let scheduled_by = e.current_contract_address();
        e.events().publish(
            (
                String::from_str(&e, "fee_update_scheduled"),
                scheduled_by.clone(),
            ),
            FeeUpdateScheduledEvent {
                scheduled_by,
                old_fee_bps: pool.fee_bps,
                new_fee_bps: target_fee,
                executable_after_ledger: execute_after,
                volatility_bps,
            },
        );

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

        if !(0..=MAX_FEE_BPS).contains(&pending.new_fee_bps) {
            return Err(Error::InvalidFee);
        }

        let mut pool = load_pool(&e)?;
        let old_fee = pool.fee_bps;
        pool.fee_bps = pending.new_fee_bps;
        save_pool(&e, &pool);
        e.storage().instance().remove(&DataKey::PendingFeeUpdate);
        pool.admin.require_auth();
        let old_fee = pool.fee_bps;
        pool.fee_bps = fee_bps;
        pool.base_fee_bps = fee_bps;
        save_pool(&e, &pool);

        e.events().publish(
            (String::from_str(&e, "fee_changed"), admin.clone()),
            FeeChangedEvent {
                admin,
                old_fee_bps: old_fee,
                new_fee_bps: pending.new_fee_bps,
            },
        );

        Ok(pending.new_fee_bps)
    }

    pub fn init_guard(e: Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        if threshold == 0 || threshold > admins.len() as u32 {
            return Err(GuardError::InvalidThreshold);
        }

        let first_admin = admins.get(0).ok_or(GuardError::NotInitialized)?;
        save_guard(
            &e,
            &GuardState {
                pause_state: load_guard(&e).pause_state,
                admins: admins.clone(),
                signature_threshold: threshold,
            },
        );

        if let Ok(mut pool) = load_pool(&e) {
            pool.admin = first_admin;
            save_pool(&e, &pool);
        }

        emit_guard_initialized(&e, &admins, threshold);

        Ok(())
    }

    pub fn get_pause_state(e: Env) -> u32 {
        load_guard(&e).pause_state
    }

    pub fn set_pause_state(e: Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        let admin = primary_admin(&e)?;
        admin.require_auth();

        let mut guard = load_guard(&e);
        if !guard.admins.iter().any(|a| a == admin) {
            return Err(GuardError::Unauthorized);
        }

        if paused {
            guard.pause_state |= operation;
        } else {
            guard.pause_state &= !operation;
        }

        save_guard(&e, &guard);
        emit_pause_state_changed(&e, &admin, operation, paused);
        Ok(())
    }

    pub fn emergency_pause_all(e: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        check_multi_sig(&e, &approvers)?;
        let mut guard = load_guard(&e);
        guard.pause_state = u32::MAX;
        save_guard(&e, &guard);
        emit_emergency_paused_all(&e, &approvers);
        Ok(())
    }

    pub fn resume_all(e: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        check_multi_sig(&e, &approvers)?;
        let mut guard = load_guard(&e);
        guard.pause_state = 0;
        save_guard(&e, &guard);
        emit_resumed_all(&e, &approvers);
        Ok(())
    }

    pub fn get_admins(e: Env) -> Vec<Address> {
        load_guard(&e).admins
    }

    pub fn get_threshold(e: Env) -> u32 {
        load_guard(&e).signature_threshold
    }

    pub fn is_admin(e: Env, addr: Address) -> bool {
        load_guard(&e).admins.iter().any(|a| a == addr)
    }

    pub fn add_admin(
        e: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        check_multi_sig(&e, &approvers)?;

        let mut guard = load_guard(&e);
        if !guard.admins.iter().any(|a| a == new_admin) {
            guard.admins.push_back(new_admin.clone());
            save_guard(&e, &guard);
            emit_admin_added(&e, &approvers, &new_admin);
        }

        Ok(())
    }

    pub fn remove_admin(e: Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        check_multi_sig(&e, &approvers)?;

        let guard = load_guard(&e);
        if guard.admins.len() as u32 <= guard.signature_threshold {
            return Err(GuardError::InvalidThreshold);
        }

        let mut new_admins = Vec::new(&e);
        let mut found = false;
        for a in guard.admins.iter() {
            if a != admin {
                new_admins.push_back(a);
            } else {
                found = true;
            }
        }

        if !found {
            return Err(GuardError::AdminNotFound);
        }

        let mut updated_guard = guard;
        updated_guard.admins = new_admins;
        save_guard(&e, &updated_guard);
        emit_admin_removed(&e, &approvers, &admin);

        if let Ok(mut pool) = load_pool(&e) {
            if let Some(first_admin) = updated_guard.admins.get(0) {
                pool.admin = first_admin;
                save_pool(&e, &pool);
            }
        }

        Ok(())
    }

    pub fn set_paused(e: Env, paused: bool) -> Result<(), Error> {
        let mut pool = load_pool(&e)?;
        pool.admin.require_auth();

        let mut guard = load_or_init_guard_from_pool(&e, &pool);
        guard.pause_state = if paused { u32::MAX } else { 0 };
        save_guard(&e, &guard);
        emit_pause_state_changed(&e, &pool.admin, u32::MAX, paused);

        Ok(())
    }

    pub fn emergency_pause(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        Self::emergency_pause_all(e, approvers).map_err(|_| Error::Unauthorized)
    }

    pub fn deposit(e: Env, to: Address, amount_a: i128, amount_b: i128) -> Result<i128, Error> {
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::DEPOSIT, &guard)?;
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
    // ── Core AMM operations ───────────────────────────────────────────────────

    /// Helper function: updates the accumulated reward per share based on elapsed ledgers.
    fn update_reward_state(e: &Env) {
        let last_reward_ledger: u32 = e
            .storage()
            .instance()
            .get(&DataKey::LastRewardLedger)
            .unwrap_or(e.ledger().sequence());
        let current_ledger = e.ledger().sequence();

        if current_ledger <= last_reward_ledger {
            return;
        }

        let total_staked: i128 = e
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);

        if total_staked > 0 {
            let ledgers_elapsed = (current_ledger - last_reward_ledger) as i128;
            let new_rewards = ledgers_elapsed
                .checked_mul(REWARDS_PER_LEDGER)
                .unwrap_or(0);
            
            let reward_per_share_increment = new_rewards
                .checked_mul(REWARD_PRECISION)
                .and_then(|v| {
                    if total_staked > 0 {
                        Some(v / total_staked)
                    } else {
                        Some(0)
                    }
                })
                .unwrap_or(0);

            let accumulated: i128 = e
                .storage()
                .instance()
                .get(&DataKey::AccumulatedRewardPerShare)
                .unwrap_or(0);

            e.storage()
                .instance()
                .set(&DataKey::AccumulatedRewardPerShare, &(accumulated + reward_per_share_increment));
        }

            e.storage().instance().set(
                &DataKey::AccumulatedRewardPerShare,
                &(accumulated + increment),
            );
        }
        e.storage()
            .instance()
            .set(&DataKey::LastRewardLedger, &current_ledger);
    }

    /// Helper function: calculate pending rewards for a user.
    fn calculate_pending_rewards(e: &Env, user: &Address, staked_amount: i128) -> i128 {
        let accumulated_per_share: i128 = e
            .storage()
            .instance()
            .get(&DataKey::AccumulatedRewardPerShare)
            .unwrap_or(0);

        let earned = staked_amount
            .checked_mul(accumulated_per_share)
            .and_then(|v| Some(v / REWARD_PRECISION))
            .unwrap_or(0);

        let claimed: i128 = e
            .storage()
            .persistent()
            .get::<_, i128>(&DataKey::UserRewards(user.clone()))
            .unwrap_or(0);

        earned.saturating_sub(claimed)
    }

    /// Stake LP shares to earn staking rewards.
    ///
    /// # Parameters
    /// - `e`: Soroban environment.
    /// - `user`: Address staking LP shares.
    /// - `amount`: Number of LP shares to stake.
    ///
    /// # Returns
    /// - `Ok(())`: Success.
    /// - `Err(Error::Paused)`: Contract is paused.
    /// - `Err(Error::InsufficientBalance)`: User lacks enough LP shares.
    pub fn stake(e: Env, user: Address, amount: i128) -> Result<(), Error> {
        check_paused(&e)?;
        user.require_auth();

        // Check user has sufficient balance
        let balance_key = DataKey::Balance(user.clone());
        let user_balance: i128 = e.storage().persistent().get(&balance_key).unwrap_or(0);
        if user_balance < amount {
            return Err(Error::InsufficientBalance);
        }

        // Update reward state
        Self::update_reward_state(&e);

        // Transfer LP shares from user's balance to staked balance
        let staked_key = DataKey::StakedBalance(user.clone());
        let current_staked: i128 = e.storage().persistent().get(&staked_key).unwrap_or(0);

        e.storage()
            .persistent()
            .set(&balance_key, &(user_balance - amount));
        e.storage().persistent().extend_ttl(&balance_key, 100, 100);

        e.storage()
            .persistent()
            .set(&staked_key, &(current_staked + amount));
        e.storage().persistent().extend_ttl(&staked_key, 100, 100);

        // Update total staked
        let total_staked: i128 = e
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        e.storage()
            .instance()
            .set(&DataKey::TotalStaked, &(total_staked + amount));

        // Emit stake event
        e.events().publish(
            (String::from_str(&e, "stake"), user.clone()),
            StakeEvent {
                user,
                amount_staked: amount,
            },
        );

        Ok(())
    }

    /// Unstake LP shares.
    ///
    /// # Parameters
    /// - `e`: Soroban environment.
    /// - `user`: Address unstaking LP shares.
    /// - `amount`: Number of LP shares to unstake.
    ///
    /// # Returns
    /// - `Ok(())`: Success.
    /// - `Err(Error::Paused)`: Contract is paused.
    /// - `Err(Error::InsufficientShares)`: User lacks enough staked LP shares.
    pub fn unstake(e: Env, user: Address, amount: i128) -> Result<(), Error> {
        check_paused(&e)?;
        user.require_auth();

        let staked_key = DataKey::StakedBalance(user.clone());
        let current_staked: i128 = e.storage().persistent().get(&staked_key).unwrap_or(0);
        if current_staked < amount {
            return Err(Error::InsufficientShares);
        }

        // Update reward state
        Self::update_reward_state(&e);

        // Transfer LP shares from staked balance back to user's balance
        let balance_key = DataKey::Balance(user.clone());
        let user_balance: i128 = e.storage().persistent().get(&balance_key).unwrap_or(0);

        e.storage()
            .persistent()
            .set(&balance_key, &(user_balance + amount));
        e.storage().persistent().extend_ttl(&balance_key, 100, 100);

        e.storage()
            .persistent()
            .set(&staked_key, &(current_staked - amount));
        e.storage().persistent().extend_ttl(&staked_key, 100, 100);

        // Update total staked
        let total_staked: i128 = e
            .storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0);
        e.storage()
            .instance()
            .set(&DataKey::TotalStaked, &(total_staked - amount));

        // Emit unstake event
        e.events().publish(
            (String::from_str(&e, "unstake"), user.clone()),
            UnstakeEvent {
                user,
                amount_unstaked: amount,
            },
        );

        Ok(())
    }

    /// Claim accumulated staking rewards.
    ///
    /// # Parameters
    /// - `e`: Soroban environment.
    /// - `user`: Address claiming rewards.
    ///
    /// # Returns
    /// - `Ok(i128)`: Amount of rewards claimed.
    /// - `Err(Error::Paused)`: Contract is paused.
    pub fn claim_rewards(e: Env, user: Address) -> Result<i128, Error> {
        check_paused(&e)?;
        user.require_auth();

        // Update reward state
        Self::update_reward_state(&e);

        let staked_key = DataKey::StakedBalance(user.clone());
        let staked_amount: i128 = e.storage().persistent().get(&staked_key).unwrap_or(0);

        // Calculate pending rewards
        let pending = Self::calculate_pending_rewards(&e, &user, staked_amount);

        if pending <= 0 {
            return Ok(0);
        }

        // Update claimed rewards
        let accumulated_per_share: i128 = e
            .storage()
            .instance()
            .get(&DataKey::AccumulatedRewardPerShare)
            .unwrap_or(0);

        let new_claimed = staked_amount
            .checked_mul(accumulated_per_share)
            .and_then(|v| Some(v / REWARD_PRECISION))
            .unwrap_or(0);

        e.storage()
            .persistent()
            .set(&DataKey::UserRewards(user.clone()), &new_claimed);
        e.storage()
            .persistent()
            .extend_ttl(&DataKey::UserRewards(user.clone()), 100, 100);

        // Emit claim rewards event
        e.events().publish(
            (String::from_str(&e, "claim_rewards"), user.clone()),
            ClaimRewardsEvent {
                user,
                rewards_amount: pending,
            },
        );

        Ok(pending)
    }

    /// Get the total amount of LP shares staked by a user.
    pub fn get_staked_balance(e: Env, user: Address) -> i128 {
        let staked_key = DataKey::StakedBalance(user);
        e.storage().persistent().get(&staked_key).unwrap_or(0)
        e.storage()
            .persistent()
            .get(&DataKey::StakedBalance(user))
            .unwrap_or(0)
    }

    /// Get the pending rewards for a user.
    pub fn get_pending_rewards(e: Env, user: Address) -> i128 {
        let staked_key = DataKey::StakedBalance(user.clone());
        let staked_amount: i128 = e.storage().persistent().get(&staked_key).unwrap_or(0);
        Self::calculate_pending_rewards(&e, &user, staked_amount)
    }

    /// Get the total amount of LP shares currently staked in the contract.
    pub fn get_total_staked(e: Env) -> i128 {
        e.storage()
            .instance()
            .get(&DataKey::TotalStaked)
            .unwrap_or(0)
    }

    /// Deposits token A and token B into the pool and mints LP shares.
    ///
    /// The caller (`to`) must authorize the transfer. For first liquidity,
    /// shares are minted as `sqrt(amount_a * amount_b)`. For subsequent
    /// deposits, shares are minted proportionally to existing reserves.
    ///
    /// # Parameters
    /// - `e`: Soroban environment.
    /// - `to`: Liquidity provider address receiving LP shares.
    /// - `amount_a`: Amount of token A to deposit.
    /// - `amount_b`: Amount of token B to deposit.
    ///
    /// # Returns
    /// - `Ok(i128)`: Number of LP shares minted.
    /// - `Err(Error::NotInitialized)`: Pool tokens were not configured.
    /// - `Err(Error::InsufficientLiquidity)`: Arithmetic failed (for example overflow).
    pub fn deposit(e: Env, to: Address, amount_a: i128, amount_b: i128) -> Result<i128, Error> {
        require_not_paused(&e, PauseType::DEPOSIT)?;
        to.require_auth();

        let mut pool = load_pool(&e)?;
        let client_a = soroban_sdk::token::Client::new(&e, &pool.token_a);
        let client_b = soroban_sdk::token::Client::new(&e, &pool.token_b);
        client_a.transfer(&to, &e.current_contract_address(), &amount_a);
        client_b.transfer(&to, &e.current_contract_address(), &amount_b);

        let shares = if pool.total_shares == 0 {
            let product = amount_a
                .checked_mul(amount_b)
                .ok_or(Error::InsufficientLiquidity)?;
            sqrt(product)
            sqrt(
                amount_a
                    .checked_mul(amount_b)
                    .ok_or(Error::InsufficientLiquidity)?,
            )
            let product = amount_a
                .checked_mul(amount_b)
                .ok_or(Error::InsufficientLiquidity)?;
            sqrt(product)
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
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::SWAP, &guard)?;
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::SWAP, &guard)?;
        guard_check_not_paused(&e, pause_op::SWAP)?;
        require_not_paused(&e, PauseType::SWAP)?;
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
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::WITHDRAW, &guard)?;
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::WITHDRAW, &guard)?;
        guard_check_not_paused(&e, pause_op::WITHDRAW)?;
        require_not_paused(&e, PauseType::WITHDRAW)?;
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

        if pool.total_shares <= 0 {
            return Err(Error::InsufficientLiquidity);
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
        soroban_sdk::token::Client::new(&e, &pool.token_a).transfer(
        soroban_sdk::token::Client::new(&e, &pool.token_a).transfer(
        soroban_sdk::token::Client::new(&e, &token_a).transfer(
        soroban_sdk::token::Client::new(&e, &pool.token_a).transfer(
        soroban_sdk::token::Client::new(&e, &token_a).transfer(
            &e.current_contract_address(),
            &to,
            &amount_a,
        );
        soroban_sdk::token::Client::new(&e, &token_b).transfer(
        soroban_sdk::token::Client::new(&e, &pool.token_b).transfer(
        soroban_sdk::token::Client::new(&e, &pool.token_b).transfer(
        soroban_sdk::token::Client::new(&e, &token_b).transfer(
        soroban_sdk::token::Client::new(&e, &pool.token_b).transfer(
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
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::BURN, &guard)?;
        let mut pool = load_pool(&e)?;
        let guard = load_or_init_guard_from_pool(&e, &pool);
        check_paused(&pool, PauseType::BURN, &guard)?;
        guard_check_not_paused(&e, pause_op::BURN)?;
        require_not_paused(&e, PauseType::BURN)?;
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
        require_not_paused(&e, PauseType::TRANSFER)?;
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
        require_not_paused(&e, PauseType::TRANSFER)?;
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
}

impl EmergencyGuardTrait for LiquidityPool {
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError> {
        let guard = load_guard(env);
        if guard.pause_state & operation != 0 {
            Err(GuardError::Paused)
        } else {
            Ok(())
        }
    }

    fn get_pause_state(env: &Env) -> u32 {
        load_guard(env).pause_state
    }

    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        let admin = primary_admin(env)?;
        admin.require_auth();

        let mut guard = load_guard(env);
        if !guard.admins.iter().any(|a| a == admin) {
            return Err(GuardError::Unauthorized);
        }

        if paused {
            guard.pause_state |= operation;
        } else {
            guard.pause_state &= !operation;
        }
        save_guard(env, &guard);
        emit_pause_state_changed(env, &admin, operation, paused);

        Ok(())
    }

    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        check_multi_sig(env, &approvers)?;
        let mut guard = load_guard(env);
        guard.pause_state = u32::MAX;
        save_guard(env, &guard);
        emit_emergency_paused_all(env, &approvers);
        Ok(())
    }

    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        check_multi_sig(env, &approvers)?;
        let mut guard = load_guard(env);
        guard.pause_state = 0;
        save_guard(env, &guard);
        emit_resumed_all(env, &approvers);
        Ok(())
    }

    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        if threshold == 0 || threshold > admins.len() as u32 {
            return Err(GuardError::InvalidThreshold);
        }

        let first_admin = admins.get(0).ok_or(GuardError::NotInitialized)?;
        save_guard(
            env,
            &GuardState {
                pause_state: load_guard(env).pause_state,
                admins: admins.clone(),
                signature_threshold: threshold,
            },
        );

        if let Ok(mut pool) = load_pool(env) {
            pool.admin = first_admin;
            save_pool(env, &pool);
        }

        emit_guard_initialized(env, &admins, threshold);

        Ok(())
    }

    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError> {
        check_multi_sig(env, &approvers)?;

        let mut guard = load_guard(env);
        if !guard.admins.iter().any(|a| a == new_admin) {
            guard.admins.push_back(new_admin.clone());
            save_guard(env, &guard);
            emit_admin_added(env, &approvers, &new_admin);
        }

        Ok(())
    }

    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        check_multi_sig(env, &approvers)?;

        let guard = load_guard(env);
        if guard.admins.len() as u32 <= guard.signature_threshold {
            return Err(GuardError::InvalidThreshold);
        }

        let mut new_admins = Vec::new(env);
        let mut found = false;
        for a in guard.admins.iter() {
            if a != admin {
                new_admins.push_back(a);
            } else {
                found = true;
            }
        }

        if !found {
            return Err(GuardError::AdminNotFound);
        }

        save_guard(
            env,
            &GuardState {
                pause_state: guard.pause_state,
                admins: new_admins,
                signature_threshold: guard.signature_threshold,
            },
        );
        emit_admin_removed(env, &approvers, &admin);

        if let Ok(mut pool) = load_pool(env) {
            let updated_guard = load_guard(env);
            if let Some(first_admin) = updated_guard.admins.get(0) {
                pool.admin = first_admin;
                save_pool(env, &pool);
            }
        }

        Ok(())
    }

    fn get_admins(env: &Env) -> Vec<Address> {
        load_guard(env).admins
    }

    fn get_threshold(env: &Env) -> u32 {
        load_guard(env).signature_threshold
    }

    fn is_admin(env: &Env, addr: &Address) -> bool {
        load_guard(env).admins.iter().any(|a| a == *addr)
#[contractimpl]
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

    fn rotate_admin(
        env: &Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError> {
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
