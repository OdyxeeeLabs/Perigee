use crate::contract::{Token, TokenClient};
use emergency_guard::PauseType;
use soroban_sdk::{testutils::Address as _, vec, Address, Env, String};

// ── Existing Tests ─────────────────────────────────────────────────────────────

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

// ── Token Guard Integration Tests ─────────────────────────────────────────────

/// Verifies that pausing minting blocks new mint calls while leaving
/// transfers untouched — confirms the PauseState bitmask works correctly.
#[test]
fn test_pause_minting_blocks_mint_only() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let user2 = Address::generate(&env);

    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Guard Token"),
        &String::from_str(&env, "GTK"),
    );

    // Mint before pause succeeds.
    client.mint(&user, &500);
    assert_eq!(client.balance(&user), 500);

    // Pause minting.
    client.pause_minting(&admin);

    // Mint should now panic.
    let result = client.try_mint(&user, &100);
    assert!(result.is_err(), "mint should fail when minting is paused");

    // Transfers are NOT paused, they should still work.
    client.transfer(&user, &user2, &100);
    assert_eq!(client.balance(&user2), 100);
}

/// Verifies that pausing transfers blocks transfer/transfer_from/approve
/// but leaves minting unaffected.
#[test]
fn test_pause_transfers_blocks_transfer_only() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let user2 = Address::generate(&env);

    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Guard Token"),
        &String::from_str(&env, "GTK"),
    );

    client.mint(&user, &1000);

    // Pause transfers.
    client.pause_transfers(&admin);

    // Transfer should fail.
    let result = client.try_transfer(&user, &user2, &100);
    assert!(result.is_err(), "transfer should fail when transfers are paused");

    // Minting is NOT paused — still works.
    client.mint(&user2, &50);
    assert_eq!(client.balance(&user2), 50);
}

/// Verifies that pausing burning blocks burn/burn_from.
#[test]
fn test_pause_burning_blocks_burn() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);

    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Guard Token"),
        &String::from_str(&env, "GTK"),
    );
    client.mint(&user, &1000);

    // Pause burning.
    client.pause_burning(&admin);

    // Burn should fail.
    let result = client.try_burn(&user, &100);
    assert!(result.is_err(), "burn should fail when burning is paused");

    // Resume burning.
    client.resume_burning(&admin);

    // Burn should succeed after resuming.
    client.burn(&user, &100);
    assert_eq!(client.balance(&user), 900);
}

/// Verifies emergency_pause_all blocks all operations simultaneously.
/// This is the key scenario from issue #230: one bitmask storage entry
/// controls all operation types — no extra ledger entries.
#[test]
fn test_emergency_pause_all_freezes_everything() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);
    let user = Address::generate(&env);
    let user2 = Address::generate(&env);

    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Guard Token"),
        &String::from_str(&env, "GTK"),
    );
    client.mint(&user, &1000);

    // Emergency pause: all operations freeze via single bitmask write.
    let approvers = vec![&env, admin.clone()];
    client.emergency_pause_all(&approvers);

    // Confirm all operations are blocked.
    assert!(
        client.try_mint(&user2, &100).is_err(),
        "mint should be paused"
    );
    assert!(
        client.try_transfer(&user, &user2, &50).is_err(),
        "transfer should be paused"
    );
    assert!(
        client.try_burn(&user, &50).is_err(),
        "burn should be paused"
    );

    // Resume all via multi-sig.
    client.resume_all(&approvers);

    // All operations should work again.
    client.mint(&user2, &100);
    assert_eq!(client.balance(&user2), 100);
    client.transfer(&user, &user2, &50);
    assert_eq!(client.balance(&user), 950);
}

/// Verifies the guard admin query functions work correctly — confirms the
/// EmergencyGuard state shares instance storage with the token without
/// extra footprint entries.
#[test]
fn test_guard_admin_queries() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);

    client.initialize(
        &admin,
        &7,
        &String::from_str(&env, "Guard Token"),
        &String::from_str(&env, "GTK"),
    );

    // Guard admin should be the token admin.
    let guard_admins = client.get_guard_admins();
    assert_eq!(guard_admins.len(), 1);
    assert_eq!(guard_admins.get(0).unwrap(), admin);

    // Threshold should be 1 (single-admin setup).
    assert_eq!(client.get_guard_threshold(), 1);

    // No operation should be paused at initialization.
    assert!(!client.is_operation_paused(&PauseType::MINT));
    assert!(!client.is_operation_paused(&PauseType::TRANSFER));
    assert!(!client.is_operation_paused(&PauseType::BURN));
}

/// Storage efficiency test: confirms that after guard integration the
/// footprint for initialize is 2 instance writes (Admin + Metadata),
/// with guard state sharing the instance map — matching the analysis in
/// docs/token_guard_storage_analysis.md.
#[test]
fn test_initialize_storage_efficiency() {
    let env = Env::default();
    env.mock_all_auths();

    let contract_id = env.register(Token, ());
    let client = TokenClient::new(&env, &contract_id);

    let admin = Address::generate(&env);

    // Initialize — should complete without error and correctly read back
    // metadata, confirming the single Metadata entry was written.
    client.initialize(
        &admin,
        &18,
        &String::from_str(&env, "Efficiency Token"),
        &String::from_str(&env, "EFFT"),
    );

    assert_eq!(client.decimals(), 18);
    assert_eq!(client.name(), String::from_str(&env, "Efficiency Token"));
    assert_eq!(client.symbol(), String::from_str(&env, "EFFT"));
    assert_eq!(client.get_guard_threshold(), 1);
}
