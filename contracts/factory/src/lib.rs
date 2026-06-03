#![no_std]
#[cfg(test)]
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, IntoVal, Vec,
use emergency_guard::{EmergencyGuard, GuardError};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, Vec,
use emergency_guard::{EmergencyGuard, GuardError};
use soroban_sdk::{
use emergency_guard::{EmergencyGuard, PauseType};
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, xdr::ToXdr, Address, BytesN, Env, IntoVal,
    contract, contractimpl, contracttype, xdr::ToXdr, Address, BytesN, Env, IntoVal, Vec,
};
#[cfg(not(test))]
use soroban_sdk::xdr::ToXdr;

use emergency_guard::{EmergencyGuard, GuardError, PauseType};

use emergency_guard::PauseType;

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    Paused = 4,
}

const PAUSE_CREATE_PAIR_FLAG: u32 = 1 << 6;
use emergency_guard::{EmergencyGuard, PauseType};

// Helper to enforce pause checks for factory operations.
fn ensure_not_paused(e: &Env, operation: u32) {
    if EmergencyGuard::is_paused(e.clone(), operation) {
        panic!("Operation paused");
    }
}

/// Storage keys for the factory contract.
/// Storage key for pair registry and multi-sig admin data.
/// Stored in **instance** storage because the factory is a singleton contract
/// and pair mappings are global state that should share the contract's TTL.
/// Using instance storage avoids per-entry persistent rent and reduces the
/// ledger footprint to a single entry per invocation.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    Pair(Address, Address),
    GuardPauseState,
}

fn pause_state(env: &Env) -> PauseType {
    env.storage()
        .instance()
        .get(&DataKey::GuardPauseState)
        .unwrap_or(PauseType::new(0))
}

fn set_pause_state(env: &Env, operation: u32, paused: bool) {
    let mut state = pause_state(env);
    state.set_paused(operation, paused);
    env.storage()
        .instance()
        .set(&DataKey::GuardPauseState, &state);
}

fn check_not_paused(env: &Env, operation: u32) -> Result<(), Error> {
    if pause_state(env).is_paused(operation) {
        Err(Error::Paused)
    } else {
        Ok(())
    }
    Admin,
}

