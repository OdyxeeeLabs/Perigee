/// Liquidity pool AMM invariant tests.
///
/// Verifies that:
/// - Total liquidity is always non-negative (sum of reserves ≥ 0).
/// - Token ratios remain positive (no reserve can go to zero or negative).
/// - No trade can cause the pool to enter an impossible state
///   (negative reserves, zero total supply, violated constant-product invariant).
/// - Slippage protection works correctly.
/// - LP share minting/burning is balanced.
///
/// Closes OdyxeeeLabs/Perigee#518

#![cfg(test)]

extern crate std;

use super::*;
use soroban_sdk::{
    testutils::{Address as _, Ledger},
    Address, Env, String,
};
use token_contract::{Token, TokenClient};

// ── Setup Helpers ─────────────────────────────────────────────────────────────

fn deploy_token(env: &Env, admin: &Address, name: &str, symbol: &str) -> (Address, TokenClient) {
    let id = env.register(Token, ());
    let client = TokenClient::new(env, &id);
    client.initialize(
        admin,
        &7,
        &String::from_str(env, name),
        &String::from_str(env, symbol),
    );
    (id, client)
}

fn setup_pool() -> (Env, Address, TokenClient, TokenClient, LiquidityPoolClient) {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let (token_a_id, token_a) = deploy_token(&env, &admin, "Token A", "TKA");
    let (token_b_id, token_b) = deploy_token(&env, &admin, "Token B", "TKB");

    let pool_id = env.register(LiquidityPool, ());
    let pool = LiquidityPoolClient::new(&env, &pool_id);

    // Initialize with 30 bps fee (0.3 %) — standard AMM fee.
    pool.initialize(&admin, &token_a_id, &token_b_id, &30u32);

    (env, pool_id, token_a, token_b, pool)
}

fn fund_and_deposit(
    env: &Env,
    admin: &Address,
    token_a: &TokenClient,
    token_b: &TokenClient,
    pool: &LiquidityPoolClient,
    provider: &Address,
    a_amount: i128,
    b_amount: i128,
) {
    token_a.mint(provider, &a_amount);
    token_b.mint(provider, &b_amount);
    pool.deposit(provider, &a_amount, &b_amount, &0, &0);
}

// ── Invariant 1: Total liquidity is always non-negative ───────────────────────

#[test]
fn total_liquidity_is_always_non_negative() {
    let (env, pool_id, token_a, token_b, pool) = setup_pool();
    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let trader = Address::generate(&env);

    fund_and_deposit(
        &env, &admin, &token_a, &token_b, &pool, &provider,
        1_000_000, 2_000_000,
    );

    // Execute a series of swaps
    token_a.mint(&trader, &500_000);
    for _ in 0..5 {
        let _ = pool.swap(
            &trader,
            &token_a.address,
            &token_b.address,
            &10_000,
            &0,
        );
    }

    // After swaps, reserves must still be positive.
    let (reserve_a, reserve_b) = pool.get_reserves();
    assert!(reserve_a > 0, "reserve_a must remain positive after swaps");
    assert!(reserve_b > 0, "reserve_b must remain positive after swaps");
    assert!(
        pool.total_shares() > 0,
        "total_shares must be positive while liquidity exists"
    );
}

// ── Invariant 2: Token ratios remain positive ─────────────────────────────────

/// No swap can drive either reserve to zero — the constant-product formula
/// ensures reserves are always strictly positive while total shares > 0.
#[test]
fn no_swap_can_drain_a_reserve_to_zero() {
    let (env, _pool_id, token_a, token_b, pool) = setup_pool();
    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let attacker = Address::generate(&env);

    fund_and_deposit(
        &env, &admin, &token_a, &token_b, &pool, &provider,
        1_000_000, 1_000_000,
    );

    // Try to swap the entire reserve_b out.
    token_a.mint(&attacker, &100_000_000);

    // Attempt an extremely large swap — should either fail with slippage or
    // succeed with the output capped by the constant-product invariant.
    let result = pool.try_swap(
        &attacker,
        &token_a.address,
        &token_b.address,
        &100_000_000, // far exceeds reserve_b
        &0,
    );

    let (reserve_a, reserve_b) = pool.get_reserves();
    // Regardless of whether the swap succeeded or was rejected, reserves ≥ 1.
    assert!(reserve_a >= 1, "reserve_a must not be drained to zero");
    assert!(reserve_b >= 1, "reserve_b must not be drained to zero");

    let _ = result; // silence unused warning
}

// ── Invariant 3: Constant-product invariant k = x * y ────────────────────────

