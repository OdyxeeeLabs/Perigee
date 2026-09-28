#![no_std]

//! # Perigee Policy Vault — vault provisioning with per-manager rate limiting
//!
//! The Policy Vault is the custody layer of Perigee: a Stellar multi-signature
//! account paired with a Soroban policy contract. Each end-client gets their
//! own policy vault, provisioned by a wealth manager.
//!
//! Vault provisioning is a state-changing, resource-consuming operation, so an
//! unthrottled `create_vault` is a griefing vector: a single manager (or a
//! compromised manager key) can spam vault creation and exhaust ledger
//! footprint, storage rent, and downstream indexer capacity.
//!
//! This contract closes that vector with **per-manager, per-epoch rate
//! limiting** (issue #505 / CONTRACT-11). The limit is configurable by the
//! admin and is expressed as a maximum number of vaults a single manager may
//! create within one *epoch*, where an epoch is a fixed number of ledgers:
//!
//! ```text
//! epoch(ledger) = ledger / epoch_length_ledgers
//! ```
//!
//! The default configuration used in tests and examples is *10 vaults per
//! 1 000 ledgers*. Each manager has an independent counter, so one manager
//! exhausting their quota never blocks another manager. When the ledger
//! advances past the epoch boundary the counter for the new epoch starts at
//! zero, so a manager's budget refreshes every epoch without any admin action.
//!
//! ## Design notes
//!
//! * **Per-manager isolation** — the counter is keyed by the manager address
//!   ([`DataKey::ManagerState`]); a noisy neighbour cannot starve others.
//! * **Deterministic epochs** — the epoch index is derived from the ledger
//!   sequence, so every caller computes the same epoch and there is no
//!   "first caller resets the window" race.
//! * **Idempotent vault creation** — a `(manager, vault_id)` pair can only be
//!   registered once. A duplicate is rejected *before* the quota is consumed,
//!   so retries are safe and never silently burn a manager's budget.
//! * **Configurable** — [`PolicyVault::set_rate_limit`] lets the admin retune
//!   both dimensions at runtime. Changing `epoch_length_ledgers` starts a fresh
//!   epoch on the next call, which is the documented behaviour below.
//!
//! ## Example
//!
//! ```ignore
//! // Deploy and configure: 10 vaults per manager per 1000 ledgers.
//! PolicyVault::initialize(env, admin, 10, 1_000);
//!
//! // A manager creates vaults; each call consumes one unit of quota.
//! PolicyVault::create_vault(env, manager, 1)?; // remaining = 9
//! // ...
//! PolicyVault::create_vault(env, manager, 10)?; // remaining = 0
//! PolicyVault::create_vault(env, manager, 11);  // Err(RateLimitExceeded)
//! ```

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, Address, Env,
};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Ledgers to live for per-manager and vault records after each write.
///
/// 17 280 ledgers ≈ 24 h at the Stellar target of one ledger every 5 s. Records
/// are re-extended on every write and on every quota read that rolls the epoch
/// forward, so an actively managing account never expires.
pub const TTL_LEDGERS: u32 = 17_280;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors returned by the Policy Vault contract.
///
/// Discriminants are stable so off-chain clients (and the unified
/// `Perigee-error-codes` decoder) can map failures to user-facing messages.
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    /// `initialize` may only be called once.
    AlreadyInitialized = 1,
    /// The contract has not been initialized yet.
    NotInitialized = 2,
    /// The caller is not the current admin.
    Unauthorized = 3,
    /// A rate-limit configuration value was zero or otherwise invalid.
    InvalidConfig = 4,
    /// The manager has created `max_vaults_per_epoch` vaults in this epoch.
    RateLimitExceeded = 5,
    /// This `(manager, vault_id)` pair has already been registered.
    VaultAlreadyExists = 6,
    /// A lifetime counter overflowed its `u32` range.
    Overflow = 7,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// The admin-configurable rate limit applied to every manager.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RateLimitConfig {
    /// Maximum vaults a single manager may create within one epoch.
    pub max_vaults_per_epoch: u32,
    /// Length of one epoch, measured in ledgers. Must be non-zero.
    pub epoch_length_ledgers: u32,
}

/// Per-manager counter for the epoch currently in progress.
///
/// `count` is only meaningful for `epoch`; when the ledger advances into a new
/// epoch the counter is treated as zero without an explicit write.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManagerEpochState {
    /// Epoch index the `count` belongs to (`ledger / epoch_length_ledgers`).
    pub epoch: u32,
    /// Vaults created by this manager in `epoch`.
    pub count: u32,
    /// Vaults created by this manager over the lifetime of the contract.
    pub total_vaults: u32,
}

/// An immutable record of a successfully provisioned vault.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultRecord {
    /// The manager that provisioned the vault.
    pub manager: Address,
    /// Caller-supplied vault identifier, unique per manager.
    pub vault_id: u32,
    /// Ledger sequence at which the vault was created.
    pub created_ledger: u32,
    /// Epoch the vault was created in.
    pub epoch: u32,
}