#[contracterror]
#[derive(Copy, Clone, Debug, Eq, PartialEq, PartialOrd, Ord)]
#[repr(u32)]
pub enum Error {
    AlreadyInitialized = 1,
    NotInitialized = 2,
    Unauthorized = 3,
    Paused = 4,
    PairAlreadyExists = 5,
    InvalidThreshold = 6,
    Unauthorized = 2,
    Paused = 3,
    MultiSigConfig,
    PendingAction(u32), // Action ID
    ApprovalCount(u32), // Action ID
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MultiSigConfig {
    pub admins: Vec<Address>,
    pub threshold: u32,
}

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum AdminAction {
    AddAdmin(Address),
    RemoveAdmin(Address),
    SetThreshold(u32),
}

#[contract]
pub struct LiquidityPoolFactory;

fn check_not_paused(env: &Env) -> Result<(), Error> {
    if EmergencyGuard::is_paused(env.clone(), PAUSE_CREATE_PAIR_FLAG) {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

#[contractimpl]
impl LiquidityPoolFactory {
    /// Initializes the factory admin committee using the shared EmergencyGuard storage.
    pub fn initialize(env: Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        EmergencyGuard::initialize(env, admins, threshold)
    }

    /// Add a new admin using the shared multi-signature approval flow.
    pub fn add_admin(
        env: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::add_admin(env, approvers, new_admin)
    }

    /// Remove an admin using the shared multi-signature approval flow.
    pub fn remove_admin(
        env: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::remove_admin(env, approvers, admin)
    }

    /// Returns the currently configured factory admins.
    pub fn get_admins(env: Env) -> Vec<Address> {
        EmergencyGuard::get_admins(env)
    }

    /// Returns the required multi-signature threshold.
    pub fn get_threshold(env: Env) -> u32 {
        EmergencyGuard::get_threshold(env)
    }

    /// Checks whether an address is currently authorized as a factory admin.
    pub fn is_admin(env: Env, addr: Address) -> bool {
        EmergencyGuard::is_admin(&env, &addr)
    /// Initializes the factory guard admin. Pair creation remains unpaused by default.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::GuardPauseState, &PauseType::new(0));
        Ok(())
    }

    /// Pause or resume a granular factory operation.
    pub fn guard_pause(
        env: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), Error> {
        admin.require_auth();

        if !EmergencyGuard::is_admin(&env, &admin) {
            return Err(Error::Unauthorized);
        }

        set_pause_state(&env, operation, paused);
        Ok(())
    }

    /// Returns true when a granular factory operation is paused.
    pub fn guard_is_paused(env: Env, operation: u32) -> bool {
        pause_state(&env).is_paused(operation)
    }

    /// Returns the factory pause state as a PauseType bitmask.
    pub fn get_pause_state(env: Env) -> PauseType {
        pause_state(&env)
    /// Initialize the factory's emergency guard state with a set of admins.
    pub fn initialize(env: Env, admins: soroban_sdk::Vec<Address>, threshold: u32) -> Result<(), Error> {
        EmergencyGuard::initialize(env.clone(), admins, threshold).map_err(|e| match e {
            GuardError::AlreadyInitialized => Error::AlreadyInitialized,
            GuardError::InvalidThreshold => Error::InvalidThreshold,
            _ => Error::Unauthorized,
        })
    }

    /// Initializes the factory contract with an admin and setup the emergency guard.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);

        // Initialize emergency guard with the admin
        let admins = soroban_sdk::vec![&env, admin];
        EmergencyGuard::initialize(env.clone(), admins, 1)
            .map_err(|_| Error::Unauthorized)?;

        Ok(())
    }

    /// Deploys a new Liquidity Pool contract for a unique pair of tokens.
    pub fn create_pair(
        env: Env,
        token_a: Address,
        token_b: Address,
        wasm_hash: BytesN<32>,
    ) -> Address {
        if check_not_paused(&env, CREATE_PAIR).is_err() || EmergencyGuard::is_paused(env.clone(), PauseType::CREATE_PAIR) {
            panic!("Factory pair creation is paused");
        }
    ) -> Result<Address, Error> {
        check_not_paused(&env)?;

        // Ensure not paused for mint operation
        ensure_not_paused(&env, PauseType::MINT);
        if EmergencyGuard::is_paused(env.clone(), PauseType::CREATE_PAIR) {
            panic!("Pair creation is paused");
        if check_not_paused(&env, PauseType::CREATE_PAIR).is_err() {
            panic!("Create pair operation is paused");
        }

        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        // Instance storage: cheaper rent, no per-entry TTL management.
        if env
            .storage()
            .instance()
            .has(&DataKey::Pair(token_0.clone(), token_1.clone()))
        {
            return Err(Error::PairAlreadyExists);
        }

        #[cfg(test)]
        let deployed_address = {
            let _ = wasm_hash;
            Address::generate(&env)
        };

        #[cfg(not(test))]
        let deployed_address = {
            let salt = env
                .crypto()
                .sha256(&(token_0.clone(), token_1.clone()).to_xdr(&env));

            let deployed_address = env
                .deployer()
                .with_current_contract(salt)
                .deploy_v2(wasm_hash, soroban_sdk::Vec::<soroban_sdk::Val>::new(&env));

            let init_args = soroban_sdk::vec![
                &env,
                env.current_contract_address().into_val(&env),
                token_0.clone().into_val(&env),
                token_1.clone().into_val(&env)
            ];

            let _res: soroban_sdk::Val = env.invoke_contract(
                &deployed_address,
                &soroban_sdk::Symbol::new(&env, "initialize"),
                init_args,
            );

            deployed_address
        };

        // One instance write instead of one persistent write.
        env.storage()
            .instance()
        Ok(deployed_address)y::Pair(token_0, token_1), &deployed_address);

        deployed_address
    }

    /// Returns the pool address for the given token pair, if it exists.
    pub fn get_pair(env: Env, token_a: Address, token_b: Address) -> Option<Address> {
        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        // One instance read instead of one persistent read.
        env.storage()
            .instance()
            .get(&DataKey::Pair(token_0, token_1))
    }

    /// Initialize the factory's emergency guard.
    pub fn initialize_guard(
        env: Env,
        admins: Vec<Address>,
        threshold: u32,
    ) -> Result<(), GuardError> {
        EmergencyGuard::initialize(env, admins, threshold)
    }

    /// Admin-only: pause or unpause a factory operation.
    pub fn set_guard_pause(
    /// Admin-only: pause or unpause a specific operation.
    pub fn set_operation_paused(
        env: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), GuardError> {
        EmergencyGuard::set_pause(env, admin, operation, paused)
    }

    /// Multi-sig: pause all guarded factory operations.
    pub fn emergency_guard_pause(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::emergency_pause(env, approvers)
    }

    /// Multi-sig: resume all guarded factory operations.
    pub fn resume_guard(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::resume(env, approvers)
    }

    /// Multi-sig: add a factory guard admin.
    pub fn add_guard_admin(
        env: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::add_admin(env, approvers, new_admin)
    }

    /// Multi-sig: remove a factory guard admin.
    pub fn remove_guard_admin(
        env: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::remove_admin(env, approvers, admin)
    }

    /// Returns whether a factory operation is currently paused.
    pub fn is_guard_paused(env: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(env, operation)
    /// Admin-only: pause or unpause pair creation.
    pub fn set_paused(env: Env, admin: Address, paused: bool) -> Result<(), Error> {
        EmergencyGuard::set_pause(env, admin, PAUSE_CREATE_PAIR_FLAG, paused).map_err(|e| match e {
            GuardError::Unauthorized => Error::Unauthorized,
            _ => Error::Unauthorized,
        })
    }

    /// Admin-only: emergency pause all factory operations.
    pub fn emergency_pause(env: Env, approvers: soroban_sdk::Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::emergency_pause(env, approvers).map_err(|e| match e {
            GuardError::NotInitialized => Error::NotInitialized,
            GuardError::InsufficientSignatures => Error::Unauthorized,
            _ => Error::Unauthorized,
        })
    }

    /// Read the factory's current pause state.
    pub fn get_pause_state(env: Env) -> u32 {
        EmergencyGuard::get_pause_state(env)
    ) -> Result<(), Error> {
        EmergencyGuard::set_pause(env, admin, operation, paused)
            .map_err(|_| Error::Unauthorized)
    }

    /// Check if a specific operation is paused.
    pub fn is_paused(env: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(env, operation)
    }

    /// Returns the list of factory admins.
    pub fn get_admins(env: Env) -> soroban_sdk::Vec<Address> {
        EmergencyGuard::get_admins(env)
    /// Initialize multi-sig admin configuration for the factory.
    pub fn init_multisig(env: Env, admins: Vec<Address>, threshold: u32) {
        if env.storage().instance().has(&DataKey::MultiSigConfig) {
            panic!("MultiSig already initialized");
        }

        if admins.len() == 0 {
            panic!("At least one admin required");
        }

        if threshold == 0 || threshold as usize > admins.len() {
            panic!("Invalid threshold");
        }

        let config = MultiSigConfig {
            admins: admins.clone(),
            threshold,
        };

        env.storage()
            .instance()
            .set(&DataKey::MultiSigConfig, &config);
    }

    /// Get the current multi-sig configuration.
    pub fn get_multisig_config(env: Env) -> MultiSigConfig {
        env.storage()
            .instance()
            .get(&DataKey::MultiSigConfig)
            .unwrap_or_else(|| panic!("MultiSig not initialized"))
    }

    /// Check if an address is an admin.
    pub fn is_admin(env: Env, address: &Address) -> bool {
        if let Some(config) = env.storage().instance().get::<_, MultiSigConfig>(&DataKey::MultiSigConfig) {
            config.admins.iter().any(|a| a == address)
        } else {
            false
        }
    }

    /// Propose an admin action (add admin, remove admin, or set threshold).
    /// Returns the action ID.
    pub fn propose_admin_action(env: Env, proposer: Address, action: AdminAction) -> u32 {
        // Verify proposer is an admin
        if !Self::is_admin(env.clone(), &proposer) {
            panic!("Only admins can propose actions");
        }

        let config = Self::get_multisig_config(env.clone());

        // Generate action ID (use timestamp as simple unique ID)
        let action_id = env.ledger().timestamp();

        // Store the pending action
        env.storage()
            .instance()
            .set(&DataKey::PendingAction(action_id), &action);

        // Initialize approval count for this action
        env.storage()
            .instance()
            .set(&DataKey::ApprovalCount(action_id), &1u32);

        action_id
    }

    /// Approve an admin action as a multi-sig signer.
    pub fn approve_admin_action(env: Env, approver: Address, action_id: u32) {
        // Verify approver is an admin
        if !Self::is_admin(env.clone(), &approver) {
            panic!("Only admins can approve actions");
        }

        // Check if action exists
        if !env.storage().instance().has(&DataKey::PendingAction(action_id)) {
            panic!("Action not found");
        }

        // Increment approval count
        let mut approval_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ApprovalCount(action_id))
            .unwrap_or_else(|| 0);

        approval_count += 1;

        env.storage()
            .instance()
            .set(&DataKey::ApprovalCount(action_id), &approval_count);
    }

    /// Execute an admin action once it has enough approvals.
    pub fn execute_admin_action(env: Env, action_id: u32) {
        let config = Self::get_multisig_config(env.clone());

        // Get approval count
        let approval_count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ApprovalCount(action_id))
            .unwrap_or_else(|| 0);

        // Check if threshold is met
        if approval_count < config.threshold {
            panic!("Insufficient approvals");
        }

        // Get and execute the action
        let action: AdminAction = env
            .storage()
            .instance()
            .get(&DataKey::PendingAction(action_id))
            .unwrap_or_else(|| panic!("Action not found"));

        match action {
            AdminAction::AddAdmin(new_admin) => {
                let mut new_config = config.clone();
                
                // Check if admin already exists
                if new_config.admins.iter().any(|a| a == &new_admin) {
                    panic!("Admin already exists");
                }

                new_config.admins.push_back(new_admin);
                env.storage()
                    .instance()
                    .set(&DataKey::MultiSigConfig, &new_config);
            }
            AdminAction::RemoveAdmin(admin_to_remove) => {
                let mut new_config = config.clone();

                // Find and remove the admin
                let initial_len = new_config.admins.len();
                let filtered_admins: Vec<Address> = new_config
                    .admins
                    .iter()
                    .filter(|a| a != &admin_to_remove)
                    .collect();

                if filtered_admins.len() == initial_len {
                    panic!("Admin not found");
                }

                if filtered_admins.len() == 0 {
                    panic!("Cannot remove last admin");
                }

                new_config.admins = filtered_admins;

                // Adjust threshold if necessary
                if new_config.threshold as usize > new_config.admins.len() {
                    new_config.threshold = new_config.admins.len() as u32;
                }

                env.storage()
                    .instance()
                    .set(&DataKey::MultiSigConfig, &new_config);
            }
            AdminAction::SetThreshold(new_threshold) => {
                if new_threshold == 0 || new_threshold as usize > config.admins.len() {
                    panic!("Invalid threshold");
                }

                let mut new_config = config.clone();
                new_config.threshold = new_threshold;
                env.storage()
                    .instance()
                    .set(&DataKey::MultiSigConfig, &new_config);
            }
        }

        // Clean up: remove the action and approval count
        env.storage().instance().remove(&DataKey::PendingAction(action_id));
        env.storage().instance().remove(&DataKey::ApprovalCount(action_id));
    }
}

mod test;
