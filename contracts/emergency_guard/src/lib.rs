#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, log, Address, Env, String, Vec,
};
#[cfg(feature = "contract")]
use soroban_sdk::{contract, contractimpl};
use soroban_sdk::{contracterror, contracttype, Address, Env, String, Vec};

/// Granular pause types using bitmask for efficient storage
/// Each bit represents a different pausable operation
#[contracttype]
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PauseType(u32);

impl PauseType {
    /// Pause swap operations
    pub const SWAP: u32 = 1 << 0;
    /// Pause deposit operations
    pub const DEPOSIT: u32 = 1 << 1;
    /// Pause withdraw operations
    pub const WITHDRAW: u32 = 1 << 2;
    /// Pause all token transfers
    pub const TRANSFER: u32 = 1 << 3;
    /// Pause minting
    pub const MINT: u32 = 1 << 4;
    /// Pause burning
    pub const BURN: u32 = 1 << 5;
    /// Pause liquidity-pool pair creation in factory contracts
    /// Pause factory pair creation
    pub const CREATE_PAIR: u32 = 1 << 6;

    pub fn new(value: u32) -> Self {
        PauseType(value)
    }

    pub fn is_paused(&self, operation: u32) -> bool {
        (self.0 & operation) != 0
    }

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

/// Data keys for emergency guard storage (named uniquely to avoid WASM metadata collisions)
#[contracttype]
pub enum GuardDataKey {
    /// Pause state bitmask: PauseType(u32)
    PauseState,
    /// List of authorized admins: Vec<Address>
    Admins,
    /// Number of signatures required for multi-sig: u32
    SignatureThreshold,
}

/// Error codes
#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u32)]
pub enum GuardError {
    NotInitialized = 0,
    Unauthorized = 1,
    Paused = 2,
    InsufficientSignatures = 3,
    InvalidThreshold = 4,
    AdminNotFound = 5,
    AlreadyInitialized = 6,
}

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
/// Result type for guard operations
// Result type for guard operations replaced inline

/// EmergencyGuard trait for standardized pause and admin management
pub trait EmergencyGuardTrait {
    /// Check if an operation is paused. Returns Err if paused.
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError>;

    /// Get current pause state
    fn get_pause_state(env: &Env) -> u32;

    /// Set pause state for a specific operation (any single admin can do this)
    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError>;

    /// Unpause a specific operation (any single admin can do this)
    fn unpause(env: &Env, operation: u32) -> Result<(), GuardError>;

    /// Unpause all operations (any single admin can do this)
    fn unpause_all(env: &Env) -> Result<(), GuardError>;

    /// Emergency pause all operations (requires multi-sig approval)
    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError>;

    /// Resume all operations (requires multi-sig approval)
    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError>;

    /// Initialize emergency guard with admins and threshold
    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError>;

    /// Add new admin (multi-sig required)
    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError>;

    /// Remove admin (multi-sig required)
    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError>;

    /// Rotate admin (multi-sig required)
    fn rotate_admin(env: &Env, approvers: Vec<Address>, old_admin: Address, new_admin: Address) -> Result<(), GuardError>;

    /// Get list of current admins
    fn get_admins(env: &Env) -> Vec<Address>;

    /// Get required signature threshold
    fn get_threshold(env: &Env) -> u32;

    /// Check if address is an admin
    fn is_admin(env: &Env, addr: &Address) -> bool;
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

#[contract]
#[cfg_attr(feature = "contract", contract)]
pub struct EmergencyGuard;

#[cfg_attr(feature = "contract", contractimpl)]
impl EmergencyGuard {
    /// Initialize the emergency guard with a list of admins and required threshold
    pub fn initialize(env: Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        if env.storage().instance().has(&GuardDataKey::Admins) {
            return Err(GuardError::AlreadyInitialized);
        }

        // Verify threshold is valid
        if threshold == 0 || threshold > admins.len() {
            return Err(GuardError::InvalidThreshold);
        }

        // Store admins
        env.storage().instance().set(&GuardDataKey::Admins, &admins);

        // Store threshold
        env.storage()
            .instance()
            .set(&GuardDataKey::SignatureThreshold, &threshold);

        // Initialize pause state to 0 (nothing paused)
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &PauseType::new(0));

        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::Initialized,
                admin: None,
                operation: 0,
                paused: false,
                threshold,
                admin_count: admins.len(),
                approver_count: 0,
            },
        );
        emit_guard_initialized(&env, &admins, threshold);

