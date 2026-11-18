#![cfg(test)]

use crate::{
    compute_dynamic_fee, DynamicFeeConfig, DynamicFees, DynamicFeesClient, Error, FeeSignals,
};
use soroban_sdk::{testutils::Address as _, Address, Env};

fn default_config() -> DynamicFeeConfig {
    DynamicFeeConfig {
        base_fee_bps: 30,
        min_fee_bps: 5,
        max_fee_bps: 100,
        utilization_weight_bps: 40,
        volatility_weight_bps: 20,
        max_undercut_bps: 10,
        max_premium_bps: 15,
    }
}

#[test]
fn utilization_raises_fee() {
    let config = default_config();
    let low = FeeSignals {
        utilization_bps: 1_000,
        volatility_bps: 0,
        competitor_fee_bps: 0,
        competitor_enabled: false,
    };
    let high = FeeSignals {
        utilization_bps: 9_000,
        volatility_bps: 0,
        competitor_fee_bps: 0,
        competitor_enabled: false,
    };
    let q_low = compute_dynamic_fee(&config, &low).unwrap();
    let q_high = compute_dynamic_fee(&config, &high).unwrap();
    assert!(q_high.fee_bps > q_low.fee_bps);
}

#[test]
fn volatility_raises_fee() {
    let config = default_config();
    let calm = FeeSignals {
        utilization_bps: 0,
        volatility_bps: 500,
        competitor_fee_bps: 0,
        competitor_enabled: false,
    };
    let wild = FeeSignals {
        utilization_bps: 0,
        volatility_bps: 5_000,
        competitor_fee_bps: 0,
        competitor_enabled: false,
    };
    assert!(
        compute_dynamic_fee(&config, &wild).unwrap().fee_bps
            > compute_dynamic_fee(&config, &calm).unwrap().fee_bps
    );
}

#[test]
fn competitor_clamp_prevents_undercutting() {
    let config = default_config();
    // Raw fee would be base 30; competitor at 50 with max undercut 10 → floor 40
    let signals = FeeSignals {
        utilization_bps: 0,
        volatility_bps: 0,
        competitor_fee_bps: 50,
        competitor_enabled: true,
    };
    let q = compute_dynamic_fee(&config, &signals).unwrap();
    assert!(q.fee_bps >= 40);
}

#[test]
fn fee_clamped_to_max() {
    let mut config = default_config();
    config.max_fee_bps = 35;
    let signals = FeeSignals {
        utilization_bps: 10_000,
        volatility_bps: 10_000,
        competitor_fee_bps: 0,
        competitor_enabled: false,
    };
    let q = compute_dynamic_fee(&config, &signals).unwrap();
    assert_eq!(q.fee_bps, 35);
}

#[test]
fn contract_quote_matches_pure_function() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(DynamicFees, ());
    let client = DynamicFeesClient::new(&env, &id);
    let config = default_config();
    client.initialize(&admin, &config);

    let signals = FeeSignals {
        utilization_bps: 5_000,
        volatility_bps: 2_000,
        competitor_fee_bps: 0,
        competitor_enabled: false,
    };
    assert_eq!(
        client.quote(&signals),
        compute_dynamic_fee(&config, &signals).unwrap()
    );
}

#[test]
fn invalid_utilization_rejected() {
    let config = default_config();
    let signals = FeeSignals {
        utilization_bps: 10_001,
        volatility_bps: 0,
        competitor_fee_bps: 0,
        competitor_enabled: false,
    };
    assert_eq!(
        compute_dynamic_fee(&config, &signals),
        Err(Error::InvalidInput)
    );
}

// --- Bounded arithmetic (#867) ---

fn signals(util: u32, vol: u32) -> FeeSignals {
    FeeSignals {
        utilization_bps: util,
        volatility_bps: vol,
        competitor_fee_bps: 0,
        competitor_enabled: false,
    }
}

fn assert_bounded(config: &DynamicFeeConfig, s: &FeeSignals) {
    let q = compute_dynamic_fee(config, s).unwrap();
    assert!(
        q.fee_bps >= config.min_fee_bps && q.fee_bps <= config.max_fee_bps,
        "fee {} outside [{}, {}]",
        q.fee_bps,
        config.min_fee_bps,
        config.max_fee_bps
    );
}

#[test]
fn utilization_at_zero_and_full_range_boundaries() {
    let config = default_config();
    // 0% utilisation: only the base fee applies.
    let q = compute_dynamic_fee(&config, &signals(0, 0)).unwrap();
    assert_eq!(q.utilization_component, 0);
    assert_eq!(q.fee_bps, 30);
    // 100% utilisation adds exactly `utilization_weight_bps`.
    let q = compute_dynamic_fee(&config, &signals(10_000, 0)).unwrap();
    assert_eq!(q.utilization_component, 40);
    assert_eq!(q.fee_bps, 70);
    // One bp over the range is invalid input, not a silently larger fee.
    assert_eq!(
        compute_dynamic_fee(&config, &signals(10_001, 0)),
        Err(Error::InvalidInput)
    );
}

#[test]
fn volatility_at_zero_boundary_and_maximum_representable_value() {
    let config = default_config();
    let q = compute_dynamic_fee(&config, &signals(0, 0)).unwrap();
    assert_eq!(q.volatility_component, 0);
    // 100% volatility (10_000 bps) adds exactly `volatility_weight_bps`.
    let q = compute_dynamic_fee(&config, &signals(0, 10_000)).unwrap();
    assert_eq!(q.volatility_component, 20);
    assert_eq!(q.fee_bps, 50);
    // u32::MAX volatility must clamp to the configured maximum, not wrap.
    let q = compute_dynamic_fee(&config, &signals(0, u32::MAX)).unwrap();
    assert_eq!(q.fee_bps, config.max_fee_bps);
}

