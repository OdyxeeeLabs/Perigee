#![no_std]

use soroban_sdk::{contract, contractimpl, contracttype, Address, BytesN, Env, Vec, Bytes};

#[contracttype]
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DataKey {
    Admin,
    StateRoot(u32), // block height mapped to state root
}

#[contract]
pub struct CrossChainVerifier;

#[contractimpl]
impl CrossChainVerifier {
    /// Initialize the contract with an admin who has the right to update state roots.
    pub fn initialize(env: Env, admin: Address) {
        if env.storage().instance().has(&DataKey::Admin) {
            panic!("already initialized");
        }
        env.storage().instance().set(&DataKey::Admin, &admin);
    }

    /// Update the state root for a specific block height.
    /// Only the admin (relayer network) can perform this action.
    pub fn update_root(env: Env, block_height: u32, new_root: BytesN<32>) {
        let admin: Address = env.storage().instance().get(&DataKey::Admin).unwrap();
        admin.require_auth();
        
        env.storage().persistent().set(&DataKey::StateRoot(block_height), &new_root);
    }

    /// Retrieve a stored state root by block height.
    pub fn get_root(env: Env, block_height: u32) -> Option<BytesN<32>> {
        env.storage().persistent().get(&DataKey::StateRoot(block_height))
    }

    /// Verifies a Binary Merkle Tree proof.
    /// In a cross-chain context, this allows proving that a specific message or transaction
    /// (the `leaf`) was included in the block matching `block_height` state root.
    /// 
    /// * `block_height`: The block height of the state root to verify against.
    /// * `leaf`: The hash of the cross-chain message to be verified.
    /// * `proof`: A list of sibling hashes forming the Merkle proof.
    /// * `proof_flags`: A list of booleans indicating if the sibling is on the left (true) or right (false).
    pub fn verify_message(
        env: Env,
        block_height: u32,
        leaf: BytesN<32>,
        proof: Vec<BytesN<32>>,
        proof_flags: Vec<bool>,
    ) -> bool {
        let expected_root: BytesN<32> = env
            .storage()
            .persistent()
            .get(&DataKey::StateRoot(block_height))
            .unwrap_or_else(|| panic!("State root not found"));

        if proof.len() != proof_flags.len() {
            panic!("Invalid proof format");
        }

        let mut current_hash = leaf.to_array();

        for i in 0..proof.len() {
            let sibling = proof.get(i).unwrap().to_array();
            let is_left_sibling = proof_flags.get(i).unwrap();

            let mut combined = [0u8; 64];
            if is_left_sibling {
                combined[0..32].copy_from_slice(&sibling);
                combined[32..64].copy_from_slice(&current_hash);
            } else {
                combined[0..32].copy_from_slice(&current_hash);
                combined[32..64].copy_from_slice(&sibling);
            }
            
            // Compute sha256 of the combined 64 bytes
            let combined_bytes = Bytes::from_slice(&env, &combined);
            current_hash = env.crypto().sha256(&combined_bytes).to_array();
        }

        let computed_root = BytesN::from_array(&env, &current_hash);
        computed_root == expected_root
    }
}

mod test;
