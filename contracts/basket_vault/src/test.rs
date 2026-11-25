#![cfg(test)]

//! Unit tests for the basket vault.
//!
//! Three themes, matching #637: composite share pricing, weight discipline, and
//! rebalancing behind a deadband.
//!
//! Closes OdyxeeeLabs/Perigee#637

use crate::{
    BasketConfig, BasketVault, BasketVaultClient, Error, MAX_BASKET_ASSETS, BPS_DENOMINATOR,
};
use soroban_sdk::{testutils::Address as _, vec, Address, Env};

/// Build a `soroban_sdk::Vec<i128>` of target weights for the rebalance tests.
fn weights(env: &Env, values: &[i128]) -> soroban_sdk::Vec<i128> {
    vec::Vec::from_array(env, values)
}

/// Build a two-component basket with weights 5000 / 5000 and prices 2 / 8, so the
/// composite price is exactly 5 and each component is worth half.
macro_rules! setup {
    () => {{
        let env = Env::default();
        env.mock_all_auths();

        let admin = Address::generate(&env);
        let manager = Address::generate(&env);
        let asset_a = Address::generate(&env);
        let asset_b = Address::generate(&env);

        let contract_id = env.register(BasketVault, ());
        let client = BasketVaultClient::new(&env, &contract_id);
        client.initialize(
            &admin,
            &BasketConfig {
                base_bps: BPS_DENOMINATOR,
                max_weight_bps: 8_000,
                drift_tolerance_bps: 100,
                max_assets: 8,
            },
        );

        let basket = client.create_basket(&manager);
        client.add_component(&manager, &basket, &asset_a, &5_000, &2);
        client.add_component(&manager, &basket, &asset_b, &5_000, &8);

        (env, contract_id, client, admin, manager, basket, asset_a, asset_b)
    }};
}

// ── Composite pricing ────────────────────────────────────────────────────────

#[test]
fn the_composite_price_is_the_weighted_sum() {
    let (env, _cid, client, _admin, _manager, basket, _a, _b) = setup!();

    // 5000/10000 × 2 + 5000/10000 × 8 = 1 + 4 = 5
    assert_eq!(client.price_share(&basket), 5);
}

#[test]
fn the_composite_price_moves_with_its_components() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();

    client.set_price(&manager, &basket, &1, &10);

    // 5000/10000 × 10 + 5000/10000 × 8 = 5 + 4 = 9
    assert_eq!(client.price_share(&basket), 9);
}

#[test]
fn share_valuation_and_withdrawals_follow_rebalanced_weights_and_prices() {
    let (env, _cid, client, _admin, manager, basket, asset_a, asset_b) = setup!();

    client.set_price(&manager, &basket, &0, &10);
    client.set_price(&manager, &basket, &1, &30);
    client.record_holding(&manager, &basket, &1_000);
    client.rebalance_to(&manager, &basket, &weights(&env, &[7_000, 3_000]));

    // 70% of 10 plus 30% of 30 is 16; five shares are worth 80.
    assert_eq!(client.price_share(&basket), 16);
    assert_eq!(client.price_shares(&basket, &5), 80);

    // A 1,000-value withdrawal allocates 700 and 300 of value, then converts
    // those values into 70 units of A and 10 units of B.
    let payout = client.components_for(&basket, &1_000);
    assert_eq!(payout.get(0), Some((asset_a, 70)));
    assert_eq!(payout.get(1), Some((asset_b, 10)));
}

#[test]
fn a_share_is_worth_its_price() {
    let (env, _cid, client, _admin, _manager, basket, _a, _b) = setup!();

    assert_eq!(client.price_shares(&basket, &10), 50);
    assert_eq!(client.price_shares(&basket, &1), 5);
}

#[test]
fn an_empty_basket_has_no_price() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let manager = Address::generate(&env);
    let contract_id = env.register(BasketVault, ());
    let client = BasketVaultClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &BasketConfig {
            base_bps: BPS_DENOMINATOR,
            max_weight_bps: 8_000,
            drift_tolerance_bps: 100,
            max_assets: 8,
        },
    );

    let empty = client.create_basket(&manager);
    assert_eq!(client.try_price_share(&empty), Err(Ok(Error::EmptyBasket)));
    assert_eq!(client.try_total_weight(&empty), Err(Ok(Error::EmptyBasket)));
}

