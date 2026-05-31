#![no_std]

use soroban_sdk::{contract, contracterror, contractimpl, contracttype, Address, Env, String, Vec};

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
}

/// Data keys for emergency guard storage
#[contracttype]
pub enum DataKey {
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

    /// Get list of current admins
    fn get_admins(env: &Env) -> Vec<Address>;

    /// Get required signature threshold
    fn get_threshold(env: &Env) -> u32;

    /// Check if address is an admin
    fn is_admin(env: &Env, addr: &Address) -> bool;
}

#[contract]
pub struct EmergencyGuard;

#[contractimpl]
impl EmergencyGuard {
    /// Initialize the emergency guard with a list of admins and required threshold
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

    /// Check if an operation is paused
    pub fn is_paused(env: Env, operation: u32) -> bool {
        let pause_state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
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
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        pause_state.set_paused(operation, paused);
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

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
            .set(&DataKey::PauseState, &pause_state);

        env.events().publish(
            (String::from_str(
                &env,
                "emergency_guard.emergency_pause_all",
            ),),
            (approvers.clone(),),
        );
        Ok(())
    }

    /// Resume all operations (requires multi-sig approval)
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
            env.events().publish(
                (
                    String::from_str(&env, "emergency_guard.admin_added"),
                    new_admin.clone(),
                ),
                (),
            );
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
            (
                String::from_str(&env, "emergency_guard.admin_removed"),
                admin.clone(),
            ),
            (),
        );
        Ok(())
    }

    /// Get list of current admins
    pub fn get_admins(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::Admins)
            .unwrap_or_else(|| Vec::new(&env))
    }

    /// Get required signature threshold
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
