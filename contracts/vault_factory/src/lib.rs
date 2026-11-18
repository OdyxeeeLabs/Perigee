#![no_std]

//! # Perigee Vault Factory — deterministic clone deployment and upgrades
//!
//! Two closely-related deployment concerns live in this crate:
//!
//! - **#632 — clone factory.** [`VaultFactory::create_vault`] deploys a
//!   [`VaultProxy`] per vault at a **deterministic address**, derived from a
//!   caller-supplied seed, so a vault's address can be known before it exists
//!   and recomputed by anyone.
//! - **#635 — upgradeability.** [`VaultProxy`] is a UUPS-style proxy: the
//!   implementation pointer lives in the proxy's own storage and is changed by
//!   [`VaultProxy::upgrade_to`], gated on a named upgrader and expected to match
//!   the caller's own view of the current implementation. Previous
//!   implementations are retained so [`VaultProxy::rollback_to`] can revert
//!   inside a bounded window.
//!
//! ## What EIP-1167 does and does not transfer to Soroban
//!
//! EIP-1167 is a 45-byte minimal proxy: the point is **one deployed artifact,
//! many instances, cheap per-instance creation**. Soroban has no EVM bytecode to
//! embed, so the literal technique does not port. Two of the three properties do,
//! and this implementation gets both of them by different means:
//!
//! | EIP-1167 property | How it is achieved here |
//! | --- | --- |
//! | One artifact, many instances | Every vault is a [`VaultProxy`] running the same tiny WASM; no per-vault custom code exists. |
//! | Cheap creation | The proxy stores four values and never deploys anything. |
//! | Deterministic address (EIP-1167 + CREATE2) | `env.deployer().with_current_contract(salt).deploy_v2(..)`, with `salt = sha256(owner, seed)`. |
//!
//! What does **not** port is transparent call delegation. An EVM minimal proxy
//! forwards *every* call; a Soroban proxy cannot, because the host resolves the
//! target contract from the invocation. So a [`VaultProxy`] exposes its own
//! explicit methods rather than proxying an implementation's method set. The
//! upgrade pointer therefore governs the **vault configuration the proxy
//! consults**, not the bytecode executing the call — and the proxy's own WASM is
//! fixed for the life of the instance.
//!
//! That is a real difference and it is why [`VaultProxy`] is small: it holds the
//! upgrade-relevant state and nothing else, and an upgraded implementation reads
//! that state through [`VaultProxy::implementation`].
//!
//! ## Storage gap
//!
//! [`STORAGE_GAP_SLOTS`] instance-storage slots are reserved in the
//! [`ProxyConfig`] layout — the key-slot analogue of an EVM `uint256[N] __gap`.
//! The reservation is **layout-only**: no ledger entry is written for a gap slot,
//! so it costs no rent, but a future version can claim `ProxyConfig` fields in the
//! reserved region without moving the existing ones, which is what keeps a layout
//! change from reinterpreting live entries.

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, symbol_short, xdr::ToXdr, Address, BytesN,
    Env, IntoVal, Symbol, Vec,
};

#[cfg(test)]
mod test;

// ── Constants ────────────────────────────────────────────────────────────────

/// Instance-storage slots reserved in [`ProxyConfig`] for future upgrades.
///
/// Layout-only: nothing is written here today, and [`VaultProxy::reserved_slots`]
/// reports the reservation so tooling can assert it was not consumed by accident.
pub const STORAGE_GAP_SLOTS: u32 = 16;

/// Version of the proxy storage layout. Bumped when a [`ProxyConfig`] field is
/// added, removed or reordered.
pub const PROXY_STORAGE_VERSION: u32 = 1;

/// Ledgers a rollback window stays open after an upgrade.
///
/// 17 280 ledgers ≈ 24 h at the Stellar target of one ledger every 5 s. Bounded
/// on purpose: rollback is for "we shipped a bad upgrade", not for "we can pick
/// any past implementation forever".
pub const ROLLBACK_WINDOW_LEDGERS: u32 = 17_280;

/// Ledgers to live for persistent records after each write.
pub const TTL_LEDGERS: u32 = 17_280;

// ── Errors ───────────────────────────────────────────────────────────────────

