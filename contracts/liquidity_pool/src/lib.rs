#![no_std]
use emergency_guard::{EmergencyGuard, PauseType};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, vec, Address, Env, String, Vec,
};

#[cfg(test)]
mod fuzz_test;
#[cfg(test)]
mod test;

/// Errors returned by the `LiquidityPool` contract.
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
    InvalidFee = 8,
    Paused = 9,
    InsufficientAllowance = 10,
    OracleNotConfigured = 11,
    InvalidOraclePrice = 12,
    TimelockNotElapsed = 13,
    NoPendingFeeUpdate = 14,
}

// ── Event types ──────────────────────────────────────────────────────────────

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

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeUpdateScheduledEvent {
    pub scheduled_by: Address,
    pub old_fee_bps: i128,
    pub new_fee_bps: i128,
    pub executable_after_ledger: u32,
    pub volatility_bps: i128,
}

// ── Grouped storage structs ───────────────────────────────────────────────────

/// Core pool state stored as a single instance-storage entry.
/// Replaces 8 separate DataKey variants: TokenA, TokenB, ReserveA, ReserveB,
/// TotalShares, FeeBasisPoints, BaseFeeBasisPoints, Admin, Paused.
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
    pub paused: bool,
}

/// Oracle / fee-governance config stored as a single instance-storage entry.
/// Replaces 4 separate DataKey variants: OracleAddress, LastOraclePrice,
/// LastVolatilityBps, FeeUpdateTimelockLedgers.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct OracleConfig {
    pub oracle: Address,
    pub last_price: i128,
    pub last_volatility_bps: i128,
    pub timelock_ledgers: u32,
}

/// Pending timelocked fee update (unchanged – single value, no grouping needed).
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PendingFeeUpdate {
    pub new_fee_bps: i128,
    pub executable_after_ledger: u32,
    pub based_on_volatility_bps: i128,
}

// ── Per-user storage keys (persistent, keyed by address) ─────────────────────

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

/// Remaining DataKey variants – only per-user keys that cannot be grouped.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Instance key: PoolState struct.
    Pool,
    /// Instance key: OracleConfig struct.
    Oracle,
    /// Instance key: PendingFeeUpdate.
    PendingFeeUpdate,
    /// Persistent key: per-user LP share balance.
    Balance(Address),
    /// Persistent key: per-user allowance.
    Allowance(AllowanceDataKey),
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

// ── Helpers ───────────────────────────────────────────────────────────────────

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

/// Load PoolState; returns Err(NotInitialized) when absent.
fn load_pool(e: &Env) -> Result<PoolState, Error> {
    e.storage()
        .instance()
        .get(&DataKey::Pool)
        .ok_or(Error::NotInitialized)
}

/// Persist PoolState back to instance storage.
fn save_pool(e: &Env, pool: &PoolState) {
    e.storage().instance().set(&DataKey::Pool, pool);
}

