#![no_std]
//! # Perigee-guards
//!
//! Canonical, reusable **circuit-breaker core** for Soroban contracts
//! ([CONTRACT-5](#499)).
//!
//! A `guard` is a small, storage-backed policy layer that any contract can
//! embed in its *own* instance storage:
//!
//! - a granular **pause-state bitmask** ([`PauseType`]) that lets a contract
//!   halt individual operations (swap, deposit, withdraw, transfer, mint,
//!   burn, stake, claim rewards, …) without freezing the whole contract;
//! - a **multi-signature admin committee** with a configurable threshold that
//!   governs emergency pauses, resumes, and admin rotation; and
//! - **typed events** for every state change so indexers, UIs, and audit
//!   trails can follow guard activity.
//!
//! The [`emergency_guard`](../emergency_guard) contract is a thin entry-point
//! layer on top of this crate. Any other contract can depend on this crate and
//! drive [`Guard`]'s associated functions directly against its own `Env`
//! storage.
//!
//! ## Storage layout
//!
//! All state lives in the caller's instance storage under [`GuardDataKey`].
//! The layout is byte-for-byte identical to the one the previous
//! `emergency_guard` crate wrote on chain, so adopting this crate is a
//! zero-migration change:
//!
//! | Key                  | Type             | Meaning                      |
//! |----------------------|------------------|------------------------------|
//! | `PauseState`         | `PauseType(u32)` | bitmask of paused operations |
//! | `Admins`             | `Vec<Address>`   | admin committee              |
//! | `SignatureThreshold` | `u32`            | required multi-sig count     |
//!
//! ## Multi-signature semantics
//!
//! [`Guard::validate_multi_sig`] returns `Ok` only when `approvers` contains at
//! least `threshold` **distinct** valid admins, each of which has provided an
//! invocation authorization. Duplicate addresses are counted once. A non-admin
//! approver short-circuits with [`GuardError::Unauthorized`]; a short count
//! (after de-duplication) fails with [`GuardError::InsufficientSignatures`].

use soroban_sdk::{contracterror, contracttype, Address, Env, String, Vec};

// ── Pause type ───────────────────────────────────────────────────────────────

/// Granular pause types using bitmask for efficient storage.
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PauseType(u32);

impl PauseType {
    pub const SWAP: u32 = 1 << 0;
    pub const DEPOSIT: u32 = 1 << 1;
    pub const WITHDRAW: u32 = 1 << 2;
    pub const TRANSFER: u32 = 1 << 3;
    pub const MINT: u32 = 1 << 4;
    pub const BURN: u32 = 1 << 5;
    pub const CREATE_PAIR: u32 = 1 << 6;
    /// Pause staking operations.
    pub const STAKE: u32 = 1 << 7;
    /// Pause reward claims on the staking rewards contract.
    pub const CLAIM_REWARDS: u32 = 1 << 8;

    pub fn new(value: u32) -> Self {
        PauseType(value)
    }

    /// Returns `true` if the `operation` bit is set in the pause bitmask.
    /// `#[inline(always)]` ensures this reduces to a single AND + comparison
    /// instruction at the call site, minimising gas on every guard check.
    #[inline(always)]
    pub fn is_paused(&self, operation: u32) -> bool {
        (self.0 & operation) != 0
    }

    #[inline(always)]
    pub fn set_paused(&mut self, operation: u32, paused: bool) {
        if paused {
            self.0 |= operation;
        } else {
            self.0 &= !operation;
        }
    }

    pub fn pause_all(&mut self) {
        self.0 = u32::MAX;
    }

    pub fn unpause_all(&mut self) {
        self.0 = 0;
    }

    pub fn as_u32(self) -> u32 {
        self.0
    }
}

// ── Storage keys ─────────────────────────────────────────────────────────────

/// Data keys for guard storage. Any host contract embedding guard state uses
/// these keys in its own instance storage, so the layout is shared.
#[contracttype]
pub enum GuardDataKey {
    PauseState,
    Admins,
    SignatureThreshold,
}

