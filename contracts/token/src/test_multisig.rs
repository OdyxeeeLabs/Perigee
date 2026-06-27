//! Multi-sig threshold tests for the token contract's embedded EmergencyGuard.

use crate::contract::{Token, TokenClient};
use emergency_guard::PauseType;
use soroban_sdk::{testutils::Address as _, vec, Address, Env, String};

fn setup_token<'a>(
    env: &'a Env,
    admins: &[Address],
    threshold: u32,
) -> (TokenClient<'a>, Address) {
    env.mock_all_auths();
    let contract_id = env.register(Token, ());
    let client = TokenClient::new(env, &contract_id);
    let mut guard_admins = vec![env];
    for admin in admins {
        guard_admins.push_back(admin.clone());
    }
    client.initialize_multisig(
        &admins[0],
        &7,
        &String::from_str(env, "MultiSig Token"),
        &String::from_str(env, "MST"),
        &guard_admins,
        &threshold,
    );
    (client, contract_id)
}

#[test]
fn test_multisig_2_of_3_pause_succeeds() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone(), a3.clone()], 2);

    let approvers = vec![&env, a1.clone(), a2.clone()];
    client.submit_emergency_pause_all(&approvers);

    assert!(client.guard_is_paused(&PauseType::MINT));
    assert!(client.guard_is_paused(&PauseType::TRANSFER));
    assert!(client.guard_is_paused(&PauseType::BURN));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_multisig_2_of_3_pause_fails_insufficient() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone(), a3.clone()], 2);

    let approvers = vec![&env, a1.clone()];
    client.submit_emergency_pause_all(&approvers);
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_multisig_3_of_3_all_required() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone(), a3.clone()], 3);

    let two_approvers = vec![&env, a1.clone(), a2.clone()];
    client.submit_emergency_pause_all(&two_approvers);
}

#[test]
fn test_multisig_3_of_3_succeeds_with_all_admins() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone(), a3.clone()], 3);

    let all_approvers = vec![&env, a1.clone(), a2.clone(), a3.clone()];
    client.submit_emergency_pause_all(&all_approvers);
    assert!(client.guard_is_paused(&PauseType::MINT));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_multisig_add_admin_requires_threshold() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone()], 2);

    let single = vec![&env, a1.clone()];
    client.submit_add_admin(&single, &new_admin);
}

#[test]
fn test_multisig_add_admin_succeeds_with_threshold() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone()], 2);

    let approvers = vec![&env, a1.clone(), a2.clone()];
    client.submit_add_admin(&approvers, &new_admin);
    assert!(client.guard_admins().iter().any(|a| a == new_admin));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_multisig_remove_admin_requires_threshold() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone(), a3.clone()], 2);

    let single = vec![&env, a1.clone()];
    client.submit_remove_admin(&single, &a3);
}

#[test]
fn test_multisig_remove_admin_succeeds_with_threshold() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let a3 = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone(), a3.clone()], 2);

    let approvers = vec![&env, a1.clone(), a2.clone()];
    client.submit_remove_admin(&approvers, &a3);
    assert!(!client.guard_admins().iter().any(|a| a == a3));
}

#[test]
fn test_multisig_resume_after_pause() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone()], 2);

    let approvers = vec![&env, a1.clone(), a2.clone()];
    client.submit_emergency_pause_all(&approvers);
    assert!(client.guard_is_paused(&PauseType::MINT));

    client.submit_resume_all(&approvers);
    assert!(!client.guard_is_paused(&PauseType::MINT));
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_multisig_duplicate_approvers_rejected() {
    let env = Env::default();
    let a1 = Address::generate(&env);
    let a2 = Address::generate(&env);
    let (client, _) = setup_token(&env, &[a1.clone(), a2.clone()], 2);

    let approvers = vec![&env, a1.clone(), a1.clone()];
    client.submit_emergency_pause_all(&approvers);
}
