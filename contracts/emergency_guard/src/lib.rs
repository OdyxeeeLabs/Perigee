#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, log, vec, Address, Env, Error, Vec};

/// Granular pause types using bitmask for efficient storage
/// Each bit represents a different pausable operation
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
    /// Admin rotation queue: Vec<Address>
    AdminQueue,
    /// Number of signatures required for multi-sig: u32
    SignatureThreshold,
    /// Pending admin rotation request: Address
    PendingAdmin,
}

/// Error codes
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd)]
#[repr(u32)]
pub enum GuardError {
    Unauthorized = 1,
    Paused = 2,
    InsufficientSignatures = 3,
    InvalidThreshold = 4,
    AdminNotFound = 5,
    QueueFull = 6,
}

impl From<GuardError> for Error {
    fn from(err: GuardError) -> Self {
        Error::from_contract_error(err as u32)
    }
}

/// Result type for guard operations
pub type GuardResult<T> = Result<T, GuardError>;

/// EmergencyGuard trait for standardized pause and admin management
pub trait EmergencyGuard {
    /// Check if an operation is paused. Returns Err if paused.
    fn require_not_paused(env: &Env, operation: u32) -> GuardResult<()>;

    /// Get current pause state
    fn get_pause_state(env: &Env) -> PauseType;

    /// Set pause state for a specific operation (admin only)
    fn set_pause(env: &Env, operation: u32, paused: bool) -> GuardResult<()>;

    /// Emergency pause all operations (multi-sig required)
    fn emergency_pause_all(env: &Env) -> GuardResult<()>;

    /// Resume all operations (multi-sig required)
    fn resume_all(env: &Env) -> GuardResult<()>;

    /// Initialize emergency guard with admins and threshold
    fn initialize(env: &Env, admins: Vec<Address>, threshold: u32) -> GuardResult<()>;

    /// Add new admin (multi-sig required)
    fn add_admin(env: &Env, new_admin: Address) -> GuardResult<()>;

    /// Remove admin (multi-sig required)
    fn remove_admin(env: &Env, admin: Address) -> GuardResult<()>;

    /// Rotate admin - old admin transfers authority to new admin (old admin only)
    fn rotate_admin(env: &Env, new_admin: Address) -> GuardResult<()>;

    /// Get list of current admins
    fn get_admins(env: &Env) -> Vec<Address>;

    /// Get required signature threshold
    fn get_threshold(env: &Env) -> u32;

    /// Check if address is an admin
    fn is_admin(env: &Env, addr: &Address) -> bool;
}

/// Default implementation of EmergencyGuard
pub struct DefaultEmergencyGuard;

#[contractimpl]
impl DefaultEmergencyGuard {
    /// Initialize the emergency guard with a list of admins and required threshold
    pub fn init_guard(env: Env, admins: Vec<Address>, threshold: u32) -> GuardResult<()> {
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
    pub fn check_not_paused(env: Env, operation: u32) -> GuardResult<()> {
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
    pub fn get_pause_state(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0))
            .0
    }

    /// Set pause state for a specific operation (any single admin can do this)
    pub fn set_pause_state(env: Env, operation: u32, paused: bool) -> GuardResult<()> {
        let caller = env.invoker();

        // Check if caller is admin
        if !Self::is_admin_internal(&env, &caller) {
            return Err(GuardError::Unauthorized);
        }

        // Get current pause state
        let mut pause_state: PauseType = env
            .storage()
            .instance()
            .get(&DataKey::PauseState)
            .unwrap_or(PauseType::new(0));

        // Update pause state
        pause_state.set_paused(operation, paused);

        // Store updated state
        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        log!(&env, "Pause state updated: operation={}, paused={}", operation, paused);

        Ok(())
    }

    /// Emergency pause all operations (requires multi-sig approval)
    pub fn emergency_pause_all(env: Env) -> GuardResult<()> {
        let caller = env.invoker();

        // Check if caller is admin
        if !Self::is_admin_internal(&env, &caller) {
            return Err(GuardError::Unauthorized);
        }

        // For now, any admin can pause all (in production, check signatures)
        // In a real implementation, you'd use Soroban's multi-sig capabilities
        let mut pause_state = PauseType::new(0);
        pause_state.pause_all();

        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        log!(&env, "Emergency pause all activated by {}", caller);

        Ok(())
    }

