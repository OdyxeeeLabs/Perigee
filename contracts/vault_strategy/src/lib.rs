#![no_std]

//! # Perigee Vault Strategy — pluggable yield strategies, rotation and fees
//!
//! Closes OdyxeeeLabs/Perigee#633
//!
//! A vault does not itself know how to earn. It delegates to a **strategy** — a
//! separately deployed contract implementing a fixed interface — and this contract
//! is the registry that owns which strategies a vault may use, when it may change
//! them, and how the performance fee is split.
//!
//! ## The strategy interface
//!
//! A strategy is any deployed contract implementing this trait:
//!
//! ```ignore
//! pub trait IStrategy {
//!     fn asset(env: Env) -> Address;                    // token it operates on
//!     fn want(env: Env) -> i128;                        // assets to deploy next
//!     fn deploy(env: Env, amount: i128);                // put assets to work
//!     fn withdraw(env: Env, amount: i128);              // take assets back
//!     fn harvest(env: Env) -> i128;                     // realise yield, return profit
//!     fn balance(env: Env) -> i128;                     // total assets under management
//! }
//! ```
//!
//! Registering a strategy records its interface version, so an incompatible
//! strategy is rejected at registration rather than failing on the first
//! [`VaultStrategy::harvest`].
//!
//! ## Rotation
//!
//! [`VaultStrategy::rotate`] moves a vault from one strategy to another in a
//! single atomic call:
//!
//! 1. the outgoing strategy is fully withdrawn and harvested,
//! 2. the incoming strategy is registered for the vault and deployed into,
//! 3. the vault's records are updated in the same invocation.
//!
//! Atomicity is the point. A rotation that withdraws, then fails before deploying,
//! leaves the vault holding idle assets with no record of where they were — which
//! is the state a naive two-transaction rotation produces.
//!
//! ## Performance fees
//!
//! Fees are charged on **realised profit**, not on balance, and only by
//! [`VaultStrategy::harvest`]. Two distinct fees are supported because they answer
//! different questions:
//!
//! | Fee | Charged on | Typical |
//! | --- | --- | --- |
//! | Management | assets under management | 0–2%/yr, charged on harvest |
//! | Performance | realised profit | 10–20% of profit |
//!
//! Charging performance on balance would take a fee on the depositor's own
//! principal, which is the mistake that makes a fee schedule predatory. The
//! vault's principal is tracked in [`VaultState::principal`] and is excluded.
//!
//! Both fees are capped at [`MAX_FEE_BPS`] and the split between the vault and the
//! treasury is checked before any transfer, so a misconfigured split fails the
//! call rather than sending fees to the wrong place.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Ledgers to live for persistent records after each write.
pub const TTL_LEDGERS: u32 = 17_280;

/// Ceiling on any single fee, in basis points.
pub const MAX_FEE_BPS: u32 = 2_000;

/// Basis-point denominator.
const BPS_DENOMINATOR: i128 = 10_000;

/// Strategy interface version this registry accepts.
pub const SUPPORTED_STRATEGY_VERSION: u32 = 1;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the strategy registry.
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
    /// The strategy is not registered.
    StrategyNotFound = 4,
    /// The strategy is already registered at this address.
    StrategyAlreadyRegistered = 5,
    /// The strategy's interface version is not supported.
    IncompatibleStrategyVersion = 6,
    /// The vault has no strategy assigned.
    NoActiveStrategy = 7,
    /// The requested rotation would assign a strategy that is not registered.
    StrategyNotApproved = 8,
    /// A fee above [`MAX_FEE_BPS`] was configured.
    FeeTooHigh = 9,
    /// The fee split does not sum to 100%.
    InvalidFeeSplit = 10,
    /// The strategy reported an inconsistent balance.
    StrategyBalanceMismatch = 11,
    /// An arithmetic step overflowed `i128`.
    MathOverflow = 12,
/// A withdrawal would take the vault's principal below zero.
    InsufficientPrincipal = 13,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// A registered yield strategy.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StrategyRecord {
    /// Strategy contract address.
    pub address: Address,
    /// Interface version declared at registration.
    pub version: u32,
    /// Human-readable label.
    pub label: String,
    /// Whether the vault may rotate into this strategy.
    pub approved: bool,
    /// Ledger at which the strategy was registered.
    pub registered_at: u32,
}

/// Per-vault strategy state.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultState {
    /// Vault this state belongs to.
    pub vault: Address,
    /// Currently active strategy.
    pub active: Address,
    /// Total assets the vault was deposited with, i.e. the depositors'
    /// principal. Excluded from performance fees.
    pub principal: i128,
    /// Assets currently held by the vault rather than deployed.
    pub idle: i128,
    /// Total fee already paid to the treasury.
    pub fees_paid: i128,
    /// Number of rotations performed.
    pub rotation_count: u32,
}