/// k = reserve_a * reserve_b should only increase (due to fees) and never
/// decrease as a result of a swap.
#[test]
fn constant_product_invariant_never_decreases() {
    let (env, _pool_id, token_a, token_b, pool) = setup_pool();
    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let trader = Address::generate(&env);

    fund_and_deposit(
        &env, &admin, &token_a, &token_b, &pool, &provider,
        1_000_000, 1_000_000,
    );

    let (ra0, rb0) = pool.get_reserves();
    let k0 = ra0 as u128 * rb0 as u128;

    token_a.mint(&trader, &100_000);
    let _ = pool.swap(&trader, &token_a.address, &token_b.address, &100_000, &0);

    let (ra1, rb1) = pool.get_reserves();
    let k1 = ra1 as u128 * rb1 as u128;

    assert!(
        k1 >= k0,
        "k = ra * rb must not decrease after a swap (k0={k0}, k1={k1})"
    );
}

// ── Invariant 4: LP shares correctly track proportional ownership ─────────────

/// After deposit, the provider's shares / total_shares == their proportion
/// of the pool (approximately — integer math means exact equality may not hold
/// for tiny amounts, so we allow a 1-unit tolerance).
#[test]
fn lp_shares_track_proportional_ownership() {
    let (env, _pool_id, token_a, token_b, pool) = setup_pool();
    let admin = Address::generate(&env);
    let provider_1 = Address::generate(&env);
    let provider_2 = Address::generate(&env);

    // Provider 1 deposits first.
    fund_and_deposit(
        &env, &admin, &token_a, &token_b, &pool, &provider_1,
        1_000_000, 1_000_000,
    );
    let shares_1 = pool.balance(&provider_1);
    let total_after_1 = pool.total_shares();
    assert_eq!(shares_1, total_after_1, "provider_1 owns 100% of pool");

    // Provider 2 deposits equal amounts.
    fund_and_deposit(
        &env, &admin, &token_a, &token_b, &pool, &provider_2,
        1_000_000, 1_000_000,
    );
    let shares_2 = pool.balance(&provider_2);
    let total_after_2 = pool.total_shares();

    // Each provider should own ~50 % — allow rounding of ±1 share.
    assert!(
        (shares_1 as i128 - shares_2 as i128).abs() <= 1,
        "equal deposits should yield equal shares (±1): shares_1={shares_1}, shares_2={shares_2}"
    );
    assert_eq!(shares_1 + shares_2, total_after_2, "total shares must equal sum of individual shares");
}

// ── Invariant 5: Withdraw returns proportional tokens ────────────────────────

#[test]
fn withdraw_returns_correct_proportional_tokens() {
    let (env, _pool_id, token_a, token_b, pool) = setup_pool();
    let admin = Address::generate(&env);
    let provider = Address::generate(&env);

    token_a.mint(&provider, &2_000_000);
    token_b.mint(&provider, &2_000_000);
    pool.deposit(&provider, &2_000_000, &2_000_000, &0, &0);

    let shares = pool.balance(&provider);
    let (ra, rb) = pool.get_reserves();

    let a_before = token_a.balance(&provider);
    let b_before = token_b.balance(&provider);

    // Withdraw half the shares.
    pool.withdraw(&provider, &(shares / 2), &0, &0);

    let a_after = token_a.balance(&provider);
    let b_after = token_b.balance(&provider);
    let received_a = a_after - a_before;
    let received_b = b_after - b_before;

    // Should receive ~50 % of each reserve (±1 for rounding).
    assert!(
        (received_a - ra / 2).abs() <= 1,
        "received_a={received_a} should be ~ra/2={}", ra/2
    );
    assert!(
        (received_b - rb / 2).abs() <= 1,
        "received_b={received_b} should be ~rb/2={}", rb/2
    );
}

// ── Invariant 6: Slippage protection rejects bad trades ──────────────────────

#[test]
fn slippage_protection_rejects_high_impact_trades() {
    let (env, _pool_id, token_a, token_b, pool) = setup_pool();
    let admin = Address::generate(&env);
    let provider = Address::generate(&env);
    let trader = Address::generate(&env);

    fund_and_deposit(
        &env, &admin, &token_a, &token_b, &pool, &provider,
        1_000_000, 1_000_000,
    );

    // Swap with an unrealistically high minimum output to trigger slippage.
    token_a.mint(&trader, &100_000);
    let result = pool.try_swap(
        &trader,
        &token_a.address,
        &token_b.address,
        &100_000,
        &1_000_000, // min_out > possible output → must fail
    );

    assert!(result.is_err(), "high-slippage trade must be rejected");
}