/// Storage keys.
///
/// `Admin` and `Config` are small and read on (almost) every call, so they live
/// in instance storage. Per-manager counters and vault records are unbounded in
/// number and therefore use persistent storage.
#[contracttype]
#[derive(Clone)]
pub enum DataKey {
    /// Current admin address. Presence also marks the contract as initialized.
    Admin,
    /// Active [`RateLimitConfig`].
    Config,
    /// Per-manager [`ManagerEpochState`].
    ManagerState(Address),
    /// [`VaultRecord`] keyed by the `(manager, vault_id)` pair.
    Vault(Address, u32),
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn load_admin(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Admin)
        .ok_or(Error::NotInitialized)
}

fn load_config(env: &Env) -> Result<RateLimitConfig, Error> {
    env.storage()
        .instance()
        .get(&DataKey::Config)
        .ok_or(Error::NotInitialized)
}

/// Gates an admin entry point on the stored admin address.
///
/// The passed `admin` must equal the address recorded at initialization, which
/// yields a decodable [`Error::Unauthorized`] for a mismatched caller instead
/// of a bare host auth error. The subsequent `require_auth` call still demands
/// the admin's signature, so passing the admin's *address* is not sufficient to
/// act as the admin.
fn require_admin(env: &Env, admin: &Address) -> Result<(), Error> {
    if *admin != load_admin(env)? {
        return Err(Error::Unauthorized);
    }
    admin.require_auth();
    Ok(())
}

/// Rejects configurations that would silently disable the rate limit.
fn validate_config(max_vaults_per_epoch: u32, epoch_length_ledgers: u32) -> Result<(), Error> {
    if max_vaults_per_epoch == 0 || epoch_length_ledgers == 0 {
        return Err(Error::InvalidConfig);
    }
    Ok(())
}

/// Epoch index for the current ledger. `epoch_length_ledgers` is validated to
/// be non-zero before it is ever stored, so the division cannot panic.
fn epoch_for(env: &Env, epoch_length_ledgers: u32) -> u32 {
    env.ledger().sequence() / epoch_length_ledgers
}

/// Loads the manager's counter for `current_epoch`, rolling the counter over to
/// the current epoch when the stored epoch is stale. A rolled-over state keeps
/// the lifetime `total_vaults` but resets `count` to zero.
fn load_manager_state(env: &Env, manager: &Address, current_epoch: u32) -> ManagerEpochState {
    let key = DataKey::ManagerState(manager.clone());
    let stored: Option<ManagerEpochState> = env.storage().persistent().get(&key);

    match stored {
        Some(state) if state.epoch == current_epoch => state,
        Some(state) => ManagerEpochState {
            epoch: current_epoch,
            count: 0,
            total_vaults: state.total_vaults,
        },
        None => ManagerEpochState {
            epoch: current_epoch,
            count: 0,
            total_vaults: 0,
        },
    }
}

// ── Contract ─────────────────────────────────────────────────────────────────

#[contract]
pub struct PolicyVault;

#[contractimpl]
impl PolicyVault {
    // ── Admin ────────────────────────────────────────────────────────────────

    /// Initialize the contract with an admin and the initial rate limit.
    ///
    /// `max_vaults_per_epoch` is the number of vaults a single manager may
    /// create per epoch and `epoch_length_ledgers` is the epoch length in
    /// ledgers (e.g. `10` and `1_000`). Both must be greater than zero.
    ///
    /// The `admin` must authorize this call, which binds initialization to the
    /// admin key at deployment time and prevents front-running of the init
    /// transaction by an attacker.
    pub fn initialize(
        env: Env,
        admin: Address,
        max_vaults_per_epoch: u32,
        epoch_length_ledgers: u32,
    ) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        validate_config(max_vaults_per_epoch, epoch_length_ledgers)?;
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage().instance().set(
            &DataKey::Config,
            &RateLimitConfig {
                max_vaults_per_epoch,
                epoch_length_ledgers,
            },
        );