/// Errors shared by the factory and the proxy.
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
    /// The vault at the computed address already exists.
    VaultExists = 4,
    /// No proxy has been registered at the supplied index or seed.
    VaultNotFound = 5,
    /// `expected_implementation` did not match the stored one, so the caller is
    /// acting on stale state and the change is refused.
    StaleImplementation = 6,
    /// The implementation is already the active one.
    SameImplementation = 7,
    /// A rollback was attempted after the window closed, or to an
    /// implementation that is not a recorded predecessor.
    RollbackUnavailable = 8,
    /// No implementation has been recorded as a rollback target.
    NoRollbackTarget = 9,
    /// The seed produced an address that is already taken.
    DeterministicAddressCollision = 10,
    /// The requested proxy WASM hash is not on the admin allowlist.
    ImplementationNotApproved = 11,
    /// A value would have overflowed its storage width.
    Overflow = 12,
}

// ── Types ────────────────────────────────────────────────────────────────────

/// Everything a [`VaultProxy`] stores.
///
/// Field order is part of the storage layout. Append new fields inside the gap
/// region and bump [`PROXY_STORAGE_VERSION`]; inserting or reordering an existing
/// field reinterprets live entries and requires a migration.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ProxyConfig {
    /// Layout version stamped at initialization.
    pub storage_version: u32,
    /// Address permitted to call `upgrade_to` / `rollback_to`.
    pub upgrader: Address,
    /// Currently active implementation.
    pub implementation: Address,
    /// Implementation to revert to, if an upgrade has happened.
    pub previous_implementation: Option<Address>,
    /// Ledger at which the last upgrade happened.
    pub last_upgrade_ledger: u32,
    /// Whether `previous_implementation` is still inside its rollback window.
    pub rollback_available: bool,
}

/// A vault deployed by the factory.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct VaultRecord {
    /// The proxy's address.
    pub address: Address,
    /// Owner entitled to manage the vault.
    pub owner: Address,
    /// Implementation the proxy was created with.
    pub implementation: Address,
    /// Ledger at creation.
    pub created_ledger: u32,
}

/// Storage keys for the proxy.
#[contracttype]
#[derive(Clone)]
pub enum ProxyKey {
    /// The single [`ProxyConfig`] record. Its presence marks the proxy as
    /// initialized, which is what makes double-initialization detectable.
    Config,
    /// Reserved for future upgrades; see [`STORAGE_GAP_SLOTS`].
    Gap(u32),
}

/// Storage keys for the factory.
#[contracttype]
#[derive(Clone)]
pub enum FactoryKey {
    /// Admin allowed to rotate the default implementation.
    Admin,
    /// Implementation used when `create_vault` is not given one.
    DefaultImplementation,
    /// Number of vaults created.
    VaultCount,
    /// [`VaultRecord`] at a given index.
    Vault(u32),
    /// Seed of the vault at a given index, for recomputation.
    VaultSeed(u32),
    /// Admin allowlist of proxy WASM hashes approved for deployment.
    ApprovedWasm(BytesN<32>),
}

// ── Proxy ────────────────────────────────────────────────────────────────────

/// A vault instance.
///
/// Holds the upgrade-relevant configuration and delegates nothing — see the
/// module documentation for why transparent delegation does not port to Soroban.
#[contract]
pub struct VaultProxy;

#[contractimpl]
impl VaultProxy {
    /// Initialize the proxy. Callable exactly once per instance.
    pub fn initialize(env: Env, owner: Address, implementation: Address) -> Result<(), Error> {
        if env.storage().instance().has(&ProxyKey::Config) {
            return Err(Error::AlreadyInitialized);
        }
        owner.require_auth();

        env.storage().instance().set(
            &ProxyKey::Config,
            &ProxyConfig {
                storage_version: PROXY_STORAGE_VERSION,
                upgrader: owner,
                implementation,
                previous_implementation: None,
                last_upgrade_ledger: env.ledger().sequence(),
                rollback_available: false,
            },
        );

        env.events().publish(
            (symbol_short!("init"),),
            (implementation, PROXY_STORAGE_VERSION),
        );
        Ok(())
    }

