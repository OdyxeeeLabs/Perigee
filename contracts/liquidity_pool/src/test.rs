use super::*;
use soroban_sdk::{
    testutils::{Address as _, Events, Ledger},
    Address, Env, String as SorobanString, TryIntoVal,
};

// Import Vec from alloc for no_std environment
extern crate alloc;
use alloc::vec::Vec;

#[test]
fn test_basic_flow() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    // Setup tokens
    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_client = soroban_sdk::token::Client::new(&e, &token_a);
    let token_b_client = soroban_sdk::token::Client::new(&e, &token_b);

    let user1 = Address::generate(&e);
    let user2 = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    // Check initialize
    client.initialize(&admin, &token_a, &token_b);

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    // Mint tokens to users
    token_a_admin.mint(&user1, &10000);
    token_b_admin.mint(&user1, &10000);
    token_a_admin.mint(&user2, &10000);
    token_b_admin.mint(&user2, &10000);

    // User 1 Deposits 1000 of each
    // With new sqrt implementation: shares = sqrt(1000 * 1000) = 1000
    let shares = client.deposit(&user1, &1000, &1000);
    assert_eq!(shares, 1000);

    // User 2 Swaps 100 A for B
    let out_amount = 90;
    let in_max = 110;

    // Swap 90 B out, pay with A
    let paid = client.swap(&user2, &false, &out_amount, &in_max);

    // Check balances
    assert_eq!(token_b_client.balance(&user2), 10000 + 90);
    assert_eq!(token_a_client.balance(&user2), 10000 - paid);

    // User 1 Withdraws
    let (withdrawn_a, withdrawn_b) = client.withdraw(&user1, &1000);
    // Should get roughly remaining reserves
    assert!(withdrawn_a > 1000); // Gained fees (paid by user2)
    assert!(withdrawn_b < 1000); // Lost due to User 2 taking B
}

#[test]
#[should_panic(expected = "Error(Contract, #1)")]
fn test_double_initialization() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    client.initialize(&admin, &token_a, &token_b);
    // Should panic with AlreadyInitialized error
    client.initialize(&admin, &token_a, &token_b);
}

#[test]
#[should_panic(expected = "Error(Contract, #2)")]
fn test_swap_insufficient_liquidity() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit
    token_a_admin.mint(&user, &1000);
    token_b_admin.mint(&user, &1000);
    client.deposit(&user, &1000, &1000);

    // Try to swap more than reserve
    client.swap(&user, &false, &1000, &10000); // Should panic with InsufficientLiquidity
}

#[test]
#[should_panic(expected = "Error(Contract, #3)")]
fn test_swap_slippage_exceeded() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit
    token_a_admin.mint(&user, &1000);
    token_b_admin.mint(&user, &1000);
    client.deposit(&user, &1000, &1000);

    // Try to swap with very low slippage tolerance
    client.swap(&user, &false, &100, &1); // Should panic with SlippageExceeded
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_withdraw_insufficient_shares() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit
    token_a_admin.mint(&user, &1000);
    token_b_admin.mint(&user, &1000);
    client.deposit(&user, &1000, &1000);

    // Try to withdraw more than owned
    client.withdraw(&user, &2000); // Should panic with InsufficientShares
}

#[test]
fn test_token_interface() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user1 = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Test token metadata
    assert_eq!(client.name(), String::from_str(&e, "Liquidity Pool Share"));
    assert_eq!(client.symbol(), String::from_str(&e, "LPS"));
    assert_eq!(client.decimals(), 7);

    // Initially no shares
    assert_eq!(client.total_supply(), 0);
    assert_eq!(client.balance(&user1), 0);

    // Mint and deposit
    token_a_admin.mint(&user1, &1000);
    token_b_admin.mint(&user1, &1000);
    let _shares = client.deposit(&user1, &1000, &1000);

    // Check balances
    assert_eq!(client.total_supply(), _shares);
    assert_eq!(client.balance(&user1), _shares);
}

