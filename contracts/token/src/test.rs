use crate::contract::{Token, TokenClient};
use emergency_guard::{GuardError, PauseType};
use soroban_sdk::{testutils::Address as _, vec, Address, Env, String};

#[test]
fn test_mint_and_transfer() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);

    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Test Token"),
        &String::from_str(&env, "TEST"),
    );

    client.mint(&user1, &1000);
    assert_eq!(client.balance(&user1), 1000);

    client.transfer(&user1, &user2, &200);
    assert_eq!(client.balance(&user1), 800);
    assert_eq!(client.balance(&user2), 200);
}

#[test]
fn test_allowance() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let spender = Address::generate(&env);

    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Test Token"),
        &String::from_str(&env, "TEST"),
    );

    client.mint(&user1, &1000);

    client.approve(&user1, &spender, &500, &200);
    assert_eq!(client.allowance(&user1, &spender), 500);

    client.transfer_from(&spender, &user1, &spender, &200);
    assert_eq!(client.balance(&user1), 800);
    assert_eq!(client.balance(&spender), 200);
    assert_eq!(client.allowance(&user1, &spender), 300);
}

#[test]
fn test_guard_initializes_with_token_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Test Token"),
        &String::from_str(&env, "TEST"),
    );

    let admins = client.guard_admins();
    assert_eq!(admins.len(), 1);
    assert_eq!(admins.get(0).unwrap(), admin);
    assert_eq!(client.guard_threshold(), 1);
    assert!(!client.guard_is_paused(&PauseType::TRANSFER));
}

#[test]
fn test_guard_pause_blocks_transfer_until_resume() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user1 = Address::generate(&env);
    let user2 = Address::generate(&env);
    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Test Token"),
        &String::from_str(&env, "TEST"),
    );
    client.mint(&user1, &1000);

    client.guard_pause(&admin, &PauseType::TRANSFER, &true);
    assert!(client.guard_is_paused(&PauseType::TRANSFER));

    let transfer_result = client.try_transfer(&user1, &user2, &100);
    assert!(transfer_result.is_err());

    client.guard_pause(&admin, &PauseType::TRANSFER, &false);
    client.transfer(&user1, &user2, &100);
    assert_eq!(client.balance(&user1), 900);
    assert_eq!(client.balance(&user2), 100);
}

#[test]
fn test_emergency_pause_blocks_mint_and_burn_until_resume() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Test Token"),
        &String::from_str(&env, "TEST"),
    );
    client.mint(&user, &1000);

    client.emergency_pause(&vec![&env, admin.clone()]);
    assert!(client.guard_is_paused(&PauseType::MINT));
    assert!(client.guard_is_paused(&PauseType::BURN));
    assert!(client.try_mint(&user, &100).is_err());
    assert!(client.try_burn(&user, &100).is_err());

    client.guard_resume(&vec![&env, admin.clone()]);
    client.mint(&user, &100);
    client.burn(&user, &50);
    assert_eq!(client.balance(&user), 1050);
}

#[test]
fn test_guard_admin_management() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Test Token"),
        &String::from_str(&env, "TEST"),
    );

    assert_eq!(
        client.try_guard_add_admin(&vec![&env, stranger], &new_admin),
        Err(Ok(GuardError::InsufficientSignatures))
    );

    client.guard_add_admin(&vec![&env, admin.clone()], &new_admin);
    let admins = client.guard_admins();
    assert_eq!(admins.len(), 2);
    assert!(admins.iter().any(|a| a == admin));
    assert!(admins.iter().any(|a| a == new_admin));

    client.guard_remove_admin(&vec![&env, admin.clone()], &new_admin);
    let admins = client.guard_admins();
    assert_eq!(admins.len(), 1);
    assert_eq!(admins.get(0).unwrap(), admin);
}

#[test]
fn test_set_admin_rotates_token_and_guard_admin() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let new_admin = Address::generate(&env);
    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Test Token"),
        &String::from_str(&env, "TEST"),
    );

    client.set_admin(&new_admin);

    let admins = client.guard_admins();
    assert_eq!(admins.len(), 1);
    assert_eq!(admins.get(0).unwrap(), new_admin);
    assert_eq!(
        client.try_guard_pause(&admin, &PauseType::MINT, &true),
        Err(Ok(GuardError::Unauthorized))
    );

    client.guard_pause(&new_admin, &PauseType::MINT, &true);
    assert!(client.guard_is_paused(&PauseType::MINT));
}