    /// Point the proxy at a new implementation (UUPS-style).
    ///
    /// # Upgrade authorization
    ///
    /// Two independent conditions, both required:
    ///
    /// 1. `caller` must equal the recorded `upgrader`, **and** must authorize the
    ///    call. Passing the upgrader's address is not sufficient.
    /// 2. `expected_implementation` must equal the currently stored
    ///    implementation. This is the upgrade equivalent of an optimistic
    ///    concurrency guard: if the caller is acting on a stale view — because
    ///    someone upgraded between them reading state and submitting — the
    ///    change is refused rather than silently clobbering theirs.
    ///
    /// # Rollback
    ///
    /// The outgoing implementation is retained in `previous_implementation` and
    /// [`VaultProxy::rollback_to`] can revert to it for
    /// [`ROLLBACK_WINDOW_LEDGERS`] after this call. The window is bounded on
    /// purpose: rollback is for "we shipped a bad upgrade", not for "any past
    /// implementation is selectable forever".
    pub fn upgrade_to(
        env: Env,
        caller: Address,
        new_implementation: Address,
        expected_implementation: Address,
    ) -> Result<(), Error> {
        let mut config = load_proxy(&env)?;

        if caller != config.upgrader {
            return Err(Error::Unauthorized);
        }
        caller.require_auth();

        if config.implementation != expected_implementation {
            return Err(Error::StaleImplementation);
        }
        if config.implementation == new_implementation {
            return Err(Error::SameImplementation);
        }

        config.previous_implementation = Some(config.implementation.clone());
        config.implementation = new_implementation.clone();
        config.last_upgrade_ledger = env.ledger().sequence();
        config.rollback_available = true;
        env.storage().instance().set(&ProxyKey::Config, &config);

        env.events().publish(
            (symbol_short!("upgrade"), new_implementation),
            (caller, config.previous_implementation),
        );
        Ok(())
    }

    /// Revert to the implementation that was active before the last upgrade.
    ///
    /// Refused once [`ROLLBACK_WINDOW_LEDGERS`] has elapsed, or when there is no
    /// recorded predecessor. `expected_implementation` carries the same
    /// anti-stale-write guarantee as [`VaultProxy::upgrade_to`].
    pub fn rollback_to(
        env: Env,
        caller: Address,
        expected_implementation: Address,
    ) -> Result<Address, Error> {
        let mut config = load_proxy(&env)?;

        if caller != config.upgrader {
            return Err(Error::Unauthorized);
        }
        caller.require_auth();

        if config.implementation != expected_implementation {
            return Err(Error::StaleImplementation);
        }
        if !config.rollback_available {
            return Err(Error::NoRollbackTarget);
        }

        let elapsed = env.ledger().sequence().saturating_sub(config.last_upgrade_ledger);
        if elapsed > ROLLBACK_WINDOW_LEDGERS {
            return Err(Error::RollbackUnavailable);
        }

        let target = config
            .previous_implementation
            .ok_or(Error::RollbackUnavailable)?;

        config.previous_implementation = Some(config.implementation.clone());
        config.implementation = target.clone();
        config.last_upgrade_ledger = env.ledger().sequence();
        config.rollback_available = false;
        env.storage().instance().set(&ProxyKey::Config, &config);

        env.events()
            .publish((symbol_short!("rollback"), target.clone()), caller);
        Ok(target)
    }

    /// Extend the rollback window by re-affirming the current implementation.
    ///
    /// The only legitimate use is deliberately rolling forward again after a
    /// rollback, which resets the window for the *new* upgrade.
    pub fn extend_rollback_window(
        env: Env,
        caller: Address,
        expected_implementation: Address,
    ) -> Result<(), Error> {
        let mut config = load_proxy(&env)?;

        if caller != config.upgrader {
            return Err(Error::Unauthorized);
        }
        caller.require_auth();

        if config.implementation != expected_implementation {
            return Err(Error::StaleImplementation);
        }

        config.last_upgrade_ledger = env.ledger().sequence();
        config.rollback_available = config.previous_implementation.is_some();
        env.storage().instance().set(&ProxyKey::Config, &config);

        Ok(())
    }

    /// The full proxy configuration.
    pub fn get_config(env: Env) -> Result<ProxyConfig, Error> {
        load_proxy(&env)
    }

    /// The currently active implementation.
    pub fn implementation(env: Env) -> Result<Address, Error> {
        Ok(load_proxy(&env)?.implementation)
    }

    /// The address permitted to upgrade.
    pub fn upgrader(env: Env) -> Result<Address, Error> {
        Ok(load_proxy(&env)?.upgrader)
    }

    /// Whether a rollback is currently possible.
    pub fn rollback_available(env: Env) -> bool {
        load_proxy(&env)
            .map(|config| config.rollback_available)
            .unwrap_or(false)
    }

    /// Ledgers remaining in the rollback window. Zero when none is open.
    pub fn rollback_ledgers_remaining(env: Env) -> Result<u32, Error> {
        let config = load_proxy(&env)?;
        if !config.rollback_available {
            return Ok(0);
        }
        let elapsed = env.ledger().sequence().saturating_sub(config.last_upgrade_ledger);
        Ok(ROLLBACK_WINDOW_LEDGERS.saturating_sub(elapsed))
    }

    /// How many storage slots are reserved for future upgrades.
    pub fn reserved_slots() -> u32 {
        STORAGE_GAP_SLOTS
    }

    /// The storage layout version this proxy was initialized with.
    pub fn storage_version(env: Env) -> Result<u32, Error> {
        Ok(load_proxy(&env)?.storage_version)
    }
}