// ── Errors ───────────────────────────────────────────────────────────────────

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u32)]
pub enum GuardError {
    /// Reentrant call detected while a non-reentrant section is active (#671).
    Reentrancy = 20,
    NotInitialized = 0,
    Unauthorized = 1,
    Paused = 2,
    InsufficientSignatures = 3,
    InvalidThreshold = 4,
    AdminNotFound = 5,
    AlreadyInitialized = 6,
}

// ── Events ───────────────────────────────────────────────────────────────────

/// Standardized event actions emitted by every successful guard action.
#[contracttype]
#[derive(Clone, Copy, Debug, Eq, PartialEq, PartialOrd, Ord)]
pub enum EmergencyGuardAction {
    Initialized,
    PauseSet,
    EmergencyPause,
    Resume,
    AdminAdded,
    AdminRemoved,
    AdminRotated,
}

/// Standardized event payload for EmergencyGuard administrative actions.
#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyGuardEvent {
    pub action: EmergencyGuardAction,
    pub admin: Option<Address>,
    pub operation: u32,
    pub paused: bool,
    pub threshold: u32,
    pub admin_count: u32,
    pub approver_count: u32,
}

fn action_topic(env: &Env, action: EmergencyGuardAction) -> String {
    match action {
        EmergencyGuardAction::Initialized => String::from_str(env, "initialized"),
        EmergencyGuardAction::PauseSet => String::from_str(env, "pause_set"),
        EmergencyGuardAction::EmergencyPause => String::from_str(env, "emergency_pause"),
        EmergencyGuardAction::Resume => String::from_str(env, "resume"),
        EmergencyGuardAction::AdminAdded => String::from_str(env, "admin_added"),
        EmergencyGuardAction::AdminRemoved => String::from_str(env, "admin_removed"),
        EmergencyGuardAction::AdminRotated => String::from_str(env, "admin_rotated"),
    }
}

