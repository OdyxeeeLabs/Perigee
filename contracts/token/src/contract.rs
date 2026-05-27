#![allow(dead_code)]

use crate::admin::{has_administrator, read_administrator, write_administrator};
use crate::allowance::{read_allowance, spend_allowance, write_allowance};
use crate::balance::{read_balance, receive_balance, spend_balance};
use crate::metadata::{read_decimal, read_name, read_symbol, write_metadata};
use emergency_guard::{EmergencyGuard, PauseType};
use soroban_sdk::{contract, contractimpl, vec, Address, Env, String, Vec};

pub trait TokenTrait {
    fn initialize(e: Env, admin: Address, decimal: u32, name: String, symbol: String);
    fn mint(e: Env, to: Address, amount: i128);
    fn set_admin(e: Env, new_admin: Address);
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
        // One write instead of three separate writes for name/symbol/decimals.
        write_metadata(&e, &name, &symbol, decimal);

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

        let admin = read_administrator(&e);
        admin.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        receive_balance(&e, to, amount);
    }

    fn set_admin(e: Env, new_admin: Address) {
        let admin = read_administrator(&e);
        admin.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        write_administrator(&e, &new_admin);
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

        from.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        spend_balance(&e, from, amount);
        receive_balance(&e, to, amount);
    }

    fn transfer_from(e: Env, spender: Address, from: Address, to: Address, amount: i128) {
        // Guard check: same transfer flag guards delegated transfers.
        if EmergencyGuard::is_paused(e.clone(), PauseType::TRANSFER) {
            panic!("transfers are paused");
        }

        spender.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        spend_allowance(&e, from.clone(), spender, amount);
        spend_balance(&e, from, amount);
        receive_balance(&e, to, amount);
    }

    fn burn(e: Env, from: Address, amount: i128) {
        // Guard check: burning can be paused independently.
        if EmergencyGuard::is_paused(e.clone(), PauseType::BURN) {
            panic!("burning is paused");
        }

        from.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        spend_balance(&e, from, amount);
    }

    fn burn_from(e: Env, spender: Address, from: Address, amount: i128) {
        // Guard check: delegated burn follows the same burn flag.
        if EmergencyGuard::is_paused(e.clone(), PauseType::BURN) {
            panic!("burning is paused");
        }

        spender.require_auth();
        e.storage().instance().extend_ttl(100, 100);

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
