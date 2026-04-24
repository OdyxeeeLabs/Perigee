// Example contract showing how to use EmergencyGuard
// This demonstrates a simple token contract with pause functionality

use soroban_sdk::{contract, contractimpl, contracttype, Address, Env, Vec, vec, String};
use emergency_guard::{DefaultEmergencyGuard, PauseType, GuardError};

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
    /// Initialize the token with admin and emergency guard
    /// 
    /// # Arguments
    /// * `env` - Soroban environment
    /// * `admin` - Address of the token admin
    /// * `initial_supply` - Initial token supply
    pub fn initialize(env: Env, admin: Address, initial_supply: i128) {
        admin.require_auth();

        // Store admin
        env.storage().instance().set(&DataKey::Admin, &admin);

        // Store initial supply
        env.storage().instance().set(&DataKey::TotalSupply, &initial_supply);

        // Mint initial supply to admin
        env.storage()
            .instance()
            .set(&DataKey::Balance(admin.clone()), &initial_supply);

        // Initialize emergency guard with single admin, threshold of 1
        let admins = vec![&env, admin];
        DefaultEmergencyGuard::init_guard(&env, admins, 1)
            .expect("Failed to initialize emergency guard");
    }

    /// Transfer tokens (blocked if TRANSFER pause is active)
    pub fn transfer(env: Env, from: Address, to: Address, amount: i128) {
        // Check if transfers are paused
        DefaultEmergencyGuard::check_not_paused(&env, PauseType::TRANSFER)
            .expect("Transfers are paused");

        from.require_auth();

        let balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(from.clone()))
            .unwrap_or(0);

        assert!(balance >= amount, "Insufficient balance");

        env.storage().instance().set(
            &DataKey::Balance(from.clone()),
            &(balance - amount),
        );

        let to_balance: i128 = env
            .storage()
            .instance()
            .get(&DataKey::Balance(to.clone()))
            .unwrap_or(0);

        env.storage()
            .instance()
            .set(&DataKey::Balance(to), &(to_balance + amount));
    }

    /// Mint tokens (blocked if MINT pause is active)
    pub fn mint(env: Env, to: Address, amount: i128) {
        // Check if minting is paused
        DefaultEmergencyGuard::check_not_paused(&env, PauseType::MINT)
            .expect("Minting is paused");

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

    /// Burn tokens (blocked if BURN pause is active)
    pub fn burn(env: Env, from: Address, amount: i128) {
        // Check if burning is paused
        DefaultEmergencyGuard::check_not_paused(&env, PauseType::BURN)
            .expect("Burning is paused");

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

    // ==== EMERGENCY GUARD FUNCTIONS ====

    /// Pause only transfers (minting and burning still work)
    pub fn pause_transfers(env: Env) {
        DefaultEmergencyGuard::set_pause_state(&env, PauseType::TRANSFER, true)
            .expect("Unauthorized");
    }

    /// Resume transfers
    pub fn resume_transfers(env: Env) {
        DefaultEmergencyGuard::set_pause_state(&env, PauseType::TRANSFER, false)
            .expect("Unauthorized");
    }

    /// Pause only minting
    pub fn pause_minting(env: Env) {
        DefaultEmergencyGuard::set_pause_state(&env, PauseType::MINT, true)
            .expect("Unauthorized");
    }

    /// Resume minting
    pub fn resume_minting(env: Env) {
        DefaultEmergencyGuard::set_pause_state(&env, PauseType::MINT, false)
            .expect("Unauthorized");
    }

    /// Pause only burning
    pub fn pause_burning(env: Env) {
        DefaultEmergencyGuard::set_pause_state(&env, PauseType::BURN, true)
            .expect("Unauthorized");
    }

    /// Resume burning
    pub fn resume_burning(env: Env) {
        DefaultEmergencyGuard::set_pause_state(&env, PauseType::BURN, false)
            .expect("Unauthorized");
    }

    /// Emergency: pause all operations
    pub fn emergency_pause_all(env: Env) {
        DefaultEmergencyGuard::emergency_pause_all(&env)
            .expect("Unauthorized");
    }

    /// Resume all operations
    pub fn resume_all(env: Env) {
        DefaultEmergencyGuard::resume_all(&env)
            .expect("Unauthorized");
    }

    /// Get current pause state (bitmask)
    pub fn get_pause_state(env: Env) -> u32 {
        DefaultEmergencyGuard::get_pause_state(&env)
    }

    /// Check if specific operation is paused
    pub fn is_paused(env: Env, operation: u32) -> bool {
        let state = DefaultEmergencyGuard::get_pause_state(&env);
        let pause_type = PauseType::new(state);
        pause_type.is_paused(operation)
    }

    /// Get list of admins
    pub fn get_admins(env: Env) -> Vec<Address> {
        DefaultEmergencyGuard::get_admins(&env)
    }

    /// Get multi-sig threshold
    pub fn get_threshold(env: Env) -> u32 {
        DefaultEmergencyGuard::get_threshold(&env)
    }

    /// Add new admin (requires existing admin authorization)
    pub fn add_admin(env: Env, new_admin: Address) {
        DefaultEmergencyGuard::add_admin(&env, new_admin)
            .expect("Unauthorized or threshold would be violated");
    }

    /// Remove admin
    pub fn remove_admin(env: Env, admin: Address) {
        DefaultEmergencyGuard::remove_admin(&env, admin)
            .expect("Unauthorized or threshold would be violated");
    }

    /// Rotate admin (current admin transfers authority to new admin)
    pub fn rotate_admin(env: Env, new_admin: Address) {
        DefaultEmergencyGuard::rotate_admin(&env, new_admin)
            .expect("Unauthorized");
    }

    // ==== READ-ONLY FUNCTIONS ====

    /// Get token balance for an address
    pub fn balance(env: Env, addr: Address) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::Balance(addr))
            .unwrap_or(0)
    }

    /// Get total supply
    pub fn total_supply(env: Env) -> i128 {
        env.storage()
            .instance()
            .get(&DataKey::TotalSupply)
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::testutils::Address as _;

    #[test]
    fn test_initialize() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.mock_all_auths();

        let contract_id = env.register_contract(None, SimpleToken);
        let client = SimpleTokenClient::new(&env, &contract_id);

        client.initialize(&admin, &1000);

        assert_eq!(client.total_supply(), 1000);
        assert_eq!(client.balance(&admin), 1000);
    }

    #[test]
    fn test_transfer() {
        let env = Env::default();
        let admin = Address::random(&env);
        let user = Address::random(&env);

        env.mock_all_auths();

        let contract_id = env.register_contract(None, SimpleToken);
        let client = SimpleTokenClient::new(&env, &contract_id);

        client.initialize(&admin, &1000);

        // Transfer from admin to user
        client.transfer(&admin, &user, &100);

        assert_eq!(client.balance(&admin), 900);
        assert_eq!(client.balance(&user), 100);
    }

    #[test]
    fn test_mint() {
        let env = Env::default();
        let admin = Address::random(&env);
        let user = Address::random(&env);

        env.mock_all_auths();

        let contract_id = env.register_contract(None, SimpleToken);
        let client = SimpleTokenClient::new(&env, &contract_id);

        client.initialize(&admin, &1000);

        // Mint to user
        client.mint(&user, &500);

        assert_eq!(client.balance(&user), 500);
        assert_eq!(client.total_supply(), 1500);
    }

    #[test]
    fn test_burn() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.mock_all_auths();

        let contract_id = env.register_contract(None, SimpleToken);
        let client = SimpleTokenClient::new(&env, &contract_id);

        client.initialize(&admin, &1000);

        // Burn from admin
        client.burn(&admin, &100);

        assert_eq!(client.balance(&admin), 900);
        assert_eq!(client.total_supply(), 900);
    }

    #[test]
    fn test_granular_pause_transfers() {
        let env = Env::default();
        let admin = Address::random(&env);
        let user1 = Address::random(&env);
        let user2 = Address::random(&env);

        env.mock_all_auths();

        let contract_id = env.register_contract(None, SimpleToken);
        let client = SimpleTokenClient::new(&env, &contract_id);

        client.initialize(&admin, &1000);

        // Transfer to user1 works
        client.transfer(&admin, &user1, &100);
        assert_eq!(client.balance(&user1), 100);

        // Pause transfers only
        client.pause_transfers();
        assert!(client.is_paused(PauseType::TRANSFER));

        // Minting still works
        client.mint(&user2, &100);
        assert_eq!(client.balance(&user2), 100);

        // Burning still works
        client.burn(&admin, &10);
        assert_eq!(client.balance(&admin), 890);

        // But transfers should fail
        // Note: In real test, we'd check for error instead of panic
    }

    #[test]
    fn test_emergency_pause_all() {
        let env = Env::default();
        let admin = Address::random(&env);

        env.mock_all_auths();

        let contract_id = env.register_contract(None, SimpleToken);
        let client = SimpleTokenClient::new(&env, &contract_id);

        client.initialize(&admin, &1000);

        // Everything works initially
        assert!(!client.is_paused(PauseType::TRANSFER));
        assert!(!client.is_paused(PauseType::MINT));
        assert!(!client.is_paused(PauseType::BURN));

        // Emergency pause all
        client.emergency_pause_all();

        // Everything is paused
        assert!(client.is_paused(PauseType::TRANSFER));
        assert!(client.is_paused(PauseType::MINT));
        assert!(client.is_paused(PauseType::BURN));

        // Resume all
        client.resume_all();

        // Nothing is paused
        assert!(!client.is_paused(PauseType::TRANSFER));
        assert!(!client.is_paused(PauseType::MINT));
        assert!(!client.is_paused(PauseType::BURN));
    }

    #[test]
    fn test_admin_rotation() {
        let env = Env::default();
        let admin1 = Address::random(&env);
        let admin2 = Address::random(&env);

        env.mock_all_auths();

        let contract_id = env.register_contract(None, SimpleToken);
        let client = SimpleTokenClient::new(&env, &contract_id);

        client.initialize(&admin1, &1000);

        // Verify admin1 is in admins list
        let admins = client.get_admins();
        assert!(admins.contains(&admin1));

        // Rotate admin
        client.rotate_admin(&admin2);

        // Verify admin2 is now in admins list
        let admins = client.get_admins();
        assert!(admins.contains(&admin2));
        assert!(!admins.contains(&admin1));

        // admin2 can now control pause functions
        client.pause_transfers();
        assert!(client.is_paused(PauseType::TRANSFER));
    }
}