        Ok(())
    }

    /// Returns the raw pause-state bitmask.
    pub fn get_pause_state(env: Env) -> u32 {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        pause_state.as_u32()
    }

    /// Check if an operation is paused
    pub fn is_paused(env: Env, operation: u32) -> bool {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        pause_state.is_paused(operation)
    }

    /// Set pause state for a specific operation (any single admin can do this)
    pub fn set_pause(
        env: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), GuardError> {
        admin.require_auth();

        // Check if caller is admin
        if !Self::is_admin_internal(&env, &admin) {
            return Err(GuardError::Unauthorized);
        }

        let mut pause_state: PauseType = env
            .storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        pause_state.set_paused(operation, paused);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);

        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::PauseSet,
                admin: Some(admin),
                operation,
                paused,
                threshold: Self::get_threshold(env.clone()),
                admin_count: Self::get_admins(env.clone()).len(),
                approver_count: 1,
            },
        );
        log!(
            &env,
            "Pause state updated: op={}, paused={}",
            operation,
            paused
        emit_pause_state_changed(&env, &admin, operation, paused);
        // Emit standardized EmergencyGuard event
        env.events().publish(
            (
                String::from_str(&env, "emergency_guard.set_pause"),
                admin.clone(),
            ),
            (operation, paused),
        );
        Ok(())
    }

    /// Emergency pause all operations (requires multi-sig approval)
    pub fn emergency_pause(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let mut pause_state = PauseType::new(0);
        pause_state.pause_all();

        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);

        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::EmergencyPause,
                admin: None,
                operation: u32::MAX,
                paused: true,
                threshold: Self::get_threshold(env.clone()),
                admin_count: Self::get_admins(env.clone()).len(),
                approver_count: approvers.len(),
            },
        emit_emergency_paused_all(&env, &approvers);
        env.events().publish(
            (String::from_str(
                &env,
                "emergency_guard.emergency_pause_all",
            ),),
            (approvers.clone(),),
        );
        log!(&env, "Emergency pause all activated");
        Ok(())
    }

    /// Resume all operations (requires multi-sig approval)
    pub fn resume(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let pause_state = PauseType::new(0);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);

        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::Resume,
                admin: None,
                operation: u32::MAX,
                paused: false,
                threshold: Self::get_threshold(env.clone()),
                admin_count: Self::get_admins(env.clone()).len(),
                approver_count: approvers.len(),
            },
        emit_resumed_all(&env, &approvers);
        env.events().publish(
            (String::from_str(&env, "emergency_guard.resume_all"),),
            (approvers.clone(),),
        );
        log!(&env, "Resume all activated");
        Ok(())
    }

    /// Add new admin (multi-sig required)
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
            emit_guard_event(
                &env,
                EmergencyGuardEvent {
                    action: EmergencyGuardAction::AdminAdded,
                    admin: Some(new_admin.clone()),
                    operation: 0,
                    paused: false,
                    threshold: Self::get_threshold(env.clone()),
                    admin_count: admins.len(),
                    approver_count: approvers.len(),
                },
            emit_admin_added(&env, &approvers, &new_admin);
            env.storage().instance().set(&GuardDataKey::Admins, &admins);
            env.events().publish(
                (
                    String::from_str(&env, "emergency_guard.admin_added"),
                    new_admin.clone(),
                ),
                (),
            );
            log!(&env, "Admin added: {}", new_admin);
        }

        Ok(())
    }

    /// Remove admin (multi-sig required)
    pub fn remove_admin(
        env: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let admins = Self::get_admins(env.clone());
        let threshold = Self::get_threshold(env.clone());

        if admins.len() <= threshold {
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
        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::AdminRemoved,
                admin: Some(admin.clone()),
                operation: 0,
                paused: false,
                threshold,
                admin_count: new_admins.len(),
                approver_count: approvers.len(),
            },
        emit_admin_removed(&env, &approvers, &admin);
        env.storage().instance().set(&GuardDataKey::Admins, &new_admins);
        env.events().publish(
            (
                String::from_str(&env, "emergency_guard.admin_removed"),
                admin.clone(),
            ),
            (),
        );
        log!(&env, "Admin removed: {}", admin);
        Ok(())
    }

    /// Rotate admin (multi-sig required)
    pub fn rotate_admin(
        env: Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)?;

        let mut admins = Self::get_admins(env.clone());
        let threshold = Self::get_threshold(env.clone());

        let mut found = false;
        let mut new_admins = soroban_sdk::Vec::new(&env);
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

        if (new_admins.len() as u32) < threshold {
            return Err(GuardError::InvalidThreshold);
        }

        env.storage().instance().set(&DataKey::Admins, &new_admins);
        emit_guard_event(
            &env,
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
        log!(&env, "Admin rotated: {} to {}", old_admin, new_admin);
        Ok(())
    }

    /// Get list of current admins
    pub fn get_admins(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&GuardDataKey::Admins)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Get required signature threshold
    pub fn get_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&GuardDataKey::SignatureThreshold)
            .unwrap_or(0)
    }

    /// Authorize administration operations by verifying the caller approvers against the guard threshold.
    pub fn authorize(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)
    }

    // Internal helpers

    fn is_admin_internal(env: &Env, addr: &Address) -> bool {
        let admins: Vec<Address> = env
            .storage()
            .instance()
            .get(&GuardDataKey::Admins)
            .unwrap_or_else(|| Vec::new(env));

        admins.iter().any(|a| a == *addr)
    }

    fn check_multi_sig(env: &Env, approvers: &Vec<Address>) -> Result<(), GuardError> {
        let threshold = env
            .storage()
            .instance()
            .get(&GuardDataKey::SignatureThreshold)
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

    /// Public method to check if an address is an admin
    pub fn is_admin_public(env: Env, addr: Address) -> bool {
        Self::is_admin_internal(&env, &addr)
    }
}

