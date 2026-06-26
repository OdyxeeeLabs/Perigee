#![no_std]

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, log, Address, Env, String, Vec,
};

/// Granular pause types using bitmask for efficient storage.
/// Each bit represents a different pausable operation.
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

/// Data keys for emergency guard storage.
#[contracttype]
pub enum GuardDataKey {
    /// Pause state bitmask: PauseType(u32)
    PauseState,
    /// List of authorized admins: Vec<Address>
    Admins,
    /// Number of signatures required for multi-sig: u32
    SignatureThreshold,
}

/// Error codes.
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
///
/// Fields:
/// - `admin_count`: total number of registered admins at the time of the event
/// - `approver_count`: number of addresses submitted in the approvers list
/// - `signatures_count`: number of valid unique admin signatures verified
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
    pub signatures_count: u32,
}

// ---------------------------------------------------------------------------
// Legacy per-action event structs — also enriched with admin_count and
// signatures_count so every emitted event carries the full context.
// ---------------------------------------------------------------------------

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GuardInitializedEvent {
    pub admins: Vec<Address>,
    pub threshold: u32,
    pub admin_count: u32,
    pub signatures_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PauseStateChangedEvent {
    pub admin: Address,
    pub operation: u32,
    pub paused: bool,
    pub admin_count: u32,
    pub signatures_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EmergencyPausedEvent {
    pub approvers: Vec<Address>,
    pub admin_count: u32,
    pub signatures_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ResumedEvent {
    pub approvers: Vec<Address>,
    pub admin_count: u32,
    pub signatures_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminAddedEvent {
    pub approvers: Vec<Address>,
    pub new_admin: Address,
    pub admin_count: u32,
    pub signatures_count: u32,
}

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AdminRemovedEvent {
    pub approvers: Vec<Address>,
    pub admin: Address,
    pub admin_count: u32,
    pub signatures_count: u32,
}

// ---------------------------------------------------------------------------
// Event constants
// ---------------------------------------------------------------------------

const EVENT_INIT_GUARD: &str = "emergency_guard_initialized";
const EVENT_SET_PAUSE: &str = "emergency_guard_pause_state_changed";
const EVENT_EMERGENCY_PAUSE_ALL: &str = "emergency_guard_emergency_paused_all";
const EVENT_RESUME_ALL: &str = "emergency_guard_resumed_all";
const EVENT_ADD_ADMIN: &str = "emergency_guard_admin_added";
const EVENT_REMOVE_ADMIN: &str = "emergency_guard_admin_removed";

// ---------------------------------------------------------------------------
// Emit helpers — unified EmergencyGuardEvent
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Emit helpers — legacy per-action events
// ---------------------------------------------------------------------------

pub fn emit_guard_initialized(
    e: &Env,
    admins: &Vec<Address>,
    threshold: u32,
    signatures_count: u32,
) {
    e.events().publish(
        (String::from_str(e, EVENT_INIT_GUARD),),
        GuardInitializedEvent {
            admins: admins.clone(),
            threshold,
            admin_count: admins.len(),
            signatures_count,
        },
    );
}

pub fn emit_pause_state_changed(
    e: &Env,
    admin: &Address,
    operation: u32,
    paused: bool,
    admin_count: u32,
    signatures_count: u32,
) {
    e.events().publish(
        (String::from_str(e, EVENT_SET_PAUSE), admin.clone()),
        PauseStateChangedEvent {
            admin: admin.clone(),
            operation,
            paused,
            admin_count,
            signatures_count,
        },
    );
}

pub fn emit_emergency_paused_all(
    e: &Env,
    approvers: &Vec<Address>,
    admin_count: u32,
    signatures_count: u32,
) {
    e.events().publish(
        (String::from_str(e, EVENT_EMERGENCY_PAUSE_ALL),),
        EmergencyPausedEvent {
            approvers: approvers.clone(),
            admin_count,
            signatures_count,
        },
    );
}

pub fn emit_resumed_all(
    e: &Env,
    approvers: &Vec<Address>,
    admin_count: u32,
    signatures_count: u32,
) {
    e.events().publish(
        (String::from_str(e, EVENT_RESUME_ALL),),
        ResumedEvent {
            approvers: approvers.clone(),
            admin_count,
            signatures_count,
        },
    );
}

pub fn emit_admin_added(
    e: &Env,
    approvers: &Vec<Address>,
    new_admin: &Address,
    admin_count: u32,
    signatures_count: u32,
) {
    e.events().publish(
        (String::from_str(e, EVENT_ADD_ADMIN), new_admin.clone()),
        AdminAddedEvent {
            approvers: approvers.clone(),
            new_admin: new_admin.clone(),
            admin_count,
            signatures_count,
        },
    );
}

pub fn emit_admin_removed(
    e: &Env,
    approvers: &Vec<Address>,
    admin: &Address,
    admin_count: u32,
    signatures_count: u32,
) {
    e.events().publish(
        (String::from_str(e, EVENT_REMOVE_ADMIN), admin.clone()),
        AdminRemovedEvent {
            approvers: approvers.clone(),
            admin: admin.clone(),
            admin_count,
            signatures_count,
        },
    );
}

// ---------------------------------------------------------------------------
// EmergencyGuardTrait
// ---------------------------------------------------------------------------

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

// ---------------------------------------------------------------------------
// Main contract
// ---------------------------------------------------------------------------

#[contract]
pub struct EmergencyGuard;

#[contractimpl]
impl EmergencyGuard {
    /// Initialize the emergency guard with a list of admins and required threshold.
    pub fn initialize(env: Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        if env.storage().instance().has(&GuardDataKey::Admins) {
            return Err(GuardError::AlreadyInitialized);
        }

        if threshold == 0 || threshold > admins.len() {
            return Err(GuardError::InvalidThreshold);
        }

        let admin_count = admins.len();

        env.storage().instance().set(&GuardDataKey::Admins, &admins);
        env.storage()
            .instance()
            .set(&GuardDataKey::SignatureThreshold, &threshold);
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
                admin_count,
                approver_count: 0,
                signatures_count: 0,
            },
        );
        emit_guard_initialized(&env, &admins, threshold, 0);

        Ok(())
    }

    /// Returns the raw pause-state bitmask.
    pub fn get_pause_state(env: Env) -> u32 {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        pause_state.as_u32()
    }

    /// Check if an operation is paused.
    pub fn is_paused(env: Env, operation: u32) -> bool {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        pause_state.is_paused(operation)
    }

    /// Set pause state for a specific operation (any single admin can do this).
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

        let mut pause_state: PauseType = env
            .storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        pause_state.set_paused(operation, paused);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);

        let admin_count = Self::get_admins(env.clone()).len();
        let threshold = Self::get_threshold(env.clone());

        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::PauseSet,
                admin: Some(admin.clone()),
                operation,
                paused,
                threshold,
                admin_count,
                approver_count: 1,
                signatures_count: 1,
            },
        );
        emit_pause_state_changed(&env, &admin, operation, paused, admin_count, 1);

        log!(&env, "Pause state updated: op={}, paused={}", operation, paused);
        Ok(())
    }

    /// Emergency pause all operations (requires multi-sig approval).
    pub fn emergency_pause(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        let signatures_count = Self::check_multi_sig(&env, &approvers)?;

        let mut pause_state = PauseType::new(0);
        pause_state.pause_all();
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);

        let admin_count = Self::get_admins(env.clone()).len();
        let threshold = Self::get_threshold(env.clone());

        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::EmergencyPause,
                admin: None,
                operation: u32::MAX,
                paused: true,
                threshold,
                admin_count,
                approver_count: approvers.len(),
                signatures_count,
            },
        );
        emit_emergency_paused_all(&env, &approvers, admin_count, signatures_count);

        log!(&env, "Emergency pause all activated");
        Ok(())
    }

    /// Resume all operations (requires multi-sig approval).
    pub fn resume(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        let signatures_count = Self::check_multi_sig(&env, &approvers)?;

        let pause_state = PauseType::new(0);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);

        let admin_count = Self::get_admins(env.clone()).len();
        let threshold = Self::get_threshold(env.clone());

        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::Resume,
                admin: None,
                operation: u32::MAX,
                paused: false,
                threshold,
                admin_count,
                approver_count: approvers.len(),
                signatures_count,
            },
        );
        emit_resumed_all(&env, &approvers, admin_count, signatures_count);

        log!(&env, "Resume all activated");
        Ok(())
    }

    /// Add new admin (multi-sig required).
    pub fn add_admin(
        env: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        let signatures_count = Self::check_multi_sig(&env, &approvers)?;

        let mut admins = Self::get_admins(env.clone());
        if !admins.iter().any(|a| a == new_admin) {
            admins.push_back(new_admin.clone());
            env.storage().instance().set(&GuardDataKey::Admins, &admins);

            let admin_count = admins.len();
            let threshold = Self::get_threshold(env.clone());

            emit_guard_event(
                &env,
                EmergencyGuardEvent {
                    action: EmergencyGuardAction::AdminAdded,
                    admin: Some(new_admin.clone()),
                    operation: 0,
                    paused: false,
                    threshold,
                    admin_count,
                    approver_count: approvers.len(),
                    signatures_count,
                },
            );
            emit_admin_added(&env, &approvers, &new_admin, admin_count, signatures_count);

            log!(&env, "Admin added: {}", new_admin);
        }

        Ok(())
    }

    /// Remove admin (multi-sig required).
    pub fn remove_admin(
        env: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError> {
        let signatures_count = Self::check_multi_sig(&env, &approvers)?;

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

        env.storage()
            .instance()
            .set(&GuardDataKey::Admins, &new_admins);

        let admin_count = new_admins.len();

        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::AdminRemoved,
                admin: Some(admin.clone()),
                operation: 0,
                paused: false,
                threshold,
                admin_count,
                approver_count: approvers.len(),
                signatures_count,
            },
        );
        emit_admin_removed(&env, &approvers, &admin, admin_count, signatures_count);

        log!(&env, "Admin removed: {}", admin);
        Ok(())
    }

    /// Rotate admin (multi-sig required).
    pub fn rotate_admin(
        env: Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        let signatures_count = Self::check_multi_sig(&env, &approvers)?;

        let admins = Self::get_admins(env.clone());
        let threshold = Self::get_threshold(env.clone());

        let mut new_admins = Vec::new(&env);
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

        if new_admins.len() < threshold {
            return Err(GuardError::InvalidThreshold);
        }

        env.storage()
            .instance()
            .set(&GuardDataKey::Admins, &new_admins);

        let admin_count = new_admins.len();

        emit_guard_event(
            &env,
            EmergencyGuardEvent {
                action: EmergencyGuardAction::AdminRotated,
                admin: Some(new_admin.clone()),
                operation: 0,
                paused: false,
                threshold,
                admin_count,
                approver_count: approvers.len(),
                signatures_count,
            },
        );

        log!(&env, "Admin rotated: {} to {}", old_admin, new_admin);
        Ok(())
    }

    /// Get list of current admins.
    pub fn get_admins(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&GuardDataKey::Admins)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Get required signature threshold.
    pub fn get_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&GuardDataKey::SignatureThreshold)
            .unwrap_or(0)
    }

    /// Public method to check if an address is an admin.
    pub fn is_admin_public(env: Env, addr: Address) -> bool {
        Self::is_admin_internal(&env, &addr)
    }

    /// Validate a set of approvers against the stored threshold.
    pub fn authorize(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        Self::check_multi_sig(&env, &approvers).map(|_| ())
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn is_admin_internal(env: &Env, addr: &Address) -> bool {
        let admins: Vec<Address> = env
            .storage()
            .instance()
            .get(&GuardDataKey::Admins)
            .unwrap_or_else(|| Vec::new(env));
        admins.iter().any(|a| a == *addr)
    }

    /// Verify multi-sig quorum and return the count of valid unique signatures.
    fn check_multi_sig(env: &Env, approvers: &Vec<Address>) -> Result<u32, GuardError> {
        let threshold: u32 = env
            .storage()
            .instance()
            .get(&GuardDataKey::SignatureThreshold)
            .ok_or(GuardError::NotInitialized)?;

        if approvers.len() < threshold {
            return Err(GuardError::InsufficientSignatures);
        }

        let mut valid_signatures: u32 = 0;
        let mut seen = Vec::new(env);

        for addr in approvers.iter() {
            if seen.iter().any(|a| a == addr) {
                continue;
            }
            seen.push_back(addr.clone());

            if Self::is_admin_internal(env, &addr) {
                addr.require_auth();
                valid_signatures += 1;
            }
        }

        if valid_signatures < threshold {
            Err(GuardError::InsufficientSignatures)
        } else {
            Ok(valid_signatures)
        }
    }
}

