use crate::LiquidityPoolClient;
use proptest::prelude::*;
use soroban_sdk::{testutils::Address as _, Address, Env};

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]
    #[test]
    fn test_swap_invariant(
        reserve_a in 1_000i128..1_000_000_000_000_000_000i128,
        reserve_b in 1_000i128..1_000_000_000_000_000_000i128,
        amount_out in 1i128..1_000_000_000_000_000_000i128,
        buy_a in any::<bool>(),
    ) {
        let e = Env::default();
        e.mock_all_auths();
        e.cost_estimate().budget().reset_unlimited();

        // Derive a strictly smaller amount_out using modulo so we don't reject generated examples
        let max_out = if buy_a { reserve_a } else { reserve_b };
        let valid_amount_out = (amount_out % (max_out - 1)) + 1;

        let admin = Address::generate(&e);
        let token_a = e.register_stellar_asset_contract_v2(admin.clone()).address();
        let token_b = e.register_stellar_asset_contract_v2(admin.clone()).address();

        let contract_id = e.register(crate::LiquidityPool, ());
        let client = LiquidityPoolClient::new(&e, &contract_id);

        client.initialize(&admin, &token_a, &token_b);

        let user = Address::generate(&e);
        let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
        let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

        // Give the user enough to deposit
        token_a_admin.mint(&user, &reserve_a);
        token_b_admin.mint(&user, &reserve_b);
        client.deposit(&user, &reserve_a, &reserve_b);

        let k_before = reserve_a * reserve_b; // Fits in i128 if inputs are < 10^18

        // We need another user for swapping
        let swapper = Address::generate(&e);
        // We need to give the swapper an effectively infinite input amount
        let in_max = 50_000_000_000_000_000_000i128; // Give swapper 50 tokens of 10^18 precision
        token_a_admin.mint(&swapper, &in_max);
        token_b_admin.mint(&swapper, &in_max);

        // Perform the swap
        // A swap can fail due to slippage exceeded if in_max wasn't enough, which is an expected error.
        // But with a huge in_max it shouldn't fail.
        let res = client.try_swap(&swapper, &buy_a, &valid_amount_out, &in_max);

        if let Ok(Ok(_)) = res {
            // Verify invariant: the reserves increased K
            let token_a_client = soroban_sdk::token::Client::new(&e, &token_a);
            let token_b_client = soroban_sdk::token::Client::new(&e, &token_b);

            let pool_balance_a = token_a_client.balance(&contract_id);
            let pool_balance_b = token_b_client.balance(&contract_id);

            let k_after = pool_balance_a * pool_balance_b;

            assert!(k_after >= k_before, "Invariant violated! K before: {}, K after: {}", k_before, k_after);
        }
    }

    // ── Deposit / Withdraw round-trip ────────────────────────────────────

    /// After depositing and then fully withdrawing, the user should receive
    /// back amounts that are within rounding error of their original deposit.
    /// Total shares must return to zero.
    #[test]
    fn fuzz_deposit_withdraw_round_trip(
        amount_a in 1_000i128..1_000_000_000_000i128,
        amount_b in 1_000i128..1_000_000_000_000i128,
    ) {
        let e = Env::default();
        e.mock_all_auths();
        e.cost_estimate().budget().reset_unlimited();

        let admin = Address::generate(&e);
        let token_a = e.register_stellar_asset_contract_v2(admin.clone()).address();
        let token_b = e.register_stellar_asset_contract_v2(admin.clone()).address();

        let contract_id = e.register(crate::LiquidityPool, ());
        let client = LiquidityPoolClient::new(&e, &contract_id);

        client.initialize(&admin, &token_a, &token_b);

        let user = Address::generate(&e);
        let token_a_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_a);
        let token_b_admin = soroban_sdk::token::StellarAssetClient::new(&e, &token_b);

        token_a_admin.mint(&user, &amount_a);
        token_b_admin.mint(&user, &amount_b);

        // Deposit
        let shares = client.deposit(&user, &amount_a, &amount_b);
        prop_assert!(shares > 0, "Should mint positive shares for positive deposits");

        // Withdraw all shares
        let (withdrawn_a, withdrawn_b) = client.withdraw(&user, &shares);

        // Round-trip invariant: withdrawn amounts ≤ deposited amounts
        // (some rounding loss is acceptable with integer sqrt)
        prop_assert!(
            withdrawn_a <= amount_a,
            "Withdrew more token A ({}) than deposited ({})",
            withdrawn_a, amount_a
        );
        prop_assert!(
            withdrawn_b <= amount_b,
            "Withdrew more token B ({}) than deposited ({})",
            withdrawn_b, amount_b
        );

        // After single-user full withdraw, pool should be effectively empty
        let token_a_client = soroban_sdk::token::Client::new(&e, &token_a);
        let token_b_client = soroban_sdk::token::Client::new(&e, &token_b);
        let remaining_a = token_a_client.balance(&contract_id);
        let remaining_b = token_b_client.balance(&contract_id);

        // Rounding loss should be minimal (at most 1 unit per token)
        prop_assert!(
            remaining_a <= 1,
            "Pool has {} leftover token A after full withdrawal",
            remaining_a
        );
        prop_assert!(
            remaining_b <= 1,
            "Pool has {} leftover token B after full withdrawal",
            remaining_b
        );
    }

    // ── Deposit share proportionality ────────────────────────────────────

    /// Two deposits of identical amounts into an empty pool should produce
    /// identical shares. Verifies the share calculation is deterministic.
    #[test]
    fn fuzz_deposit_share_proportionality(
        amount_a in 1_000i128..1_000_000_000_000i128,
        amount_b in 1_000i128..1_000_000_000_000i128,
    ) {
        // Test 1: determinism — same inputs → same shares
        let e1 = Env::default();
        e1.mock_all_auths();
        e1.cost_estimate().budget().reset_unlimited();

        let admin1 = Address::generate(&e1);
        let token_a1 = e1.register_stellar_asset_contract_v2(admin1.clone()).address();
        let token_b1 = e1.register_stellar_asset_contract_v2(admin1.clone()).address();

        let contract_id1 = e1.register(crate::LiquidityPool, ());
        let client1 = LiquidityPoolClient::new(&e1, &contract_id1);
        client1.initialize(&admin1, &token_a1, &token_b1);

        let user1 = Address::generate(&e1);
        soroban_sdk::token::StellarAssetClient::new(&e1, &token_a1).mint(&user1, &amount_a);
        soroban_sdk::token::StellarAssetClient::new(&e1, &token_b1).mint(&user1, &amount_b);
        let shares1 = client1.deposit(&user1, &amount_a, &amount_b);

        // Repeat in a fresh env
        let e2 = Env::default();
        e2.mock_all_auths();
        e2.cost_estimate().budget().reset_unlimited();

        let admin2 = Address::generate(&e2);
        let token_a2 = e2.register_stellar_asset_contract_v2(admin2.clone()).address();
        let token_b2 = e2.register_stellar_asset_contract_v2(admin2.clone()).address();

        let contract_id2 = e2.register(crate::LiquidityPool, ());
        let client2 = LiquidityPoolClient::new(&e2, &contract_id2);
        client2.initialize(&admin2, &token_a2, &token_b2);

        let user2 = Address::generate(&e2);
        soroban_sdk::token::StellarAssetClient::new(&e2, &token_a2).mint(&user2, &amount_a);
        soroban_sdk::token::StellarAssetClient::new(&e2, &token_b2).mint(&user2, &amount_b);
        let shares2 = client2.deposit(&user2, &amount_a, &amount_b);

        prop_assert_eq!(
            shares1, shares2,
            "Same deposits produced different shares: {} vs {}",
            shares1, shares2
        );
    }
}