#[test]
fn test_transfer() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user1 = Address::generate(&e);
    let user2 = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit
    token_a_admin.mint(&user1, &1000);
    token_b_admin.mint(&user1, &1000);
    let shares = client.deposit(&user1, &1000, &1000);

    // Transfer shares from user1 to user2
    client.transfer(&user1, &user2, &500);

    // Check balances
    assert_eq!(client.balance(&user1), shares - 500);
    assert_eq!(client.balance(&user2), 500);
    assert_eq!(client.total_supply(), shares); // Total supply unchanged
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_transfer_insufficient_balance() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user1 = Address::generate(&e);
    let user2 = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit
    token_a_admin.mint(&user1, &1000);
    token_b_admin.mint(&user1, &1000);
    client.deposit(&user1, &1000, &1000);

    // Try to transfer more than owned
    client.transfer(&user1, &user2, &2000); // Should panic with InsufficientBalance
}

#[test]
fn test_events() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user1 = Address::generate(&e);
    let user2 = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint tokens to users
    token_a_admin.mint(&user1, &2000);
    token_b_admin.mint(&user1, &2000);
    token_a_admin.mint(&user2, &1000);
    token_b_admin.mint(&user2, &1000);

    // === Test Deposit Event ===
    let deposit_shares = client.deposit(&user1, &1000, &1000);

    let events = e.events().all();
    let deposit_event_name = String::from_str(&e, "deposit");
    let deposit_events: Vec<_> = events
        .iter()
        .filter(|(_, topics, _)| {
            if topics.len() != 2 {
                return false;
            }
            // Compare by converting Val to String
            let topic_str: Result<SorobanString, _> = topics.get(0).unwrap().try_into_val(&e);
            topic_str.is_ok() && topic_str.unwrap() == deposit_event_name
        })
        .collect();

    // Should have exactly one deposit event
    assert_eq!(deposit_events.len(), 1);

    // Verify deposit event data
    let (contract_addr, topics, data) = &deposit_events[0];
    assert_eq!(
        contract_addr, &contract_id,
        "Event should be emitted from liquidity pool contract"
    );

    // Convert topic Val to Address for comparison
    let topic_user: Address = topics.get(1).unwrap().try_into_val(&e).unwrap();
    assert_eq!(
        topic_user, user1,
        "Deposit event should contain user1 address in topics"
    );

    // Convert data Val to DepositEvent
    let deposit_event: DepositEvent = data.try_into_val(&e).unwrap();
    assert_eq!(deposit_event.user, user1);
    assert_eq!(deposit_event.amount_a, 1000);
    assert_eq!(deposit_event.amount_b, 1000);
    assert_eq!(deposit_event.shares_minted, deposit_shares);

    // === Test Swap Event ===
    let out_amount = 100;
    let in_max = 150;
    let amount_paid = client.swap(&user2, &false, &out_amount, &in_max);

    let events = e.events().all();
    let swap_event_name = String::from_str(&e, "swap");
    let swap_events: Vec<_> = events
        .iter()
        .filter(|(_, topics, _)| {
            if topics.len() != 2 {
                return false;
            }
            // Compare by converting Val to String
            let topic_str: Result<SorobanString, _> = topics.get(0).unwrap().try_into_val(&e);
            topic_str.is_ok() && topic_str.unwrap() == swap_event_name
        })
        .collect();

    // Should have exactly one swap event
    assert_eq!(swap_events.len(), 1);

    // Verify swap event data
    let (contract_addr, topics, data) = &swap_events[0];
    assert_eq!(
        contract_addr, &contract_id,
        "Event should be emitted from liquidity pool contract"
    );

    // Convert topic Val to Address for comparison
    let topic_user: Address = topics.get(1).unwrap().try_into_val(&e).unwrap();
    assert_eq!(
        topic_user, user2,
        "Swap event should contain user2 address in topics"
    );

    // Convert data Val to SwapEvent
    let swap_event: SwapEvent = data.try_into_val(&e).unwrap();
    assert_eq!(swap_event.user, user2);
    // buy_a = false means we're buying token B (token_out) by paying token A (token_in)
    assert_eq!(swap_event.token_in, token_a);
    assert_eq!(swap_event.token_out, token_b);
    assert_eq!(swap_event.amount_in, amount_paid);
    assert_eq!(swap_event.amount_out, out_amount);

    // === Test Withdraw Event ===
    let withdraw_shares = 500;
    let (withdrawn_a, withdrawn_b) = client.withdraw(&user1, &withdraw_shares);

    let events = e.events().all();
    let withdraw_event_name = String::from_str(&e, "withdraw");
    let withdraw_events: Vec<_> = events
        .iter()
        .filter(|(_, topics, _)| {
            if topics.len() != 2 {
                return false;
            }
            // Compare by converting Val to String
            let topic_str: Result<SorobanString, _> = topics.get(0).unwrap().try_into_val(&e);
            topic_str.is_ok() && topic_str.unwrap() == withdraw_event_name
        })
        .collect();

    // Should have exactly one withdraw event
    assert_eq!(withdraw_events.len(), 1);

    // Verify withdraw event data
    let (contract_addr, topics, data) = &withdraw_events[0];
    assert_eq!(
        contract_addr, &contract_id,
        "Event should be emitted from liquidity pool contract"
    );

    // Convert topic Val to Address for comparison
    let topic_user: Address = topics.get(1).unwrap().try_into_val(&e).unwrap();
    assert_eq!(
        topic_user, user1,
        "Withdraw event should contain user1 address in topics"
    );

    // Convert data Val to WithdrawEvent
    let withdraw_event: WithdrawEvent = data.try_into_val(&e).unwrap();
    assert_eq!(withdraw_event.user, user1);
    assert_eq!(withdraw_event.shares_burned, withdraw_shares);
    assert_eq!(withdraw_event.amount_a, withdrawn_a);
    assert_eq!(withdraw_event.amount_b, withdrawn_b);

    // Verify all event types are present
    assert!(!deposit_events.is_empty());
    assert!(!swap_events.is_empty());
    assert!(!withdraw_events.is_empty());
}