// ---------------------------------------------------------------------------
// DefaultEmergencyGuard — trait implementation for embedding in other contracts
// ---------------------------------------------------------------------------

pub struct DefaultEmergencyGuard;

impl EmergencyGuardTrait for DefaultEmergencyGuard {
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError> {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        if pause_state.is_paused(operation) {
            Err(GuardError::Paused)
        } else {
            Ok(())
        }
    }

    fn get_pause_state(env: &Env) -> u32 {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        pause_state.0
    }

    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        let mut pause_state: PauseType = env
            .storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        pause_state.set_paused(operation, paused);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);
        log!(env, "Pause state updated: op={}, paused={}", operation, paused);
        Ok(())
    }

    fn unpause(env: &Env, operation: u32) -> Result<(), GuardError> {
        Self::set_pause_state(env, operation, false)
    }

    fn unpause_all(env: &Env) -> Result<(), GuardError> {
        let pause_state = PauseType::new(0);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);
        log!(env, "All operations unpaused");
        Ok(())
    }

    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(env, &approvers)?;
        let mut pause_state = PauseType::new(0);
        pause_state.pause_all();
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);
        log!(env, "Emergency pause all activated");
        Ok(())
    }

    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(env, &approvers)?;
        let pause_state = PauseType::new(0);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);
        log!(env, "All operations resumed (unpaused)");
        Ok(())
    }

    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        if env.storage().instance().has(&GuardDataKey::Admins) {
            return Err(GuardError::AlreadyInitialized);
        }
        if threshold == 0 || threshold > admins.len() {
            return Err(GuardError::InvalidThreshold);
        }
        env.storage().instance().set(&GuardDataKey::Admins, &admins);
        env.storage()
            .instance()
            .set(&GuardDataKey::SignatureThreshold, &threshold);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &PauseType::new(0));
        Ok(())
    }

    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(env, &approvers)?;
        let mut admins = Self::get_admins(env);
        if !admins.iter().any(|a| a == new_admin) {
            admins.push_back(new_admin.clone());
            env.storage().instance().set(&GuardDataKey::Admins, &admins);
            log!(env, "Admin added: {}", new_admin);
        }
        Ok(())
    }

    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(env, &approvers)?;
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
        log!(env, "Admin removed: {}", admin);
        Ok(())
    }

    fn rotate_admin(
        env: &Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(env, &approvers)?;
        let admins = Self::get_admins(env);
        let threshold = Self::get_threshold(env);
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
        if new_admins.len() < threshold {
            return Err(GuardError::InvalidThreshold);
        }
        env.storage()
            .instance()
            .set(&GuardDataKey::Admins, &new_admins);
        log!(env, "Admin rotated: {} to {}", old_admin, new_admin);
        Ok(())
    }

    fn get_admins(env: &Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&GuardDataKey::Admins)
            .unwrap_or_else(|| Vec::new(env))
    }

    fn get_threshold(env: &Env) -> u32 {
        env.storage()
            .instance()
            .get(&GuardDataKey::SignatureThreshold)
            .unwrap_or(0)
    }

    fn is_admin(env: &Env, addr: &Address) -> bool {
        let admins: Vec<Address> = env
            .storage()
            .instance()
            .get(&GuardDataKey::Admins)
            .unwrap_or_else(|| Vec::new(env));
        admins.iter().any(|a| a == *addr)
    }
}

impl DefaultEmergencyGuard {
    pub fn unpause_operation(env: &Env, operation: u32) -> Result<(), GuardError> {
        Self::set_pause_state(env, operation, false)
    }

    pub fn unpause_all_emergency(env: &Env) -> Result<(), GuardError> {
        let pause_state = PauseType::new(0);
        env.storage()
            .instance()
            .set(&GuardDataKey::PauseState, &pause_state);
        log!(env, "All operations unpaused");
        Ok(())
    }

    pub fn is_operation_paused(env: &Env, operation: u32) -> bool {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&GuardDataKey::PauseState)
            .unwrap_or(PauseType::new(0));
        pause_state.is_paused(operation)
    }

    pub fn pause(env: &Env, operation: u32) -> Result<(), GuardError> {
        Self::set_pause_state(env, operation, true)
    }

    pub fn validate_multi_sig(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::check_multi_sig(&env, &approvers).map(|_| ())
    }
}
