#![cfg(test)]
extern crate std;

use crate::{ReentrancyGuard, Guard, GuardDataKey, GuardError, PauseType};
use soroban_sdk::{
    testutils::{Address as _, Events},
    vec, Address, Env, Vec as SorobanVec,
};
use std::vec::Vec;

fn gen_admins(env: &Env, n: u32) -> SorobanVec<Address> {
    let mut admins = SorobanVec::new(env);
    for _ in 0..n {
        admins.push_back(Address::generate(env));
    }
    admins
}

fn setup(threshold: u32, n_admins: u32) -> (Env, SorobanVec<Address>, Vec<Address>) {
    let env = Env::default();
    env.mock_all_auths();
    let admins = gen_admins(&env, n_admins);
    Guard::init_guard(&env, admins.clone(), threshold).unwrap();
    let std_admins: Vec<Address> = admins.iter().collect();
    (env, admins, std_admins)
}

// ── PauseType ────────────────────────────────────────────────────────────────

#[test]
fn test_pause_type_granular_bitmask() {
    let mut pause = PauseType::new(0);
    pause.set_paused(PauseType::SWAP, true);
    assert!(pause.is_paused(PauseType::SWAP));
    assert!(!pause.is_paused(PauseType::DEPOSIT));

    pause.set_paused(PauseType::DEPOSIT, true);
    assert!(pause.is_paused(PauseType::SWAP));
    assert!(pause.is_paused(PauseType::DEPOSIT));

    pause.set_paused(PauseType::SWAP, false);
    assert!(!pause.is_paused(PauseType::SWAP));
    assert!(pause.is_paused(PauseType::DEPOSIT));

    pause.set_paused(PauseType::MINT, true);
    assert!(pause.is_paused(PauseType::MINT));

    pause.pause_all();
    assert!(pause.is_paused(PauseType::WITHDRAW));
    assert!(pause.is_paused(PauseType::TRANSFER));

    pause.unpause_all();
    assert_eq!(pause.as_u32(), 0);
}

#[test]
fn test_pause_type_bitmask_combination() {
    let mut pause = PauseType::new(0);
    let combined = PauseType::SWAP | PauseType::DEPOSIT | PauseType::MINT;
    pause.set_paused(combined, true);
    assert!(pause.is_paused(PauseType::SWAP));
    assert!(pause.is_paused(PauseType::DEPOSIT));
    assert!(pause.is_paused(PauseType::MINT));
    assert!(!pause.is_paused(PauseType::WITHDRAW));
}

// ── Initialization ───────────────────────────────────────────────────────────

#[test]
fn test_init_guard_stores_admins_and_threshold() {
    let (env, admins, _) = setup(2, 3);
    let guard_admins = Guard::get_admins(&env);
    assert_eq!(guard_admins.len(), admins.len());
    assert_eq!(Guard::get_threshold(&env), 2);
    assert!(!Guard::is_paused(&env, PauseType::SWAP));
    assert!(!Guard::is_admin(&env, &Address::generate(&env)));
}

#[test]
fn test_init_guard_rejects_zero_threshold() {
    let env = Env::default();
    let admins = gen_admins(&env, 2);
    assert_eq!(
        Guard::init_guard(&env, admins, 0),
        Err(GuardError::InvalidThreshold)
    );
}

#[test]
fn test_init_guard_rejects_threshold_greater_than_admin_count() {
    let env = Env::default();
    let admins = gen_admins(&env, 2);
    assert_eq!(
        Guard::init_guard(&env, admins, 3),
        Err(GuardError::InvalidThreshold)
    );
}

#[test]
fn test_init_guard_cannot_be_called_twice() {
    let env = Env::default();
    let admins = gen_admins(&env, 2);
    Guard::init_guard(&env, admins.clone(), 1).unwrap();
    assert_eq!(
        Guard::init_guard(&env, admins, 1),
        Err(GuardError::AlreadyInitialized)
    );
}

// ── Multi-signature validation ───────────────────────────────────────────────

#[test]
fn test_multisig_exactly_threshold_approvers_succeeds() {
    let (env, admins, std_admins) = setup(2, 3);
    let approvers = vec![&env, std_admins[0].clone(), std_admins[1].clone()];
    let new_admin = Address::generate(&env);
    Guard::add_admin(&env, approvers, new_admin.clone()).unwrap();

    let stored: Vec<Address> = Guard::get_admins(&env).iter().collect();
    assert_eq!(stored.len(), admins.len() + 1);
    assert!(stored.contains(&new_admin));
}

#[test]
fn test_multisig_one_below_threshold_fails() {
    let (env, _admins, std_admins) = setup(2, 3);
    let approvers = vec![&env, std_admins[0].clone()];
    assert_eq!(
        Guard::validate_multi_sig(&env, &approvers),
        Err(GuardError::InsufficientSignatures)
    );
}