// ===== Allowance and TransferFrom Tests =====

#[test]
fn test_approve() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user1 = Address::generate(&e);
    let spender = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit to get shares
    token_a_admin.mint(&user1, &1000);
    token_b_admin.mint(&user1, &1000);
    let _shares = client.deposit(&user1, &1000, &1000);

    // Approve spender to use 500 shares
    let expiration_ledger = e.ledger().sequence() + 1000;
    client.approve(&user1, &spender, &500, &expiration_ledger);

    // Check allowance
    assert_eq!(client.allowance(&user1, &spender), 500);

    // Try to approve more - should overwrite
    client.approve(&user1, &spender, &300, &expiration_ledger);
    assert_eq!(client.allowance(&user1, &spender), 300);
}

#[test]
fn test_approve_expired() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user1 = Address::generate(&e);
    let spender = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit to get shares
    token_a_admin.mint(&user1, &1000);
    token_b_admin.mint(&user1, &1000);
    client.deposit(&user1, &1000, &1000);

    // Approve with short expiration
    let expiration_ledger = e.ledger().sequence() + 10;
    client.approve(&user1, &spender, &500, &expiration_ledger);

    // Advance ledger to expire allowance
    let mut ledger_info = e.ledger().get();
    ledger_info.sequence_number += 15;
    e.ledger().set(ledger_info);

    // Check that allowance is now 0 (expired)
    assert_eq!(client.allowance(&user1, &spender), 0);
}

#[test]
fn test_transfer_from() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user1 = Address::generate(&e);
    let user2 = Address::generate(&e);
    let spender = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit to get shares
    token_a_admin.mint(&user1, &1000);
    token_b_admin.mint(&user1, &1000);
    let shares = client.deposit(&user1, &1000, &1000);

    // Approve spender to use 500 shares
    let expiration_ledger = e.ledger().sequence() + 1000;
    client.approve(&user1, &spender, &500, &expiration_ledger);

    // Spender transfers 200 shares from user1 to user2
    client.transfer_from(&spender, &user1, &user2, &200);

    // Check balances
    assert_eq!(client.balance(&user1), shares - 200);
    assert_eq!(client.balance(&user2), 200);
    assert_eq!(client.allowance(&user1, &spender), 300); // 500 - 200 = 300 remaining

    // Spender transfers remaining 300 shares
    client.transfer_from(&spender, &user1, &user2, &300);

    // Check final balances
    assert_eq!(client.balance(&user1), shares - 500);
    assert_eq!(client.balance(&user2), 500);
    assert_eq!(client.allowance(&user1, &spender), 0); // Allowance depleted
}