/// Default implementation of EmergencyGuardTrait using static methods
pub struct DefaultEmergencyGuard;

impl EmergencyGuardTrait for DefaultEmergencyGuard {
    /// Check if an operation is paused. Returns Err if paused.
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError> {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        if pause_state.is_paused(operation) {
            Err(GuardError::Paused)
        } else {
            Ok(())
        }
    }

    /// Get current pause state
    fn get_pause_state(env: &Env) -> u32 {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        pause_state.0
    }

    /// Set pause state for a specific operation (any single admin can do this)
    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        let mut pause_state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        pause_state.set_paused(operation, paused);
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        log!(
            env,
            "Pause state updated: op={}, paused={}",
            operation,
            paused
        );
        Ok(())
    }

    /// Unpause a specific operation (any single admin can do this)
    fn unpause(env: &Env, operation: u32) -> Result<(), GuardError> {
        Self::set_pause_state(env, operation, false)
    }

    /// Unpause all operations (any single admin can do this)
    fn unpause_all(env: &Env) -> Result<(), GuardError> {
        let pause_state = PauseType::new(0);
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        log!(env, "All operations unpaused");
        Ok(())
    }

    /// Emergency pause all operations (requires multi-sig approval)
    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(env, &approvers)?;

        let mut pause_state = PauseType::new(0);
        pause_state.pause_all();

        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        log!(env, "Emergency pause all activated");
        Ok(())
    }

    /// Resume all operations (unpause all) - requires multi-sig approval
    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(env, &approvers)?;

        let pause_state = PauseType::new(0);
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        log!(env, "All operations resumed (unpaused)");
        Ok(())
    }

    /// Initialize emergency guard with admins and threshold
    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
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

    /// Add new admin (multi-sig required)
    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(env, &approvers)?;

        let mut admins = Self::get_admins(env);
        if !admins.iter().any(|a| a == new_admin) {
            admins.push_back(new_admin.clone());
            env.storage().instance().set(&DataKey::Admins, &admins);
            log!(env, "Admin added: {}", new_admin);
        }

        Ok(())
    }

    /// Remove admin (multi-sig required)
    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(env, &approvers)?;

        let admins = Self::get_admins(env);
        let threshold = Self::get_threshold(env);

        if admins.len() as u32 <= threshold {
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

        env.storage().instance().set(&DataKey::Admins, &new_admins);
        log!(env, "Admin removed: {}", admin);
        Ok(())
    }

    /// Get list of current admins
    fn get_admins(env: &Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::Admins)
            .unwrap_or_else(|| Vec::new(env))
    }

    /// Get required signature threshold
    fn get_threshold(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SignatureThreshold)
            .unwrap_or(0)
    }

    /// Check if address is an admin
    fn is_admin(env: &Env, addr: &Address) -> bool {
        let admins: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Admins)
            .unwrap_or_else(|| Vec::new(env));

        admins.iter().any(|a| a == *addr)
    }
}

/// Extension methods for unpause operations
impl DefaultEmergencyGuard {
    /// Unpause a specific operation (uses set_pause_state internally)
    pub fn unpause(env: &Env, operation: u32) -> Result<(), GuardError> {
        Self::set_pause_state(env, operation, false)
    }

    /// Unpause all operations (single-admin helper; same effect as `unpause_all`)
    pub fn unpause_all_emergency(env: &Env) -> Result<(), GuardError> {
        let pause_state = PauseType::new(0);
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        log!(env, "All operations unpaused");
        Ok(())
    }

    /// Check if a specific operation is paused
    pub fn is_operation_paused(env: &Env, operation: u32) -> bool {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        pause_state.is_paused(operation)
    }

    /// Pause a specific operation
    pub fn pause(env: &Env, operation: u32) -> Result<(), GuardError> {
        Self::set_pause_state(env, operation, true)
    /// Public wrapper to validate a set of approvers against the stored threshold.
    pub fn validate_multi_sig(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers)
    }
}
