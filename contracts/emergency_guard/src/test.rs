#![cfg(test)]

use soroban_sdk::{testutils::Address as _, vec, Address, Env};

#[test]
fn test_emergency_guard_initialization() {
    let env = Env::default();
    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admins = vec![&env, admin1.clone(), admin2.clone()];

    let pause_state = crate::PauseType::new(0);
    assert_eq!(pause_state.as_u32(), 0);

    let contract_id = env.register(crate::EmergencyGuard, ());
    let client = crate::EmergencyGuardClient::new(&env, &contract_id);
    client.initialize(&admins, &1u32);
}

#[test]
fn test_granular_pause_types() {
    let mut pause = crate::PauseType::new(0);

    pause.set_paused(crate::PauseType::SWAP, true);
    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(!pause.is_paused(crate::PauseType::DEPOSIT));

    pause.set_paused(crate::PauseType::DEPOSIT, true);
    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));

    pause.set_paused(crate::PauseType::WITHDRAW, true);
    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));
    assert!(pause.is_paused(crate::PauseType::WITHDRAW));

    pause.set_paused(crate::PauseType::SWAP, false);
    assert!(!pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));
    assert!(pause.is_paused(crate::PauseType::WITHDRAW));
}

#[test]
fn test_pause_all_and_unpause_all() {
    let mut pause = crate::PauseType::new(0);

    pause.pause_all();
    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));
    assert!(pause.is_paused(crate::PauseType::WITHDRAW));
    assert!(pause.is_paused(crate::PauseType::TRANSFER));
    assert!(pause.is_paused(crate::PauseType::MINT));
    assert!(pause.is_paused(crate::PauseType::BURN));

    pause.unpause_all();
    assert!(!pause.is_paused(crate::PauseType::SWAP));
    assert!(!pause.is_paused(crate::PauseType::DEPOSIT));
    assert!(!pause.is_paused(crate::PauseType::WITHDRAW));
    assert!(!pause.is_paused(crate::PauseType::TRANSFER));
    assert!(!pause.is_paused(crate::PauseType::MINT));
    assert!(!pause.is_paused(crate::PauseType::BURN));
}

#[test]
fn test_multiple_pause_types() {
    let mut pause = crate::PauseType::new(0);

    let combined = crate::PauseType::SWAP | crate::PauseType::DEPOSIT | crate::PauseType::MINT;
    pause.set_paused(combined, true);

    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));
    assert!(!pause.is_paused(crate::PauseType::WITHDRAW));
    assert!(pause.is_paused(crate::PauseType::MINT));
    assert!(!pause.is_paused(crate::PauseType::BURN));
}

#[test]
fn test_guard_operations_work_end_to_end() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(crate::EmergencyGuard, ());
    let client = crate::EmergencyGuardClient::new(&e, &contract_id);

    let admin1 = Address::generate(&e);
    let admin2 = Address::generate(&e);
    let admins = vec![&e, admin1.clone(), admin2.clone()];

    client.initialize(&admins, &1u32);
    client.set_pause(&admin1, &crate::PauseType::TRANSFER, &true);
    assert!(client.is_paused(&crate::PauseType::TRANSFER));

    let approvers = vec![&e, admin1.clone()];
    client.emergency_pause(&approvers);
    assert!(client.is_paused(&crate::PauseType::SWAP));

    client.resume(&approvers);
    assert!(!client.is_paused(&crate::PauseType::TRANSFER));

    let new_admin = Address::generate(&e);
    client.add_admin(&approvers, &new_admin);
    client.remove_admin(&approvers, &new_admin);
}
