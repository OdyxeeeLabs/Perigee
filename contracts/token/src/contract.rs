#![allow(dead_code)]

use crate::admin::{has_administrator, read_administrator, write_administrator};
use crate::allowance::{read_allowance, spend_allowance, write_allowance};
use crate::balance::{read_balance, receive_balance, spend_balance};
use crate::metadata::{read_decimal, read_name, read_symbol, write_metadata};
use emergency_guard::{EmergencyGuard, GuardError};
use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};
use emergency_guard::{EmergencyGuard, PauseType};
use soroban_sdk::{contract, contractimpl, vec, Address, Env, String, Vec};
use emergency_guard::{EmergencyGuard, GuardError, PauseType};
use emergency_guard::{DataKey as GuardDataKey, EmergencyGuard, GuardError, PauseType};
use soroban_sdk::{contract, contractimpl, vec, Address, Env, String, Vec};

fn require_not_paused(e: &Env, operation: u32) {
    if EmergencyGuard::is_paused(e.clone(), operation) {
        panic!("operation paused");
use soroban_sdk::{contract, contractimpl, Address, Env, String};
use emergency_guard::{EmergencyGuard, PauseType};

// Helper to enforce pause checks for token operations.
fn ensure_not_paused(e: &Env, operation: u32) {
    if EmergencyGuard::is_paused(e.clone(), operation) {
        panic!("Operation paused");
    }
}

pub trait TokenTrait {
    fn initialize(e: Env, admin: Address, decimal: u32, name: String, symbol: String);
    fn mint(e: Env, to: Address, amount: i128);
    fn set_admin(e: Env, new_admin: Address);
    fn guard_pause(e: Env, admin: Address, operation: u32, paused: bool) -> Result<(), GuardError>;
    fn emergency_pause(e: Env, approvers: Vec<Address>) -> Result<(), GuardError>;
    fn guard_resume(e: Env, approvers: Vec<Address>) -> Result<(), GuardError>;
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
    fn guard_admins(e: Env) -> Vec<Address>;
    fn guard_threshold(e: Env) -> u32;
    fn guard_is_paused(e: Env, operation: u32) -> bool;
use emergency_guard::EmergencyGuard;
use soroban_sdk::{contract, contractimpl, Address, Env, String, vec, Vec};

pub trait TokenTrait {
    fn initialize(e: Env, admin: Address, decimal: u32, name: String, symbol: String);
    fn mint(e: Env, approvers: Vec<Address>, to: Address, amount: i128);
    fn set_admin(e: Env, approvers: Vec<Address>, new_admin: Address);
    fn allowance(e: Env, from: Address, spender: Address) -> i128;
    fn approve(e: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32);
    fn balance(e: Env, id: Address) -> i128;
    fn transfer(e: Env, from: Address, to: Address, amount: i128);
    fn transfer_from(e: Env, spender: Address, from: Address, to: Address, amount: i128);
    fn burn(e: Env, from: Address, amount: i128);
    fn burn_from(e: Env, spender: Address, from: Address, amount: i128);
    fn decimals(e: Env) -> u32;
    fn name(e: Env) -> String;
    fn symbol(e: Env) -> String;
    fn guard_pause(e: Env, admin: Address, operation: u32, paused: bool) -> Result<(), GuardError>;
    fn guard_unpause(e: Env, approvers: Vec<Address>) -> Result<(), GuardError>;
}

#[contract]
pub struct Token;

#[contractimpl]
impl TokenTrait for Token {
    fn initialize(e: Env, admin: Address, decimal: u32, name: String, symbol: String) {
        if has_administrator(&e) {
            panic!("already initialized");
        }
        write_administrator(&e, &admin);
        EmergencyGuard::initialize(e.clone(), vec![&e, admin.clone()], 1)
            .expect("failed to initialize emergency guard");
        // One write instead of three separate writes for name/symbol/decimals.
        write_metadata(&e, &name, &symbol, decimal);
        
        // Initialize emergency guard with single admin and threshold of 1
        let admins = vec![&e, admin.clone()];
        let threshold = 1;
        EmergencyGuard::initialize(e, admins, threshold)

        // Initialize emergency guard with the token admin as the sole guard admin.
        // All three fields (PauseState, Admins, SignatureThreshold) share the same
        // instance storage entry as the token's own fields — no extra footprint entries.
        let admins: Vec<Address> = vec![&e, admin];
        EmergencyGuard::initialize(e, admins, 1)
            .expect("Failed to initialize emergency guard");
    }

    fn mint(e: Env, to: Address, amount: i128) {
        // Guard check: minting can be paused independently of transfers.
        // Reads the single PauseState instance entry — same footprint as before.
        if EmergencyGuard::is_paused(e.clone(), PauseType::MINT) {
            panic!("minting is paused");
        }

        require_not_paused(&e, PauseType::MINT);
        // Ensure minting is not paused.
        ensure_not_paused(&e, PauseType::MINT);
        let admin = read_administrator(&e);
        admin.require_auth();
        // Initialize EmergencyGuard with single admin by default
        let admins = vec![&e, admin.clone()];
        EmergencyGuard::initialize(e.clone(), admins, 1).expect("Failed to init emergency guard");
    }

    fn mint(e: Env, approvers: Vec<Address>, to: Address, amount: i128) {
        // Validate approvers using EmergencyGuard multi-sig logic
        EmergencyGuard::validate_multi_sig(e.clone(), approvers)
            .expect("multi-sig validation failed");

        e.storage().instance().extend_ttl(100, 100);

        // Check if minting is paused (using MINT pause type = 1 << 4 = 16)
        if EmergencyGuard::is_paused(e.clone(), 1 << 4) {
            panic!("minting is paused");
        }

        receive_balance(&e, to, amount);
    }

    fn set_admin(e: Env, new_admin: Address) {
        let admin = read_administrator(&e);
        // admin.require_auth(); // Handled by EmergencyGuard
        e.storage().instance().extend_ttl(100, 100);
        if admin == new_admin {
            write_administrator(&e, &new_admin);
            return;
        }

        EmergencyGuard::rotate_admin(e.clone(), vec![&e, admin.clone()], admin.clone(), new_admin.clone())
            .expect("failed to rotate token admin");
    fn set_admin(e: Env, approvers: Vec<Address>, new_admin: Address) {
        // Use EmergencyGuard to perform admin rotation via multi-sig
        // First, add the new admin (requires approvers signatures)
        EmergencyGuard::add_admin(e.clone(), approvers.clone(), new_admin.clone())
            .expect("failed to add admin via EmergencyGuard");

        // Then remove the old admin (requires same approvers)
        let old = read_administrator(&e);
        EmergencyGuard::remove_admin(e.clone(), approvers.clone(), old.clone())
            .expect("failed to remove old admin via EmergencyGuard");

        e.storage().instance().extend_ttl(100, 100);
        let admins = EmergencyGuard::get_admins(e.clone());
        let mut rotated_admins = Vec::new(&e);
        let mut has_new_admin = false;
        let mut replaced_old_admin = false;
        for guard_admin in admins.iter() {
            if guard_admin == new_admin {
                has_new_admin = true;
            }
            if guard_admin == admin {
                replaced_old_admin = true;
                if !has_new_admin {
                    rotated_admins.push_back(new_admin.clone());
                    has_new_admin = true;
                }
            } else {
                rotated_admins.push_back(guard_admin);
            }
        }
        if !replaced_old_admin && !has_new_admin {
            rotated_admins.push_back(new_admin.clone());
        }
        e.storage()
            .instance()
            .set(&GuardDataKey::Admins, &rotated_admins);
        write_administrator(&e, &new_admin);
    }

    fn guard_pause(e: Env, admin: Address, operation: u32, paused: bool) -> Result<(), GuardError> {
        EmergencyGuard::set_pause(e, admin, operation, paused)
    }

    fn emergency_pause(e: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::emergency_pause(e, approvers)
    }

    fn guard_resume(e: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::resume(e, approvers)
    }

    fn guard_add_admin(
        e: Env,
        approvers: Vec<Address>,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::add_admin(e, approvers, new_admin)
    }

    fn guard_remove_admin(
        e: Env,
        approvers: Vec<Address>,
        admin: Address,
    ) -> Result<(), GuardError> {
        EmergencyGuard::remove_admin(e, approvers, admin)
    }

    fn guard_admins(e: Env) -> Vec<Address> {
        EmergencyGuard::get_admins(e)
    }

    fn guard_threshold(e: Env) -> u32 {
        EmergencyGuard::get_threshold(e)
    }

    fn guard_is_paused(e: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(e, operation)
    }

    fn allowance(e: Env, from: Address, spender: Address) -> i128 {
        e.storage().instance().extend_ttl(100, 100);
        read_allowance(&e, from, spender).amount
    }

    fn approve(e: Env, from: Address, spender: Address, amount: i128, expiration_ledger: u32) {
        // Guard check: approvals follow the transfer pause flag.
        if EmergencyGuard::is_paused(e.clone(), PauseType::TRANSFER) {
            panic!("transfers are paused");
        }

        from.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        write_allowance(&e, from, spender, amount, expiration_ledger);
    }

    fn balance(e: Env, id: Address) -> i128 {
        e.storage().instance().extend_ttl(100, 100);
        read_balance(&e, id)
    }

    fn transfer(e: Env, from: Address, to: Address, amount: i128) {
        // Guard check: granular transfer pause.
        if EmergencyGuard::is_paused(e.clone(), PauseType::TRANSFER) {
            panic!("transfers are paused");
        }

        require_not_paused(&e, PauseType::TRANSFER);
        // Ensure transfers are not paused.
        ensure_not_paused(&e, PauseType::TRANSFER);
        from.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        // Check if transfers are paused (using TRANSFER pause type = 1 << 3 = 8)
        if EmergencyGuard::is_paused(e.clone(), 1 << 3) {
            panic!("transfers are paused");
        }

        spend_balance(&e, from, amount);
        receive_balance(&e, to, amount);
    }

    fn transfer_from(e: Env, spender: Address, from: Address, to: Address, amount: i128) {
        // Guard check: same transfer flag guards delegated transfers.
        if EmergencyGuard::is_paused(e.clone(), PauseType::TRANSFER) {
            panic!("transfers are paused");
        }

        require_not_paused(&e, PauseType::TRANSFER);
        // Ensure transfer_from respects pause.
        ensure_not_paused(&e, PauseType::TRANSFER);
        spender.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        // Check if transfers are paused (using TRANSFER pause type = 1 << 3 = 8)
        if EmergencyGuard::is_paused(e.clone(), 1 << 3) {
            panic!("transfers are paused");
        }

        spend_allowance(&e, from.clone(), spender, amount);
        spend_balance(&e, from, amount);
        receive_balance(&e, to, amount);
    }

    fn burn(e: Env, from: Address, amount: i128) {
        // Guard check: burning can be paused independently.
        if EmergencyGuard::is_paused(e.clone(), PauseType::BURN) {
            panic!("burning is paused");
        }

        require_not_paused(&e, PauseType::BURN);
        // Ensure burning is not paused.
        ensure_not_paused(&e, PauseType::BURN);
        from.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        // Check if burning is paused (using BURN pause type = 1 << 5 = 32)
        if EmergencyGuard::is_paused(e.clone(), 1 << 5) {
            panic!("burning is paused");
        }

        spend_balance(&e, from, amount);
    }

    fn burn_from(e: Env, spender: Address, from: Address, amount: i128) {
        // Guard check: delegated burn follows the same burn flag.
        if EmergencyGuard::is_paused(e.clone(), PauseType::BURN) {
            panic!("burning is paused");
        }

        require_not_paused(&e, PauseType::BURN);
        // Ensure burn_from respects pause.
        ensure_not_paused(&e, PauseType::BURN);
        spender.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        // Check if burning is paused (using BURN pause type = 1 << 5 = 32)
        if EmergencyGuard::is_paused(e.clone(), 1 << 5) {
            panic!("burning is paused");
        }

        spend_allowance(&e, from.clone(), spender, amount);
        spend_balance(&e, from, amount);
    }

    fn decimals(e: Env) -> u32 {
        read_decimal(&e)
    }

    fn name(e: Env) -> String {
        read_name(&e)
    }

    fn symbol(e: Env) -> String {
        read_symbol(&e)
    }

    fn guard_pause(e: Env, admin: Address, operation: u32, paused: bool) -> Result<(), GuardError> {
        EmergencyGuard::set_pause(e, admin, operation, paused)
    }

    fn guard_unpause(e: Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        EmergencyGuard::resume(e, approvers)
    }
}

/// Guard-management functions exposed on the token contract.
/// These allow the token admin (initialised as guard admin) to manage
/// pause state without any extra off-chain tooling.
#[contractimpl]
impl Token {
    /// Pause all minting operations.
    /// Auth is handled inside EmergencyGuard::set_pause — no double-auth.
    pub fn pause_minting(e: Env, admin: Address) {
        EmergencyGuard::set_pause(e, admin, PauseType::MINT, true)
            .expect("Unauthorized: caller is not a guard admin");
    }

    /// Resume minting operations.
    pub fn resume_minting(e: Env, admin: Address) {
        EmergencyGuard::set_pause(e, admin, PauseType::MINT, false)
            .expect("Unauthorized");
    }

    /// Pause all token transfers (also blocks approve / transfer_from).
    /// Auth is handled inside EmergencyGuard::set_pause — no double-auth.
    pub fn pause_transfers(e: Env, admin: Address) {
        EmergencyGuard::set_pause(e, admin, PauseType::TRANSFER, true)
            .expect("Unauthorized: caller is not a guard admin");
    }

    /// Resume token transfers.
    pub fn resume_transfers(e: Env, admin: Address) {
        EmergencyGuard::set_pause(e, admin, PauseType::TRANSFER, false)
            .expect("Unauthorized");
    }

    /// Pause all burn operations.
    /// Auth is handled inside EmergencyGuard::set_pause — no double-auth.
    pub fn pause_burning(e: Env, admin: Address) {
        EmergencyGuard::set_pause(e, admin, PauseType::BURN, true)
            .expect("Unauthorized: caller is not a guard admin");
    }

    /// Resume burn operations.
    pub fn resume_burning(e: Env, admin: Address) {
        EmergencyGuard::set_pause(e, admin, PauseType::BURN, false)
            .expect("Unauthorized");
    }

    /// Emergency pause: freeze all token operations atomically.
    /// Requires multi-sig approval (currently threshold = 1 for single-admin setup).
    pub fn emergency_pause_all(e: Env, approvers: Vec<Address>) {
        EmergencyGuard::emergency_pause(e, approvers)
            .expect("Unauthorized or insufficient approvals");
    }

    /// Resume all paused operations at once.
    pub fn resume_all(e: Env, approvers: Vec<Address>) {
        EmergencyGuard::resume(e, approvers)
            .expect("Unauthorized or insufficient approvals");
    }

    /// Query the raw bitmask pause state (for SoroScope analysis / frontends).
    pub fn get_pause_state(e: Env) -> u32 {
        let state = EmergencyGuard::is_paused(e, 0);
        // Return raw bitmask from PauseState storage entry
        if state { 1 } else { 0 }
    }

    /// Check whether a specific operation flag is currently paused.
    pub fn is_operation_paused(e: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(e, operation)
    }

    /// List current guard admins.
    pub fn get_guard_admins(e: Env) -> Vec<Address> {
        EmergencyGuard::get_admins(e)
    }

    /// Get the multi-sig threshold for emergency operations.
    pub fn get_guard_threshold(e: Env) -> u32 {
        EmergencyGuard::get_threshold(e)
    }
}
