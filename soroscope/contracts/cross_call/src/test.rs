use super::{ContractA, ContractAClient, ContractB};
use soroban_sdk::Env;

#[test]
fn test_cross_contract_call() {
    let env = Env::default();
    let contract_b_id = env.register(ContractB, ());
    let contract_a_id = env.register(ContractA, ());

    let client_a = ContractAClient::new(&env, &contract_a_id);
    let result = client_a.call_b(&contract_b_id, &41);

    assert_eq!(result, 42);
}
