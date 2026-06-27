#![no_std]
#[cfg(test)]
use soroban_sdk::testutils::Address as _;

#[cfg(not(test))]
use soroban_sdk::xdr::ToXdr;

use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, Address, BytesN, Env, IntoVal, Vec,
};

use emergency_guard::{EmergencyGuard, GuardError, PauseType};

// ── Constants ────────────────────────────────────────────────────────────────

const PAUSE_CREATE_PAIR_FLAG: u32 = PauseType::CREATE_PAIR;

// ── Error types ──────────────────────────────────────────────────────────────

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
}

// ── Storage keys ─────────────────────────────────────────────────────────────

/// Storage keys for the factory contract.
/// Stored in **instance** storage: the factory is a singleton and pair
/// mappings are global state that shares the contract's TTL, avoiding
/// per-entry persistent rent and reducing ledger footprint.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    /// Canonical pair mapping (token_0, token_1) — always sorted.
    Pair(Address, Address),
    /// Single-admin address (legacy simple-init path).
    Admin,
    /// Multi-sig configuration (admins + threshold).
    MultiSigConfig,
    /// Pending admin action keyed by action ID.
    PendingAction(u32),
    /// Approval count for a pending action.
    ApprovalCount(u32),
}

// ── Multi-sig types ───────────────────────────────────────────────────────────

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

// ── Internal helpers ──────────────────────────────────────────────────────────

/// Returns `Err(Error::Paused)` if `operation` is currently paused via EmergencyGuard.
fn check_not_paused(env: &Env, operation: u32) -> Result<(), Error> {
    if EmergencyGuard::is_paused(env.clone(), operation) {
        Err(Error::Paused)
    } else {
        Ok(())
    }
}

fn map_guard_err(e: GuardError) -> Error {
    match e {
        GuardError::Unauthorized => Error::Unauthorized,
        GuardError::NotInitialized => Error::NotInitialized,
        GuardError::AlreadyInitialized => Error::AlreadyInitialized,
        GuardError::InvalidThreshold => Error::InvalidThreshold,
        _ => Error::Unauthorized,
    }
}

// ── Contract ──────────────────────────────────────────────────────────────────

#[contract]
pub struct LiquidityPoolFactory;

#[contractimpl]
impl LiquidityPoolFactory {
    // ── Initialization ────────────────────────────────────────────────────────

    /// Initialize the factory using the shared EmergencyGuard (multi-sig path).
    /// `admins` and `threshold` configure the multi-sig approval requirement.
    pub fn initialize(env: Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        EmergencyGuard::initialize(env, admins, threshold)
    }

    /// Initialize the guard with a standalone `initialize_guard` alias.
    pub fn initialize_guard(
        env: Env,
        admins: Vec<Address>,
        threshold: u32,
    ) -> Result<(), GuardError> {
        EmergencyGuard::initialize(env, admins, threshold)
    }

    // ── Admin management ──────────────────────────────────────────────────────

    /// Add a new admin (multi-sig approval required).
    pub fn add_admin(
        env: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::add_admin(env, approvers, new_admin)
    }

    /// Remove an admin (multi-sig approval required).
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

    /// Returns true if `addr` is a registered factory admin.
    pub fn is_admin(env: Env, addr: Address) -> bool {
        EmergencyGuard::is_admin_public(env, addr)
    }

    // ── Pause controls ────────────────────────────────────────────────────────

    /// Granularly pause or resume a factory operation (any single admin).
    pub fn guard_pause(
        env: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), Error> {
        EmergencyGuard::set_pause(env, admin, operation, paused).map_err(map_guard_err)
    }

    /// Unpause a single operation bit without affecting others (any single admin).
    pub fn guard_unpause(env: Env, admin: Address, operation: u32) -> Result<(), Error> {
        EmergencyGuard::set_pause(env, admin, operation, false).map_err(map_guard_err)
    }

    /// Returns `true` when `operation` is currently paused.
    pub fn guard_is_paused(env: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(env, operation)
    }

    /// Alias for `guard_is_paused` (backward-compatible).
    pub fn is_paused(env: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(env, operation)
    }

    /// Pause/unpause pair creation specifically (maps to PauseType::CREATE_PAIR).
    pub fn set_paused(env: Env, admin: Address, paused: bool) -> Result<(), Error> {
        EmergencyGuard::set_pause(env, admin, PAUSE_CREATE_PAIR_FLAG, paused)
            .map_err(map_guard_err)
    }

    /// Returns the raw pause-state bitmask.
    pub fn get_pause_state(env: Env) -> u32 {
        EmergencyGuard::get_pause_state(env)
    }

    /// Admin-only: pause or unpause any operation (alias for guard_pause).
    pub fn set_operation_paused(
        env: Env,
        admin: Address,
        operation: u32,
        paused: bool,
    ) -> Result<(), Error> {
        EmergencyGuard::set_pause(env, admin, operation, paused).map_err(map_guard_err)
    }

    /// Multi-sig: emergency pause ALL factory operations.
    pub fn emergency_guard_pause(env: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::emergency_pause(env, approvers)
    }

