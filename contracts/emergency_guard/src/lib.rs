#![no_std]
#![allow(missing_docs)]

//! Emergency guard contract utilities for pausing operations and managing admin approvals.

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String, Vec};

/// Bitmask-based pause state for emergency guard operations.
///
/// Each bit in the stored value represents a distinct pausable operation.
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
    /// Pauses factory pair creation when set in the bitmask.
    pub const CREATE_PAIR: u32 = 1 << 6;

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

    /// Returns the raw pause-state bitmask value.
    pub fn as_u32(self) -> u32 {
        self.0
    }
}

/// Storage keys used by the emergency guard contract.
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
#[contract]
pub struct EmergencyGuard;

#[contractimpl]
impl EmergencyGuard {
    /// Initializes the emergency guard with a list of admins and a required threshold.
    pub fn initialize(env: Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        if env.storage().instance().has(&DataKey::Admins) {
            return Err(GuardError::AlreadyInitialized);
        }
        if threshold == 0 || threshold > admins.len() as u32 {
            return Err(GuardError::InvalidThreshold);
        }

        env.storage().instance().set(&DataKey::Admins, &admins);
        env.storage()
            .instance()
            .set(&DataKey::SignatureThreshold, &threshold);
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &PauseType::new(0));

        emit_guard_initialized(&env, &admins, threshold);
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

    /// Returns the raw pause-state bitmask.
    pub fn get_pause_state(env: Env) -> u32 {
        let state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        state.as_u32()
    }

    /// Sets the pause state for a specific operation, requiring the caller to be an admin.
    pub fn set_pause(
        env: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), GuardError> {
        admin.require_auth();

        if !Self::is_admin_internal(&env, &admin) {
            return Err(GuardError::Unauthorized);
        }

        let mut state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        state.set_paused(operation, paused);
        env.storage().instance().set(&DataKey::PauseState, &state);

        emit_pause_state_changed(&env, &admin, operation, paused);
        Ok(())
    }

    /// Emergency pauses all supported operations after the required multi-signature approval.
    pub fn emergency_pause(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let mut state = PauseType::new(0);
        state.pause_all();
        env.storage().instance().set(&DataKey::PauseState, &state);

        emit_emergency_paused_all(&env, &approvers);
        Ok(())
    }

    /// Resumes all operations after the required multi-signature approval.
    pub fn resume(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        env.storage()
            .instance()
            .set(&DataKey::PauseState, &PauseType::new(0));
        emit_resumed_all(&env, &approvers);
        Ok(())
    }

    /// Adds a new admin after the required multi-signature approval.
    pub fn add_admin(
        env: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let mut admins = Self::get_admins(env.clone());
        if !admins.iter().any(|a| a == new_admin) {
            admins.push_back(new_admin.clone());
            env.storage().instance().set(&DataKey::Admins, &admins);
            emit_admin_added(&env, &approvers, &new_admin);
        }

        Ok(())
    }

    /// Removes an existing admin after the required multi-signature approval.
    pub fn remove_admin(
        env: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError> {
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
        emit_admin_removed(&env, &approvers, &admin);
        Ok(())
    }

    /// Rotates the admin list by replacing one admin with another.
    pub fn rotate_admin(
        env: Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let admins = Self::get_admins(env.clone());
        let mut new_admins = Vec::new(&env);
        let mut found = false;
        for a in admins.iter() {
            if a == old_admin {
                found = true;
                if !new_admins.iter().any(|x| x == new_admin) {
                    new_admins.push_back(new_admin.clone());
                }
            } else if !new_admins.iter().any(|x| x == a) {
                new_admins.push_back(a);
            }
        }

        if !found {
            return Err(GuardError::AdminNotFound);
        }

        env.storage().instance().set(&DataKey::Admins, &new_admins);
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

    /// Returns whether a given address is an admin.
    pub fn is_admin_public(env: Env, addr: Address) -> bool {
        Self::is_admin_internal(&env, &addr)
    }

    /// Public wrapper to validate approvers against the stored threshold.
    pub fn validate_multi_sig(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)
    }

    /// Panics when the requested operation bit is set in the pause bitmask.
    pub fn ensure_not_paused(env: &Env, operation: u32) {
        if Self::is_paused(env.clone(), operation) {
            panic!("operation paused");
        }
    }

    fn is_admin_internal(env: &Env, addr: &Address) -> bool {
        let admins: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Admins)
            .unwrap_or_else(|| Vec::new(env));

        admins.iter().any(|a| a == *addr)
    }

    fn check_multi_sig(env: &Env, approvers: &Vec<Address>) -> Result<(), GuardError> {
        let threshold: u32 = env
            .storage()
            .instance()
            .get(&DataKey::SignatureThreshold)
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
            if Self::is_admin_internal(env, &addr) {
                addr.require_auth();
                valid += 1;
            }
        }

        if valid < threshold {
            Err(GuardError::InsufficientSignatures)
        } else {
            Ok(())
        }
    }
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