#[test]
fn components_for_returns_one_leg_per_component() {
    let (env, _cid, client, _admin, _manager, basket, asset_a, asset_b) = setup!();

    let legs = client.components_for(&basket, &1_000);

    assert_eq!(legs.len(), 2);
    assert_eq!(legs.get(0), Some((asset_a, 250)));
    assert_eq!(legs.get(1), Some((asset_b, 62)));
}

#[test]
fn components_for_never_over_commits_the_basket() {
    let (_env, _cid, client, _admin, _manager, basket, _a, _b) = setup!();

    let legs = client.components_for(&basket, &999);
    let total_value = legs.get(0).unwrap().1 * 2 + legs.get(1).unwrap().1 * 8;

    assert!(
        total_value <= 999,
        "payout value was {total_value}, more than requested"
    );
}

#[test]
fn an_incompletely_weighted_basket_cannot_be_valued_or_withdrawn() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let manager = Address::generate(&env);
    let contract_id = env.register(BasketVault, ());
    let client = BasketVaultClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &BasketConfig {
            base_bps: BPS_DENOMINATOR,
            max_weight_bps: 8_000,
            drift_tolerance_bps: 100,
            max_assets: 8,
        },
    );
    let basket = client.create_basket(&manager);
    client.add_component(&manager, &basket, &Address::generate(&env), &5_000, &2);

    assert_eq!(
        client.try_price_share(&basket),
        Err(Ok(Error::WeightsDoNotSum))
    );
    assert_eq!(
        client.try_price_shares(&basket, &1),
        Err(Ok(Error::WeightsDoNotSum))
    );
    assert_eq!(
        client.try_components_for(&basket, &1_000),
        Err(Ok(Error::WeightsDoNotSum))
    );
}

#[test]
fn a_zero_price_update_is_rejected_without_changing_share_value() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();

    assert_eq!(
        client.try_set_price(&manager, &basket, &0, &0),
        Err(Ok(Error::ZeroAmount))
    );
    assert_eq!(client.price_share(&basket), 5);
}

// ── Weight discipline ────────────────────────────────────────────────────────

#[test]
fn total_weight_is_ten_thousand_bps() {
    let (env, _cid, client, _admin, _manager, basket, _a, _b) = setup!();

    assert_eq!(client.total_weight(&basket), BPS_DENOMINATOR);
}

#[test]
fn a_component_above_the_ceiling_is_refused() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();

    assert_eq!(
        client.try_add_component(&manager, &basket, &Address::generate(&env), &9_000, &1),
        Err(Ok(Error::WeightTooLarge))
    );
    assert_eq!(client.component_count(&basket), 2);
}

#[test]
fn the_same_asset_cannot_be_added_twice() {
    let (env, _cid, client, _admin, manager, basket, asset_a, _b) = setup!();

    assert_eq!(
        client.try_add_component(&manager, &basket, &asset_a, &1_000, &1),
        Err(Ok(Error::AssetAlreadyInBasket))
    );
}

#[test]
fn a_non_positive_price_or_weight_is_refused() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();
    let fresh = Address::generate(&env);

    assert_eq!(
        client.try_add_component(&manager, &basket, &fresh, &0, &1),
        Err(Ok(Error::ZeroAmount))
    );
    assert_eq!(
        client.try_add_component(&manager, &basket, &fresh, &1_000, &0),
        Err(Ok(Error::ZeroAmount))
    );
}

#[test]
fn only_the_manager_may_amend_a_basket() {
    let (env, _cid, client, _admin, _manager, basket, _a, _b) = setup!();
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_add_component(&stranger, &basket, &Address::generate(&env), &1_000, &1),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.try_set_price(&stranger, &basket, &0, &1),
        Err(Ok(Error::Unauthorized))
    );
}

// ── Drift and the deadband ───────────────────────────────────────────────────

#[test]
fn a_balanced_basket_has_no_drift() {
    let (env, _cid, client, _admin, _manager, basket, _a, _b) = setup!();

    assert_eq!(client.drift(&basket, &0), 0);
    assert_eq!(client.drift(&basket, &1), 0);
    assert!(!client.needs_rebalance(&basket).unwrap());
}

#[test]
fn a_price_move_shows_up_as_drift() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();

    client.set_price(&manager, &basket, &0, &20);

    // Component 0 is now worth 5000/10000 × 20 = 10 of a composite price of
    // 10 + 4 = 14, so it is overweight.
    assert!(client.drift(&basket, &0) > 0);
    assert!(client.drift(&basket, &1) < 0);
    assert!(client.needs_rebalance(&basket).unwrap());
}