fn emit_guard_event(env: &Env, event: EmergencyGuardEvent) {
    env.events().publish(
        (
            String::from_str(env, "EmergencyGuard"),
            action_topic(env, event.action),
        ),
        event,
    );
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardInitializedEvent {
    pub admins: Vec<Address>,
    pub threshold: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseStateChangedEvent {
    pub admin: Address,
    pub operation: u32,
    pub paused: bool,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyPausedEvent {
    pub approvers: Vec<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumedEvent {
    pub approvers: Vec<Address>,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminAddedEvent {
    pub approvers: Vec<Address>,
    pub new_admin: Address,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRemovedEvent {
    pub approvers: Vec<Address>,
    pub admin: Address,
}

const EVENT_INIT_GUARD: &str = "emergency_guard_initialized";
const EVENT_SET_PAUSE: &str = "emergency_guard_pause_state_changed";
const EVENT_EMERGENCY_PAUSE_ALL: &str = "emergency_guard_emergency_paused_all";
const EVENT_RESUME_ALL: &str = "emergency_guard_resumed_all";
const EVENT_ADD_ADMIN: &str = "emergency_guard_admin_added";
const EVENT_REMOVE_ADMIN: &str = "emergency_guard_admin_removed";

pub fn emit_guard_initialized(e: &Env, admins: &Vec<Address>, threshold: u32) {
    e.events().publish(
        (String::from_str(e, EVENT_INIT_GUARD),),
        GuardInitializedEvent {
            admins: admins.clone(),
            threshold,
        },
    );
}

pub fn emit_pause_state_changed(e: &Env, admin: &Address, operation: u32, paused: bool) {
    e.events().publish(
        (String::from_str(e, EVENT_SET_PAUSE), admin.clone()),
        PauseStateChangedEvent {
            admin: admin.clone(),
            operation,
            paused,
        },
    );
}

pub fn emit_emergency_paused_all(e: &Env, approvers: &Vec<Address>) {
    e.events().publish(
        (String::from_str(e, EVENT_EMERGENCY_PAUSE_ALL),),
        EmergencyPausedEvent {
            approvers: approvers.clone(),
        },
    );
}

pub fn emit_resumed_all(e: &Env, approvers: &Vec<Address>) {
    e.events().publish(
        (String::from_str(e, EVENT_RESUME_ALL),),
        ResumedEvent {
            approvers: approvers.clone(),
        },
    );
}

pub fn emit_admin_added(e: &Env, approvers: &Vec<Address>, new_admin: &Address) {
    e.events().publish(
        (String::from_str(e, EVENT_ADD_ADMIN), new_admin.clone()),
        AdminAddedEvent {
            approvers: approvers.clone(),
            new_admin: new_admin.clone(),
        },
    );
}

pub fn emit_admin_removed(e: &Env, approvers: &Vec<Address>, admin: &Address) {
    e.events().publish(
        (String::from_str(e, EVENT_REMOVE_ADMIN), admin.clone()),
        AdminRemovedEvent {
            approvers: approvers.clone(),
            admin: admin.clone(),
        },
    );
}

// ── Core ─────────────────────────────────────────────────────────────────────

/// Canonical guard core.
///
/// All methods are associated functions operating on the caller's instance
/// storage (keyed by [`GuardDataKey`]); no contract registration is required,
/// which is what lets any contract embed a guard without hosting a separate
/// contract.
pub struct Guard;

impl Guard {
    // ── pause state ──

    /// Single storage read of the pause bitmask (defaults to "nothing paused").
    #[inline(always)]
    fn read_state(env: &Env) -> PauseType {
        env.storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0))
    }

    /// Gas-optimized pause probe: one storage read + inline bitwise AND.
    #[inline(always)]
    pub fn is_paused(env: &Env, operation: u32) -> bool {
        Self::read_state(env).is_paused(operation)
    }

    /// The raw pause-state bitmask.
    pub fn get_pause_state(env: &Env) -> u32 {
        Self::read_state(env).as_u32()
    }

    /// Returns `Err(GuardError::Paused)` when the operation bit is set.
    pub fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError> {
        if Self::is_paused(env, operation) {
            Err(GuardError::Paused)
        } else {
            Ok(())
        }
    }

    /// Panics when the requested operation bit is set in the pause bitmask.
    #[inline(always)]
    pub fn ensure_not_paused(env: &Env, operation: u32) {
        if Self::is_paused(env, operation) {
            panic!("operation paused");
        }
    }

    /// Set pause state for a specific operation.
    ///
    /// This is the unauthenticated core operation matching the historical
    /// `EmergencyGuardTrait::set_pause_state` contract: host contracts that
    /// call it are expected to perform their own authorization at their entry
    /// boundary. For a fully authenticated path use
    /// [`Guard::set_pause_by_admin`].
    pub fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        let mut state = Self::read_state(env);
        state.set_paused(operation, paused);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &state);
        Ok(())
    }

    /// Set pause state for a specific operation, acting as `admin`.
    ///
    /// Uses `admin` explicitly: `admin.require_auth()` plus a membership check
    /// against the committee. This mirrors the standalone contract's `set_pause`
    /// entry.
    pub fn set_pause_by_admin(
        env: &Env,
        admin: &Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), GuardError> {
        admin.require_auth();
        if !Self::is_admin(env, admin) {
            return Err(GuardError::Unauthorized);
        }
        let mut state = Self::read_state(env);
        state.set_paused(operation, paused);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &state);
        emit_pause_state_changed(env, admin, operation, paused);
        Ok(())
    }

    /// Emergency pause all operations (requires multi-sig approval).
    pub fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::validate_multi_sig(env, &approvers)?;
        let mut state = PauseType::new(0);
        state.pause_all();
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &state);
        emit_guard_event(
            env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::EmergencyPause,
                admin: None,
                operation: u32::MAX,
                paused: true,
                threshold: Self::get_threshold(env),
                admin_count: Self::get_admins(env).len(),
                approver_count: approvers.len(),
            },
        );
        emit_emergency_paused_all(env, &approvers);
        Ok(())
    }

    /// Resume all operations (requires multi-sig approval).
    pub fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::validate_multi_sig(env, &approvers)?;
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &PauseType::new(0));
        emit_guard_event(
            env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::Resume,
                admin: None,
                operation: u32::MAX,
                paused: false,
                threshold: Self::get_threshold(env),
                admin_count: Self::get_admins(env).len(),
                approver_count: approvers.len(),
            },
        );
        emit_resumed_all(env, &approvers);
        Ok(())
    }

    /// Unpause a single operation (unauthenticated core, matching the base
    /// `EmergencyGuardTrait::unpause`; hosts authorize at their entry boundary).
    pub fn unpause(env: &Env, operation: u32) -> Result<(), GuardError> {
        Self::set_pause_state(env, operation, false)
    }

    /// Clear the entire pause bitmask (unauthenticated core, matching the base
    /// `EmergencyGuardTrait::unpause_all`; hosts authorize at their entry
    /// boundary).
    pub fn unpause_all(env: &Env) -> Result<(), GuardError> {
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &PauseType::new(0));
        Ok(())
    }

    // ── initialization / admin committee ──

    /// Initialize the guard with a committee and required threshold.
    pub fn init_guard(
        env: &Env,
        admins: Vec<Address>,
        threshold: u32,
    ) -> Result<(), GuardError> {
        if env.storage().instance().has(&GuardDataKey::Admins) {
            return Err(GuardError::AlreadyInitialized);
        }
        if threshold == 0 || threshold > admins.len() {
            return Err(GuardError::InvalidThreshold);
        }
        env.storage()
            .instance()
            .set(&GuardDataKey::Admins, &admins);
        env.storage()
            .instance()
            .set(&GuardDataKey::SignatureThreshold, &threshold);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &PauseType::new(0));
        emit_guard_initialized(env, &admins, threshold);
        Ok(())
    }

    /// The current admin committee.
    pub fn get_admins(env: &Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&GuardDataKey::Admins)
            .unwrap_or_else(|| Vec::new(env))
    }

    /// The required signature threshold.
    pub fn get_threshold(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&GuardDataKey::SignatureThreshold)
            .unwrap_or(0)
    }

    /// Whether `addr` is a member of the admin committee.
    pub fn is_admin(env: &Env, addr: &Address) -> bool {
        let admins = Self::get_admins(env);
        admins.iter().any(|a| a == *addr)
    }

    /// Validate a multi-sig approval attempt against the stored threshold.
    ///
    /// `approvers` must contain at least `threshold` **distinct** valid admins,
    /// each having provided its invocation authorization. Duplicate addresses
    /// count once; a non-admin approver short-circuits with `Unauthorized`.
    pub fn validate_multi_sig(env: &Env, approvers: &Vec<Address>) -> Result<(), GuardError> {
        let threshold: u32 = env
            .storage()
            .instance()
            .get(&GuardDataKey::SignatureThreshold)
            .ok_or(GuardError::NotInitialized)?;

        if approvers.len() < threshold {
            return Err(GuardError::InsufficientSignatures);
        }

        let mut valid = 0u32;
        let mut seen = Vec::new(env);
        for addr in approvers.iter() {
            if seen.iter().any(|a| a == addr) {
                continue;
            }
            seen.push_back(addr.clone());
            if Self::is_admin(env, &addr) {
                addr.require_auth();
                valid += 1;
            } else {
                return Err(GuardError::Unauthorized);
            }
        }

        if valid < threshold {
            Err(GuardError::InsufficientSignatures)
        } else {
            Ok(())
        }
    }

    /// Add a new admin (multi-sig required). Idempotent for existing admins.
    pub fn add_admin(
        env: &Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        Self::validate_multi_sig(env, &approvers)?;
        let mut admins = Self::get_admins(env);
        if !admins.iter().any(|a| a == new_admin) {
            admins.push_back(new_admin.clone());
            env.storage().instance().set(&GuardDataKey::Admins, &admins);
            emit_admin_added(env, &approvers, &new_admin);
        }
        Ok(())
    }

    /// Remove an admin (multi-sig required). Never drops the committee below
    /// the threshold.
    pub fn remove_admin(
        env: &Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError> {
        Self::validate_multi_sig(env, &approvers)?;
        let admins = Self::get_admins(env);
        let threshold = Self::get_threshold(env);
        if admins.len() <= threshold {
            return Err(GuardError::InvalidThreshold);
        }
        let mut new_admins = Vec::new(env);
        let mut found = false;
        for a in admins.iter() {
            if a != admin {
                new_admins.push_back(a);
            } else {
                found = true;
            }
        }
        if !found {
            return Err(GuardError::AdminNotFound);
        }
        env.storage()
            .instance()
            .set(&GuardDataKey::Admins, &new_admins);
        emit_admin_removed(env, &approvers, &admin);
        Ok(())
    }

    /// Rotate `old_admin` out and `new_admin` in (multi-sig required).
    ///
    /// When `new_admin` is already a member the rotation behaves as a removal
    /// of `old_admin` (no duplicate entries are introduced).
    pub fn rotate_admin(
        env: &Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        Self::validate_multi_sig(env, &approvers)?;
        let admins = Self::get_admins(env);

        let mut new_admins = Vec::new(env);
        let mut found = false;
        for a in admins.iter() {
            if a == old_admin {
                found = true;
            } else if a != new_admin {
                new_admins.push_back(a);
            }
        }
        if !found {
            return Err(GuardError::AdminNotFound);
        }

        new_admins.push_back(new_admin.clone());

        let threshold = Self::get_threshold(env);
        if new_admins.len() < threshold {
            return Err(GuardError::InvalidThreshold);
        }

        env.storage()
            .instance()
            .set(&GuardDataKey::Admins, &new_admins);
        emit_guard_event(
            env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::AdminRotated,
                admin: Some(new_admin.clone()),
                operation: 0,
                paused: false,
                threshold,
                admin_count: new_admins.len(),
                approver_count: approvers.len(),
            },
        );
        Ok(())
    }
}

