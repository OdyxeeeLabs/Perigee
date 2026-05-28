#![cfg(test)]

use soroban_sdk::{testutils::Address as _, vec, Address, Env};
use soroban_sdk::{String as SorobanString};

#[test]
fn test_emergency_guard_initialization() {
    let env = Env::default();
    let admin1 = Address::random(&env);
    let admin2 = Address::random(&env);
    let admins = vec![&env, admin1.clone(), admin2.clone()];

    // This would be called during contract initialization
    // For testing, we're just verifying the PauseType structure works
    let pause_state = crate::PauseType::new(0);
    assert_eq!(pause_state.0, 0);
}

#[test]
fn test_granular_pause_types() {
    let mut pause = crate::PauseType::new(0);

    // Test SWAP pause
    pause.set_paused(crate::PauseType::SWAP, true);
    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(!pause.is_paused(crate::PauseType::DEPOSIT));

    // Test DEPOSIT pause
    pause.set_paused(crate::PauseType::DEPOSIT, true);
    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));

    // Test WITHDRAW pause
    pause.set_paused(crate::PauseType::WITHDRAW, true);
    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));
    assert!(pause.is_paused(crate::PauseType::WITHDRAW));

    // Test unpausing
    pause.set_paused(crate::PauseType::SWAP, false);
    assert!(!pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));
    assert!(pause.is_paused(crate::PauseType::WITHDRAW));
}

#[test]
fn test_pause_all_and_unpause_all() {
    let mut pause = crate::PauseType::new(0);

    // Pause all
    pause.pause_all();
    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));
    assert!(pause.is_paused(crate::PauseType::WITHDRAW));
    assert!(pause.is_paused(crate::PauseType::TRANSFER));
    assert!(pause.is_paused(crate::PauseType::MINT));
    assert!(pause.is_paused(crate::PauseType::BURN));

    // Unpause all
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

    // Create a custom pause state with multiple operations
    let combined = crate::PauseType::SWAP | crate::PauseType::DEPOSIT | crate::PauseType::MINT;
    pause.set_paused(combined, true);

    assert!(pause.is_paused(crate::PauseType::SWAP));
    assert!(pause.is_paused(crate::PauseType::DEPOSIT));
    assert!(!pause.is_paused(crate::PauseType::WITHDRAW));
    assert!(pause.is_paused(crate::PauseType::MINT));
    assert!(!pause.is_paused(crate::PauseType::BURN));
}


#[test]
fn test_event_emission_for_guard_actions() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(crate::EmergencyGuard, ());
    let client = crate::EmergencyGuardClient::new(&e, &contract_id);

    // Setup admins
    let admin1 = Address::random(&e);
    let admin2 = Address::random(&e);
    let admins = vec![&e, admin1.clone(), admin2.clone()];

    // Initialize guard
    client.initialize(&admins, &1u32);

    // Call set_pause
    client.set_pause(&admin1, &crate::PauseType::TRANSFER, &true).unwrap();

    // Emergency pause all
    let approvers = vec![&e, admin1.clone()];
    client.emergency_pause(&approvers).unwrap();

    // Resume all
    client.resume(&approvers).unwrap();

    // Add admin
    let new_admin = Address::random(&e);
    client.add_admin(&approvers, &new_admin).unwrap();

    // Remove admin
    client.remove_admin(&approvers, &new_admin).unwrap();

    // Inspect events
    let events = e.events().all();

    // Helper to find events by name
    let find_events = |name: &str| {
        let name_val = String::from_str(&e, name);
        events
            .iter()
            .filter(|(_, topics, _)| {
                if topics.is_empty() {
                    return false;
                }
                let topic_str: Result<SorobanString, _> = topics.get(0).unwrap().try_into_val(&e);
                topic_str.is_ok() && topic_str.unwrap() == name_val
            })
            .collect::<Vec<_>>()
    };

    assert!(!find_events("emergency_guard.set_pause").is_empty());
    assert!(!find_events("emergency_guard.emergency_pause_all").is_empty());
    assert!(!find_events("emergency_guard.resume_all").is_empty());
    assert!(!find_events("emergency_guard.admin_added").is_empty());
    assert!(!find_events("emergency_guard.admin_removed").is_empty());
}
