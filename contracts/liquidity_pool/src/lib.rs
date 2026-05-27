#![no_std]
use emergency_guard::{EmergencyGuard, PauseType};
use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String, Vec};

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

// Helper function: integer square root using Newton's method
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
    TokenA,
    TokenB,
    ReserveA,
    ReserveB,
    ShareToken,
    TotalShares,
    Balance(Address),
    Allowance(AllowanceDataKey),
    Admin,
    FeeBasisPoints,
    Pool,
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

#[soroban_sdk::contractclient(name = "PriceOracleClient")]
pub trait PriceOracle {
    fn latest_price(e: Env) -> i128;
}

fn check_not_paused(e: &Env, operation: u32) -> Result<(), Error> {
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
        e.storage().instance().set(&DataKey::Admin, &admin);
        e.storage().instance().set(&DataKey::TokenA, &token_a);
        e.storage().instance().set(&DataKey::TokenB, &token_b);
        e.storage().instance().set(&DataKey::ReserveA, &0i128);
        e.storage().instance().set(&DataKey::ReserveB, &0i128);
        e.storage().instance().set(&DataKey::TotalShares, &0i128);
        // Default fee: 30 bps (≈ 0.3%)
        e.storage()
            .instance()
            .set(&DataKey::FeeBasisPoints, &30i128);
        Ok(())
    }

    /// Returns the current fee in basis points.
    pub fn get_fee(e: Env) -> i128 {
        // One read instead of one read per field.
        e.storage()
            .instance()
            .get(&DataKey::FeeBasisPoints)
            .unwrap_or(30)
    }

    /// Admin-only: update the swap fee. Valid range: 0–100 bps.
    pub fn set_fee(e: Env, fee_bps: i128) -> Result<(), Error> {
        if !(0..=100).contains(&fee_bps) {
            return Err(Error::InvalidFee);
        }
        let admin: Address = e
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)?;
        admin.require_auth();
        let old_fee: i128 = e
            .storage()
            .instance()
            .get(&DataKey::FeeBasisPoints)
            .unwrap_or(30);
        e.storage()
            .instance()
            .set(&DataKey::FeeBasisPoints, &fee_bps);
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

    /// Admin-only: pause or unpause the pool for a specific operation.
    pub fn set_paused(e: Env, admin: Address, operation: u32, paused: bool) -> Result<(), Error> {
        EmergencyGuard::set_pause(e, admin, operation, paused).map_err(|_| Error::Unauthorized)
    }

    /// Admin-only: emergency pause all operations.
    pub fn emergency_pause(e: Env, approvers: Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::emergency_pause(e, approvers).map_err(|_| Error::Unauthorized)
    }

    /// Deposits token A and token B into the pool and mints LP shares.
    pub fn deposit(e: Env, to: Address, amount_a: i128, amount_b: i128) -> Result<i128, Error> {
        check_not_paused(&e, PauseType::DEPOSIT)?;
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
        check_not_paused(&e, PauseType::SWAP)?;
        to.require_auth();

        let (reserve_in, reserve_out, token_in, token_out) = if buy_a {
            (pool.reserve_b, pool.reserve_a, pool.token_b.clone(), pool.token_a.clone())
        } else {
            (pool.reserve_a, pool.reserve_b, pool.token_a.clone(), pool.token_b.clone())
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

        soroban_sdk::token::Client::new(&e, &token_in)
            .transfer(&to, &e.current_contract_address(), &amount_in);
        soroban_sdk::token::Client::new(&e, &token_out)
            .transfer(&e.current_contract_address(), &to, &out);

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
        check_not_paused(&e, PauseType::WITHDRAW)?;
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

        soroban_sdk::token::Client::new(&e, &pool.token_a)
            .transfer(&e.current_contract_address(), &to, &amount_a);
        soroban_sdk::token::Client::new(&e, &pool.token_b)
            .transfer(&e.current_contract_address(), &to, &amount_b);

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
        check_not_paused(&e, PauseType::BURN)?;
        from.require_auth();

        let user_key = DataKey::Balance(from.clone());
        let current: i128 = e.storage().persistent().get(&user_key).unwrap_or(0);
        if amount > current {
            return Err(Error::InsufficientShares);
        }

        e.storage()
            .persistent()
            .set(&user_key, &(current - amount));
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
        e.storage()
            .persistent()
            .set(&key, &AllowanceValue { amount, expiration_ledger });
        e.storage().persistent().extend_ttl(&key, 100, 100);

        Ok(())
    }

    pub fn allowance(e: Env, from: Address, spender: Address) -> i128 {
        let key = DataKey::Allowance(AllowanceDataKey { from, spender });
        match e
            .storage()
            .persistent()
            .get::<_, AllowanceValue>(&key)
        {
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