#[test]
fn drift_inside_the_deadband_does_not_ask_for_a_rebalance() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();

    // A small move, well inside the 100 bps tolerance.
    client.set_price(&manager, &basket, &0, &2);
    assert!(!client.needs_rebalance(&basket).unwrap());
}

// ── Rebalancing ──────────────────────────────────────────────────────────────

#[test]
fn a_rebalance_must_supply_one_weight_per_component() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();
    client.record_holding(&manager, &basket, &1_000);

    let bad = client.try_rebalance_to(&manager, &basket, &weights(&env, &[6_000]));
    assert_eq!(bad, Err(Ok(Error::WeightsDoNotSum)));
}

#[test]
fn weights_that_do_not_sum_to_one_hundred_percent_are_refused() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();
    client.record_holding(&manager, &basket, &1_000);

    assert_eq!(
        client.try_rebalance_to(&manager, &basket, &weights(&env, &[6_000, 3_000])),
        Err(Ok(Error::WeightsDoNotSum))
    );
}

#[test]
fn a_new_weight_above_the_ceiling_is_refused() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();
    client.record_holding(&manager, &basket, &1_000);

    // Sums to 10 000 but component 0 is above the 8 000 ceiling.
    assert_eq!(
        client.try_rebalance_to(&manager, &basket, &weights(&env, &[9_000, 1_000])),
        Err(Ok(Error::WeightTooLarge))
    );
}

#[test]
fn a_rebalance_of_an_empty_basket_is_refused() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let manager = Address::generate(&env);
    let contract_id = env.register(BasketVault, ());
    let client = BasketVaultClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &BasketConfig {
            base_bps: BPS_DENOMINATOR,
            max_weight_bps: 8_000,
            drift_tolerance_bps: 100,
            max_assets: 8,
        },
    );

    let basket = client.create_basket(&manager);
    assert_eq!(
        client.try_rebalance_to(&manager, &basket, &weights(&env, &[5_000, 5_000])),
        Err(Ok(Error::EmptyBasket))
    );
}

#[test]
fn a_rebalance_without_any_holding_is_refused() {
    // The settlement check compares the movement against what the basket holds;
    // with nothing held there is nothing to move.
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();

    assert_eq!(
        client.try_rebalance_to(&manager, &basket, &weights(&env, &[5_000, 5_000])),
        Err(Ok(Error::InsufficientBasketBalance))
    );
}

#[test]
fn a_rebalance_updates_the_weights_and_counts() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();
    client.record_holding(&manager, &basket, &1_000);

    client.rebalance_to(&manager, &basket, &weights(&env, &[7_000, 3_000]));

    assert_eq!(client.get_component(&basket, &0).weight_bps, 7_000);
    assert_eq!(client.get_component(&basket, &1).weight_bps, 3_000);
    assert_eq!(client.get_basket(&basket).rebalance_count, 1);
    assert_eq!(client.total_weight(&basket), BPS_DENOMINATOR);
}

#[test]
fn rebalancing_counts_accumulate() {
    let (env, _cid, client, _admin, manager, basket, _a, _b) = setup!();
    client.record_holding(&manager, &basket, &1_000);

    client.rebalance_to(&manager, &basket, &weights(&env, &[6_000, 4_000]));
    client.rebalance_to(&manager, &basket, &weights(&env, &[5_000, 5_000]));

    assert_eq!(client.get_basket(&basket).rebalance_count, 2);
}

#[test]
fn only_the_manager_may_rebalance() {
    let (env, _cid, client, _admin, _manager, basket, _a, _b) = setup!();
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_rebalance_to(&stranger, &basket, &weights(&env, &[5_000, 5_000])),
        Err(Ok(Error::Unauthorized))
    );
}

// ── Registry and administration ──────────────────────────────────────────────

#[test]
fn an_unknown_basket_is_reported() {
    let (env, _cid, client, _admin, _manager, _basket, _a, _b) = setup!();

    assert_eq!(
        client.try_get_basket(&999),
        Err(Ok(Error::BasketNotFound))
    );
}