fn load_proxy(env: &Env) -> Result<ProxyConfig, Error> {
    env.storage()
        .instance()
        .get(&ProxyKey::Config)
        .ok_or(Error::NotInitialized)
}

// ── Factory ──────────────────────────────────────────────────────────────────

/// Deploys [`VaultProxy`] instances at deterministic addresses.
#[contract]
pub struct VaultFactory;

#[contractimpl]
impl VaultFactory {
    /// Initialize the factory with an admin and a default implementation.
    pub fn initialize(env: Env, admin: Address, default_implementation: Address) -> Result<(), Error> {
        if env.storage().instance().has(&FactoryKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        admin.require_auth();

        env.storage().instance().set(&FactoryKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&FactoryKey::DefaultImplementation, &default_implementation);
        env.storage().instance().set(&FactoryKey::VaultCount, &0u32);

        env.events().publish(
            (symbol_short!("init"), admin),
            default_implementation,
        );
        Ok(())
    }

    /// Deploy a vault proxy for `owner`.
///
/// `seed` makes the address deterministic:
/// `salt = sha256(owner, seed, implementation)`, fed to
/// `env.deployer().with_current_contract(salt)`. The same triple therefore
/// always yields the same address, which is what lets an integrator address a
/// vault before it exists.
///
/// `proxy_wasm` must be a hash the admin has already approved with
/// [`VaultFactory::approve_implementation`]. The allowlist is the point: without
/// it, the factory would deploy whatever wasm the caller names, which turns it
/// into a deployer for arbitrary contracts.
///
/// # Errors
/// - [`Error::Unauthorized`] — the caller is not the configured admin.
/// - [`Error::ImplementationNotApproved`] — `proxy_wasm` is not on the allowlist.
/// - [`Error::DeterministicAddressCollision`] — the salt resolved to an address
///   that is already taken.
pub fn create_vault(
    env: Env,
    caller: Address,
    owner: Address,
    implementation: Address,
    proxy_wasm: BytesN<32>,
        seed: BytesN<32>,
    ) -> Result<Address, Error> {
        let admin = load_factory_admin(&env)?;
        if caller != admin {
            return Err(Error::Unauthorized);
        }
        caller.require_auth();

        if !is_approved(&env, &proxy_wasm) {
            return Err(Error::ImplementationNotApproved);
        }

        let salt = env.crypto().sha256(
            &(owner.clone(), seed.clone(), implementation.clone()).to_xdr(&env),
        );

        let proxy = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(proxy_wasm, Vec::<soroban_sdk::Val>::new(&env))
            .map_err(|_| Error::DeterministicAddressCollision)?;

        // Initialize the proxy immediately, so a registered vault is never
        // observable in an uninitialized state between deployment and a second
        // transaction.
        env.invoke_contract::<soroban_sdk::Val>(
            &proxy,
            &Symbol::new(&env, "initialize"),
            (owner.clone(), implementation.clone()).into_val(&env),
        )
        .map_err(|_| Error::DeterministicAddressCollision)?;

        let index = get_vault_count(&env);
        env.storage().instance().set(
            &FactoryKey::Vault(index),
            &VaultRecord {
                address: proxy.clone(),
                owner: owner.clone(),
                implementation: implementation.clone(),
                created_ledger: env.ledger().sequence(),
            },
        );
        env.storage()
            .instance()
            .set(&FactoryKey::VaultSeed(index), &salt);
        env.storage()
            .instance()
            .set(&FactoryKey::VaultCount, &index.checked_add(1).ok_or(Error::Overflow)?);

        env.events().publish(
            (symbol_short!("vlt_new"), proxy.clone()),
            (owner, implementation, index),
        );
        Ok(proxy)
    }

    /// The salt `create_vault` will use for a given triple.
    ///
    /// Exposed so an integrator can compute the address off-chain before the
    /// vault exists, rather than only learning the salt afterwards.
    pub fn vault_salt(env: Env, owner: Address, seed: BytesN<32>, implementation: Address) -> BytesN<32> {
        env.crypto()
            .sha256(&(owner, seed, implementation).to_xdr(&env))
    }

    /// The address `create_vault` will deploy to for `(owner, seed, implementation)`.
    ///
    /// Pure function of the factory address and the three arguments: it reads no
    /// storage and does not depend on the caller, the ledger, the admin, the
    /// default implementation or the approved-WASM list. `proxy_wasm` is
    /// deliberately not an input — the salt does not bind it.
    pub fn predict_vault_address(
        env: Env,
        owner: Address,
        seed: BytesN<32>,
        implementation: Address,
    ) -> Address {
        let salt = env
            .crypto()
            .sha256(&(owner, seed, implementation).to_xdr(&env));
        env.deployer().with_current_contract(salt).deployed_address()
    }

    /// How many vaults the factory has created.
    pub fn vault_count(env: Env) -> Result<u32, Error> {
        load_factory_admin(&env)?;
        Ok(get_vault_count(&env))
    }

    /// Approve a proxy WASM hash for deployment.
    ///
    /// Only an approved hash may be passed to [`VaultFactory::create_vault`].
    /// Without this the factory would deploy whatever the caller named, which
    /// makes it a deployer for arbitrary contracts.
    pub fn approve_implementation(
        env: Env,
        caller: Address,
        proxy_wasm: BytesN<32>,
    ) -> Result<(), Error> {
        let admin = load_factory_admin(&env)?;
        if caller != admin {
            return Err(Error::Unauthorized);
        }
        caller.require_auth();

        env.storage()
            .instance()
            .set(&FactoryKey::ApprovedWasm(proxy_wasm.clone()), &true);
        env.events()
            .publish((symbol_short!("approve"), proxy_wasm.clone()), caller);
        Ok(())
    }

    /// Revoke a previously approved WASM hash.
    pub fn revoke_implementation(
        env: Env,
        caller: Address,
        proxy_wasm: BytesN<32>,
    ) -> Result<(), Error> {
        let admin = load_factory_admin(&env)?;
        if caller != admin {
            return Err(Error::Unauthorized);
        }
        caller.require_auth();

        env.storage()
            .instance()
            .remove(&FactoryKey::ApprovedWasm(proxy_wasm.clone()));
        env.events()
            .publish((symbol_short!("revoke"), proxy_wasm), caller);
        Ok(())
    }

    /// Whether `proxy_wasm` is approved for deployment.
    pub fn is_approved(env: Env, proxy_wasm: BytesN<32>) -> bool {
        is_approved(&env, &proxy_wasm)
    }

    /// The record for the vault at `index`.
    pub fn get_vault(env: Env, index: u32) -> Result<VaultRecord, Error> {
        env.storage()
            .instance()
            .get(&FactoryKey::Vault(index))
            .ok_or(Error::VaultNotFound)
    }

    /// The seed recorded for the vault at `index`.
    pub fn get_vault_seed(env: Env, index: u32) -> Result<BytesN<32>, Error> {
        env.storage()
            .instance()
            .get(&FactoryKey::VaultSeed(index))
            .ok_or(Error::VaultNotFound)
    }

    /// The factory admin.
    pub fn admin(env: Env) -> Result<Address, Error> {
        load_factory_admin(&env)
    }

    /// The implementation used when none is supplied.
    pub fn default_implementation(env: Env) -> Result<Address, Error> {
        env.storage()
            .instance()
            .get(&FactoryKey::DefaultImplementation)
            .ok_or(Error::NotInitialized)
    }

    /// Rotate the default implementation. Existing vaults are untouched: their
    /// implementation lives in their own proxy.
    pub fn set_default_implementation(
        env: Env,
        caller: Address,
        implementation: Address,
    ) -> Result<(), Error> {
        let admin = load_factory_admin(&env)?;
        if caller != admin {
            return Err(Error::Unauthorized);
        }
        caller.require_auth();

        env.storage()
            .instance()
            .set(&FactoryKey::DefaultImplementation, &implementation);
        env.events()
            .publish((symbol_short!("set_impl"),), implementation);
        Ok(())
    }

    /// Rotate the factory admin.
    pub fn set_admin(env: Env, caller: Address, new_admin: Address) -> Result<(), Error> {
        let admin = load_factory_admin(&env)?;
        if caller != admin {
            return Err(Error::Unauthorized);
        }
        caller.require_auth();

        env.storage().instance().set(&FactoryKey::Admin, &new_admin);
        env.events()
            .publish((symbol_short!("set_admin"),), new_admin);
        Ok(())
    }
}

fn is_approved(env: &Env, proxy_wasm: &BytesN<32>) -> bool {
    env.storage()
        .instance()
        .get(&FactoryKey::ApprovedWasm(proxy_wasm.clone()))
        .unwrap_or(false)
}

fn load_factory_admin(env: &Env) -> Result<Address, Error> {
    env.storage()
        .instance()
        .get(&FactoryKey::Admin)
        .ok_or(Error::NotInitialized)
}

fn get_vault_count(env: &Env) -> u32 {
    env.storage()
        .instance()
        .get(&FactoryKey::VaultCount)
        .unwrap_or(0)
}