#[test]
#[should_panic(expected = "Error(Contract, #10)")]
fn test_transfer_from_insufficient_allowance() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user1 = Address::generate(&e);
    let user2 = Address::generate(&e);
    let spender = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit to get shares
    token_a_admin.mint(&user1, &1000);
    token_b_admin.mint(&user1, &1000);
    client.deposit(&user1, &1000, &1000);

    // Approve only 100 shares
    let expiration_ledger = e.ledger().sequence() + 1000;
    client.approve(&user1, &spender, &100, &expiration_ledger);

    // Try to transfer 200 shares (more than approved)
    client.transfer_from(&spender, &user1, &user2, &200); // Should panic with InsufficientBalance
}

#[test]
#[should_panic(expected = "Error(Contract, #6)")]
fn test_transfer_from_insufficient_balance() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user1 = Address::generate(&e);
    let user2 = Address::generate(&e);
    let spender = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit to get shares
    token_a_admin.mint(&user1, &1000);
    token_b_admin.mint(&user1, &1000);
    let shares = client.deposit(&user1, &1000, &1000);

    // Approve more shares than user has (should still fail on balance check)
    let expiration_ledger = e.ledger().sequence() + 1000;
    let allowance_amount = shares + 100;
    client.approve(&user1, &spender, &allowance_amount, &expiration_ledger);

    // Try to transfer more than user's balance
    let transfer_amount = shares + 50;
    client.transfer_from(&spender, &user1, &user2, &transfer_amount); // Should panic with InsufficientBalance
}

// ===== Pausable Functionality Tests =====

#[test]
fn test_pause_and_unpause() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint tokens
    token_a_admin.mint(&user, &1000);
    token_b_admin.mint(&user, &1000);

    // Deposit should work when not paused
    let shares = client.deposit(&user, &1000, &1000);
    assert_eq!(shares, 1000);

    // Admin pauses the contract
    client.set_paused(&true);

    // Unpause the contract
    client.set_paused(&false);

    // Operations should work again
    token_a_admin.mint(&user, &500);
    token_b_admin.mint(&user, &500);
    let more_shares = client.deposit(&user, &500, &500);
    assert!(more_shares > 0);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_deposit_when_paused() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint tokens
    token_a_admin.mint(&user, &1000);
    token_b_admin.mint(&user, &1000);

    // Pause the contract
    client.set_paused(&true);

    // Try to deposit - should panic with Paused error
    client.deposit(&user, &1000, &1000);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_swap_when_paused() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit liquidity first
    token_a_admin.mint(&user, &2000);
    token_b_admin.mint(&user, &2000);
    client.deposit(&user, &1000, &1000);

    // Pause the contract
    client.set_paused(&true);

    // Try to swap - should panic with Paused error
    client.swap(&user, &false, &100, &200);
}

#[test]
#[should_panic(expected = "Error(Contract, #9)")]
fn test_withdraw_when_paused() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit liquidity first
    token_a_admin.mint(&user, &1000);
    token_b_admin.mint(&user, &1000);
    let shares = client.deposit(&user, &1000, &1000);

    // Pause the contract
    client.set_paused(&true);

    // Try to withdraw - should panic with Paused error
    client.withdraw(&user, &shares);
}

// ===== Admin Fee Control Tests =====

#[test]
fn test_get_fee_default() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    client.initialize(&admin, &token_a, &token_b);

    // Default fee should be 30 bps
    assert_eq!(client.get_fee(), 30);
}

#[test]
fn test_set_fee_valid() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    client.initialize(&admin, &token_a, &token_b);

    // Admin updates fee to 10 bps
    client.set_fee(&10);
    assert_eq!(client.get_fee(), 10);

    // 0 bps (free swaps)
    client.set_fee(&0);
    assert_eq!(client.get_fee(), 0);

    // 100 bps (1%)
    client.set_fee(&100);
    assert_eq!(client.get_fee(), 100);
}