fn emit_guard_initialized(e: &Env, admins: &Vec<Address>, threshold: u32) {
    e.events().publish(
        (String::from_str(e, EVENT_INIT_GUARD),),
        GuardInitializedEvent {
            admins: admins.clone(),
            threshold,
        },
    );
}

fn emit_pause_state_changed(e: &Env, admin: &Address, operation: u32, paused: bool) {
    e.events().publish(
        (String::from_str(e, EVENT_SET_PAUSE), admin.clone()),
        PauseStateChangedEvent {
            admin: admin.clone(),
            operation,
            paused,
        },
    );
}

fn emit_emergency_paused_all(e: &Env, approvers: &Vec<Address>) {
    e.events().publish(
        (String::from_str(e, EVENT_EMERGENCY_PAUSE_ALL),),
        EmergencyPausedEvent {
            approvers: approvers.clone(),
        },
    );
}

fn emit_resumed_all(e: &Env, approvers: &Vec<Address>) {
    e.events().publish(
        (String::from_str(e, EVENT_RESUME_ALL),),
        ResumedEvent {
            approvers: approvers.clone(),
        },
    );
}

fn emit_admin_added(e: &Env, approvers: &Vec<Address>, new_admin: &Address) {
    e.events().publish(
        (String::from_str(e, EVENT_ADD_ADMIN), new_admin.clone()),
        AdminAddedEvent {
            approvers: approvers.clone(),
            new_admin: new_admin.clone(),
        },
    );
}

fn emit_admin_removed(e: &Env, approvers: &Vec<Address>, admin: &Address) {
    e.events().publish(
        (String::from_str(e, EVENT_REMOVE_ADMIN), admin.clone()),
        AdminRemovedEvent {
            approvers: approvers.clone(),
            admin: admin.clone(),
        },
    );
}

/// Default adapter for host contracts that embed the guard storage.
pub struct DefaultEmergencyGuard;

impl DefaultEmergencyGuard {
    /// Checks whether an operation is paused and returns an error if it is.
    pub fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError> {
        if EmergencyGuard::is_paused(env.clone(), operation) {
            Err(GuardError::Paused)
        } else {
            Ok(())
        }
    }

    /// Returns the current pause state bitmask.
    pub fn get_pause_state(env: &Env) -> u32 {
        EmergencyGuard::get_pause_state(env.clone())
    }

    /// Sets the pause state for one operation.
    pub fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        let admins = EmergencyGuard::get_admins(env.clone());
        if let Some(admin) = admins.get(0) {
            EmergencyGuard::set_pause(env.clone(), admin, operation, paused)
        } else {
            Err(GuardError::Unauthorized)
        }
    }

    /// Unpauses one operation.
    pub fn unpause(env: &Env, operation: u32) -> Result<(), GuardError> {
        let admins = EmergencyGuard::get_admins(env.clone());
        if let Some(admin) = admins.get(0) {
            EmergencyGuard::set_pause(env.clone(), admin, operation, false)
        } else {
            Err(GuardError::Unauthorized)
        }
    }

    /// Unpauses all operations.
    pub fn unpause_all(env: &Env) -> Result<(), GuardError> {
        let admins = EmergencyGuard::get_admins(env.clone());
        if let Some(admin) = admins.get(0) {
            EmergencyGuard::set_pause(env.clone(), admin, u32::MAX, false)
        } else {
            Err(GuardError::Unauthorized)
        }
    }

    /// Emergency pauses all operations after multi-signature approval.
    pub fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::emergency_pause(env.clone(), approvers)
    }

    /// Resumes all paused operations after multi-signature approval.
    pub fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::resume(env.clone(), approvers)
    }

    /// Initializes the guard with admins and a threshold.
    pub fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        EmergencyGuard::initialize(env.clone(), admins, threshold)
    }

    /// Adds a new admin after multi-signature approval.
    pub fn add_admin(
        env: &Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::add_admin(env.clone(), approvers, new_admin)
    }

    /// Removes an existing admin after multi-signature approval.
    pub fn remove_admin(
        env: &Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::remove_admin(env.clone(), approvers, admin)
    }

    /// Rotates admin roles after multi-signature approval.
    pub fn rotate_admin(
        env: &Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::rotate_admin(env.clone(), approvers, old_admin, new_admin)
    }

    /// Returns the list of current admins.
    pub fn get_admins(env: &Env) -> Vec<Address> {
        EmergencyGuard::get_admins(env.clone())
    }

    /// Returns the required signature threshold.
    pub fn get_threshold(env: &Env) -> u32 {
        EmergencyGuard::get_threshold(env.clone())
    }

    /// Returns whether the provided address is an admin.
    pub fn is_admin(env: &Env, addr: Address) -> bool {
        EmergencyGuard::is_admin_public(env.clone(), addr)
    }
}

#[cfg(test)]
mod test;
