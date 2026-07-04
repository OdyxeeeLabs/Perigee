extern crate std;

use crate::{EmergencyGuard, EmergencyGuardClient, GuardError, PauseType};
use soroban_sdk::{
    testutils::{Address as _, Events},
    vec, Address, Env, Vec as SorobanVec,
};
use std::vec::Vec;

fn make_admins(env: &Env, n: u32) -> SorobanVec<Address> {
    let mut admins = SorobanVec::new(env);
    for _ in 0..n {
        admins.push_back(Address::generate(env));
    }
    admins
}

fn setup(threshold: u32, n_admins: u32) -> (Env, EmergencyGuardClient<'static>, Vec<Address>) {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(EmergencyGuard, ());
    let client = EmergencyGuardClient::new(&env, &contract_id);
    let admins = make_admins(&env, n_admins);
    client.initialize(&admins, &threshold);
    let std_admins: Vec<Address> = admins.iter().collect();
    (env, client, std_admins)
}

#[test]
fn test_granular_pause_types() {
    let mut pause = PauseType::new(0);

    pause.set_paused(PauseType::SWAP, true);
    assert!(pause.is_paused(PauseType::SWAP));
    assert!(!pause.is_paused(PauseType::DEPOSIT));

    pause.set_paused(PauseType::DEPOSIT, true);
    assert!(pause.is_paused(PauseType::SWAP));
    assert!(pause.is_paused(PauseType::DEPOSIT));

    pause.set_paused(PauseType::WITHDRAW, true);
    assert!(pause.is_paused(PauseType::WITHDRAW));

    pause.set_paused(PauseType::SWAP, false);
    assert!(!pause.is_paused(PauseType::SWAP));
    assert!(pause.is_paused(PauseType::DEPOSIT));
    assert!(pause.is_paused(PauseType::WITHDRAW));
}

#[test]
fn test_bitwise_pause_logic() {
    let mut pause = PauseType::new(0);

    // Test setting operations
    pause.set_paused(PauseType::SWAP, true);
    assert!(pause.is_paused(PauseType::SWAP));
    assert!(!pause.is_paused(PauseType::DEPOSIT));

    // Test checking multiple operations
    pause.set_paused(PauseType::MINT, true);
    assert!(pause.is_paused(PauseType::SWAP));
    assert!(pause.is_paused(PauseType::MINT));

    // Test clearing operations
    pause.set_paused(PauseType::SWAP, false);
    assert!(!pause.is_paused(PauseType::SWAP));
    assert!(pause.is_paused(PauseType::MINT));

    // Test clearing all manually
    pause.set_paused(PauseType::MINT, false);
    assert_eq!(pause.as_u32(), 0);
}

#[test]
fn test_pause_all_and_unpause_all() {
    let mut pause = PauseType::new(0);
    pause.pause_all();
    for op in [
        PauseType::SWAP,
        PauseType::DEPOSIT,
        PauseType::WITHDRAW,
        PauseType::TRANSFER,
        PauseType::MINT,
        PauseType::BURN,
    ] {
        assert!(pause.is_paused(op));
    }
    pause.unpause_all();
    for op in [
        PauseType::SWAP,
        PauseType::DEPOSIT,
        PauseType::WITHDRAW,
        PauseType::TRANSFER,
        PauseType::MINT,
        PauseType::BURN,
    ] {
        assert!(!pause.is_paused(op));
    }
}

#[test]
fn test_unpause_operation() {
    let (_env, client, admins) = setup(1, 1);
    let admin = admins[0].clone();

    client.set_pause(&admin, &PauseType::SWAP, &true);
    assert!(client.is_paused(&PauseType::SWAP));

    client.set_pause(&admin, &PauseType::SWAP, &false);
    assert!(!client.is_paused(&PauseType::SWAP));
}

#[test]
fn test_unpause_all_operations() {
    let (env, client, admins) = setup(1, 1);
    let approvers = vec![&env, admins[0].clone()];

    client.emergency_pause(&approvers);
    assert!(client.is_paused(&PauseType::SWAP));
    assert!(client.is_paused(&PauseType::DEPOSIT));

    client.resume(&approvers);
    assert!(!client.is_paused(&PauseType::SWAP));
    assert!(!client.is_paused(&PauseType::DEPOSIT));
    assert!(!client.is_paused(&PauseType::WITHDRAW));
    assert!(!client.is_paused(&PauseType::TRANSFER));
    assert!(!client.is_paused(&PauseType::MINT));
    assert!(!client.is_paused(&PauseType::BURN));
}

