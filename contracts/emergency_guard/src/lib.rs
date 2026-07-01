#![no_std]
#![allow(missing_docs)]

//! Emergency guard contract utilities for pausing operations and managing admin approvals.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, log, Address, Env, String, Vec};

/// Bitmask-based pause state for emergency guard operations.
///
/// Each bit in the stored value represents a distinct pausable operation.
#[allow(missing_docs)]
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PauseType(u32);

impl PauseType {
    /// Pauses swap operations when set in the bitmask.
    pub const SWAP: u32 = 1 << 0;
    /// Pauses deposit operations when set in the bitmask.
    pub const DEPOSIT: u32 = 1 << 1;
    /// Pauses withdraw operations when set in the bitmask.
    pub const WITHDRAW: u32 = 1 << 2;
    /// Pauses token transfers when set in the bitmask.
    pub const TRANSFER: u32 = 1 << 3;
    /// Pauses minting when set in the bitmask.
    pub const MINT: u32 = 1 << 4;
    /// Pauses burning when set in the bitmask.
    pub const BURN: u32 = 1 << 5;

    /// Creates a new pause state from a raw bitmask value.
    pub fn new(value: u32) -> Self {
        PauseType(value)
    }

    /// Returns whether the provided operation bit is currently paused.
    pub fn is_paused(&self, operation: u32) -> bool {
        (self.0 & operation) != 0
    }

    /// Updates the pause state for a specific operation.
    pub fn set_paused(&mut self, operation: u32, paused: bool) {
        if paused {
            self.0 |= operation;
        } else {
            self.0 &= !operation;
        }
    }

    /// Pauses every supported operation by setting all bits.
    pub fn pause_all(&mut self) {
        self.0 = u32::MAX;
    }

    /// Clears the pause state and resumes all operations.
    pub fn unpause_all(&mut self) {
        self.0 = 0;
    }
}

/// Storage keys used by the emergency guard contract.
#[allow(missing_docs)]
#[contracttype]
pub enum DataKey {
    /// Pause state bitmask stored as a `PauseType`.
    PauseState,
    /// Authorized admin list stored as a vector of addresses.
    Admins,
    /// Threshold of signatures required for multi-signature actions.
    SignatureThreshold,
}

/// Error codes returned by the emergency guard contract.
#[allow(missing_docs)]
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u32)]
pub enum GuardError {
    /// The contract has not been initialized yet.
    NotInitialized = 0,
    /// The caller is not authorized to perform the requested action.
    Unauthorized = 1,
    /// The requested operation is currently paused.
    Paused = 2,
    /// The provided approver set does not meet the required threshold.
    InsufficientSignatures = 3,
    /// The provided signature threshold is invalid.
    InvalidThreshold = 4,
    /// The requested admin was not found.
    AdminNotFound = 5,
    /// The contract has already been initialized.
    AlreadyInitialized = 6,
}

/// Standard interface for pause and admin management operations.
pub trait EmergencyGuardTrait {
    /// Checks whether an operation is paused and returns an error if it is.
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError>;

    /// Returns the current pause state bitmask.
    fn get_pause_state(env: &Env) -> u32;

    /// Sets the pause state for one operation.
    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError>;

    /// Stops all operations using a multi-signature emergency pause.
    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError>;

    /// Resumes all paused operations after multi-signature approval.
    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError>;

    /// Initializes the contract with a set of admins and a signature threshold.
    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError>;

    /// Adds a new admin after multi-signature approval.
    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError>;

    /// Removes an existing admin after multi-signature approval.
    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError>;

    /// Returns the current list of admins.
    fn get_admins(env: &Env) -> Vec<Address>;

    /// Returns the current signature threshold.
    fn get_threshold(env: &Env) -> u32;

    /// Returns whether the provided address is an admin.
    fn is_admin(env: &Env, addr: &Address) -> bool;
}

/// Contract entry points for the emergency guard module.
#[allow(missing_docs)]
#[contract]
pub struct EmergencyGuard;