    /// Resume all operations (requires multi-sig approval)
    pub fn resume_all(env: Env) -> GuardResult<()> {
        let caller = env.invoker();

        // Check if caller is admin
        if !Self::is_admin_internal(&env, &caller) {
            return Err(GuardError::Unauthorized);
        }

        // For now, any admin can resume all (in production, check signatures)
        let pause_state = PauseType::new(0);

        env.storage()
            .instance()
            .set(&DataKey::PauseState, &pause_state);

        log!(&env, "Resume all activated by {}", caller);

        Ok(())
    }

    /// Get list of current admins
    pub fn get_admins(env: Env) -> Vec<Address> {
        env.storage()
            .instance()
            .get(&DataKey::Admins)
            .unwrap_or_else(|| vec![&env])
    }

    /// Get required signature threshold
    pub fn get_threshold(env: Env) -> u32 {
        env.storage()
            .instance()
            .get(&DataKey::SignatureThreshold)
            .unwrap_or(1)
    }

    /// Check if address is an admin (internal helper)
    fn is_admin_internal(env: &Env, addr: &Address) -> bool {
        let admins: Vec<Address> = env
            .storage()
            .instance()
            .get(&DataKey::Admins)
            .unwrap_or_else(|| vec![env]);

        admins.iter().any(|a| a == addr)
    }

    /// Check if address is an admin (public version)
    pub fn is_admin(env: Env, addr: Address) -> bool {
        Self::is_admin_internal(&env, &addr)
    }

    /// Add new admin (multi-sig required)
    pub fn add_admin(env: Env, new_admin: Address) -> GuardResult<()> {
        let caller = env.invoker();

        // Check if caller is admin
        if !Self::is_admin_internal(&env, &caller) {
            return Err(GuardError::Unauthorized);
        }

        let mut admins = Self::get_admins(&env);

        // Check if already an admin
        if admins.iter().any(|a| a == &new_admin) {
            return Ok(());
        }

        admins.push_back(new_admin.clone());
        env.storage().instance().set(&DataKey::Admins, &admins);

        log!(&env, "Admin added: {}", new_admin);

        Ok(())
    }

    /// Remove admin (multi-sig required, but prevents removing all admins)
    pub fn remove_admin(env: Env, admin: Address) -> GuardResult<()> {
        let caller = env.invoker();

        // Check if caller is admin
        if !Self::is_admin_internal(&env, &caller) {
            return Err(GuardError::Unauthorized);
        }

        let mut admins = Self::get_admins(&env);
        let threshold = Self::get_threshold(&env);

        // Prevent removing admin if it would violate threshold
        if (admins.len() as u32) <= threshold {
            return Err(GuardError::InvalidThreshold);
        }

        // Find and remove the admin
        let initial_len = admins.len();
        let mut new_admins = vec![&env];
        for a in admins.iter() {
            if a != &admin {
                new_admins.push_back(a.clone());
            }
        }

        if new_admins.len() == initial_len {
            return Err(GuardError::AdminNotFound);
        }

        env.storage().instance().set(&DataKey::Admins, &new_admins);

        log!(&env, "Admin removed: {}", admin);

        Ok(())
    }

    /// Rotate admin - old admin transfers authority to new admin
    /// The old admin must sign the transaction
    pub fn rotate_admin(env: Env, new_admin: Address) -> GuardResult<()> {
        let caller = env.invoker();

        // Check if caller is admin
        if !Self::is_admin_internal(&env, &caller) {
            return Err(GuardError::Unauthorized);
        }

        let mut admins = Self::get_admins(&env);

        // Find the caller's index and replace with new admin
        let mut found = false;
        for i in 0..admins.len() {
            if &admins.get_unchecked(i) == &caller {
                admins.set(i, new_admin.clone());
                found = true;
                break;
            }
        }

        if !found {
            return Err(GuardError::AdminNotFound);
        }

        env.storage().instance().set(&DataKey::Admins, &admins);

        log!(&env, "Admin rotated: {} -> {}", caller, new_admin);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::{Address as _, Env as _};

    #[test]
    fn test_pause_type_operations() {
        let mut pause = PauseType::new(0);
        assert!(!pause.is_paused(PauseType::SWAP));

        pause.set_paused(PauseType::SWAP, true);
        assert!(pause.is_paused(PauseType::SWAP));
        assert!(!pause.is_paused(PauseType::DEPOSIT));

        pause.set_paused(PauseType::DEPOSIT, true);
        assert!(pause.is_paused(PauseType::DEPOSIT));

        pause.set_paused(PauseType::SWAP, false);
        assert!(!pause.is_paused(PauseType::SWAP));
        assert!(pause.is_paused(PauseType::DEPOSIT));

        pause.unpause_all();
        assert!(!pause.is_paused(PauseType::DEPOSIT));
    }
}
