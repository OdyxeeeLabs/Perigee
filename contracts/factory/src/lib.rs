#![no_std]
use soroban_sdk::{
    contract, contractimpl, contracttype, xdr::ToXdr, Address, BytesN, Env, IntoVal,
};

#[contracttype]
pub enum DataKey {
    Pair(Address, Address), // (TokenA, TokenB) -> PoolAddress
}

#[contract]
pub struct LiquidityPoolFactory;

#[contractimpl]
impl LiquidityPoolFactory {
    // create_pair deploys a new Liquidity Pool contract for a unique pair of tokens.
    // Use `wasm_hash` to specify which contract to deploy (should be the hash of the compiled LP contract).
    pub fn create_pair(
        env: Env,
        token_a: Address,
        token_b: Address,
        wasm_hash: BytesN<32>,
    ) -> Address {
        // 1. Sort tokens to ensure uniqueness (A-B is same as B-A)
        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        // 2. check if pair already exists
        if env
            .storage()
            .persistent()
            .has(&DataKey::Pair(token_0.clone(), token_1.clone()))
        {
            panic!("Pair already exists");
        }

        // 3. Deploy the contract using the Salt
        // We use the pair (token_0, token_1) as entropy for the salt to ensure deterministic addresses
        let salt = env
            .crypto()
            .sha256(&(token_0.clone(), token_1.clone()).to_xdr(&env));

        // 4. Initialize the deployed contract
        let deployed_address = env
            .deployer()
            .with_current_contract(salt)
            .deploy_v2(wasm_hash, soroban_sdk::Vec::<soroban_sdk::Val>::new(&env));

        // Call `initialize` on the newly deployed LP contract.
        // LP signature: fn initialize(e: Env, admin: Address, token_a: Address, token_b: Address)
        // The factory itself acts as the pool admin.
        let admin = env.current_contract_address();
        let init_args = soroban_sdk::vec![
            &env,
            admin.into_val(&env),
            token_0.clone().into_val(&env),
            token_1.clone().into_val(&env)
        ];

        // Invoke the initialize function.
        let _res: () = env.invoke_contract(
            &deployed_address,
            &soroban_sdk::Symbol::new(&env, "initialize"),
            init_args,
        );

        // 5. Store the pair mapping
        env.storage()
            .persistent()
            .set(&DataKey::Pair(token_0, token_1), &deployed_address);

        deployed_address
    }

    // get_pair returns the address of the pool for the given tokens, if it exists.
    pub fn get_pair(env: Env, token_a: Address, token_b: Address) -> Option<Address> {
        let (token_0, token_1) = if token_a < token_b {
            (token_a, token_b)
        } else {
            (token_b, token_a)
        };

        env.storage()
            .persistent()
            .get(&DataKey::Pair(token_0, token_1))
    }
}

mod test;