#[test]
fn test_multiple_pause_types() {
    let mut pause = PauseType::new(0);
    let combined = PauseType::SWAP | PauseType::DEPOSIT | PauseType::MINT;
    pause.set_paused(combined, true);
    assert!(pause.is_paused(PauseType::SWAP));
    assert!(pause.is_paused(PauseType::DEPOSIT));
    assert!(!pause.is_paused(PauseType::WITHDRAW));
    assert!(pause.is_paused(PauseType::MINT));
    assert!(!pause.is_paused(PauseType::BURN));
}

// â”€â”€â”€ Initialization â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[test]
fn test_initialize_stores_admins_and_threshold() {
    let (_env, client, _admins) = setup(2, 3);
    let stored: Vec<Address> = client.get_admins().iter().collect();
    assert_eq!(stored.len(), 3);
    assert_eq!(client.get_threshold(), 2);
    assert!(!client.is_paused(&PauseType::SWAP));
}

#[test]
fn test_initialize_rejects_zero_threshold() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(EmergencyGuard, ());
    let client = EmergencyGuardClient::new(&env, &contract_id);
    let admins = vec![&env, Address::generate(&env)];
    let result = client.try_initialize(&admins, &0);
    assert_eq!(result, Err(Ok(GuardError::InvalidThreshold)));
}

#[test]
fn test_initialize_rejects_threshold_greater_than_admin_count() {
    let env = Env::default();
    env.mock_all_auths();
    let contract_id = env.register(EmergencyGuard, ());
    let client = EmergencyGuardClient::new(&env, &contract_id);
    let admins = vec![&env, Address::generate(&env), Address::generate(&env)];
    let result = client.try_initialize(&admins, &3);
    assert_eq!(result, Err(Ok(GuardError::InvalidThreshold)));
}

#[test]
fn test_initialize_cannot_be_called_twice() {
    let (env, client, _admins) = setup(1, 2);
    let result = client.try_initialize(&soroban_sdk::Vec::new(&env), &1);
    assert_eq!(result, Err(Ok(GuardError::AlreadyInitialized)));
}

// â”€â”€â”€ Admin rotation: add_admin â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[test]
fn test_add_admin_with_sufficient_approvers() {
    let (env, client, admins) = setup(2, 3);
    let new_admin = Address::generate(&env);
    let approvers = vec![&env, admins[0].clone(), admins[1].clone()];
    client.add_admin(&approvers, &new_admin);
    let stored: Vec<Address> = client.get_admins().iter().collect();
    assert_eq!(stored.len(), 4);
    assert!(stored.contains(&new_admin));
}

#[test]
fn test_add_admin_fails_with_insufficient_approvers() {
    let (env, client, admins) = setup(2, 3);
    let new_admin = Address::generate(&env);
    // Only 1 approver but threshold is 2
    let approvers = vec![&env, admins[0].clone()];
    let result = client.try_add_admin(&approvers, &new_admin);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));
}

#[test]
fn test_add_admin_fails_with_non_admin_approvers() {
    let (env, client, _admins) = setup(1, 2);
    let new_admin = Address::generate(&env);
    let outsider = Address::generate(&env);
    let approvers = vec![&env, outsider];
    let result = client.try_add_admin(&approvers, &new_admin);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));
}

#[test]
fn test_add_admin_deduplicates_approvers() {
    // Passing the same admin twice must not count as 2 approvals
    let (env, client, admins) = setup(2, 3);
    let new_admin = Address::generate(&env);
    let approvers = vec![&env, admins[0].clone(), admins[0].clone()];
    let result = client.try_add_admin(&approvers, &new_admin);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));
}

#[test]
fn test_add_admin_idempotent_for_existing_admin() {
    // Adding an already-existing admin should succeed but not duplicate the entry
    let (env, client, admins) = setup(1, 2);
    let existing = admins[0].clone();
    let approvers = vec![&env, admins[0].clone()];
    client.add_admin(&approvers, &existing);
    let stored: Vec<Address> = client.get_admins().iter().collect();
    assert_eq!(stored.len(), 2, "duplicate admin must not be inserted");
}

#[test]
fn test_add_admin_threshold_one_single_approver_sufficient() {
    let (env, client, admins) = setup(1, 2);
    let new_admin = Address::generate(&env);
    let approvers = vec![&env, admins[0].clone()];
    client.add_admin(&approvers, &new_admin);
    assert_eq!(client.get_admins().len(), 3);
}