        env.events().publish(
            (symbol_short!("init"), admin),
            (max_vaults_per_epoch, epoch_length_ledgers),
        );
        Ok(())
    }

    /// Update the rate limit. Admin only.
    ///
    /// `admin` must be the address recorded at initialization and must authorize
    /// the call; any other address fails with [`Error::Unauthorized`].
    ///
    /// Both dimensions are configurable at runtime so the operator can react to
    /// abuse (lower the limit) or to a legitimate onboarding spike (raise it)
    /// without redeploying the contract.
    ///
    /// Changing `epoch_length_ledgers` re-partitions the ledger timeline, so the
    /// epoch stored for each manager no longer matches the computed epoch and
    /// the next [`PolicyVault::create_vault`] starts a fresh epoch. This is
    /// intentional and documented; the lifetime `total_vaults` counter is
    /// unaffected.
    pub fn set_rate_limit(
        env: Env,
        admin: Address,
        max_vaults_per_epoch: u32,
        epoch_length_ledgers: u32,
    ) -> Result<(), Error> {
        require_admin(&env, &admin)?;
        validate_config(max_vaults_per_epoch, epoch_length_ledgers)?;

        env.storage().instance().set(
            &DataKey::Config,
            &RateLimitConfig {
                max_vaults_per_epoch,
                epoch_length_ledgers,
            },
        );

        env.events().publish(
            (symbol_short!("rate_cfg"),),
            (max_vaults_per_epoch, epoch_length_ledgers),
        );
        Ok(())
    }

    /// Rotate the admin key. Admin only.
    ///
    /// `admin` must be the address recorded at initialization and must authorize
    /// the call; any other address fails with [`Error::Unauthorized`].
    ///
    /// Rotating the admin does not touch per-manager counters or vault records,
    /// so a key rotation cannot be used to reset (or lose) a manager's quota.
    pub fn set_admin(env: Env, admin: Address, new_admin: Address) -> Result<(), Error> {
        require_admin(&env, &admin)?;

        env.storage().instance().set(&DataKey::Admin, &new_admin);
        env.events()
            .publish((symbol_short!("set_admin"),), new_admin);
        Ok(())
    }

    // ── Vault provisioning ───────────────────────────────────────────────────

    /// Provision a new vault for `manager`, subject to the per-manager rate
    /// limit.
    ///
    /// `manager` must authorize the call. The call fails with
    /// [`Error::RateLimitExceeded`] when the manager has already created
    /// `max_vaults_per_epoch` vaults in the current epoch, and with
    /// [`Error::VaultAlreadyExists`] when this `(manager, vault_id)` pair was
    /// registered before.
    ///
    /// The duplicate check runs *before* the quota is consumed, so a rejected
    /// retry does not spend the manager's budget. On success the manager's
    /// per-epoch counter and lifetime total are both incremented and the vault
    /// record is returned.
    pub fn create_vault(env: Env, manager: Address, vault_id: u32) -> Result<VaultRecord, Error> {
        manager.require_auth();

        let config = load_config(&env)?;
        let ledger = env.ledger().sequence();
        let epoch = epoch_for(&env, config.epoch_length_ledgers);

        let state_key = DataKey::ManagerState(manager.clone());
        let mut state = load_manager_state(&env, &manager, epoch);

        if state.count >= config.max_vaults_per_epoch {
            return Err(Error::RateLimitExceeded);
        }

        let vault_key = DataKey::Vault(manager.clone(), vault_id);
        if env.storage().persistent().has(&vault_key) {
            return Err(Error::VaultAlreadyExists);
        }

        let record = VaultRecord {
            manager: manager.clone(),
            vault_id,
            created_ledger: ledger,
            epoch,
        };
        env.storage().persistent().set(&vault_key, &record);
        env.storage()
            .persistent()
            .extend_ttl(&vault_key, TTL_LEDGERS, TTL_LEDGERS);

        state.count += 1;
        state.total_vaults = state.total_vaults.checked_add(1).ok_or(Error::Overflow)?;
        env.storage().persistent().set(&state_key, &state);
        env.storage()
            .persistent()
            .extend_ttl(&state_key, TTL_LEDGERS, TTL_LEDGERS);

        env.events().publish(
            (symbol_short!("vlt_new"), manager, vault_id),
            (ledger, epoch, state.count),
        );

        Ok(record)
    }

    // ── Views ────────────────────────────────────────────────────────────────

    /// Returns the current admin address.
    pub fn get_admin(env: Env) -> Result<Address, Error> {
        load_admin(&env)
    }

    /// Returns the active rate-limit configuration.
    pub fn get_config(env: Env) -> Result<RateLimitConfig, Error> {
        load_config(&env)
    }

    /// Returns the epoch index for the current ledger.
    pub fn current_epoch(env: Env) -> Result<u32, Error> {
        let config = load_config(&env)?;
        Ok(epoch_for(&env, config.epoch_length_ledgers))
    }

    /// Returns the number of vaults `manager` has created in the current epoch.
    pub fn vaults_this_epoch(env: Env, manager: Address) -> Result<u32, Error> {
        let config = load_config(&env)?;
        let epoch = epoch_for(&env, config.epoch_length_ledgers);
        Ok(load_manager_state(&env, &manager, epoch).count)
    }

    /// Returns the total number of vaults `manager` has ever created.
    pub fn total_vaults(env: Env, manager: Address) -> Result<u32, Error> {
        let config = load_config(&env)?;
        let epoch = epoch_for(&env, config.epoch_length_ledgers);
        Ok(load_manager_state(&env, &manager, epoch).total_vaults)
    }

    /// Returns how many more vaults `manager` may create in the current epoch.
    ///
    /// Returns `0` once the quota is exhausted; the value resets to
    /// `max_vaults_per_epoch` after the epoch boundary.
    pub fn remaining_quota(env: Env, manager: Address) -> Result<u32, Error> {
        let config = load_config(&env)?;
        let epoch = epoch_for(&env, config.epoch_length_ledgers);
        let state = load_manager_state(&env, &manager, epoch);
        Ok(config.max_vaults_per_epoch.saturating_sub(state.count))
    }

    /// Returns the record for `(manager, vault_id)`, if it exists.
    pub fn get_vault(env: Env, manager: Address, vault_id: u32) -> Option<VaultRecord> {
        env.storage()
            .persistent()
            .get(&DataKey::Vault(manager, vault_id))
    }
}