#[test]
#[should_panic(expected = "Error(Contract, #8)")]
fn test_set_fee_above_max() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    client.initialize(&admin, &token_a, &token_b);

    // 101 bps — should panic with InvalidFee
    client.set_fee(&101);
}

#[test]
fn test_burn() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint and deposit
    token_a_admin.mint(&user, &1000);
    token_b_admin.mint(&user, &1000);
    let _shares = client.deposit(&user, &1000, &1000);

    let supply_before = client.total_supply();
    let balance_before = client.balance(&user);

    // Burn 400 shares
    client.burn(&user, &400);

    let supply_after = client.total_supply();
    let balance_after = client.balance(&user);

    assert_eq!(supply_before - 400, supply_after);
    assert_eq!(balance_before - 400, balance_after);
}

#[test]
#[should_panic(expected = "Error(Contract, #4)")]
fn test_burn_insufficient_shares() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    token_a_admin.mint(&user, &1000);
    token_b_admin.mint(&user, &1000);
    client.deposit(&user, &1000, &1000);

    // Try to burn more than user has
    client.burn(&user, &2000);
}

// ===== Zero-Value Edge Case Tests =====

#[test]
fn test_deposit_zero_amount() {
    let e = Env::default();
    e.mock_all_auths();

    let contract_id = e.register(LiquidityPool, ());
    let client = LiquidityPoolClient::new(&e, &contract_id);

    let admin = Address::generate(&e);
    let token_a = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();
    let token_b = e
        .register_stellar_asset_contract_v2(admin.clone())
        .address();

    let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
    let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

    let user = Address::generate(&e);

    e.cost_estimate().budget().reset_unlimited();

    client.initialize(&admin, &token_a, &token_b);

    // Mint tokens so the user has balance for subsequent tests
    token_a_admin.mint(&user, &10000);
    token_b_admin.mint(&user, &10000);

    // --- Scenario 1: First deposit with both amounts = 0 ---
    // sqrt(0 * 0) = 0, so 0 shares should be minted without panicking.
    let shares = client.deposit(&user, &0, &0);
    assert_eq!(
        shares, 0,
        "Depositing (0, 0) as first liquidity must mint 0 shares"
    );
    assert_eq!(
        client.total_supply(),
        0,
        "Total supply must remain 0 after zero deposit"
    );

    // --- Scenario 2: Seed the pool with real liquidity, then deposit zero ---
    let initial_shares = client.deposit(&user, &1000, &1000);
    assert_eq!(
        initial_shares, 1000,
        "Initial deposit should mint sqrt(1000*1000) = 1000 shares"
    );

    let token_a_client = soroban_sdk::token::Client::new(&e, &token_a);
    let token_b_client = soroban_sdk::token::Client::new(&e, &token_b);
    let balance_a_before = token_a_client.balance(&user);
    let balance_b_before = token_b_client.balance(&user);

    // Deposit 0 of each into a pool that already has reserves
    // Proportional formula: min(0 * total / reserve_a, 0 * total / reserve_b) = 0
    let zero_shares = client.deposit(&user, &0, &0);
    assert_eq!(
        zero_shares, 0,
        "Depositing (0, 0) into funded pool must mint 0 shares"
    );
    assert_eq!(
        client.total_supply(),
        initial_shares,
        "Total supply must be unchanged after zero deposit"
    );

    // Verify user token balances are unchanged (no tokens transferred)
    assert_eq!(
        token_a_client.balance(&user),
        balance_a_before,
        "Token A balance must be unchanged after zero deposit"
    );
    assert_eq!(
        token_b_client.balance(&user),
        balance_b_before,
        "Token B balance must be unchanged after zero deposit"
    );

    // --- Scenario 3: Only one amount is zero on initial-like deposit ---
    // Deposit with amount_a = 0 and amount_b > 0 into the funded pool
    // min(0 * total / reserve_a, amount_b * total / reserve_b) = 0
    let one_zero_shares = client.deposit(&user, &0, &500);
    assert_eq!(
        one_zero_shares, 0,
        "Depositing (0, 500) must mint 0 shares (limited by zero side)"
    );
}
