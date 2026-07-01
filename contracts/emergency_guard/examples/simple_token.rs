#![no_std]
use emergency_guard::{EmergencyGuard, PauseType};
use soroban_sdk::{contract, contractimpl, contracttype, vec, Address, Env, Vec};

#[contracttype]
pub enum DataKey {
    Admin,
    TotalSupply,
    Balance(Address),
    Allowance(AllowanceKey),
}

#[contracttype]
pub struct AllowanceKey {
    from: Address,
    to: Address,
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
}

#[cfg(not(target_family = "wasm"))]
fn main() {}