#[test]
fn test_multisig_duplicate_approvers_not_double_counted() {
    let (env, _admins, std_admins) = setup(2, 3);
    // Same admin twice must not count as two approvals.
    let approvers = vec![&env, std_admins[0].clone(), std_admins[0].clone()];
    assert_eq!(
        Guard::validate_multi_sig(&env, &approvers),
        Err(GuardError::InsufficientSignatures)
    );
}

#[test]
fn test_multisig_non_admin_approver_is_unauthorized() {
    let (env, _admins, _std_admins) = setup(1, 2);
    let outsider = Address::generate(&env);
    let approvers = vec![&env, outsider];
    assert_eq!(
        Guard::validate_multi_sig(&env, &approvers),
        Err(GuardError::Unauthorized)
    );
}

#[test]
fn test_multisig_without_init_is_not_initialized() {
    let env = Env::default();
    env.mock_all_auths();
    let approvers = vec![&env, Address::generate(&env)];
    assert_eq!(
        Guard::validate_multi_sig(&env, &approvers),
        Err(GuardError::NotInitialized)
    );
}

// ── Pause / resume ───────────────────────────────────────────────────────────

#[test]
fn test_set_pause_state_and_is_paused() {
    let (env, _admins, _std_admins) = setup(1, 2);
    Guard::set_pause_state(&env, PauseType::DEPOSIT, true).unwrap();
    assert!(Guard::is_paused(&env, PauseType::DEPOSIT));
    assert!(!Guard::is_paused(&env, PauseType::SWAP));
    assert_eq!(Guard::get_pause_state(&env), PauseType::DEPOSIT);
    assert_eq!(
        Guard::check_not_paused(&env, PauseType::DEPOSIT),
        Err(GuardError::Paused)
    );
    assert!(Guard::check_not_paused(&env, PauseType::SWAP).is_ok());

    Guard::set_pause_state(&env, PauseType::DEPOSIT, false).unwrap();
    assert!(!Guard::is_paused(&env, PauseType::DEPOSIT));
    // ensure_not_paused panic guard
    Guard::ensure_not_paused(&env, PauseType::DEPOSIT);
}

#[test]
fn test_set_pause_by_admin_rejects_non_admin() {
    let (env, _admins, _std_admins) = setup(1, 2);
    let outsider = Address::generate(&env);
    assert_eq!(
        Guard::set_pause_by_admin(&env, &outsider, PauseType::SWAP, true),
        Err(GuardError::Unauthorized)
    );
}

#[test]
fn test_emergency_pause_and_resume_roundtrip() {
    let (env, _admins, std_admins) = setup(2, 3);
    let approvers = vec![&env, std_admins[0].clone(), std_admins[1].clone()];

    // Below threshold must not pause anything.
    let below = vec![&env, std_admins[0].clone()];
    assert_eq!(
        Guard::emergency_pause_all(&env, below),
        Err(GuardError::InsufficientSignatures)
    );
    assert!(!Guard::is_paused(&env, PauseType::SWAP));

    // At threshold: everything pauses.
    Guard::emergency_pause_all(&env, approvers.clone()).unwrap();
    assert!(Guard::is_paused(&env, PauseType::SWAP));
    assert!(Guard::is_paused(&env, PauseType::BURN));
    assert_eq!(Guard::get_pause_state(&env), u32::MAX);

    Guard::resume_all(&env, approvers).unwrap();
    assert!(!Guard::is_paused(&env, PauseType::SWAP));
    assert_eq!(Guard::get_pause_state(&env), 0);
}

// ── Admin lifecycle ──────────────────────────────────────────────────────────

#[test]
fn test_add_and_remove_admin_lifecycle() {
    let (env, admins, std_admins) = setup(2, 3);
    let new_admin = Address::generate(&env);
    let approvers = vec![&env, std_admins[0].clone(), std_admins[1].clone()];

    Guard::add_admin(&env, approvers.clone(), new_admin.clone()).unwrap();
    let stored: Vec<Address> = Guard::get_admins(&env).iter().collect();
    assert_eq!(stored.len(), admins.len() + 1);
    assert!(Guard::is_admin(&env, &new_admin));

    // Adding the same admin again is idempotent.
    Guard::add_admin(&env, approvers.clone(), new_admin.clone()).unwrap();
    assert_eq!(Guard::get_admins(&env).len(), admins.len() + 1);

    Guard::remove_admin(&env, approvers, new_admin).unwrap();
    assert!(!Guard::is_admin(&env, &new_admin));
}

#[test]
fn test_remove_admin_cannot_drop_below_threshold() {
    let (env, _admins, std_admins) = setup(2, 2);
    let approvers = vec![&env, std_admins[0].clone(), std_admins[1].clone()];
    assert_eq!(
        Guard::remove_admin(&env, approvers, std_admins[1].clone()),
        Err(GuardError::InvalidThreshold)
    );
}

#[test]
fn test_rotate_admin_replaces_old() {
    let (env, admins, std_admins) = setup(2, 3);
    let new_admin = Address::generate(&env);
    let approvers = vec![&env, std_admins[0].clone(), std_admins[1].clone()];

    Guard::rotate_admin(&env, approvers, std_admins[2].clone(), new_admin.clone()).unwrap();

    let stored: Vec<Address> = Guard::get_admins(&env).iter().collect();
    assert_eq!(stored.len(), admins.len());
    assert!(!stored.contains(&std_admins[2]));
    assert!(stored.contains(&new_admin));
}

