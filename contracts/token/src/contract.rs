use crate::admin::{has_administrator, read_administrator, write_administrator};
use crate::allowance::{read_allowance, spend_allowance, write_allowance};
use crate::balance::{read_balance, receive_balance, spend_balance};
use crate::metadata::{read_decimal, read_name, read_symbol, write_metadata};
use emergency_guard::{EmergencyGuard, GuardError};
use soroban_sdk::{contract, contractimpl, Address, Env, String, Vec};

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
        // One write instead of three separate writes for name/symbol/decimals.
        write_metadata(&e, &name, &symbol, decimal);
        
        // Initialize emergency guard with single admin and threshold of 1
        let admins = vec![&e, admin.clone()];
        let threshold = 1;
        EmergencyGuard::initialize(e, admins, threshold)
            .expect("Failed to initialize emergency guard");
    }

    fn mint(e: Env, to: Address, amount: i128) {
        let admin = read_administrator(&e);
        admin.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        // Check if minting is paused (using MINT pause type = 1 << 4 = 16)
        if EmergencyGuard::is_paused(e.clone(), 1 << 4) {
            panic!("minting is paused");
        }

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
        from.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        write_allowance(&e, from, spender, amount, expiration_ledger);
    }

    fn balance(e: Env, id: Address) -> i128 {
        e.storage().instance().extend_ttl(100, 100);
        read_balance(&e, id)
    }

    fn transfer(e: Env, from: Address, to: Address, amount: i128) {
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
        from.require_auth();
        e.storage().instance().extend_ttl(100, 100);

        // Check if burning is paused (using BURN pause type = 1 << 5 = 32)
        if EmergencyGuard::is_paused(e.clone(), 1 << 5) {
            panic!("burning is paused");
        }

        spend_balance(&e, from, amount);
    }

    fn burn_from(e: Env, spender: Address, from: Address, amount: i128) {
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