// â”€â”€â”€ Admin rotation: remove_admin â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[test]
fn test_remove_admin_with_sufficient_approvers() {
    let (env, client, admins) = setup(2, 3);
    let to_remove = admins[2].clone();
    let approvers = vec![&env, admins[0].clone(), admins[1].clone()];
    client.remove_admin(&approvers, &to_remove);
    let stored: Vec<Address> = client.get_admins().iter().collect();
    assert_eq!(stored.len(), 2);
    assert!(!stored.contains(&to_remove));
}

#[test]
fn test_remove_admin_fails_with_insufficient_approvers() {
    let (env, client, admins) = setup(2, 3);
    let to_remove = admins[2].clone();
    let approvers = vec![&env, admins[0].clone()];
    let result = client.try_remove_admin(&approvers, &to_remove);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));
}

#[test]
fn test_remove_admin_fails_when_admin_not_found() {
    let (env, client, admins) = setup(2, 3);
    let outsider = Address::generate(&env);
    let approvers = vec![&env, admins[0].clone(), admins[1].clone()];
    let result = client.try_remove_admin(&approvers, &outsider);
    assert_eq!(result, Err(Ok(GuardError::AdminNotFound)));
}

#[test]
fn test_remove_admin_fails_when_would_drop_below_threshold() {
    // 2 admins, threshold 2 â†’ removing one would leave 1 < threshold
    let (env, client, admins) = setup(2, 2);
    let to_remove = admins[1].clone();
    let approvers = vec![&env, admins[0].clone(), admins[1].clone()];
    let result = client.try_remove_admin(&approvers, &to_remove);
    assert_eq!(result, Err(Ok(GuardError::InvalidThreshold)));
}

#[test]
fn test_unauthorized_admin_removal() {
    let (env, client, admins) = setup(1, 2);
    let outsider = Address::generate(&env);
    let approvers = vec![&env, outsider];
    let result = client.try_remove_admin(&approvers, &admins[1]);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));
}

// â”€â”€â”€ Full rotation cycle â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[test]
fn test_full_admin_rotation_add_then_remove_old() {
    let (env, client, admins) = setup(2, 3);
    let new_admin = Address::generate(&env);

    // Step 1: add new admin
    let approvers = vec![&env, admins[0].clone(), admins[1].clone()];
    client.add_admin(&approvers, &new_admin);
    assert_eq!(client.get_admins().len(), 4);

    // Step 2: remove one of the original admins using new quorum
    let approvers2 = vec![&env, admins[0].clone(), new_admin.clone()];
    client.remove_admin(&approvers2, &admins[2]);

    let stored: Vec<Address> = client.get_admins().iter().collect();
    assert_eq!(stored.len(), 3);
    assert!(!stored.contains(&admins[2]));
    assert!(stored.contains(&new_admin));
}

#[test]
fn test_rotate_admin() {
    let (env, client, admins) = setup(2, 3);
    let old_admin = admins[2].clone();
    let new_admin = Address::generate(&env);

    let approvers = vec![&env, admins[0].clone(), admins[1].clone()];
    client.rotate_admin(&approvers, &old_admin, &new_admin);

    let stored: Vec<Address> = client.get_admins().iter().collect();
    assert_eq!(stored.len(), 3);
    assert!(!stored.contains(&old_admin));
    assert!(stored.contains(&new_admin));
}

#[test]
fn test_rotate_admin_duplicate_prevented() {
    let (env, client, admins) = setup(2, 3);
    let old_admin = admins[2].clone();
    let new_admin = admins[1].clone(); // already an admin

    let approvers = vec![&env, admins[0].clone(), admins[1].clone()];
    // Rotating to an existing admin should just remove the old admin and reduce the list size
    client.rotate_admin(&approvers, &old_admin, &new_admin);

    let stored: Vec<Address> = client.get_admins().iter().collect();
    assert_eq!(stored.len(), 2); // 3 - 1
    assert!(!stored.contains(&old_admin));
    assert!(stored.contains(&new_admin));
}

#[test]
fn test_removed_admin_cannot_approve_operations() {
    let (env, client, admins) = setup(1, 3);

    // Remove admins[2]
    let approvers = vec![&env, admins[0].clone()];
    client.remove_admin(&approvers, &admins[2]);

    // admins[2] tries to add a new admin â€” should fail
    let new_admin = Address::generate(&env);
    let bad_approvers = vec![&env, admins[2].clone()];
    let result = client.try_add_admin(&bad_approvers, &new_admin);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));
}