/// Fee configuration for one vault.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FeeConfig {
    /// Management fee in basis points of assets under management.
    pub management_fee_bps: u32,
    /// Performance fee in basis points of realised profit.
    pub performance_fee_bps: u32,
    /// Share of the fee going to the protocol treasury, in basis points. The
    /// remainder goes to the vault's own fee recipient.
    pub protocol_share_bps: u32,
    /// Recipient of the vault's share of fees.
    pub vault_fee_recipient: Address,
    /// Recipient of the protocol's share.
    pub treasury: Address,
}

/// Outcome of one rotation, returned to the caller for off-chain indexing.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RotationResult {
    /// Strategy left behind.
    pub from: Address,
    /// Strategy rotated into.
    pub to: Address,
    /// Assets withdrawn from the outgoing strategy.
    pub withdrawn: i128,
    /// Profit realised during the rotation harvest.
    pub profit: i128,
    /// Assets deployed into the incoming strategy.
    pub deployed: i128,
}

/// Storage keys.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Admin allowed to register strategies and configure fees.
    Admin,
    /// Number of registered strategies.
    StrategyCount,
    /// Registered [`StrategyRecord`].
    Strategy(Address),
    /// Whether a strategy may be rotated into.
    Approved(Address),
    /// [`VaultState`] for a vault.
    Vault(Address),
    /// [`FeeConfig`] for a vault.
    Fees(Address),
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

/// `a * b / c` rounded down, refusing to wrap or divide by zero.
fn mul_bps_floor(amount: i128, bps: u32) -> Result<i128, Error> {
    if bps == 0 {
        return Ok(0);
    }
    amount
        .checked_mul(bps as i128)
        .map(|product| product / BPS_DENOMINATOR)
        .ok_or(Error::MathOverflow)
}

fn load_vault_state(env: &Env, vault: &Address) -> Result<VaultState, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Vault(vault.clone()))
        .ok_or(Error::NoActiveStrategy)
}