#[allow(missing_docs)]
#[contractimpl]
impl EmergencyGuard {
    /// Initializes the emergency guard with a list of admins and a required threshold.
    pub fn initialize(env: Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        if env.storage().instance().has(&DataKey::Admins) {
            return Err(GuardError::AlreadyInitialized);
        }

        // Verify threshold is valid
        if threshold == 0 || threshold > admins.len() as u32 {
            return Err(GuardError::InvalidThreshold);
        }

        // Store admins
        env.storage().instance().set(&DataKey::Admins, &admins);

        // Store threshold
        env.storage()
            .instance()
            .set(&DataKey::SignatureThreshold, &threshold);

        // Initialize pause state to 0 (nothing paused)
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &PauseType::new(0));

        Ok(())
    }

    /// Returns whether the provided operation is currently paused.
    pub fn is_paused(env: Env, operation: u32) -> bool {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        pause_state.is_paused(operation)
    }

    /// Sets the pause state for a specific operation, requiring the caller to be an admin.
    pub fn set_pause(env: Env, admin: Address, operation: u32, paused: bool) -> Result<(), GuardError> {
        admin.require_auth();

        // Check if caller is admin
        if !Self::is_admin_internal(&env, &admin) {
            return Err(GuardError::Unauthorized);
        }

        let mut pause_state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        pause_state.set_paused(operation, paused);
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        // Emit standardized EmergencyGuard event
        env.events().publish(
            (String::from_str(&env, "emergency_guard.set_pause"), admin.clone()),
            (operation, paused),
        );
        Ok(())
    }

    /// Pauses all supported operations after the required multi-signature approval.
    pub fn emergency_pause(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let mut pause_state = PauseType::new(0);
        pause_state.pause_all();

        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        env.events().publish(
            (String::from_str(&env, "emergency_guard.emergency_pause_all"),),
            (approvers.clone(),),
        );
        Ok(())
    }

    /// Resumes all operations after the required multi-signature approval.
    pub fn resume(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let pause_state = PauseType::new(0);
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        env.events().publish(
            (String::from_str(&env, "emergency_guard.resume_all"),),
            (approvers.clone(),),
        );
        Ok(())
    }

    /// Adds a new admin after the required multi-signature approval.
    pub fn add_admin(env: Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let mut admins = Self::get_admins(env.clone());
        if !admins.iter().any(|a| a == new_admin) {
            admins.push_back(new_admin.clone());
            env.storage().instance().set(&DataKey::Admins, &admins);
            env.events().publish(
                (String::from_str(&env, "emergency_guard.admin_added"), new_admin.clone()),
                (),
            );
        }

        Ok(())
    }

    /// Removes an existing admin after the required multi-signature approval.
    pub fn remove_admin(env: Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let admins = Self::get_admins(env.clone());
        let threshold = Self::get_threshold(env.clone());

        if admins.len() as u32 <= threshold {
            return Err(GuardError::InvalidThreshold);
        }

        let mut new_admins = Vec::new(&env);
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

        env.storage().instance().set(&DataKey::Admins, &new_admins);
        env.events().publish(
            (String::from_str(&env, "emergency_guard.admin_removed"), admin.clone()),
            (),
        );
        Ok(())
    }

    /// Returns the list of current admins.
    pub fn get_admins(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::Admins)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Returns the required signature threshold.
    pub fn get_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SignatureThreshold)
            .unwrap_or(0)
    }

    // Internal helpers

    fn is_admin_internal(env: &Env, addr: &Address) -> bool {
        let admins: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Admins)
            .unwrap_or_else(|| Vec::new(env));

        admins.iter().any(|a| a == *addr)
    }

    fn check_multi_sig(env: &Env, approvers: &Vec<Address>) -> Result<(), GuardError> {
        let threshold = env
            .storage()
            .instance()
            .get(&DataKey::SignatureThreshold)
            .ok_or(GuardError::NotInitialized)?;

        if approvers.len() < threshold {
            return Err(GuardError::InsufficientSignatures);
        }

        let mut valid_approvers = 0;
        let mut seen = Vec::new(env);

        for addr in approvers.iter() {
            // Avoid duplicates
            if seen.iter().any(|a| a == addr) {
                continue;
            }
            seen.push_back(addr.clone());

            // Check if address is an admin
            if Self::is_admin_internal(env, &addr) {
                addr.require_auth();
                valid_approvers += 1;
            }
        }

        if valid_approvers < threshold {
            Err(GuardError::InsufficientSignatures)
        } else {
            Ok(())
        }
    }
}
