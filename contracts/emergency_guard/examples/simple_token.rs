#![no_std]
use emergency_guard::{EmergencyGuard, PauseType};
use soroban_sdk::{contract, contractimpl, contracttype, vec, Address, Env, Vec};

#[contracttype]
pub enum DataKey {
    Admin,
    TotalSupply,
    Balance(Address),
}

#[contract]
pub struct SimpleToken;

#[contractimpl]
impl SimpleToken {
    pub fn initialize(env: Env, admin: Address, initial_supply: i128) {
        admin.require_auth();

        env.storage().instance().set(&DataKey::Admin, &admin);
        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &initial_supply);
        env.storage()
            .instance()
            .set(&DataKey::Balance(admin.clone()), &initial_supply);

        let admins = vec![&env, admin];
        EmergencyGuard::initialize(env.clone(), admins, 1).expect("Failed to init guard");
    }

    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        if EmergencyGuard::is_paused(env.clone(), PauseType::TRANSFER) {
            panic!("Transfers are paused");
        }

        from.require_auth();

        let balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(from.clone()))
            .unwrap_or(0);
        assert!(balance >= amount, "Insufficient balance");

        env.storage()
            .instance()
            .set(&DataKey::Balance(from.clone()), &(balance - amount));

        let to_balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(to.clone()))
            .unwrap_or(0);
        env.storage()
            .instance()
            .set(&DataKey::Balance(to), &(to_balance + amount));
    }

    pub fn mint(env: Env, to: Address, amount: i128) {
        if EmergencyGuard::is_paused(env.clone(), PauseType::MINT) {
            panic!("Minting is paused");
        }

        let admin: Address = env
            .storage()
            .instance()
            .get(&DataKey::Admin)
            .expect("Admin not found");

        admin.require_auth();

        let balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(to.clone()))
            .unwrap_or(0);

        env.storage()
            .instance()
            .set(&DataKey::Balance(to), &(balance + amount));

        let supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);

        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(supply + amount));
    }

    pub fn burn(env: Env, from: Address, amount: i128) {
        if EmergencyGuard::is_paused(env.clone(), PauseType::BURN) {
            panic!("Burning is paused");
        }

        from.require_auth();

        let balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(from.clone()))
            .unwrap_or(0);

        assert!(balance >= amount, "Insufficient balance");

        env.storage()
            .instance()
            .set(&DataKey::Balance(from), &(balance - amount));

        let supply: i128 = env
            .storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0);

        env.storage()
            .instance()
            .set(&DataKey::TotalSupply, &(supply - amount));
    }

    pub fn get_pause_state(env: Env) -> u32 {
        EmergencyGuard::get_pause_state(env)
    }

    pub fn is_paused(env: Env, operation: u32) -> bool {
        EmergencyGuard::is_paused(env, operation)
    }

    pub fn get_admins(env: Env) -> Vec<Address> {
        EmergencyGuard::get_admins(env)
    }

    pub fn get_threshold(env: Env) -> u32 {
        EmergencyGuard::get_threshold(env)
    }

    pub fn balance(env: Env, addr: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Balance(addr))
            .unwrap_or(0)
    }

    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }
}

#[cfg(not(target_family = "wasm"))]
fn main() {}