#[test]
fn a_basket_is_capped_by_max_assets() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let manager = Address::generate(&env);
    let contract_id = env.register(BasketVault, ());
    let client = BasketVaultClient::new(&env, &contract_id);
    client.initialize(
        &admin,
        &BasketConfig {
            base_bps: BPS_DENOMINATOR,
            max_weight_bps: 10_000,
            drift_tolerance_bps: 100,
            max_assets: 2,
        },
    );

    let basket = client.create_basket(&manager);
    client.add_component(&manager, &basket, &Address::generate(&env), &3_333, &1);
    client.add_component(&manager, &basket, &Address::generate(&env), &3_333, &1);

    assert_eq!(
        client.try_add_component(
            &manager,
            &basket,
            &Address::generate(&env),
            &3_334,
            &1
        ),
        Err(Ok(Error::BasketFull))
    );
    assert_eq!(client.component_count(&basket), 2);
}

#[test]
fn only_the_admin_may_change_the_global_config() {
    let (env, _cid, client, _admin, _manager, _basket, _a, _b) = setup!();
    let stranger = Address::generate(&env);

    assert_eq!(
        client.try_set_global_config(&stranger, &BasketConfig {
            base_bps: BPS_DENOMINATOR,
            max_weight_bps: 10_000,
            drift_tolerance_bps: 1_000,
            max_assets: 8,
        }),
        Err(Ok(Error::Unauthorized))
    );
}

#[test]
fn a_config_with_too_many_assets_is_refused() {
    let (env, _cid, client, admin, _manager, _basket, _a, _b) = setup!();

    assert_eq!(
        client.try_set_global_config(&admin, &BasketConfig {
            base_bps: BPS_DENOMINATOR,
            max_weight_bps: 10_000,
            drift_tolerance_bps: 100,
            max_assets: MAX_BASKET_ASSETS + 1,
        }),
        Err(Ok(Error::TooManyAssets))
    );
}

#[test]
fn components_lists_every_component() {
    let (env, _cid, client, _admin, _manager, basket, asset_a, asset_b) = setup!();

    let list = client.components(&basket);

    assert_eq!(list.len(), 2);
    assert_eq!(list.get(0).unwrap().asset, asset_a);
    assert_eq!(list.get(1).unwrap().asset, asset_b);
}

// ── #849 — initialization state ─────────────────────────────────────────────

#[test]
fn initialization_sets_admin_and_global_config_once() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let config = BasketConfig {
        base_bps: BPS_DENOMINATOR,
        max_weight_bps: 8_000,
        drift_tolerance_bps: 100,
        max_assets: 8,
    };

    let contract_id = env.register(BasketVault, ());
    let client = BasketVaultClient::new(&env, &contract_id);

    assert_eq!(client.try_admin(), Err(Ok(Error::NotInitialized)));

    client.initialize(&admin, &config);

    assert_eq!(client.admin(), admin);
    assert_eq!(client.global_config(), config);
    assert_eq!(
        client.try_initialize(&admin, &config),
        Err(Ok(Error::AlreadyInitialized))
    );
}

#[test]
fn calls_before_initialization_are_rejected_without_touching_state() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let manager = Address::generate(&env);
    let config = BasketConfig {
        base_bps: BPS_DENOMINATOR,
        max_weight_bps: 8_000,
        drift_tolerance_bps: 100,
        max_assets: 8,
    };

    let contract_id = env.register(BasketVault, ());
    let client = BasketVaultClient::new(&env, &contract_id);

    assert_eq!(client.try_admin(), Err(Ok(Error::NotInitialized)));
    assert_eq!(client.try_global_config(), Err(Ok(Error::NotInitialized)));
    assert_eq!(
        client.try_create_basket(&manager),
        Err(Ok(Error::NotInitialized))
    );
    assert_eq!(
        client.try_set_global_config(&admin, &config),
        Err(Ok(Error::NotInitialized))
    );
}

#[test]
fn only_the_initialized_admin_may_change_global_config() {
    let env = Env::default();
    env.mock_all_auths();

    let admin = Address::generate(&env);
    let stranger = Address::generate(&env);
    let config = BasketConfig {
        base_bps: BPS_DENOMINATOR,
        max_weight_bps: 8_000,
        drift_tolerance_bps: 100,
        max_assets: 8,
    };

    let contract_id = env.register(BasketVault, ());
    let client = BasketVaultClient::new(&env, &contract_id);
    client.initialize(&admin, &config);

    let changed = BasketConfig {
        max_weight_bps: 7_000,
        ..config.clone()
    };
    assert_eq!(
        client.try_set_global_config(&stranger, &changed),
        Err(Ok(Error::Unauthorized))
    );
    assert_eq!(
        client.global_config(),
        config,
        "rejected write must not change config"
    );
}