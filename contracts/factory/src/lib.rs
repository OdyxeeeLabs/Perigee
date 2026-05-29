#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, xdr::ToXdr, Address, BytesN, Env, IntoVal, Vec,
};

/// Storage key for pair registry and multi-sig admin data.
/// Stored in **instance** storage because the factory is a singleton contract
/// and pair mappings are global state that should share the contract's TTL.
/// Using instance storage avoids per-entry persistent rent and reduces the
/// ledger footprint to a single entry per invocation.
#[contracttype]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DataKey {
    Pair(Address, Address),
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

#[contractimpl]
impl LiquidityPoolFactory {
    /// Deploys a new Liquidity Pool contract for a unique pair of tokens.
    pub fn create_pair(
        env: Env,
        token_a: Address,
        token_b: Address,
        wasm_hash: BytesN<32>,
    ) -> Address {
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
            panic!("Pair already exists");
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

        // One instance write instead of one persistent write.
        env.storage()
            .instance()
            .set(&DataKey::Pair(token_0, token_1), &deployed_address);

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