#[test]
fn test_rotate_admin_deduplicates_existing_target() {
    let (env, _admins, std_admins) = setup(2, 3);
    let approvers = vec![&env, std_admins[0].clone(), std_admins[1].clone()];

    // Rotating to an existing admin removes the old one without duplication.
    Guard::rotate_admin(&env, approvers, std_admins[2].clone(), std_admins[1].clone()).unwrap();

    let stored: Vec<Address> = Guard::get_admins(&env).iter().collect();
    assert_eq!(stored.len(), 2);
    assert!(!stored.contains(&std_admins[2]));
    assert!(stored.contains(&std_admins[1]));
}

#[test]
fn test_rotate_unknown_admin_not_found() {
    let (env, _admins, std_admins) = setup(2, 2);
    let outsider = Address::generate(&env);
    let approvers = vec![&env, std_admins[0].clone(), std_admins[1].clone()];
    assert_eq!(
        Guard::rotate_admin(&env, approvers, outsider, std_admins[0].clone()),
        Err(GuardError::AdminNotFound)
    );
}

// ── Events / isolation ───────────────────────────────────────────────────────

#[test]
fn test_state_changes_emit_typed_events() {
    let (env, _admins, std_admins) = setup(1, 2);
    let approvers = vec![&env, std_admins[0].clone()];
    let new_admin = Address::generate(&env);

    Guard::set_pause_state(&env, PauseType::SWAP, true).unwrap();
    Guard::emergency_pause_all(&env, approvers.clone()).unwrap();
    Guard::resume_all(&env, approvers.clone()).unwrap();
    Guard::add_admin(&env, approvers.clone(), new_admin.clone()).unwrap();
    Guard::remove_admin(&env, approvers, new_admin).unwrap();

    let events = env.events().all();
    assert!(!events.is_empty());
}

#[test]
fn test_guards_are_isolated_between_envs() {
    let env_a = Env::default();
    env_a.mock_all_auths();
    let env_b = Env::default();
    env_b.mock_all_auths();

    let admins_a = gen_admins(&env_a, 2);
    let admins_b = gen_admins(&env_b, 2);

    Guard::init_guard(&env_a, admins_a.clone(), 1).unwrap();
    Guard::init_guard(&env_b, admins_b.clone(), 2).unwrap();

    Guard::set_pause_state(&env_a, PauseType::SWAP, true).unwrap();
    assert!(Guard::is_paused(&env_a, PauseType::SWAP));
    assert!(!Guard::is_paused(&env_b, PauseType::SWAP));

    assert_eq!(Guard::get_threshold(&env_a), 1);
    assert_eq!(Guard::get_threshold(&env_b), 2);

    // A fresh env has no guard state at all.
    let env_c = Env::default();
    env_c.mock_all_auths();
    assert!(!env_c
        .storage()
        .instance()
        .has(&GuardDataKey::SignatureThreshold));
}

#[test]
fn test_trait_surface_delegates_to_core() {
    let (env, _admins, std_admins) = setup(1, 2);
    use crate::EmergencyGuardTrait;
    let approvers = vec![&env, std_admins[0].clone()];

    assert!(EmergencyGuardTrait::check_not_paused(&env, PauseType::SWAP).is_ok());
    EmergencyGuardTrait::set_pause_state(&env, PauseType::SWAP, true).unwrap();
    assert_eq!(
        EmergencyGuardTrait::check_not_paused(&env, PauseType::SWAP),
        Err(GuardError::Paused)
    );
    assert!(EmergencyGuardTrait::is_admin(&env, &std_admins[0]));
    assert_eq!(EmergencyGuardTrait::get_pause_state(&env), PauseType::SWAP);
    assert_eq!(EmergencyGuardTrait::get_admins(&env).len(), 2);
    assert_eq!(EmergencyGuardTrait::get_threshold(&env), 1);

    EmergencyGuardTrait::emergency_pause_all(&env, approvers.clone()).unwrap();
    assert_eq!(EmergencyGuardTrait::get_pause_state(&env), u32::MAX);
    EmergencyGuardTrait::resume_all(&env, approvers).unwrap();
    assert_eq!(EmergencyGuardTrait::get_pause_state(&env), 0);
}

#[test]
fn reentrancy_guard_blocks_nested_enter() {
    let env = Env::default();
    assert!(ReentrancyGuard::enter(&env).is_ok());
    assert!(ReentrancyGuard::is_entered(&env));
    assert_eq!(ReentrancyGuard::enter(&env), Err(GuardError::Reentrancy));
    ReentrancyGuard::exit(&env);
    assert!(!ReentrancyGuard::is_entered(&env));
    assert!(ReentrancyGuard::enter(&env).is_ok());
    ReentrancyGuard::exit(&env);
}