#[test]
fn extreme_weights_and_inputs_never_wrap_below_the_maximum() {
    // weight * input exceeds u32::MAX, so a truncating cast would produce a small
    // value. The result must saturate and then clamp to max_fee_bps.
    let config = DynamicFeeConfig {
        base_fee_bps: 30,
        min_fee_bps: 5,
        max_fee_bps: 100,
        utilization_weight_bps: u32::MAX,
        volatility_weight_bps: u32::MAX,
        max_undercut_bps: u32::MAX,
        max_premium_bps: u32::MAX,
    };
    let s = signals(10_000, u32::MAX);
    let q = compute_dynamic_fee(&config, &s).unwrap();
    assert_eq!(q.fee_bps, 100);
    assert!(q.utilization_component <= u32::MAX);
    assert_eq!(q.volatility_component, u32::MAX);

    // With a competitor quote at the extreme the result is still bounded.
    let with_comp = FeeSignals {
        competitor_fee_bps: u32::MAX,
        competitor_enabled: true,
        ..s
    };
    assert_bounded(&config, &with_comp);
}

#[test]
fn output_is_bounded_over_an_input_grid() {
    let config = default_config();
    let utils = [0, 1, 5_000, 9_999, 10_000];
    let vols = [0, 1, 1_500, 10_000, 1_000_000, u32::MAX];
    let comps = [0u32, 1, 50, 10_000, u32::MAX];
    for u in utils {
        for v in vols {
            assert_bounded(&config, &signals(u, v));
            for c in comps {
                assert_bounded(
                    &config,
                    &FeeSignals {
                        utilization_bps: u,
                        volatility_bps: v,
                        competitor_fee_bps: c,
                        competitor_enabled: true,
                    },
                );
            }
        }
    }
}

#[test]
fn competitor_floor_and_ceiling_boundaries() {
    let config = default_config(); // undercut 10, premium 15
    let mk = |comp: u32| FeeSignals {
        utilization_bps: 0,
        volatility_bps: 0,
        competitor_fee_bps: comp,
        competitor_enabled: true,
    };
    // base fee is 30. Competitor 40 -> floor 30: fee stays exactly at the floor.
    assert_eq!(compute_dynamic_fee(&config, &mk(40)).unwrap().fee_bps, 30);
    // Competitor 41 -> floor 31: lifted by one.
    assert_eq!(compute_dynamic_fee(&config, &mk(41)).unwrap().fee_bps, 31);
    // Competitor 15 -> ceiling 30: fee stays exactly at the ceiling.
    assert_eq!(compute_dynamic_fee(&config, &mk(15)).unwrap().fee_bps, 30);
    // Competitor 14 -> ceiling 29: pulled down by one.
    assert_eq!(compute_dynamic_fee(&config, &mk(14)).unwrap().fee_bps, 29);
}

#[test]
fn rounding_truncates_toward_zero() {
    // 40 * 1 / 10_000 = 0.004 -> 0; 40 * 2_500 / 10_000 = 10 exactly;
    // 40 * 2_499 / 10_000 = 9.996 -> 9.
    let config = default_config();
    assert_eq!(
        compute_dynamic_fee(&config, &signals(1, 0)).unwrap().utilization_component,
        0
    );
    assert_eq!(
        compute_dynamic_fee(&config, &signals(2_500, 0)).unwrap().utilization_component,
        10
    );
    assert_eq!(
        compute_dynamic_fee(&config, &signals(2_499, 0)).unwrap().utilization_component,
        9
    );
}

#[test]
fn invalid_configuration_is_rejected_at_initialize_and_set_config() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(DynamicFees, ());
    let client = DynamicFeesClient::new(&env, &id);

    let min_above_max = DynamicFeeConfig { min_fee_bps: 101, ..default_config() };
    let base_above_max = DynamicFeeConfig { base_fee_bps: 101, ..default_config() };
    let max_above_100pct = DynamicFeeConfig { max_fee_bps: 10_001, ..default_config() };
    for bad in [&min_above_max, &base_above_max, &max_above_100pct] {
        assert_eq!(client.try_initialize(&admin, bad), Err(Ok(Error::InvalidConfig)));
    }

    client.initialize(&admin, &default_config());
    for bad in [&min_above_max, &base_above_max, &max_above_100pct] {
        assert_eq!(client.try_set_config(&admin, bad), Err(Ok(Error::InvalidConfig)));
    }
    // The rejected updates left the stored configuration alone.
    assert_eq!(client.get_config(), default_config());
}

#[test]
fn boundary_configurations_are_accepted() {
    let env = Env::default();
    env.mock_all_auths();
    let admin = Address::generate(&env);
    let id = env.register(DynamicFees, ());
    let client = DynamicFeesClient::new(&env, &id);
    let edge = DynamicFeeConfig {
        base_fee_bps: 10_000,
        min_fee_bps: 10_000,
        max_fee_bps: 10_000,
        ..default_config()
    };
    client.initialize(&admin, &edge);
    // A degenerate min == max pins the fee at that value for any input.
    assert_eq!(client.quote(&signals(0, 0)).fee_bps, 10_000);
    assert_eq!(client.quote(&signals(10_000, u32::MAX)).fee_bps, 10_000);
}