fn check_paused(pool: &PoolState) -> Result<(), Error> {
    if pool.paused {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

fn check_not_operation_paused(e: &Env, operation: u32) -> Result<(), Error> {
    if EmergencyGuard::is_paused(e.clone(), operation) {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

fn target_fee_from_volatility(base_fee_bps: i128, volatility_bps: i128) -> i128 {
    let dynamic_fee = if volatility_bps >= HIGH_VOLATILITY_THRESHOLD_BPS {
        HIGH_VOLATILITY_FEE_BPS
    } else if volatility_bps >= MEDIUM_VOLATILITY_THRESHOLD_BPS {
        MEDIUM_VOLATILITY_FEE_BPS
    } else if volatility_bps >= LOW_VOLATILITY_THRESHOLD_BPS {
        LOW_VOLATILITY_FEE_BPS
    } else {
        base_fee_bps
    };
    if dynamic_fee > MAX_FEE_BPS {
        MAX_FEE_BPS
    } else {
        dynamic_fee
    }
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct LiquidityPool;

#[contractimpl]
impl LiquidityPool {
    /// Initializes the liquidity pool once with token pair addresses.
    pub fn initialize(
        e: Env,
        admin: Address,
        token_a: Address,
        token_b: Address,
    ) -> Result<(), Error> {
        if e.storage().instance().has(&DataKey::Pool) {
            return Err(Error::AlreadyInitialized);
        }
        // One write instead of 9 separate writes.
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
        EmergencyGuard::initialize(e.clone(), vec![&e, admin], 1)
            .map_err(|_| Error::Unauthorized)?;
        Ok(())
    }

    /// Returns the current fee in basis points.
    pub fn get_fee(e: Env) -> i128 {
        // One read instead of one read per field.
        e.storage()
            .instance()
            .get::<_, PoolState>(&DataKey::Pool)
            .map(|p| p.fee_bps)
            .unwrap_or(DEFAULT_BASE_FEE_BPS)
    }

    /// Admin-only: update the swap fee. Valid range: 0–100 bps.
    pub fn set_fee(e: Env, fee_bps: i128) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&fee_bps) {
            return Err(Error::InvalidFee);
        }
        // One read + one write instead of 3 reads + 2 writes.
        let mut pool = load_pool(&e)?;
        pool.admin.require_auth();
        let old_fee = pool.fee_bps;
        pool.fee_bps = fee_bps;
        pool.base_fee_bps = fee_bps;
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

    /// Admin-only: configure external oracle and timelock parameters.
    pub fn configure_fee_oracle(
        e: Env,
        oracle: Address,
        base_fee_bps: i128,
        timelock_ledgers: u32,
    ) -> Result<(), Error> {
        if !(0..=MAX_FEE_BPS).contains(&base_fee_bps) {
            return Err(Error::InvalidFee);
        }
        // One read (pool) + one write (oracle config) instead of 1 read + 3 writes.
        let mut pool = load_pool(&e)?;
        pool.admin.require_auth();
        pool.base_fee_bps = base_fee_bps;
        save_pool(&e, &pool);

        let cfg = OracleConfig {
            oracle,
            last_price: 0,
            last_volatility_bps: 0,
            timelock_ledgers,
        };
        e.storage().instance().set(&DataKey::Oracle, &cfg);
        Ok(())
    }

    pub fn get_last_volatility_bps(e: Env) -> i128 {
        e.storage()
            .instance()
            .get::<_, OracleConfig>(&DataKey::Oracle)
            .map(|c| c.last_volatility_bps)
            .unwrap_or(0)
    }

    pub fn get_pending_fee_update(e: Env) -> Option<PendingFeeUpdate> {
        e.storage().instance().get(&DataKey::PendingFeeUpdate)
    }

    /// Pulls price from oracle, computes volatility and schedules a timelocked fee update.
    pub fn sync_fee_from_oracle(e: Env) -> Result<Option<PendingFeeUpdate>, Error> {
        // One read (oracle config) instead of 5 separate reads.
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

        let prev = cfg.last_price;
        cfg.last_price = current_price;

        if prev <= 0 {
            cfg.last_volatility_bps = 0;
            // One write instead of 2 writes.
            e.storage().instance().set(&DataKey::Oracle, &cfg);
            return Ok(None);
        }

        let price_delta = if current_price >= prev {
            current_price - prev
        } else {
            prev - current_price
        };
        let volatility_bps = price_delta
            .checked_mul(10_000)
            .ok_or(Error::InvalidOraclePrice)?
            / prev;

        cfg.last_volatility_bps = volatility_bps;
        // One write instead of 2 writes.
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

    /// Applies a previously scheduled fee update after timelock elapses.
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

        // One read + one write instead of 2 reads + 1 write.
        let mut pool = load_pool(&e)?;
        let old_fee = pool.fee_bps;
        pool.fee_bps = pending.new_fee_bps;
        save_pool(&e, &pool);
        e.storage().instance().remove(&DataKey::PendingFeeUpdate);

        e.events().publish(
            (String::from_str(&e, "fee_changed"), pool.admin.clone()),
            FeeChangedEvent {
                admin: pool.admin,
                old_fee_bps: old_fee,
                new_fee_bps: pending.new_fee_bps,
            },
        );

        Ok(pending.new_fee_bps)
    }

    /// Admin-only: pause or unpause the pool.
    pub fn set_paused(e: Env, paused: bool) -> Result<(), Error> {
        let mut pool = load_pool(&e)?;
        pool.admin.require_auth();
        pool.paused = paused;
        save_pool(&e, &pool);
        Ok(())
    }

    pub fn get_admin(e: Env) -> Result<Address, Error> {
        Ok(load_pool(&e)?.admin)
    }

    pub fn get_admins(e: Env) -> Vec<Address> {
        EmergencyGuard::get_admins(e)
    }

    pub fn get_admin_threshold(e: Env) -> u32 {
        EmergencyGuard::get_threshold(e)
    }

    pub fn set_operation_paused(
        e: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), Error> {
        EmergencyGuard::set_pause(e, admin, operation, paused).map_err(|_| Error::Unauthorized)
    }

    pub fn emergency_pause(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::emergency_pause(e, approvers).map_err(|_| Error::Unauthorized)
    }

    pub fn resume(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::resume(e, approvers).map_err(|_| Error::Unauthorized)
    }

    pub fn add_admin(e: Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), Error> {
        EmergencyGuard::add_admin(e, approvers, new_admin).map_err(|_| Error::Unauthorized)
    }

    pub fn remove_admin(e: Env, approvers: Vec<Address>, admin: Address) -> Result<(), Error> {
        EmergencyGuard::remove_admin(e.clone(), approvers, admin.clone())
            .map_err(|_| Error::Unauthorized)?;

        let admins = EmergencyGuard::get_admins(e.clone());
        let mut pool = load_pool(&e)?;
        if pool.admin == admin {
            pool.admin = admins.get(0).ok_or(Error::Unauthorized)?;
            save_pool(&e, &pool);
        }
        Ok(())
    }

    pub fn rotate_admin(
        e: Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), Error> {
        let admins = EmergencyGuard::get_admins(e.clone());
        if !admins.iter().any(|admin| admin == old_admin) {
            return Err(Error::Unauthorized);
        }

        EmergencyGuard::add_admin(e.clone(), approvers.clone(), new_admin.clone())
            .map_err(|_| Error::Unauthorized)?;
        EmergencyGuard::remove_admin(e.clone(), approvers, old_admin.clone())
            .map_err(|_| Error::Unauthorized)?;

        let mut pool = load_pool(&e)?;
        if pool.admin == old_admin {
            pool.admin = new_admin;
            save_pool(&e, &pool);
        }
        Ok(())
    }

    /// Deposits token A and token B into the pool and mints LP shares.
    pub fn deposit(e: Env, to: Address, amount_a: i128, amount_b: i128) -> Result<i128, Error> {
        // One read instead of 5 separate reads.
        let mut pool = load_pool(&e)?;
        check_paused(&pool)?;
        check_not_operation_paused(&e, PauseType::DEPOSIT)?;
        to.require_auth();

        let client_a = soroban_sdk::token::Client::new(&e, &pool.token_a);
        let client_b = soroban_sdk::token::Client::new(&e, &pool.token_b);
        client_a.transfer(&to, &e.current_contract_address(), &amount_a);
        client_b.transfer(&to, &e.current_contract_address(), &amount_b);

        let shares: i128 = if pool.total_shares == 0 {
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

        // Update user balance (persistent, per-user).
        let user_key = DataKey::Balance(to.clone());
        let current: i128 = e.storage().persistent().get(&user_key).unwrap_or(0);
        e.storage().persistent().set(&user_key, &(current + shares));
        e.storage().persistent().extend_ttl(&user_key, 100, 100);

        // Update pool state – one write instead of 3 writes.
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

    /// Swaps into one side of the pool using constant-product pricing.
    pub fn swap(e: Env, to: Address, buy_a: bool, out: i128, in_max: i128) -> Result<i128, Error> {
        // One read instead of 5 separate reads.
        let mut pool = load_pool(&e)?;
        check_paused(&pool)?;
        check_not_operation_paused(&e, PauseType::SWAP)?;
        to.require_auth();

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

        // Update reserves – one write instead of 2 writes.
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

    /// Burns LP shares and withdraws proportional token A and token B reserves.
    pub fn withdraw(e: Env, to: Address, share_amount: i128) -> Result<(i128, i128), Error> {
        // One read instead of 4 separate reads.
        let mut pool = load_pool(&e)?;
        check_paused(&pool)?;
        check_not_operation_paused(&e, PauseType::WITHDRAW)?;
        to.require_auth();

        let user_key = DataKey::Balance(to.clone());
        let current: i128 = e.storage().persistent().get(&user_key).unwrap_or(0);
        if share_amount > current {
            return Err(Error::InsufficientShares);
        }

        let amount_a = share_amount * pool.reserve_a / pool.total_shares;
        let amount_b = share_amount * pool.reserve_b / pool.total_shares;

        e.storage()
            .persistent()
            .set(&user_key, &(current - share_amount));
        e.storage().persistent().extend_ttl(&user_key, 100, 100);

        // One write instead of 3 writes.
        pool.total_shares -= share_amount;
        pool.reserve_a -= amount_a;
        pool.reserve_b -= amount_b;
        save_pool(&e, &pool);

        soroban_sdk::token::Client::new(&e, &pool.token_a).transfer(
            &e.current_contract_address(),
            &to,
            &amount_a,
        );
        soroban_sdk::token::Client::new(&e, &pool.token_b).transfer(
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

    /// Burns LP shares without withdrawing token reserves.
    pub fn burn(e: Env, from: Address, amount: i128) -> Result<(), Error> {
        let mut pool = load_pool(&e)?;
        check_paused(&pool)?;
        check_not_operation_paused(&e, PauseType::BURN)?;
        from.require_auth();

        let user_key = DataKey::Balance(from.clone());
        let current: i128 = e.storage().persistent().get(&user_key).unwrap_or(0);
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

    // ── Token interface ───────────────────────────────────────────────────────

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
        e.storage()
            .instance()
            .get::<_, PoolState>(&DataKey::Pool)
            .map(|p| p.total_shares)
            .unwrap_or(0)
    }

    pub fn transfer(e: Env, from: Address, to: Address, amount: i128) -> Result<(), Error> {
        from.require_auth();

        let from_key = DataKey::Balance(from.clone());
        let to_key = DataKey::Balance(to.clone());

        let from_balance: i128 = e.storage().persistent().get(&from_key).unwrap_or(0);
        if from_balance < amount {
            return Err(Error::InsufficientBalance);
        }

        e.storage()
            .persistent()
            .set(&from_key, &(from_balance - amount));
        e.storage().persistent().extend_ttl(&from_key, 100, 100);

        let to_balance: i128 = e.storage().persistent().get(&to_key).unwrap_or(0);
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