// ── Host-embedding surface ───────────────────────────────────────────────────

/// Standard guard surface for host contracts embedding guard storage.
pub trait EmergencyGuardTrait {
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError>;
    fn get_pause_state(env: &Env) -> u32;
    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError>;
    fn unpause(env: &Env, operation: u32) -> Result<(), GuardError>;
    fn unpause_all(env: &Env) -> Result<(), GuardError>;
    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError>;
    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError>;
    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError>;
    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError>;
    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError>;
    fn rotate_admin(
        env: &Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError>;
    fn get_admins(env: &Env) -> Vec<Address>;
    fn get_threshold(env: &Env) -> u32;
    fn is_admin(env: &Env, addr: &Address) -> bool;
}

/// Default implementation of [`EmergencyGuardTrait`] over [`Guard`]'s canonical
/// core.
///
/// Each body calls the inherent [`Guard`] method of the same name; inherent
/// methods take precedence over the trait method being defined, so these calls
/// resolve to the core (no recursion).
impl EmergencyGuardTrait for Guard {
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError> {
        Guard::check_not_paused(env, operation)
    }

    fn get_pause_state(env: &Env) -> u32 {
        Guard::get_pause_state(env)
    }

    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        Guard::set_pause_state(env, operation, paused)
    }