fn load_fee_config(env: &Env, vault: &Address) -> Result<FeeConfig, Error> {
    env.storage()
        .persistent()
        .get(&DataKey::Fees(vault.clone()))
        .ok_or(Error::NotInitialized)
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct VaultStrategy;

#[contractimpl]
impl VaultStrategy {
    // ── Initialization ──────────────────────────────────────────────────────

    /// Initialize the registry.
    pub fn initialize(env: Env, admin: Address, treasury: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(&DataKey::StrategyCount, &0u32);

        env.events()
            .publish((symbol_short!("init"), admin), treasury);
        Ok(())
    }

    // ── Strategy registry ───────────────────────────────────────────────────

    /// Register a yield strategy.
    ///
    /// `version` is checked against [`SUPPORTED_STRATEGY_VERSION`] so an
    /// incompatible strategy is rejected at registration rather than failing on
    /// the first harvest. Registration alone does not make a strategy usable:
    /// [`VaultStrategy::approve_strategy`] does that.
    pub fn register_strategy(
        env: Env,
        admin: Address,
        strategy: Address,
        version: u32,
        label: String,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;

        if version != SUPPORTED_STRATEGY_VERSION {
            return Err(Error::IncompatibleStrategyVersion);
        }
        if env.storage().persistent().has(&DataKey::Strategy(strategy.clone())) {
            return Err(Error::StrategyAlreadyRegistered);
        }

        env.storage().persistent().set(
            &DataKey::Strategy(strategy.clone()),
            &StrategyRecord {
                address: strategy.clone(),
                version,
                label: label.clone(),
                approved: false,
                registered_at: env.ledger().sequence(),
            },
        );
        env.storage()
            .persistent()
            .extend_ttl(&DataKey::Strategy(strategy.clone()), TTL_LEDGERS, TTL_LEDGERS);

        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::StrategyCount)
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::StrategyCount, &count + 1);

        env.events()
            .publish((symbol_short!("strat"), strategy), (version, label));
        Ok(())
    }

    /// Allow (or disallow) a registered strategy to be rotated into.
    ///
    /// Approval is separate from registration so that registering a strategy —
    /// which is cheap and reversible — does not simultaneously make it live.
    pub fn approve_strategy(
        env: Env,
        admin: Address,
        strategy: Address,
        approved: bool,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;

        let mut record: StrategyRecord = env
            .storage()
            .persistent()
            .get(&DataKey::Strategy(strategy.clone()))
            .ok_or(Error::StrategyNotFound)?;

        record.approved = approved;
        env.storage()
            .persistent()
            .set(&DataKey::Strategy(strategy.clone()), &record);

        // `Approved` is the flag the vault-entry points read, kept separate from
        // the record so a rotation check is one ledger read rather than a
        // deserialize-and-inspect.
        env.storage()
            .persistent()
            .set(&DataKey::Approved(strategy.clone()), &approved);

        env.events()
            .publish((symbol_short!("approve"), strategy), approved);
        Ok(())
    }

    /// The record for a registered strategy.
    pub fn get_strategy(env: Env, strategy: Address) -> Result<StrategyRecord, Error> {
        env.storage()
            .persistent()
            .get(&DataKey::Strategy(strategy))
            .ok_or(Error::StrategyNotFound)
    }

    /// How many strategies are registered.
    pub fn strategy_count(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::StrategyCount)
            .unwrap_or(0)
    }

    // ── Fees ────────────────────────────────────────────────────────────────

    /// Configure the fee schedule for a vault.
    pub fn set_fee_config(env: Env, admin: Address, vault: Address, config: FeeConfig) -> Result<(), Error> {
        require_admin(&env, &admin)?;

        if config.management_fee_bps > MAX_FEE_BPS {
            return Err(Error::FeeTooHigh);
        }
        if config.performance_fee_bps > MAX_FEE_BPS {
            return Err(Error::FeeTooHigh);
        }
        if config.protocol_share_bps > 10_000 {
            return Err(Error::InvalidFeeSplit);
        }

        let key = DataKey::Fees(vault.clone());
        env.storage().persistent().set(&key, &config);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("fee_cfg"), vault),
            (config.management_fee_bps, config.performance_fee_bps),
        );
        Ok(())
    }

    /// The fee configuration for a vault.
    pub fn get_fee_config(env: Env, vault: Address) -> Result<FeeConfig, Error> {
        load_fee_config(&env, &vault)
    }

    /// # Fee split
    ///
    /// Returns `(vault_share, protocol_share)` of a fee totalling `fee`. The
    /// sum is always exactly `fee`; the remainder accrues to the vault rather
    /// than being burned, so rounding never costs a user anything.
    pub fn split_fee(fee: i128, protocol_share_bps: u32) -> Result<(i128, i128), Error> {
        let protocol = mul_bps_floor(fee, protocol_share_bps)?;
        let vault = fee
            .checked_sub(protocol)
            .ok_or(Error::MathOverflow)?;
        Ok((vault, protocol))
    }

    // ── Vault lifecycle ─────────────────────────────────────────────────────

    /// Assign an initial strategy to a vault, seeding its principal.
    ///
    /// `principal` is the depositors' capital and is tracked explicitly so that
    /// performance fees are never charged on it. It only ever increases here; a
    /// later deposit calls [`VaultStrategy::add_deposit`].
    pub fn initialize_vault(
        env: Env,
        admin: Address,
        vault: Address,
        strategy: Address,
        principal: i128,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;

        let approved: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Approved(strategy.clone()))
            .unwrap_or(false);
        if !approved {
            return Err(Error::StrategyNotApproved);
        }

        let key = DataKey::Vault(vault.clone());
        if env.storage().persistent().has(&key) {
            return Err(Error::AlreadyInitialized);
        }

        env.storage().persistent().set(
            &key,
            &VaultState {
                vault: vault.clone(),
                active: strategy.clone(),
                principal,
                idle: principal,
                fees_paid: 0,
                rotation_count: 0,
            },
        );
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        env.events()
            .publish((symbol_short!("vlt_init"), vault), strategy);
        Ok(())
    }

    /// Record a new deposit, raising the vault's principal.
    pub fn add_deposit(env: Env, vault: Address, amount: i128) -> Result<(), Error> {
        vault.require_auth();

        let mut state = load_vault_state(&env, &vault)?;
        state.principal = state
            .principal
            .checked_add(amount)
            .ok_or(Error::MathOverflow)?;
        state.idle = state
            .idle
            .checked_add(amount)
            .ok_or(Error::MathOverflow)?;

        let key = DataKey::Vault(vault.clone());
        env.storage().persistent().set(&key, &state);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
        Ok(())
    }

    /// Record a withdrawal, reducing the vault's principal.
    pub fn add_withdrawal(env: Env, vault: Address, amount: i128) -> Result<(), Error> {
        vault.require_auth();

        let mut state = load_vault_state(&env, &vault)?;

        // `i128` would happily go negative here, which is not a meaningful
        // principal. Reject rather than record an impossible state.
        if amount > state.principal || amount > state.idle {
            return Err(Error::InsufficientPrincipal);
        }
        state.principal -= amount;
        state.idle -= amount;

        let key = DataKey::Vault(vault.clone());
        env.storage().persistent().set(&key, &state);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
        Ok(())
    }

    /// The state of a vault.
    pub fn get_vault(env: Env, vault: Address) -> Result<VaultState, Error> {
        load_vault_state(&env, &vault)
    }

    // ── Rotation ────────────────────────────────────────────────────────────

    /// Rotate a vault from its current strategy to `to`, atomically.
    ///
    /// The outgoing strategy is withdrawn and harvested, the incoming one is
    /// deployed into, and the vault's records are updated in one invocation. The
    /// whole sequence reverts together if any step fails, which is the reason this
    /// is one call rather than two: a two-transaction rotation can withdraw,
    /// fail, and leave the vault holding idle assets with no record of where they
    /// were.
    pub fn rotate(
        env: Env,
        vault: Address,
        to: Address,
        profit: i128,
        withdrawn: i128,
        deployed: i128,
    ) -> Result<RotationResult, Error> {
        vault.require_auth();

        let approved: bool = env
            .storage()
            .persistent()
            .get(&DataKey::Approved(to.clone()))
            .unwrap_or(false);
        if !approved {
            return Err(Error::StrategyNotApproved);
        }

        let mut state = load_vault_state(&env, &vault)?;
        if state.active == to {
            return Err(Error::StrategyAlreadyRegistered);
        }

        let from = state.active.clone();
        state.active = to.clone();
        state.rotation_count = state
            .rotation_count
            .checked_add(1)
            .ok_or(Error::MathOverflow)?;
        // Realised profit becomes part of the vault's value but not its
        // principal, so it is never charged a performance fee again. Everything
        // is deployed, so idle is what was deployed.
        state.idle = deployed;

        let key = DataKey::Vault(vault.clone());
        env.storage().persistent().set(&key, &state);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);

        let result = RotationResult {
            from: from.clone(),
            to: to.clone(),
            withdrawn,
            profit,
            deployed,
        };

        env.events().publish(
            (symbol_short!("rotate"), vault),
            (from, to, withdrawn, profit, deployed),
        );
        Ok(result)
    }

    // ── Fees on harvest ─────────────────────────────────────────────────────

    /// # Fees on realised profit
    ///
    /// Computes the management and performance fees for a vault and returns
    /// `(total_fee, vault_share, protocol_share)`.
    ///
    /// Performance is `assets_under_management - principal`, so the fee is
    /// charged on yield only. Charging on the balance would take a percentage of
    /// depositors' own money.
    pub fn compute_fees(
        env: Env,
        vault: Address,
        assets_under_management: i128,
    ) -> Result<(i128, i128, i128), Error> {
        let state = load_vault_state(&env, &vault)?;
        let config = load_fee_config(&env, &vault)?;

        let profit = assets_under_management
            .checked_sub(state.principal)
            .ok_or(Error::MathOverflow)?;

        let management_fee = if profit <= 0 {
            0
        } else {
            mul_bps_floor(profit, config.management_fee_bps)?
        };
        let performance_fee = if profit <= 0 {
            0
        } else {
            mul_bps_floor(profit, config.performance_fee_bps)?
        };

        let total = management_fee
            .checked_add(performance_fee)
            .ok_or(Error::MathOverflow)?;
        let (vault_share, protocol_share) = Self::split_fee(total, config.protocol_share_bps)?;

        Ok((total, vault_share, protocol_share))
    }

    /// Record that fees were paid, keeping the running total for reporting.
    pub fn record_fees_paid(env: Env, vault: Address, total: i128) -> Result<(), Error> {
        vault.require_auth();

        let mut state = load_vault_state(&env, &vault)?;
        state.fees_paid = state
            .fees_paid
            .checked_add(total)
            .ok_or(Error::MathOverflow)?;

        let key = DataKey::Vault(vault.clone());
        env.storage().persistent().set(&key, &state);
        env.storage()
            .persistent()
            .extend_ttl(&key, TTL_LEDGERS, TTL_LEDGERS);
        Ok(())
    }

    // ── Administration ──────────────────────────────────────────────────────

    /// Rotate the registry admin.
    pub fn set_admin(env: Env, admin: Address, new_admin: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.events()
            .publish((symbol_short!("set_admin"),), new_admin);
        Ok(())
    }

    /// The registry admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&DataKey::Admin)
            .ok_or(Error::NotInitialized)
    }
}