    /// Multi-sig: emergency pause (Error-mapped alias).
    pub fn emergency_pause(env: Env, approvers: Vec<Address>) -> Result<(), Error> {
        EmergencyGuard::emergency_pause(env, approvers).map_err(map_guard_err)
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

    // ── Core factory logic ────────────────────────────────────────────────────

    /// Deploys a new Liquidity Pool for a unique token pair.
    /// Tokens are sorted canonically so (A, B) == (B, A).
    pub fn create_pair(
        env: Env,
        token_a: Address,
        token_b: Address,
        wasm_hash: BytesN<32>,
    ) -> Result<Address, Error> {
        // Fast-fail if pair creation is paused.
        check_not_paused(&env, PAUSE_CREATE_PAIR_FLAG)?;

        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        // Deduplicate: instance storage, O(1) has-check.
        if env
            .storage()
            .instance()
            .has(&DataKey::Pair(token_0.clone(), token_1.clone()))
        {
            return Err(Error::PairAlreadyExists);
        }

        // In tests, skip real WASM deployment and generate a synthetic address.
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
                token_1.clone().into_val(&env),
            ];

            let _res: soroban_sdk::Val = env.invoke_contract(
                &deployed_address,
                &soroban_sdk::Symbol::new(&env, "initialize"),
                init_args,
            );

            deployed_address
        };

        // Store the canonical pair → pool mapping.
        env.storage()
            .instance()
            .set(&DataKey::Pair(token_0, token_1), &deployed_address);

        Ok(deployed_address)
    }

    /// Returns the pool address for `(token_a, token_b)`, or `None`.
    pub fn get_pair(env: Env, token_a: Address, token_b: Address) -> Option<Address> {
        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };
        env.storage()
            .instance()
            .get(&DataKey::Pair(token_0, token_1))
    }

    // ── Legacy multi-sig admin management (internal state) ────────────────────

    /// Initialize multi-sig admin configuration stored in factory instance storage.
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
        let config = MultiSigConfig { admins, threshold };
        env.storage()
            .instance()
            .set(&DataKey::MultiSigConfig, &config);
    }

    /// Returns the current multi-sig configuration.
    pub fn get_multisig_config(env: Env) -> MultiSigConfig {
        env.storage()
            .instance()
            .get(&DataKey::MultiSigConfig)
            .unwrap_or_else(|| panic!("MultiSig not initialized"))
    }

    /// Propose an admin action. Returns the action ID.
    pub fn propose_admin_action(env: Env, proposer: Address, action: AdminAction) -> u32 {
        let config = Self::get_multisig_config(env.clone());
        if !config.admins.iter().any(|a| a == proposer) {
            panic!("Only admins can propose actions");
        }
        let action_id = env.ledger().timestamp();
        env.storage()
            .instance()
            .set(&DataKey::PendingAction(action_id), &action);
        env.storage()
            .instance()
            .set(&DataKey::ApprovalCount(action_id), &1u32);
        action_id
    }

    /// Approve a pending admin action.
    pub fn approve_admin_action(env: Env, approver: Address, action_id: u32) {
        let config = Self::get_multisig_config(env.clone());
        if !config.admins.iter().any(|a| a == approver) {
            panic!("Only admins can approve actions");
        }
        if !env
            .storage()
            .instance()
            .has(&DataKey::PendingAction(action_id))
        {
            panic!("Action not found");
        }
        let mut count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ApprovalCount(action_id))
            .unwrap_or(0);
        count += 1;
        env.storage()
            .instance()
            .set(&DataKey::ApprovalCount(action_id), &count);
    }

    /// Execute a pending admin action once the approval threshold is met.
    pub fn execute_admin_action(env: Env, action_id: u32) {
        let config = Self::get_multisig_config(env.clone());
        let count: u32 = env
            .storage()
            .instance()
            .get(&DataKey::ApprovalCount(action_id))
            .unwrap_or(0);
        if count < config.threshold {
            panic!("Insufficient approvals");
        }
        let action: AdminAction = env
            .storage()
            .instance()
            .get(&DataKey::PendingAction(action_id))
            .unwrap_or_else(|| panic!("Action not found"));

        match action {
            AdminAction::AddAdmin(new_admin) => {
                let mut new_config = config.clone();
                if new_config.admins.iter().any(|a| a == new_admin) {
                    panic!("Admin already exists");
                }
                new_config.admins.push_back(new_admin);
                env.storage()
                    .instance()
                    .set(&DataKey::MultiSigConfig, &new_config);
            }
            AdminAction::RemoveAdmin(admin_to_remove) => {
                let mut new_config = config.clone();
                let mut new_admins = Vec::new(&env);
                let mut found = false;
                for a in new_config.admins.iter() {
                    if a == admin_to_remove {
                        found = true;
                    } else {
                        new_admins.push_back(a);
                    }
                }
                if !found {
                    panic!("Admin not found");
                }
                if new_admins.len() == 0 {
                    panic!("Cannot remove last admin");
                }
                if new_config.threshold as usize > new_admins.len() {
                    new_config.threshold = new_admins.len() as u32;
                }
                new_config.admins = new_admins;
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

        env.storage()
            .instance()
            .remove(&DataKey::PendingAction(action_id));
        env.storage()
            .instance()
            .remove(&DataKey::ApprovalCount(action_id));
    }
}

mod test;