#[test]
fn test_newly_added_admin_can_approve_operations() {
    let (env, client, admins) = setup(1, 2);
    let new_admin = Address::generate(&env);

    // Add new_admin
    let approvers = vec![&env, admins[0].clone()];
    client.add_admin(&approvers, &new_admin);

    // new_admin approves a pause operation
    client.set_pause(&new_admin, &PauseType::SWAP, &true);
    assert!(client.is_paused(&PauseType::SWAP));
}

// â”€â”€â”€ get_admins / get_threshold â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[test]
fn test_get_admins_returns_all_admins() {
    let (_env, client, admins) = setup(1, 3);
    let stored: Vec<Address> = client.get_admins().iter().collect();
    assert_eq!(stored.len(), 3);
    for a in &admins {
        assert!(stored.contains(a));
    }
}

#[test]
fn test_get_threshold_returns_correct_value() {
    let (_env, client, _admins) = setup(2, 4);
    assert_eq!(client.get_threshold(), 2);
}

// â”€â”€â”€ Pause / resume integration â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

#[test]
fn test_set_pause_by_single_admin() {
    let (_env, client, admins) = setup(1, 2);
    client.set_pause(&admins[0], &PauseType::DEPOSIT, &true);
    assert!(client.is_paused(&PauseType::DEPOSIT));
    assert!(!client.is_paused(&PauseType::SWAP));
}

#[test]
fn test_set_pause_rejected_for_non_admin() {
    let (env, client, _admins) = setup(1, 2);
    let outsider = Address::generate(&env);
    let result = client.try_set_pause(&outsider, &PauseType::SWAP, &true);
    assert_eq!(result, Err(Ok(GuardError::Unauthorized)));
}

#[test]
fn test_emergency_pause_requires_multi_sig() {
    let (env, client, admins) = setup(2, 3);

    // Only 1 approver â€” should fail
    let approvers = vec![&env, admins[0].clone()];
    let result = client.try_emergency_pause(&approvers);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));

    // 2 approvers â€” should succeed and pause everything
    let approvers = vec![&env, admins[0].clone(), admins[1].clone()];
    client.emergency_pause(&approvers);
    for op in [
        PauseType::SWAP,
        PauseType::DEPOSIT,
        PauseType::WITHDRAW,
        PauseType::TRANSFER,
        PauseType::MINT,
        PauseType::BURN,
    ] {
        assert!(client.is_paused(&op));
    }
}

#[test]
fn test_resume_requires_multi_sig() {
    let (env, client, admins) = setup(2, 3);

    // Pause everything first
    let approvers = vec![&env, admins[0].clone(), admins[1].clone()];
    client.emergency_pause(&approvers);

    // Try resume with 1 approver â€” should fail
    let approvers1 = vec![&env, admins[0].clone()];
    let result = client.try_resume(&approvers1);
    assert_eq!(result, Err(Ok(GuardError::InsufficientSignatures)));

    // Resume with 2 approvers â€” should succeed
    let approvers2 = vec![&env, admins[0].clone(), admins[1].clone()];
    client.resume(&approvers2);
    assert!(!client.is_paused(&PauseType::SWAP));
    assert!(!client.is_paused(&PauseType::DEPOSIT));
}

#[test]
fn test_guard_events() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(EmergencyGuard, ());
    let client = EmergencyGuardClient::new(&env, &contract_id);

    let admin1 = Address::generate(&env);
    let admin2 = Address::generate(&env);
    let admin3 = Address::generate(&env);
    let admins = vec![&env, admin1.clone(), admin2.clone()];

    client.initialize(&admins, &1);

    let _initial_events = env.events().all();

    client.set_pause(&admin1, &crate::PauseType::SWAP, &true);
    client.emergency_pause(&vec![&env, admin1.clone()]);
    client.resume(&vec![&env, admin1.clone()]);
    client.add_admin(&vec![&env, admin1.clone()], &admin3);
    client.remove_admin(&vec![&env, admin1.clone()], &admin3);

    let events = env.events().all();
    assert!(!events.is_empty());
}

#[test]
fn test_pause_type_as_u32_bitmask() {
    let mut pause = crate::PauseType::new(0);
    pause.set_paused(crate::PauseType::SWAP, true);
    pause.set_paused(crate::PauseType::DEPOSIT, true);
    assert_eq!(
        pause.as_u32(),
        crate::PauseType::SWAP | crate::PauseType::DEPOSIT
    );
}
