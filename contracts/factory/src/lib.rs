#![no_std]
use emergency_guard::{DefaultEmergencyGuard, EmergencyGuardTrait, GuardError};
#[cfg(test)]
use soroban_sdk::testutils::Address as _;
use soroban_sdk::{
    contract, contracterror, contractimpl, contracttype, xdr::ToXdr, Address, BytesN, Env, IntoVal,
    Vec,
};

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

const PAUSE_CREATE_PAIR_FLAG: u32 = 1 << 6;

#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    Pair(Address, Address),
    Admin,
}

#[contract]
pub struct LiquidityPoolFactory;

#[contractimpl]
impl EmergencyGuardTrait for LiquidityPoolFactory {
    fn check_not_paused(env: &Env, operation: u32) -> Result<(), GuardError> {
        DefaultEmergencyGuard::check_not_paused(env, operation)
    }

    fn get_pause_state(env: &Env) -> u32 {
        DefaultEmergencyGuard::get_pause_state(env)
    }

    fn set_pause_state(env: &Env, operation: u32, paused: bool) -> Result<(), GuardError> {
        DefaultEmergencyGuard::set_pause_state(env, operation, paused)
    }

    fn unpause(env: &Env, operation: u32) -> Result<(), GuardError> {
        DefaultEmergencyGuard::unpause(env, operation)
    }

    fn unpause_all(env: &Env) -> Result<(), GuardError> {
        DefaultEmergencyGuard::unpause_all(env)
    }

    fn emergency_pause_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        DefaultEmergencyGuard::emergency_pause_all(env, approvers)
    }

    fn resume_all(env: &Env, approvers: Vec<Address>) -> Result<(), GuardError> {
        DefaultEmergencyGuard::resume_all(env, approvers)
    }

    fn init_guard(env: &Env, admins: Vec<Address>, threshold: u32) -> Result<(), GuardError> {
        DefaultEmergencyGuard::init_guard(env, admins, threshold)
    }

    fn add_admin(env: &Env, approvers: Vec<Address>, new_admin: Address) -> Result<(), GuardError> {
        DefaultEmergencyGuard::add_admin(env, approvers, new_admin)
    }

    fn remove_admin(env: &Env, approvers: Vec<Address>, admin: Address) -> Result<(), GuardError> {
        DefaultEmergencyGuard::remove_admin(env, approvers, admin)
    }

    fn rotate_admin(
        env: &Env,
        approvers: Vec<Address>,
        old_admin: Address,
        new_admin: Address,
    ) -> Result<(), GuardError> {
        DefaultEmergencyGuard::rotate_admin(env, approvers, old_admin, new_admin)
    }

    fn get_admins(env: &Env) -> Vec<Address> {
        DefaultEmergencyGuard::get_admins(env)
    }

    fn get_threshold(env: &Env) -> u32 {
        DefaultEmergencyGuard::get_threshold(env)
    }

    fn is_admin(env: &Env, addr: Address) -> bool {
        DefaultEmergencyGuard::is_admin(env, addr)
    }
}

#[contractimpl]
impl LiquidityPoolFactory {
    /// Initializes the factory contract with an admin and setup the emergency guard.
    pub fn initialize(env: Env, admin: Address) -> Result<(), Error> {
        if env.storage().instance().has(&DataKey::Admin) {
            return Err(Error::AlreadyInitialized);
        }
        env.storage().instance().set(&DataKey::Admin, &admin);

        let mut admins = Vec::new(&env);
        admins.push_back(admin);
        DefaultEmergencyGuard::init_guard(&env, admins, 1).map_err(|_| Error::Unauthorized)?;

        Ok(())
    }

    pub fn create_pair(
        env: Env,
        token_a: Address,
        token_b: Address,
        wasm_hash: BytesN<32>,
    ) -> Result<Address, Error> {
        DefaultEmergencyGuard::check_not_paused(&env, PAUSE_CREATE_PAIR_FLAG)
            .map_err(|_| Error::Paused)?;

        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        if env
            .storage()
            .instance()
            .has(&DataKey::Pair(token_0.clone(), token_1.clone()))
        {
            return Err(Error::PairAlreadyExists);
        }

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

        env.storage()
            .instance()
            .set(&DataKey::Pair(token_0, token_1), &deployed_address);
        Ok(deployed_address)
    }

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
}

mod test;
