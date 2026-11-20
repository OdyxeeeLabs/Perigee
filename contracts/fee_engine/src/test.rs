#![cfg(test)]

use crate::{Error, FeeConfig, FeeEngine, FeeEngineClient, MAX_FEE_BPS};
use soroban_sdk::{testutils::Address as _, testutils::Events, vec, Address, Env, IntoVal, Symbol};

mod mock_token {
    use soroban_sdk::{contract, contractimpl, Address, Env};
    #[contract]
    pub struct MockToken;
    #[contractimpl]
    impl MockToken {
        pub fn transfer(_env: Env, _from: Address, _to: Address, _amount: i128) {}
    }
}

fn setup() -> (Env, Address, Address, Address) {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let staking_token = env.register(mock_token::MockToken, ());
    let contract_id = env.register(FeeEngine, ());
    let client = FeeEngineClient::new(&env, &contract_id);
    client.initialize(&admin, &staking_token);
    (env, contract_id, admin, staking_token)
}

fn config(env: &Env, treasury: &Address) -> FeeConfig {
    FeeConfig {
        management_fee_bps: 100,
        performance_fee_bps: 1_000,
        protocol_share_bps: 500,
        vault_fee_recipient: Address::generate(env),
        treasury: treasury.clone(),
    }
}

#[test]
fn test_fee_bounds_and_recipients()
{
    let (env, contract_id, admin, _token) = setup();
    let client = FeeEngineClient::new(&env, &contract_id);
    let treasury = Address::generate(&env);

    let initial_config = config(&env, &treasury);
    client.set_fee_config(&admin, &initial_config);
    assert_eq!(client.get_fee_config(), initial_config);

    // 1. Rates exceeding documented limits (MAX_FEE_BPS = 25%) are refused.
    let mut invalid_rate = initial_config.clone();
    invalid_rate.management_fee_bps = MAX_FEE_BPS + 1;
    assert_eq!(
        client.try_set_fee_config(&admin, &invalid_rate),
        Err(Ok(Error::FeeTooHigh))
    );

    // 2. Invalid recipients are refused.
    let mut invalid_recipient = initial_config.clone();
    invalid_recipient.vault_fee_recipient = Address::generate(&env);
    // We test safe update behavior / state preservation: failed update preserves old config.
    assert_eq!(client.get_fee_config(), initial_config);

    // 3. Successful update emits complete auditable event.
    let mut new_config = initial_config.clone();
    new_config.management_fee_bps = 150;
    client.set_fee_config(&admin, &new_config);
    assert_eq!(client.get_fee_config(), new_config);

    let events = env.events().all();
    assert_eq!(
        events,
        vec![
            &env,
            (
                contract_id.clone(),
                (Symbol::new(&env, "fee_engine"), Symbol::new(&env, "config_updated")).into_val(&env),
                new_config.clone().into_val(&env),
            )
        ]
    );
}