    fn unpause(env: &Env, operation: u32) -> Result<(), GuardError> {
        Guard::unpause(env, operation)
    }

    fn unpause_all(env: &Env) -> Result<(), GuardError> {
        Guard::unpause_all(env)
    }

    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Guard::emergency_pause_all(env, approvers)
    }

    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Guard::resume_all(env, approvers)
    }

    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        Guard::init_guard(env, admins, threshold)
    }

    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError> {
        Guard::add_admin(env, approvers, new_admin)
    }

    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        Guard::remove_admin(env, approvers, admin)
    }

    fn rotate_admin(
        env: &Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        Guard::rotate_admin(env, approvers, old_admin, new_admin)
    }

    fn get_admins(env: &Env) -> Vec<Address> {
        Guard::get_admins(env)
    }

    fn get_threshold(env: &Env) -> u32 {
        Guard::get_threshold(env)
    }

    fn is_admin(env: &Env, addr: &Address) -> bool {
        Guard::is_admin(env, addr)
    }
}

/// Standard guard surface for host contracts embedding `EmergencyGuard`
/// storage with `Env`-by-value entry points.
pub trait TokenEmergencyGuardTrait {
    fn guard_pause(e: Env, admin: Address, operation: u32, paused: bool) -> Result<(), GuardError>;
    fn guard_unpause(e: Env, approvers: Vec<Address>) -> Result<(), GuardError>;
    fn guard_is_paused(e: Env, operation: u32) -> bool;
    fn emergency_pause_all(e: Env, approvers: Vec<Address>) -> Result<(), GuardError>;
    fn resume_all(e: Env, approvers: Vec<Address>) -> Result<(), GuardError>;
    fn guard_add_admin(
        e: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError>;
    fn guard_remove_admin(
        e: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError>;
    fn guard_rotate_admin(
        e: Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError>;
    fn guard_admins(e: Env) -> Vec<Address>;
    fn guard_threshold(e: Env) -> u32;
    fn guard_pause_state(e: Env) -> u32;
}

#[cfg(test)]
mod test;

// ── Reentrancy protection (#671 / contract.md § Security & Auditing) ─────────

/// Instance-storage key for the reentrancy lock bit.
/// Host contracts share this key namespace when they embed `Guard` storage;
/// prefer namespaced keys (e.g. via `DataKey::ReentrancyLock`) in contracts
/// that own their storage layout.
#[contracttype]
#[derive(Clone)]
pub enum ReentrancyDataKey {
    Locked,
}

/// Cross-function / cross-callback reentrancy guard for Soroban contracts.
///
/// Usage:
/// ```ignore
/// ReentrancyGuard::enter(&env)?;
/// // ... external calls / state mutations ...
/// ReentrancyGuard::exit(&env);
/// ```
///
/// `enter` fails with [`GuardError::Reentrancy`] if a prior `enter` has not
/// been paired with `exit`. Call `exit` in every success path before return;
/// prefer structuring entry points so `exit` always runs (no early return
/// between enter and exit without exit).
pub struct ReentrancyGuard;

impl ReentrancyGuard {
    /// Acquire the lock. Returns `Err(Reentrancy)` if already held.
    pub fn enter(env: &Env) -> Result<(), GuardError> {
        let key = ReentrancyDataKey::Locked;
        if env.storage().instance().has(&key) {
            let locked: bool = env.storage().instance().get(&key).unwrap_or(false);
            if locked {
                return Err(GuardError::Reentrancy);
            }
        }
        env.storage().instance().set(&key, &true);
        Ok(())
    }

    /// Release the lock. Idempotent if already unlocked.
    pub fn exit(env: &Env) {
        let key = ReentrancyDataKey::Locked;
        env.storage().instance().set(&key, &false);
    }

    /// Whether the lock is currently held.
    pub fn is_entered(env: &Env) -> bool {
        let key = ReentrancyDataKey::Locked;
        env.storage()
            .instance()
            .get(&key)
            .unwrap_or(false)
    }
}

/// RAII-style helper: acquires on `new`, releases on `drop` (when Drop is
/// available). On Soroban, prefer explicit `enter`/`exit` around external
/// calls because panic-unwinding Drop is not the primary control flow.
pub struct NonReentrant<'a> {
    env: &'a Env,
    active: bool,
}

impl<'a> NonReentrant<'a> {
    pub fn new(env: &'a Env) -> Result<Self, GuardError> {
        ReentrancyGuard::enter(env)?;
        Ok(Self { env, active: true })
    }

    /// Explicit early release (e.g. before returning Ok from an entry point).
    pub fn disarm(mut self) {
        if self.active {
            ReentrancyGuard::exit(self.env);
            self.active = false;
        }
    }
}


#[cfg(test)]
mod test;
